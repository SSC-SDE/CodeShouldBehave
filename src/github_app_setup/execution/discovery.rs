//! Durable bounded inventory traversal, independent of webhook delivery.
//! REST pages are not a snapshot. Every completed walk is repeated, so page
//! movement is repaired on a later walk; completion never claims snapshot proof.
use super::super::installation::discovery::{DISCOVERY_PAGE_SIZE, OpenPullPage};
use super::*;

const RETRY_SECONDS: i64 = 600;
const SWEEP_SECONDS: i64 = 3_600;
const LEASE_SECONDS: i64 = 120;
const REPOSITORY_PENDING_LIMIT: i64 = 1_000;
const GLOBAL_PENDING_LIMIT: i64 = 10_000;

struct DiscoveryWork {
    installation: i64,
    repository: i64,
    fingerprint: String,
    lease: String,
    page: i64,
    sweep: i64,
    number: Option<i64>,
    policy: PublicPolicy,
}

impl AppExecutionQueue {
    /// At most four REST requests for one inventory page, OR five for one fresh
    /// comparison (including the two repository-limited credential requests).
    /// Errors leave pending numbers and the page checkpoint intact. The caller
    /// must continue ordinary analysis/delivery/repair even if discovery fails.
    pub fn discover_one(
        &self,
        budget: &AppRequestBudget,
        broker: &GitHubInstallationBroker,
    ) -> Result<bool> {
        let Some(work) = self.claim_discovery_at(now()?)? else {
            return Ok(false);
        };
        let result = (|| {
            // Recheck the exact current grant before reservation/credential IO.
            self.discovery_fence(&work, now()?)?;
            let tickets = if work.number.is_some() { 5 } else { 4 };
            // Retries in one ten-minute window share the 50-request ceiling;
            // restart cannot reset it. Day/App limits are shared with all work.
            let scope = digest(&format!(
                "discovery:{}:{}:{}:{}:{}:{}",
                work.repository,
                work.fingerprint,
                work.sweep,
                work.page,
                work.number.unwrap_or(0),
                now()? / RETRY_SECONDS
            ));
            budget.reserve(work.installation, &scope, tickets, true)?;
            let _pass = broker.begin_pass(tickets)?;
            // Serialize credential issuance with grant revocation.
            let session = {
                let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
                let tx = storage
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db)?;
                fence(&tx, self.app_id, &work, now()?)?;
                let session = broker.session(work.installation, &work.policy.repository)?;
                tx.commit().map_err(db)?;
                session
            };
            if let Some(number) = work.number {
                self.reconcile(work.installation, work.repository, number, &session)?;
                self.finish_discovery_number(&work, now()?)
            } else {
                // Hold the serialized grant fence over inventory IO too, just as
                // reconcile holds it over its authoritative current-PR read.
                let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
                let tx = storage
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db)?;
                fence(&tx, self.app_id, &work, now()?)?;
                let page = session.open_pull_page(work.page)?;
                persist_page(&tx, self.app_id, &work, page, now()?)?;
                tx.commit().map_err(db)
            }
        })();
        if let Err(error) = &result {
            self.defer_discovery(&work, error, now()?)?;
        }
        result.map(|()| true)
    }

    fn discovery_fence(&self, work: &DiscoveryWork, at: i64) -> Result<()> {
        let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        fence(&storage.connection, self.app_id, work, at)
    }

    fn claim_discovery_at(&self, at: i64) -> Result<Option<DiscoveryWork>> {
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        // Independent durable installation/repository turns prevent a large or
        // capacity-blocked repository from monopolizing discovery. Only one
        // candidate is inspected per call, including stale/revoked candidates.
        let target: Option<(i64, i64)> = tx.query_row(
            "SELECT g.installation_id,g.repository_id FROM app_execution_grants g LEFT JOIN app_pr_discovery_cursors c USING(app_id,installation_id,repository_id) WHERE g.app_id=?1 AND g.enabled=1 AND coalesce(c.lease_until,0)<=?2 AND (coalesce(c.next_scan_at,0)<=?2 OR EXISTS(SELECT 1 FROM app_pr_discovery_pending p WHERE p.app_id=g.app_id AND p.installation_id=g.installation_id AND p.repository_id=g.repository_id AND p.available_at<=?2)) ORDER BY coalesce((SELECT max(i.turn) FROM app_pr_discovery_cursors i WHERE i.app_id=g.app_id AND i.installation_id=g.installation_id),0),coalesce(c.turn,0),g.installation_id,g.repository_id LIMIT 1",
            params![self.app_id, at], |r| Ok((r.get(0)?,r.get(1)?))
        ).optional().map_err(db)?;
        let Some((installation, repository)) = target else {
            tx.commit().map_err(db)?;
            return Ok(None);
        };
        let current = grant(&tx, self.app_id, installation, repository);
        let fingerprint = current
            .as_ref()
            .map(grant_fingerprint)
            .unwrap_or_else(|_| "0".repeat(64));
        tx.execute("INSERT INTO app_pr_discovery_cursors(app_id,installation_id,repository_id,grant_fingerprint) VALUES(?1,?2,?3,?4) ON CONFLICT(app_id,installation_id,repository_id) DO NOTHING",params![self.app_id,installation,repository,fingerprint]).map_err(db)?;
        let turn: i64 = tx
            .query_row(
                "SELECT coalesce(max(turn),0)+1 FROM app_pr_discovery_cursors WHERE app_id=?1",
                [self.app_id],
                |r| r.get(0),
            )
            .map_err(db)?;
        tx.execute("UPDATE app_pr_discovery_cursors SET turn=?4 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![self.app_id,installation,repository,turn]).map_err(db)?;
        let current = match current {
            Ok(value) => value,
            Err(SetupError::Denied) => {
                tx.execute("UPDATE app_pr_discovery_cursors SET next_scan_at=?4,last_error='denied' WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![self.app_id,installation,repository,at+RETRY_SECONDS]).map_err(db)?;
                tx.execute("UPDATE app_pr_discovery_pending SET available_at=?4 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND available_at<?4",params![self.app_id,installation,repository,at+RETRY_SECONDS]).map_err(db)?;
                tx.commit().map_err(db)?;
                return Ok(None);
            }
            Err(error) => {
                tx.commit().map_err(db)?;
                return Err(error);
            }
        };
        // New exact authority restarts the walk, but never deletes pending hints.
        tx.execute("UPDATE app_pr_discovery_cursors SET grant_fingerprint=?4,next_page=1,last_number=0,next_scan_at=0 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND grant_fingerprint!=?4",params![self.app_id,installation,repository,fingerprint]).map_err(db)?;
        let (page,sweep): (i64,i64) = tx.query_row("SELECT next_page,sweep FROM app_pr_discovery_cursors WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![self.app_id,installation,repository],|r|Ok((r.get(0)?,r.get(1)?))).map_err(db)?;
        let number: Option<i64> = tx.query_row("SELECT pr_number FROM app_pr_discovery_pending WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND available_at<=?4 ORDER BY turn,pr_number LIMIT 1",params![self.app_id,installation,repository,at],|r|r.get(0)).optional().map_err(db)?;
        if let Some(number) = number {
            // A crash or repeated capacity refusal rotates the pending number;
            // it cannot pin all later admitted numbers behind the first one.
            tx.execute("UPDATE app_pr_discovery_pending SET available_at=?5,turn=(SELECT coalesce(max(turn),0)+1 FROM app_pr_discovery_pending WHERE app_id=?1) WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND pr_number=?4",params![self.app_id,installation,repository,number,at+RETRY_SECONDS]).map_err(db)?;
        }
        let lease = secret()?;
        tx.execute("UPDATE app_pr_discovery_cursors SET lease=?4,lease_until=?5 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![self.app_id,installation,repository,lease,at+LEASE_SECONDS]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(Some(DiscoveryWork {
            installation,
            repository,
            fingerprint,
            lease,
            page,
            sweep,
            number,
            policy: current.policy,
        }))
    }

    fn finish_discovery_number(&self, work: &DiscoveryWork, at: i64) -> Result<()> {
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        fence(&tx, self.app_id, work, at)?;
        tx.execute("DELETE FROM app_pr_discovery_pending WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND pr_number=?4",params![self.app_id,work.installation,work.repository,work.number.ok_or(SetupError::Invalid)?]).map_err(db)?;
        release(&tx, self.app_id, work)?;
        tx.commit().map_err(db)
    }

    fn defer_discovery(&self, work: &DiscoveryWork, error: &SetupError, at: i64) -> Result<()> {
        let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let status = match error {
            SetupError::Capacity => "capacity",
            SetupError::Denied | SetupError::Authentication => "denied",
            SetupError::Invalid => "invalid",
            SetupError::Conflict => "conflict",
            _ => "unavailable",
        };
        // Never delete an unsuccessful number. Inventory failures do not advance
        // the page. Deferring the scan permits other repositories and pending
        // numbers to proceed, including after malformed or moving inventories.
        storage.connection.execute("UPDATE app_pr_discovery_cursors SET lease=NULL,lease_until=0,last_error=?5,next_scan_at=max(next_scan_at,?6) WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND lease=?4",params![self.app_id,work.installation,work.repository,work.lease,status,at+RETRY_SECONDS]).map_err(db)?;
        if work.number.is_none() && matches!(error, SetupError::Conflict) {
            // A moving/backward inventory needs a fresh walk, not endless
            // retries of the same offset. Persisted pending hints survive it.
            storage.connection.execute("UPDATE app_pr_discovery_cursors SET next_page=1,last_number=0 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND lease IS NULL AND next_page=?4 AND sweep=?5 AND last_error='conflict'",params![self.app_id,work.installation,work.repository,work.page,work.sweep]).map_err(db)?;
        }
        Ok(())
    }
}

