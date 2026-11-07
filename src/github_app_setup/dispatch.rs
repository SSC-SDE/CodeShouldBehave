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
    budget: Arc<AppRequestBudget>,
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
    pub budget: Arc<AppRequestBudget>,
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
            budget: config.budget,
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
        self.reports.maintain(2)?;
        if !self.queue.has_enabled_repository()? {
            return Ok(DispatchOutcome::Inactive);
        }
        if let Some(n) = self.inbox.claim()? {
            let target = if n.pr_number.is_some() {
                n.pr_number
            } else {
                self.queue.tracked_check(&n)?
            };
            if let (Some(repository), Some(pr)) = (n.repository_id, target) {
                match self
                    .queue
                    .installation_policy(n.installation_id, repository)
                {
                    Ok(policy) => {
                        self.budget.reserve(
                            n.installation_id,
                            &digest(&n.delivery_id),
                            10,
                            true,
                        )?;
                        let _requests = self.broker.begin_pass(10)?;
                        let session = self.broker.session(n.installation_id, &policy.repository)?;
                        let result = if n.event == "check_run" {
                            // Persist the idempotent fresh comparison before acknowledging.
                            let value = self.queue.reconcile(
                                n.installation_id,
                                repository,
                                pr,
                                &session,
                            )?;
                            self.inbox.finish(&n)?;
                            Ok(value)
                        } else {
                            self.queue.reconcile_notification(&n, &session)
                        };
                        match result {
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
        let lease = if self.queue.analysis_turn()? {
            self.queue.claim()?
        } else {
            None
        };
        if lease.is_none() {
            if let Some(id) = self.queue.pending_delivery()?.first() {
                self.queue.begin_delivery(id)?;
                let result = self.deliver(id);
                if result.is_err() {
                    self.queue.defer_delivery(id)?;
                    return Ok(DispatchOutcome::Deferred);
                }
                return Ok(DispatchOutcome::Delivered);
            }
        }
        let Some(lease) = (match lease {
            Some(lease) => Some(lease),
            None => self.queue.claim()?,
        }) else {
            return self.repair_one();
        };
        let result = (|| {
            let (policy, context) = self.queue.work_configuration(&lease)?;
            self.budget
                .reserve(lease.input.installation_id, &lease.input.id()?, 80, false)?;
            let _requests = self.broker.begin_pass(80)?;
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
    fn repair_one(&self) -> Result<DispatchOutcome> {
        let Some((installation, repository, pr)) = self.queue.claim_reconciliation()? else {
            return Ok(DispatchOutcome::Idle);
        };
        let policy = match self.queue.installation_policy(installation, repository) {
            Ok(p) => p,
            Err(SetupError::Denied) => return Ok(DispatchOutcome::Idle),
            Err(e) => return Err(e),
        };
        let scope = digest(&format!("repair:{repository}:{pr}:{}", now()? / 600));
        self.budget.reserve(installation, &scope, 10, true)?;
        let _requests = self.broker.begin_pass(10)?;
        let session = self.broker.session(installation, &policy.repository)?;
        match self.queue.reconcile(installation, repository, pr, &session) {
            Ok(_) | Err(SetupError::Conflict) => Ok(DispatchOutcome::Idle),
            Err(e) => Err(e),
        }
    }
    fn deliver(&self, id: &str) -> Result<()> {
        let (input, policy) = self
            .queue
            .delivery_configuration(id, self.signer.public_key())?;
        self.budget.reserve(input.installation_id, id, 32, false)?;
        let _requests = self.broker.begin_pass(32)?;
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
