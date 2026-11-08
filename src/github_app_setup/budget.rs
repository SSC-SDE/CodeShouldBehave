//! Conservative durable reservations: unused tickets are never refunded.
use super::*;
pub struct AppRequestBudget {
    storage: Mutex<Storage>,
    app: i64,
}
impl AppRequestBudget {
    pub fn new(storage: Storage, app: i64) -> Result<Self> {
        if app <= 0 {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            storage: Mutex::new(storage),
            app,
        })
    }
    /// Scope is an immutable job ID or notification digest. Reserve before any
    /// provider request; an interrupted pass consumes its entire allocation.
    pub fn reserve(
        &self,
        installation: i64,
        scope: &str,
        tickets: usize,
        notification: bool,
    ) -> Result<()> {
        self.reserve_at(installation, scope, tickets, notification, now()?)
    }
    fn reserve_at(
        &self,
        installation: i64,
        scope: &str,
        tickets: usize,
        notification: bool,
        at: i64,
    ) -> Result<()> {
        if installation <= 0 || !valid_secret(scope) || tickets == 0 || tickets > 100 || at < 0 {
            return Err(SetupError::Invalid);
        }
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let buckets = [
            (format!("day:{}", at / 86400), 10000),
            (
                format!("installation:{installation}:day:{}", at / 86400),
                2000,
            ),
            (
                format!(
                    "{}:{installation}:{scope}",
                    if notification { "notification" } else { "job" }
                ),
                if notification { 50 } else { 384 },
            ),
        ];
        let count: i64 = tx
            .query_row("SELECT count(*) FROM app_request_budgets", [], |r| r.get(0))
            .map_err(db)?;
        if count > 20000 - 3 {
            return Err(SetupError::Capacity);
        }
        for (bucket, limit) in buckets {
            let used: i64 = tx
                .query_row(
                    "SELECT used FROM app_request_budgets WHERE app_id=?1 AND bucket=?2",
                    params![self.app, bucket],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?
                .unwrap_or(0);
            if used + tickets as i64 > limit {
                return Err(SetupError::Capacity);
            }
            tx.execute("INSERT INTO app_request_budgets VALUES(?1,?2,?3) ON CONFLICT(app_id,bucket) DO UPDATE SET used=used+excluded.used",params![self.app,bucket,tickets as i64]).map_err(db)?;
        }
        tx.commit().map_err(db)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservations_survive_restart_and_fail_atomically() {
        let f = super::super::pipeline_tests::Fixture::new();
        let open = || {
            AppRequestBudget::new(
                Storage::open(&f.root.join("budget.db"), &f.root.join("budget-artifacts")).unwrap(),
                1,
            )
            .unwrap()
        };
        let b = open();
        for _ in 0..3 {
            b.reserve_at(2, &"a".repeat(64), 100, false, 86400).unwrap();
        }
        drop(b);
        let b = open();
        assert_eq!(
            b.reserve_at(2, &"a".repeat(64), 100, false, 86400),
            Err(SetupError::Capacity)
        );
        b.reserve_at(2, &"a".repeat(64), 84, false, 86400).unwrap();
        assert_eq!(
            b.reserve_at(2, &"a".repeat(64), 1, false, 172800),
            Err(SetupError::Capacity)
        );
        for n in 0..16 {
            b.reserve_at(2, &format!("{n:064x}"), 100, false, 86400)
                .unwrap();
        }
        assert_eq!(
            b.reserve_at(2, &"b".repeat(64), 17, false, 86400),
            Err(SetupError::Capacity)
        );
        b.reserve_at(3, &"b".repeat(64), 100, false, 86400).unwrap();
        let c = rusqlite::Connection::open(f.root.join("budget.db")).unwrap();
        assert!(
            c.execute("UPDATE app_request_budgets SET used=0", [])
                .is_err()
        );
    }
}