fn grant_fingerprint(grant: &Grant) -> String {
    digest(&format!(
        "{}:{}:{}:{}",
        grant.revision, grant.consent, grant.engine, grant.metadata
    ))
}

fn fence(c: &rusqlite::Connection, app: i64, work: &DiscoveryWork, at: i64) -> Result<()> {
    let current = grant(c, app, work.installation, work.repository)?;
    if grant_fingerprint(&current) != work.fingerprint {
        return Err(SetupError::Denied);
    }
    let valid: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM app_pr_discovery_cursors WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND lease=?4 AND lease_until>?5 AND grant_fingerprint=?6 AND next_page=?7 AND sweep=?8)",params![app,work.installation,work.repository,work.lease,at,work.fingerprint,work.page,work.sweep],|r|r.get(0)).map_err(db)?;
    if !valid {
        return Err(SetupError::Conflict);
    }
    Ok(())
}

fn release(c: &rusqlite::Connection, app: i64, work: &DiscoveryWork) -> Result<()> {
    c.execute("UPDATE app_pr_discovery_cursors SET lease=NULL,lease_until=0,last_error=NULL WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3 AND lease=?4",params![app,work.installation,work.repository,work.lease]).map_err(db)?;
    Ok(())
}

fn persist_page(
    c: &rusqlite::Connection,
    app: i64,
    work: &DiscoveryWork,
    page: OpenPullPage,
    at: i64,
) -> Result<()> {
    if work.number.is_some()
        || page.numbers.len() > DISCOVERY_PAGE_SIZE
        || page.numbers.iter().any(|n| *n <= 0)
        || page.numbers.windows(2).any(|w| w[0] >= w[1])
        || page
            .supported
            .iter()
            .any(|n| page.numbers.binary_search(n).is_err())
    {
        return Err(SetupError::Invalid);
    }
    let previous: i64 = c.query_row("SELECT last_number FROM app_pr_discovery_cursors WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![app,work.installation,work.repository],|r|r.get(0)).map_err(db)?;
    if page
        .numbers
        .last()
        .is_some_and(|number| *number <= previous)
    {
        // An all-repeated/backward page is not successful forward progress. Keep
        // its pending numbers and fail explicitly instead of looping forever.
        return Err(SetupError::Conflict);
    }
    let current = grant(c, app, work.installation, work.repository)?;
    for number in &page.supported {
        // Once an exact-authority job exists, ordinary tracked-PR repair owns
        // head/base changes. Drafts have no job and remain discoverable on every
        // walk until ready. A previous consent/engine job does not suppress a
        // newly authorized initial comparison.
        c.execute("INSERT INTO app_pr_discovery_pending(app_id,installation_id,repository_id,pr_number) SELECT ?1,?2,?3,?4 WHERE NOT EXISTS(SELECT 1 FROM app_analysis_jobs j WHERE j.app_id=?1 AND j.installation_id=?2 AND j.repository_id=?3 AND j.pr_number=?4 AND j.consent_digest=?5 AND json_extract(j.input,'$.scope_revision')=?6 AND json_extract(j.input,'$.engine_digest')=?7 AND json_extract(j.input,'$.engine_metadata_digest')=?8) ON CONFLICT(app_id,installation_id,repository_id,pr_number) DO NOTHING",params![app,work.installation,work.repository,number,current.consent,current.revision,current.engine,current.metadata]).map_err(db)?;
    }
    let local: i64 = c.query_row("SELECT count(*) FROM app_pr_discovery_pending WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![app,work.installation,work.repository],|r|r.get(0)).map_err(db)?;
    let total: i64 = c
        .query_row("SELECT count(*) FROM app_pr_discovery_pending", [], |r| {
            r.get(0)
        })
        .map_err(db)?;
    if local > REPOSITORY_PENDING_LIMIT || total > GLOBAL_PENDING_LIMIT {
        return Err(SetupError::Capacity);
    }
    // Probe one more numeric page even after a short page. Only an empty page
    // terminates a walk; short/truncated-looking nonempty pages are not EOF.
    if page.numbers.is_empty() {
        let sweep = work.sweep.checked_add(1).ok_or(SetupError::Capacity)?;
        c.execute("UPDATE app_pr_discovery_cursors SET next_page=1,sweep=?4,last_number=0,next_scan_at=?5,last_pass_at=?6 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![app,work.installation,work.repository,sweep,at+SWEEP_SECONDS,at]).map_err(db)?;
    } else {
        let next = work.page.checked_add(1).ok_or(SetupError::Capacity)?;
        c.execute("UPDATE app_pr_discovery_cursors SET next_page=?4,last_number=?5,next_scan_at=?6 WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3",params![app,work.installation,work.repository,next,page.numbers.last().unwrap(),at]).map_err(db)?;
    }
    release(c, app, work)
}

#[cfg(test)]
mod tests;
