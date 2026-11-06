//! Bounded host dispatch. No active grant means no GitHub request or source read.
use super::*;
use std::sync::Arc;
#[derive(Debug, PartialEq, Eq)]
pub enum DispatchOutcome {
    Inactive,
    Idle,
    Analyzed,
    Delivered,
    Deferred,
}
pub struct AppDispatcher {
    queue: Arc<AppExecutionQueue>,
    inbox: Arc<AppWebhookInbox>,
    broker: Arc<GitHubInstallationBroker>,
    worker: Arc<dyn AppPrWorker>,
    reports: Arc<AppReportStore>,
    readback: Arc<dyn AppReportReadback>,
    signer: Arc<AppReceiptSigner>,
    origin: String,
    admission: Mutex<()>,
}
pub struct AppDispatchConfig {
    pub queue: Arc<AppExecutionQueue>,
    pub inbox: Arc<AppWebhookInbox>,
    pub broker: Arc<GitHubInstallationBroker>,
    pub worker: Arc<dyn AppPrWorker>,
    pub reports: Arc<AppReportStore>,
    pub readback: Arc<dyn AppReportReadback>,
    pub signer: Arc<AppReceiptSigner>,
    pub report_origin: String,
}
impl AppDispatcher {
    pub fn new(config: AppDispatchConfig) -> Result<Self> {
        SetupConfig::new(1, "dispatch".into(), config.report_origin.clone())?;
        Ok(Self {
            queue: config.queue,
            inbox: config.inbox,
            broker: config.broker,
            worker: config.worker,
            reports: config.reports,
            readback: config.readback,
            signer: config.signer,
            origin: config.report_origin,
            admission: Mutex::new(()),
        })
    }
    /// One notification and at most one work/delivery item. Caller provides the
    /// periodic timer; finite methods keep shutdown between passes predictable.
    pub fn tick(&self) -> Result<DispatchOutcome> {
        let _guard = self
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        if !self.queue.has_enabled_repository()? {
            return Ok(DispatchOutcome::Inactive);
        }
        if let Some(n) = self.inbox.claim()? {
            if let (Some(repository), Some(_)) = (n.repository_id, n.pr_number) {
                match self
                    .queue
                    .installation_policy(n.installation_id, repository)
                {
                    Ok(policy) => {
                        let session = self.broker.session(n.installation_id, &policy.repository)?;
                        match self.queue.reconcile_notification(&n, &session) {
                            Ok(_) => {}
                            Err(SetupError::Invalid) => {
                                self.inbox.finish(&n)?;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    Err(SetupError::Denied) => {
                        let _ = self.inbox.finish(&n);
                    }
                    Err(e) => return Err(e),
                }
            } else {
                let _ = self.inbox.finish(&n);
            }
        }
        if let Some(id) = self.queue.pending_delivery()?.first() {
            let result = self.deliver(id);
            if result.is_err() {
                self.queue.defer_delivery(id)?;
                return Ok(DispatchOutcome::Deferred);
            }
            return Ok(DispatchOutcome::Delivered);
        }
        let Some(lease) = self.queue.claim()? else {
            return Ok(DispatchOutcome::Idle);
        };
        let result = (|| {
            let (policy, context) = self.queue.work_configuration(&lease)?;
            if self.worker.engine_digest() != lease.input.engine_digest {
                return Err(SetupError::Denied);
            }
            let session = self
                .broker
                .session(lease.input.installation_id, &policy.repository)?;
            if session.current_pr(
                lease.input.installation_id,
                lease.input.repository_id,
                lease.input.comparison.number,
            )? != lease.input.comparison
            {
                return Err(SetupError::Conflict);
            }
            let request = session.fetch_pr_objects(&lease.input, policy.clone(), context)?;
            // The token-bearing session is discarded before entering the worker.
            drop(session);
            let bundle = self.worker.analyze(&request)?;
            self.reports.stage_bytes(&bundle)?;
            let session = self
                .broker
                .session(lease.input.installation_id, &policy.repository)?;
            self.queue.stage(
                &lease,
                &bundle,
                &bundle.manifest.engine,
                &self.origin,
                &self.signer,
                &session,
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(DispatchOutcome::Analyzed),
            Err(e) => {
                let _ = self.queue.fail(
                    &lease,
                    matches!(e, SetupError::Unavailable | SetupError::Capacity),
                );
                Err(e)
            }
        }
    }
    fn deliver(&self, id: &str) -> Result<()> {
        let (input, policy) = self
            .queue
            .delivery_configuration(id, self.signer.public_key())?;
        let session = self
            .broker
            .session(input.installation_id, &policy.repository)?;
        self.reports.expose(id, &session)?;
        let receipt = self.queue.intent(id, self.signer.public_key())?;
        drop(session);
        let bundle = self.readback.readback(&receipt)?;
        let session = self
            .broker
            .session(input.installation_id, &policy.repository)?;
        self.queue
            .confirm_readback(id, self.signer.public_key(), &bundle, &session)?;
        self.queue
            .deliver(id, self.signer.public_key(), &session, &session)?;
        Ok(())
    }
}
