use crate::Storage;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobLane {
    Warm,
    Cold,
}

impl JobLane {
    fn as_str(self) -> &'static str {
        match self {
            Self::Warm => "warm",
            Self::Cold => "cold",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryClass {
    Retryable,
    Terminal,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedJob {
    pub id: i64,
    pub review_id: String,
    pub kind: String,
    pub repository_id: i64,
    pub provider_pr_id: String,
    pub pr_generation: i64,
    pub attempt: i64,
    pub lease_generation: i64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    pub queued: usize,
    pub running: usize,
    pub failed: usize,
    pub oldest_queued_age_ms: u64,
    pub total_attempts: usize,
    pub last_terminal_reason: Option<String>,
}

pub struct Scheduler<'a> {
    storage: &'a mut Storage,
}

impl<'a> Scheduler<'a> {
    pub fn new(storage: &'a mut Storage) -> Self {
        Self { storage }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register_review(
        &mut self,
        id: &str,
        repository_id: i64,
        scope_id: i64,
        selected_base: &str,
        analysis_base: &str,
        proposal: &str,
        comparison_mode: &str,
        now_ms: u64,
    ) -> Result<(), String> {
        validate_label("review identity", id)?;
        validate_commit(selected_base)?;
        validate_commit(analysis_base)?;
        validate_commit(proposal)?;
        if !matches!(comparison_mode, "exact" | "merge-base") {
            return Err("Invalid comparison mode".into());
        }
        self.storage
            .connection
            .execute(
                "INSERT OR IGNORE INTO reviews
             (id,repository_id,selected_base_commit,analysis_base_commit,proposal_commit,
              comparison_mode,scope_id,created_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    id,
                    repository_id,
                    selected_base,
                    analysis_base,
                    proposal,
                    comparison_mode,
                    scope_id,
                    to_i64(now_ms)?
                ],
            )
            .map_err(|error| format!("Cannot register review: {error}"))?;
        let matches: i64 = self
            .storage
            .connection
            .query_row(
                "SELECT count(*) FROM reviews WHERE id=?1 AND repository_id=?2 AND scope_id=?3
             AND selected_base_commit=?4 AND analysis_base_commit=?5 AND proposal_commit=?6
             AND comparison_mode=?7",
                params![
                    id,
                    repository_id,
                    scope_id,
                    selected_base,
                    analysis_base,
                    proposal,
                    comparison_mode
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot verify review: {error}"))?;
        if matches != 1 {
            return Err("Review identity conflicts with an existing review".into());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn observe_pr(
        &mut self,
        repository_id: i64,
        provider_pr_id: &str,
        generation: i64,
        review_id: &str,
        head_commit: &str,
        base_commit: &str,
        now_ms: u64,
    ) -> Result<bool, String> {
        validate_label("provider PR identity", provider_pr_id)?;
        validate_commit(head_commit)?;
        validate_commit(base_commit)?;
        if generation < 0 {
            return Err("PR generation must be nonnegative".into());
        }
        let changed = self.storage.connection.execute(
            "INSERT INTO pr_heads(repository_id,provider_pr_id,generation,head_commit,base_commit,observed_at_ms,current_review_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(repository_id,provider_pr_id) DO UPDATE SET
               generation=excluded.generation,head_commit=excluded.head_commit,
               base_commit=excluded.base_commit,observed_at_ms=excluded.observed_at_ms,
               current_review_id=excluded.current_review_id
             WHERE excluded.generation > pr_heads.generation",
            params![repository_id,provider_pr_id,generation,head_commit,base_commit,to_i64(now_ms)?,review_id],
        ).map_err(|error| format!("Cannot observe PR head: {error}"))? == 1;
        Ok(changed)
    }

    /// Replace the current review without advancing the provider generation.
    /// This is reserved for an explicit immutable scope change. The transition
    /// fences every old job before the new review can be enqueued.
    pub fn replace_current_scope(
        &mut self,
        repository_id: i64,
        provider_pr_id: &str,
        generation: i64,
        old_review_id: &str,
        new_review_id: &str,
        now_ms: u64,
    ) -> Result<(), String> {
        validate_label("provider PR identity", provider_pr_id)?;
        validate_label("old review identity", old_review_id)?;
        validate_label("new review identity", new_review_id)?;
        let transaction = self
            .storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start scope replacement: {error}"))?;
        let changed = transaction.execute(
            "UPDATE pr_heads SET current_review_id=?1,observed_at_ms=?2
             WHERE repository_id=?3 AND provider_pr_id=?4 AND generation=?5 AND current_review_id=?6",
            params![new_review_id,to_i64(now_ms)?,repository_id,provider_pr_id,generation,old_review_id],
        ).map_err(|error| format!("Cannot replace current scope: {error}"))?;
        if changed != 1 {
            return Err("Scope replacement conflicts with the current PR generation".into());
        }
        transaction
            .execute(
                "UPDATE attempts SET finished_at_ms=?1,outcome='superseded_scope'
             WHERE finished_at_ms IS NULL AND job_id IN (SELECT id FROM jobs WHERE review_id=?2)",
                params![to_i64(now_ms)?, old_review_id],
            )
            .map_err(|error| format!("Cannot close superseded scope attempts: {error}"))?;
        transaction
            .execute(
                "UPDATE jobs SET state='superseded',terminal_reason='review scope changed',
             superseded_by_review_id=?1,lease_owner=NULL,lease_expires_at_ms=NULL
             WHERE review_id=?2 AND state IN ('queued','retry','running')",
                params![new_review_id, old_review_id],
            )
            .map_err(|error| format!("Cannot fence superseded scope jobs: {error}"))?;
        transaction.execute(
            "UPDATE pr_revisions SET review_id=?1 WHERE repository_id=?2 AND provider_pr_id=?3 AND generation=?4 AND review_id=?5",
            params![new_review_id,repository_id,provider_pr_id,generation,old_review_id],
        ).map_err(|error| format!("Cannot bind revision to new scope: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit scope replacement: {error}"))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &mut self,
        review_id: &str,
        kind: &str,
        priority: i64,
        lane: JobLane,
        provider_pr_id: &str,
        pr_generation: i64,
        now_ms: u64,
    ) -> Result<(i64, bool), String> {
        validate_label("job kind", kind)?;
        validate_label("provider PR identity", provider_pr_id)?;
        let transaction = self
            .storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start enqueue transaction: {error}"))?;
        let repository_id: i64 = transaction
            .query_row(
                "SELECT repository_id FROM reviews WHERE id=?1",
                [review_id],
                |row| row.get(0),
            )
            .map_err(|_| "Review is unavailable".to_owned())?;
        let current: Option<(i64, String)> = transaction
            .query_row(
                "SELECT generation,current_review_id FROM pr_heads
             WHERE repository_id=?1 AND provider_pr_id=?2",
                params![repository_id, provider_pr_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("Cannot read PR generation: {error}"))?;
        if current.as_ref() != Some(&(pr_generation, review_id.to_owned())) {
            return Err("Review is not the current PR generation".into());
        }
        transaction
            .execute(
                "UPDATE jobs SET state='superseded',terminal_reason='newer PR generation',
             superseded_by_review_id=?1,lease_owner=NULL,lease_expires_at_ms=NULL
             WHERE state IN ('queued','retry') AND provider_pr_id=?2 AND pr_generation<?3
               AND review_id IN (SELECT id FROM reviews WHERE repository_id=?4)",
                params![review_id, provider_pr_id, pr_generation, repository_id],
            )
            .map_err(|error| format!("Cannot supersede old jobs: {error}"))?;
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO jobs
             (review_id,kind,priority,state,available_at_ms,lane,provider_pr_id,pr_generation,enqueued_at_ms)
             VALUES (?1,?2,?3,'queued',?4,?5,?6,?7,?4)",
                params![
                    review_id,
                    kind,
                    priority,
                    to_i64(now_ms)?,
                    lane.as_str(),
                    provider_pr_id,
                    pr_generation
                ],
            )
            .map_err(|error| format!("Cannot enqueue job: {error}"))?
            == 1;
        let id = transaction
            .query_row(
                "SELECT id FROM jobs WHERE review_id=?1 AND kind=?2",
                params![review_id, kind],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot read queued job: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit enqueue: {error}"))?;
        Ok((id, inserted))
    }

    pub fn claim(
        &mut self,
        worker: &str,
        lane: JobLane,
        lane_limit: usize,
        lease_ms: u64,
        now_ms: u64,
    ) -> Result<Option<ClaimedJob>, String> {
        validate_label("worker identity", worker)?;
        if lane_limit == 0 || lane_limit > 256 || !(1_000..=600_000).contains(&lease_ms) {
            return Err("Invalid scheduler claim limits".into());
        }
        let now = to_i64(now_ms)?;
        let expires = to_i64(now_ms.checked_add(lease_ms).ok_or("Lease time overflow")?)?;
        let transaction = self
            .storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start claim transaction: {error}"))?;
        transaction
            .execute(
                "UPDATE attempts SET finished_at_ms=?1,outcome='lease_expired'
             WHERE finished_at_ms IS NULL AND job_id IN
               (SELECT id FROM jobs WHERE state='running' AND lease_expires_at_ms<=?1)",
                [now],
            )
            .map_err(|error| format!("Cannot close expired attempts: {error}"))?;
        transaction
            .execute(
                "UPDATE jobs SET state='retry',available_at_ms=?1,lease_owner=NULL,
             lease_expires_at_ms=NULL,terminal_reason='worker lease expired'
             WHERE state='running' AND lease_expires_at_ms<=?1",
                [now],
            )
            .map_err(|error| format!("Cannot recover expired leases: {error}"))?;
        let active: i64 = transaction
            .query_row(
                "SELECT count(*) FROM jobs WHERE state='running' AND lane=?1",
                [lane.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot inspect lane capacity: {error}"))?;
        if active >= lane_limit as i64 {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(None);
        }
        let candidate: Option<(i64,String,String,i64,String,i64,i64)> = transaction.query_row(
            "SELECT j.id,j.review_id,j.kind,r.repository_id,j.provider_pr_id,j.pr_generation,j.attempt_count+1
             FROM jobs j JOIN reviews r ON r.id=j.review_id
             WHERE j.state IN ('queued','retry') AND j.lane=?1 AND j.available_at_ms<=?2
               AND NOT EXISTS (
                 SELECT 1 FROM jobs active JOIN reviews ar ON ar.id=active.review_id
                 WHERE active.state='running' AND ar.repository_id=r.repository_id)
             ORDER BY j.priority DESC,j.available_at_ms,j.id LIMIT 1",
            params![lane.as_str(),now], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?))
        ).optional().map_err(|error| format!("Cannot select queued job: {error}"))?;
        let Some((id, review_id, kind, repository_id, provider_pr_id, pr_generation, attempt)) =
            candidate
        else {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(None);
        };
        transaction
            .execute(
                "UPDATE jobs SET state='running',lease_owner=?1,lease_expires_at_ms=?2,
             lease_generation=lease_generation+1,attempt_count=?3,terminal_reason=NULL WHERE id=?4",
                params![worker, expires, attempt, id],
            )
            .map_err(|error| format!("Cannot claim job: {error}"))?;
        let lease_generation: i64 = transaction
            .query_row(
                "SELECT lease_generation FROM jobs WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot read lease token: {error}"))?;
        transaction
            .execute(
                "INSERT INTO attempts(job_id,attempt,lease_generation,started_at_ms)
             VALUES (?1,?2,?3,?4)",
                params![id, attempt, lease_generation, now],
            )
            .map_err(|error| format!("Cannot record attempt: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit claim: {error}"))?;
        Ok(Some(ClaimedJob {
            id,
            review_id,
            kind,
            repository_id,
            provider_pr_id,
            pr_generation,
            attempt,
            lease_generation,
            lease_expires_at_ms: expires as u64,
        }))
    }

    pub fn heartbeat(
        &mut self,
        job: &ClaimedJob,
        worker: &str,
        lease_ms: u64,
        now_ms: u64,
    ) -> Result<(), String> {
        let now = to_i64(now_ms)?;
        let expires = to_i64(now_ms.checked_add(lease_ms).ok_or("Lease time overflow")?)?;
        let changed = self
            .storage
            .connection
            .execute(
                "UPDATE jobs SET lease_expires_at_ms=?1 WHERE id=?2 AND state='running'
             AND lease_owner=?3 AND lease_generation=?4 AND lease_expires_at_ms>?5",
                params![expires, job.id, worker, job.lease_generation, now],
            )
            .map_err(|error| format!("Cannot heartbeat job: {error}"))?;
        if changed != 1 {
            return Err("Lease is stale or expired".into());
        }
        Ok(())
    }

    pub fn fail(
        &mut self,
        job: &ClaimedJob,
        worker: &str,
        class: RetryClass,
        reason: &str,
        now_ms: u64,
    ) -> Result<(), String> {
        if reason.is_empty() || reason.len() > 1024 {
            return Err("Invalid failure reason".into());
        }
        let now = to_i64(now_ms)?;
        let transaction = self
            .storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start failure transaction: {error}"))?;
        verify_lease(&transaction, job, worker, now)?;
        let (state, available) = match class {
            RetryClass::Retryable if job.attempt < 5 => {
                let exponent = u32::try_from(job.attempt.saturating_sub(1))
                    .unwrap_or(31)
                    .min(6);
                let delay = 1_000_i64.saturating_mul(1_i64 << exponent).min(60_000);
                ("retry", now.saturating_add(delay))
            }
            RetryClass::Retryable | RetryClass::Terminal => ("failed", now),
            RetryClass::Cancelled => ("cancelled", now),
        };
        transaction.execute(
            "UPDATE jobs SET state=?1,available_at_ms=?2,lease_owner=NULL,lease_expires_at_ms=NULL,
             terminal_reason=?3 WHERE id=?4", params![state,available,reason,job.id]
        ).map_err(|error| format!("Cannot fail job: {error}"))?;
        transaction
            .execute(
                "UPDATE attempts SET finished_at_ms=?1,outcome=?2
             WHERE job_id=?3 AND attempt=?4 AND lease_generation=?5",
                params![now, state, job.id, job.attempt, job.lease_generation],
            )
            .map_err(|error| format!("Cannot finish attempt: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit failure: {error}"))
    }

    pub fn complete(
        &mut self,
        job: &ClaimedJob,
        worker: &str,
        result_identity: &str,
        now_ms: u64,
    ) -> Result<(), String> {
        validate_label("result identity", result_identity)?;
        let now = to_i64(now_ms)?;
        let transaction = self
            .storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start completion transaction: {error}"))?;
        verify_lease(&transaction, job, worker, now)?;
        let current: Option<(i64, String)> = transaction
            .query_row(
                "SELECT generation,current_review_id FROM pr_heads
             WHERE repository_id=?1 AND provider_pr_id=?2",
                params![job.repository_id, job.provider_pr_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("Cannot verify current PR: {error}"))?;
        if current.as_ref() != Some(&(job.pr_generation, job.review_id.clone())) {
            return Err("Job no longer represents the current PR generation".into());
        }
        transaction
            .execute(
                "INSERT INTO current_results(repository_id,provider_pr_id,kind,review_id,job_id,
             lease_generation,result_identity,published_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(repository_id,provider_pr_id,kind) DO UPDATE SET
               review_id=excluded.review_id,job_id=excluded.job_id,
               lease_generation=excluded.lease_generation,result_identity=excluded.result_identity,
               published_at_ms=excluded.published_at_ms",
                params![
                    job.repository_id,
                    job.provider_pr_id,
                    job.kind,
                    job.review_id,
                    job.id,
                    job.lease_generation,
                    result_identity,
                    now
                ],
            )
            .map_err(|error| format!("Cannot publish current result: {error}"))?;
        transaction
            .execute(
                "UPDATE jobs SET state='completed',lease_owner=NULL,lease_expires_at_ms=NULL,
             terminal_reason=NULL WHERE id=?1",
                [job.id],
            )
            .map_err(|error| format!("Cannot complete job: {error}"))?;
        transaction
            .execute(
                "UPDATE attempts SET finished_at_ms=?1,outcome='completed'
             WHERE job_id=?2 AND attempt=?3 AND lease_generation=?4",
                params![now, job.id, job.attempt, job.lease_generation],
            )
            .map_err(|error| format!("Cannot complete attempt: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit completion: {error}"))
    }

    pub fn checkpoint(&self, job: &ClaimedJob) -> Result<(), String> {
        let current: Option<(String, i64, String)> = self
            .storage
            .connection
            .query_row(
                "SELECT j.state,h.generation,h.current_review_id FROM jobs j
             JOIN pr_heads h ON h.repository_id=?2 AND h.provider_pr_id=j.provider_pr_id
             WHERE j.id=?1",
                params![job.id, job.repository_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| format!("Cannot inspect cancellation: {error}"))?;
        match current {
            Some((state, generation, review))
                if state == "running"
                    && generation == job.pr_generation
                    && review == job.review_id =>
            {
                Ok(())
            }
            _ => Err("Job was cancelled, superseded, or is no longer running".into()),
        }
    }

    pub fn current_result(
        &self,
        repository_id: i64,
        provider_pr_id: &str,
        kind: &str,
    ) -> Result<Option<(String, String)>, String> {
        self.storage
            .connection
            .query_row(
                "SELECT review_id,result_identity FROM current_results
             WHERE repository_id=?1 AND provider_pr_id=?2 AND kind=?3",
                params![repository_id, provider_pr_id, kind],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("Cannot read current result: {error}"))
    }

    pub fn stats(&self, now_ms: u64) -> Result<QueueStats, String> {
        let now = to_i64(now_ms)?;
        self.storage.connection.query_row(
            "SELECT
               sum(CASE WHEN state IN ('queued','retry') THEN 1 ELSE 0 END),
               sum(CASE WHEN state='running' THEN 1 ELSE 0 END),
               sum(CASE WHEN state='failed' THEN 1 ELSE 0 END),
               min(CASE WHEN state IN ('queued','retry') THEN enqueued_at_ms END),
               sum(attempt_count),
               (SELECT terminal_reason FROM jobs WHERE terminal_reason IS NOT NULL ORDER BY id DESC LIMIT 1)
             FROM jobs", [], |row| {
                let oldest: Option<i64> = row.get(3)?;
                Ok(QueueStats {
                    queued: row.get::<_,Option<i64>>(0)?.unwrap_or(0) as usize,
                    running: row.get::<_,Option<i64>>(1)?.unwrap_or(0) as usize,
                    failed: row.get::<_,Option<i64>>(2)?.unwrap_or(0) as usize,
                    oldest_queued_age_ms: oldest.map_or(0, |value| now.saturating_sub(value) as u64),
                    total_attempts: row.get::<_,Option<i64>>(4)?.unwrap_or(0) as usize,
                    last_terminal_reason: row.get(5)?,
                })
             }
        ).map_err(|error| format!("Cannot read queue statistics: {error}"))
    }
}

fn verify_lease(
    transaction: &rusqlite::Transaction<'_>,
    job: &ClaimedJob,
    worker: &str,
    now: i64,
) -> Result<(), String> {
    let valid: i64 = transaction
        .query_row(
            "SELECT count(*) FROM jobs WHERE id=?1 AND state='running' AND lease_owner=?2
         AND lease_generation=?3 AND lease_expires_at_ms>?4",
            params![job.id, worker, job.lease_generation, now],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot verify lease: {error}"))?;
    if valid == 1 {
        Ok(())
    } else {
        Err("Lease is stale or expired".into())
    }
}

fn validate_label(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err(format!("Invalid {kind}"))
    } else {
        Ok(())
    }
}

fn validate_commit(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("Invalid commit identity".into())
    }
}

fn to_i64(value: u64) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| "Scheduler time exceeds SQLite range".into())
}

#[cfg(test)]
#[allow(clippy::drop_non_drop)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc, Barrier,
            atomic::{AtomicU64, Ordering},
        },
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn fixture(name: &str) -> (Storage, PathBuf, PathBuf, i64, i64) {
        let root = std::env::temp_dir().join(format!(
            "flowdiff-scheduler-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let database = root.join("state.db");
        let mut storage = Storage::open(&database, &root.join("artifacts")).unwrap();
        let repository = storage
            .register_repository("tenant", "github", "acme", name)
            .unwrap();
        let scope = storage.register_scope(repository, "v1", b"scope").unwrap();
        (storage, database, root, repository, scope)
    }

    fn commit(digit: char) -> String {
        std::iter::repeat_n(digit, 40).collect()
    }

    fn review(
        scheduler: &mut Scheduler<'_>,
        id: &str,
        repository: i64,
        scope: i64,
        proposal: char,
        now: u64,
    ) {
        scheduler
            .register_review(
                id,
                repository,
                scope,
                &commit('1'),
                &commit('1'),
                &commit(proposal),
                "exact",
                now,
            )
            .unwrap();
    }

    #[test]
    fn duplicate_enqueue_expired_lease_and_restart_are_safe() {
        let (mut storage, database, root, repository, scope) = fixture("restart");
        let stale = {
            let mut scheduler = Scheduler::new(&mut storage);
            review(&mut scheduler, "review-1", repository, scope, '2', 10);
            scheduler
                .observe_pr(
                    repository,
                    "17",
                    1,
                    "review-1",
                    &commit('2'),
                    &commit('1'),
                    10,
                )
                .unwrap();
            let first = scheduler
                .enqueue("review-1", "analyze", 10, JobLane::Warm, "17", 1, 10)
                .unwrap();
            assert!(first.1);
            assert_eq!(
                scheduler
                    .enqueue("review-1", "analyze", 10, JobLane::Warm, "17", 1, 10)
                    .unwrap(),
                (first.0, false)
            );
            let claim = scheduler
                .claim("worker-a", JobLane::Warm, 4, 1_000, 20)
                .unwrap()
                .unwrap();
            scheduler.heartbeat(&claim, "worker-a", 1_000, 500).unwrap();
            claim
        };
        drop(storage);
        let mut storage = Storage::open(&database, &root.join("artifacts")).unwrap();
        let mut scheduler = Scheduler::new(&mut storage);
        assert_eq!(scheduler.stats(600).unwrap().running, 1);
        let current = scheduler
            .claim("worker-b", JobLane::Warm, 4, 1_000, 1_501)
            .unwrap()
            .unwrap();
        assert_eq!((current.attempt, current.lease_generation), (2, 2));
        assert!(
            scheduler
                .complete(&stale, "worker-a", "stale", 1_502)
                .is_err()
        );
        scheduler
            .complete(&current, "worker-b", "result-2", 1_502)
            .unwrap();
        assert_eq!(
            scheduler
                .current_result(repository, "17", "analyze")
                .unwrap(),
            Some(("review-1".into(), "result-2".into()))
        );
        drop(scheduler);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_lane_capacity_and_repository_fairness_are_bounded() {
        let (mut storage, _, root, repository, scope) = fixture("fairness");
        let mut scheduler = Scheduler::new(&mut storage);
        review(&mut scheduler, "review-a", repository, scope, '2', 0);
        review(&mut scheduler, "review-b", repository, scope, '3', 0);
        scheduler
            .observe_pr(
                repository,
                "a",
                1,
                "review-a",
                &commit('2'),
                &commit('1'),
                0,
            )
            .unwrap();
        scheduler
            .observe_pr(
                repository,
                "b",
                1,
                "review-b",
                &commit('3'),
                &commit('1'),
                0,
            )
            .unwrap();
        scheduler
            .enqueue("review-a", "analyze", 10, JobLane::Warm, "a", 1, 0)
            .unwrap();
        scheduler
            .enqueue("review-b", "analyze", 1, JobLane::Warm, "b", 1, 0)
            .unwrap();
        let first = scheduler
            .claim("worker-a", JobLane::Warm, 4, 1_000, 1)
            .unwrap()
            .unwrap();
        assert!(
            scheduler
                .claim("worker-b", JobLane::Warm, 4, 1_000, 1)
                .unwrap()
                .is_none()
        );
        scheduler
            .fail(&first, "worker-a", RetryClass::Retryable, "temporary", 10)
            .unwrap();
        let other = scheduler
            .claim("worker-b", JobLane::Warm, 4, 1_000, 11)
            .unwrap()
            .unwrap();
        assert_eq!(other.review_id, "review-b");
        scheduler
            .fail(&other, "worker-b", RetryClass::Terminal, "invalid", 12)
            .unwrap();
        assert!(
            scheduler
                .claim("worker-a", JobLane::Warm, 4, 1_000, 1_009)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            scheduler
                .claim("worker-a", JobLane::Warm, 4, 1_000, 1_010)
                .unwrap()
                .unwrap()
                .review_id,
            "review-a"
        );
        drop(scheduler);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn superseded_generation_cannot_publish_and_cold_lane_is_separate() {
        let (mut storage, _, root, repository, scope) = fixture("fencing");
        let mut scheduler = Scheduler::new(&mut storage);
        review(&mut scheduler, "old-review", repository, scope, '2', 0);
        scheduler
            .observe_pr(
                repository,
                "9",
                1,
                "old-review",
                &commit('2'),
                &commit('1'),
                0,
            )
            .unwrap();
        scheduler
            .enqueue("old-review", "analyze", 1, JobLane::Warm, "9", 1, 0)
            .unwrap();
        scheduler
            .enqueue("old-review", "secondary", 1, JobLane::Warm, "9", 1, 0)
            .unwrap();
        let old = scheduler
            .claim("old-worker", JobLane::Warm, 4, 1_000, 1)
            .unwrap()
            .unwrap();
        review(&mut scheduler, "new-review", repository, scope, '3', 2);
        scheduler
            .observe_pr(
                repository,
                "9",
                2,
                "new-review",
                &commit('3'),
                &commit('1'),
                2,
            )
            .unwrap();
        scheduler
            .enqueue("new-review", "secondary", 1, JobLane::Warm, "9", 2, 2)
            .unwrap();
        let superseded: String = scheduler
            .storage
            .connection
            .query_row(
                "SELECT state FROM jobs WHERE review_id='old-review' AND kind='secondary'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(superseded, "superseded");
        assert!(scheduler.checkpoint(&old).is_err());
        assert!(
            scheduler
                .complete(&old, "old-worker", "obsolete", 3)
                .is_err()
        );
        scheduler
            .fail(&old, "old-worker", RetryClass::Cancelled, "superseded", 3)
            .unwrap();
        scheduler
            .enqueue("new-review", "analyze", 1, JobLane::Cold, "9", 2, 3)
            .unwrap();
        let new = scheduler
            .claim("cold-worker", JobLane::Cold, 1, 1_000, 4)
            .unwrap()
            .unwrap();
        scheduler
            .complete(&new, "cold-worker", "current", 5)
            .unwrap();
        assert_eq!(
            scheduler
                .current_result(repository, "9", "analyze")
                .unwrap(),
            Some(("new-review".into(), "current".into()))
        );
        drop(scheduler);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_generation_scope_change_fences_running_and_queued_work() {
        let (mut storage, _, root, repository, scope) = fixture("scope-fence");
        let narrowed = storage
            .register_scope(repository, "v2", b"narrowed")
            .unwrap();
        let mut scheduler = Scheduler::new(&mut storage);
        review(&mut scheduler, "broad-review", repository, scope, '2', 0);
        scheduler
            .observe_pr(
                repository,
                "31",
                4,
                "broad-review",
                &commit('2'),
                &commit('1'),
                0,
            )
            .unwrap();
        scheduler
            .enqueue("broad-review", "analyze", 1, JobLane::Warm, "31", 4, 0)
            .unwrap();
        scheduler
            .enqueue("broad-review", "secondary", 1, JobLane::Warm, "31", 4, 0)
            .unwrap();
        let running = scheduler
            .claim("worker", JobLane::Warm, 2, 1_000, 1)
            .unwrap()
            .unwrap();
        review(
            &mut scheduler,
            "narrow-review",
            repository,
            narrowed,
            '2',
            2,
        );
        scheduler
            .replace_current_scope(repository, "31", 4, "broad-review", "narrow-review", 2)
            .unwrap();
        scheduler
            .enqueue("narrow-review", "analyze", 1, JobLane::Warm, "31", 4, 2)
            .unwrap();
        assert!(
            scheduler
                .complete(&running, "worker", "late-result", 3)
                .is_err()
        );
        let states: Vec<String> = scheduler
            .storage
            .connection
            .prepare("SELECT state FROM jobs WHERE review_id='broad-review' ORDER BY kind")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(states, vec!["superseded", "superseded"]);
        assert_eq!(
            scheduler
                .claim("new-worker", JobLane::Warm, 2, 1_000, 3)
                .unwrap()
                .unwrap()
                .review_id,
            "narrow-review"
        );
        drop(scheduler);
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn two_workers_racing_receive_one_lease() {
        let (mut storage, database, root, repository, scope) = fixture("race");
        {
            let mut scheduler = Scheduler::new(&mut storage);
            review(&mut scheduler, "review-race", repository, scope, '2', 0);
            scheduler
                .observe_pr(
                    repository,
                    "race",
                    1,
                    "review-race",
                    &commit('2'),
                    &commit('1'),
                    0,
                )
                .unwrap();
            scheduler
                .enqueue("review-race", "analyze", 1, JobLane::Warm, "race", 1, 0)
                .unwrap();
        }
        drop(storage);
        let barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();
        for worker in ["one", "two"] {
            let database = database.clone();
            let artifacts = root.join("artifacts");
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let mut storage = Storage::open(&database, &artifacts).unwrap();
                barrier.wait();
                Scheduler::new(&mut storage)
                    .claim(worker, JobLane::Warm, 4, 1_000, 1)
                    .unwrap()
            }));
        }
        let claims = workers
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(claims.len(), 1);
        drop(claims);
        fs::remove_dir_all(root).unwrap();
    }
}
