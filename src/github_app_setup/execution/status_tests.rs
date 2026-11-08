use super::*;
use crate::github_app_setup::pipeline_tests::{Fixture, Provider, signer};
use std::cell::{Cell, RefCell};

struct Checks {
    posts: Cell<usize>,
    visible: Cell<bool>,
    lose_response: Cell<bool>,
    value: RefCell<Option<Value>>,
}
impl Checks {
    fn new() -> Self {
        Self {
            posts: Cell::new(0),
            visible: Cell::new(false),
            lose_response: Cell::new(false),
            value: RefCell::new(None),
        }
    }
    fn status(&self) -> String {
        self.value.borrow().as_ref().unwrap()["status"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}
impl AppCheckDelivery for Checks {
    fn find(&self, _: i64, _: &str, payload: &Value) -> Result<Option<i64>> {
        if !self.visible.get() {
            return Ok(None);
        }
        if self.value.borrow().as_ref() != Some(payload) {
            return Err(SetupError::Conflict);
        }
        Ok(Some(31))
    }
    fn create(&self, _: i64, payload: &Value) -> Result<i64> {
        self.posts.set(self.posts.get() + 1);
        self.value.replace(Some(payload.clone()));
        if self.lose_response.get() {
            return Err(SetupError::Unavailable);
        }
        self.visible.set(true);
        Ok(31)
    }
    fn update(&self, _: i64, id: i64, payload: &Value) -> Result<i64> {
        let previous = self.value.borrow();
        if id != 31
            || !self.visible.get()
            || previous.as_ref().is_none_or(|p| {
                ["name", "head_sha", "external_id"]
                    .iter()
                    .any(|k| p[k] != payload[k])
            })
        {
            return Err(SetupError::Denied);
        }
        drop(previous);
        self.value.replace(Some(payload.clone()));
        Ok(id)
    }
}
#[test]
fn same_check_transitions_from_queue_through_verified_report() {
    let f = Fixture::new();
    let q = f.queue();
    let provider = Provider(f.request.input.comparison.clone());
    let id = q.reconcile(2, 3, 7, &provider).unwrap().unwrap();
    let checks = Checks::new();
    assert_eq!(q.pending_progress().unwrap().as_deref(), Some(id.as_str()));
    q.deliver_progress(&id, &provider, &checks).unwrap();
    assert_eq!(checks.status(), "queued");
    assert!(q.pending_progress().unwrap().is_none());
    let lease = q.claim().unwrap().unwrap();
    q.deliver_progress(&id, &provider, &checks).unwrap();
    assert_eq!(checks.status(), "in_progress");
    assert!(checks.value.borrow().as_ref().unwrap()["details_url"].is_null());
    let bundle = f.bundle();
    let key = signer();
    q.stage(
        &lease,
        &bundle,
        &bundle.manifest.engine,
        "https://reports.example.test",
        &key,
        &provider,
    )
    .unwrap();
    assert!(
        q.deliver(&id, key.public_key(), &provider, &checks)
            .is_err()
    );
    assert_eq!(checks.status(), "in_progress");
    q.confirm_readback(&id, key.public_key(), &bundle, &provider)
        .unwrap();
    assert_eq!(
        q.deliver(&id, key.public_key(), &provider, &checks)
            .unwrap(),
        31
    );
    assert_eq!(checks.posts.get(), 1);
    let value = checks.value.borrow();
    let value = value.as_ref().unwrap();
    assert_eq!(value["status"], "completed");
    assert_eq!(value["conclusion"], "neutral");
    assert!(
        value["details_url"]
            .as_str()
            .unwrap()
            .contains("mode=focus&design=hld")
    );
    assert!(q.pending_progress().unwrap().is_none());
}
#[test]
fn unknown_queued_create_is_reconciled_after_restart_without_another_post() {
    let f = Fixture::new();
    let q = f.queue();
    let provider = Provider(f.request.input.comparison.clone());
    let id = q.reconcile(2, 3, 7, &provider).unwrap().unwrap();
    let checks = Checks::new();
    checks.lose_response.set(true);
    assert_eq!(
        q.deliver_progress(&id, &provider, &checks),
        Err(SetupError::Unavailable)
    );
    let reopened = AppExecutionQueue::new(
        Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
        1,
    )
    .unwrap();
    let _lease = reopened.claim().unwrap().unwrap();
    assert_eq!(
        reopened.deliver_progress(&id, &provider, &checks),
        Err(SetupError::Conflict)
    );
    assert_eq!(checks.posts.get(), 1);
    checks.visible.set(true);
    reopened
        .storage
        .lock()
        .unwrap()
        .connection
        .execute("UPDATE app_check_progress SET next_at=0", [])
        .unwrap();
    reopened.deliver_progress(&id, &provider, &checks).unwrap();
    assert_eq!(checks.status(), "in_progress");
    assert_eq!(checks.posts.get(), 1);
    assert!(
        reopened
            .storage
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT delivery_attempted FROM app_analysis_jobs",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
}
#[test]
fn terminal_failure_is_not_a_clean_result_and_revocation_prevents_status_io() {
    let f = Fixture::new();
    let q = f.queue();
    let provider = Provider(f.request.input.comparison.clone());
    let id = q.reconcile(2, 3, 7, &provider).unwrap().unwrap();
    let checks = Checks::new();
    let lease = q.claim().unwrap().unwrap();
    q.fail(&lease, false).unwrap();
    q.deliver_progress(&id, &provider, &checks).unwrap();
    let value = checks.value.borrow();
    let value = value.as_ref().unwrap();
    assert_eq!(value["status"], "completed");
    assert_eq!(value["conclusion"], "failure");
    assert!(value["details_url"].is_null());
    q.storage
        .lock()
        .unwrap()
        .connection
        .execute("UPDATE app_execution_grants SET enabled=0", [])
        .unwrap();
    let blocked = Checks::new();
    assert_eq!(
        q.deliver_progress(&id, &provider, &blocked),
        Err(SetupError::Denied)
    );
    assert_eq!(blocked.posts.get(), 0);
}

#[test]
fn exhausted_progress_budget_does_not_hide_other_checks_after_restart() {
    struct CurrentByNumber(CurrentPullRequest);
    impl AppExecutionProvider for CurrentByNumber {
        fn current_pr(&self, _: i64, _: i64, pr: i64) -> Result<CurrentPullRequest> {
            let mut current = self.0.clone();
            current.number = pr;
            Ok(current)
        }
    }
    let f = Fixture::new();
    let q = f.queue();
    let provider = CurrentByNumber(f.request.input.comparison.clone());
    let first = q.reconcile(2, 3, 7, &provider).unwrap().unwrap();
    let second = q.reconcile(2, 3, 8, &provider).unwrap().unwrap();
    let budget = AppRequestBudget::new(
        Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
        1,
    )
    .unwrap();
    for tickets in [100, 100, 100, 84] {
        budget.reserve(2, &first, tickets, false).unwrap();
    }
    assert_eq!(
        budget.reserve(2, &first, 12, false),
        Err(SetupError::Capacity)
    );
    q.backoff_progress(&first).unwrap();
    let reopened = AppExecutionQueue::new(
        Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
        1,
    )
    .unwrap();
    assert_eq!(reopened.pending_progress().unwrap(), Some(second.clone()));
    budget.reserve(2, &second, 12, false).unwrap();
    assert_eq!(
        budget.reserve(2, &first, 1, false),
        Err(SetupError::Capacity)
    );
}
