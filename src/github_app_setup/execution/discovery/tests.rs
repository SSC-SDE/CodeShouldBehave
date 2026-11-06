use super::*;
use crate::github_app_setup::pipeline_tests::Fixture;

struct Provider(CurrentPullRequest);
impl AppExecutionProvider for Provider {
    fn current_pr(
        &self,
        installation: i64,
        repository: i64,
        number: i64,
    ) -> Result<CurrentPullRequest> {
        assert_eq!((installation, repository), (2, 3));
        let mut comparison = self.0.clone();
        comparison.number = number;
        Ok(comparison)
    }
}
fn store(q: &AppExecutionQueue, work: &DiscoveryWork, numbers: &[i64], at: i64) -> Result<()> {
    let mut storage = q.storage.lock().unwrap();
    let tx = storage
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    fence(&tx, q.app_id, work, at)?;
    persist_page(
        &tx,
        q.app_id,
        work,
        OpenPullPage {
            numbers: numbers.to_vec(),
            supported: numbers.to_vec(),
        },
        at,
    )?;
    tx.commit().map_err(db)
}
fn pending(q: &AppExecutionQueue) -> Vec<i64> {
    let storage = q.storage.lock().unwrap();
    let mut stmt = storage
        .connection
        .prepare("SELECT pr_number FROM app_pr_discovery_pending ORDER BY pr_number")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}
fn sql(q: &AppExecutionQueue, text: &str) {
    q.storage
        .lock()
        .unwrap()
        .connection
        .execute_batch(text)
        .unwrap();
}
fn restart(f: &Fixture) -> AppExecutionQueue {
    AppExecutionQueue::new(
        Storage::open(&f.root.join("state.db"), &f.root.join("artifacts")).unwrap(),
        1,
    )
    .unwrap()
}

#[test]
fn initial_inventory_enqueues_every_number_across_pages_and_restart_without_webhooks() {
    let f = Fixture::new();
    let provider = Provider(f.request.input.comparison.clone());
    let q = f.queue();
    let mut at = now().unwrap();
    let first = q.claim_discovery_at(at).unwrap().unwrap();
    assert_eq!((first.page, first.number), (1, None));
    store(&q, &first, &(1..=20).collect::<Vec<_>>(), at).unwrap();
    assert_eq!(pending(&q).len(), 20);
    drop(q);
    let q = restart(&f);
    for number in 1..=20 {
        at += 1;
        let work = q.claim_discovery_at(at).unwrap().unwrap();
        assert_eq!(work.number, Some(number));
        q.reconcile(2, 3, number, &provider).unwrap();
        q.finish_discovery_number(&work, at).unwrap();
    }
    let second = q.claim_discovery_at(at + 1).unwrap().unwrap();
    assert_eq!((second.page, second.number), (2, None));
    store(&q, &second, &[21, 22], at + 1).unwrap();
    for number in [21, 22] {
        at += 2;
        let work = q.claim_discovery_at(at).unwrap().unwrap();
        assert_eq!(work.number, Some(number));
        q.reconcile(2, 3, number, &provider).unwrap();
        q.finish_discovery_number(&work, at).unwrap();
    }
    let terminal = q.claim_discovery_at(at + 1).unwrap().unwrap();
    assert_eq!(terminal.page, 3); // A short page was not silently treated as EOF.
    store(&q, &terminal, &[], at + 1).unwrap();
    assert!(pending(&q).is_empty());
    assert!(q.claim_discovery_at(at + 2).unwrap().is_none());
    let storage = q.storage.lock().unwrap();
    assert_eq!(
        storage
            .connection
            .query_row("SELECT count(*) FROM app_analysis_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        22
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT sweep FROM app_pr_discovery_cursors", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn capacity_refusal_rotates_pending_and_restart_never_acknowledges_it() {
    let f = Fixture::new();
    let provider = Provider(f.request.input.comparison.clone());
    let q = f.queue();
    for number in 100..125 {
        q.reconcile(2, 3, number, &provider).unwrap();
    }
    let at = now().unwrap();
    let page = q.claim_discovery_at(at).unwrap().unwrap();
    store(&q, &page, &[1, 2, 3], at).unwrap();
    let first = q.claim_discovery_at(at + 1).unwrap().unwrap();
    assert!(matches!(
        q.reconcile(2, 3, 1, &provider),
        Err(SetupError::Capacity)
    ));
    q.defer_discovery(&first, &SetupError::Capacity, at + 1)
        .unwrap();
    drop(q);
    let q = restart(&f);
    let second = q.claim_discovery_at(at + 2).unwrap().unwrap();
    assert_eq!(second.number, Some(2));
    assert_eq!(pending(&q), vec![1, 2, 3]);
    // A crash before processing #2 preserves it too. #3 is still served next.
    let third = q
        .claim_discovery_at(at + 2 + LEASE_SECONDS)
        .unwrap()
        .unwrap();
    assert_eq!(third.number, Some(3));
    sql(&q, "UPDATE app_analysis_jobs SET state='delivered'");
    q.reconcile(2, 3, 3, &provider).unwrap();
    q.finish_discovery_number(&third, at + 2 + LEASE_SECONDS)
        .unwrap();
    assert_eq!(pending(&q), vec![1, 2]);
    let retry = q
        .claim_discovery_at(at + 3 + RETRY_SECONDS)
        .unwrap()
        .unwrap();
    assert_eq!(retry.number, Some(1));
    q.reconcile(2, 3, 1, &provider).unwrap();
    q.finish_discovery_number(&retry, at + 3 + RETRY_SECONDS)
        .unwrap();
    assert_eq!(pending(&q), vec![2]);
}

#[test]
fn crash_after_queue_commit_before_discovery_ack_is_idempotent() {
    let f = Fixture::new();
    let q = f.queue();
    let at = now().unwrap();
    let page = q.claim_discovery_at(at).unwrap().unwrap();
    store(&q, &page, &[7], at).unwrap();
    let work = q.claim_discovery_at(at + 1).unwrap().unwrap();
    let provider = Provider(f.request.input.comparison.clone());
    let id = q.reconcile(2, 3, 7, &provider).unwrap();
    drop(q);
    let q = restart(&f);
    let retry = q
        .claim_discovery_at(at + 1 + RETRY_SECONDS)
        .unwrap()
        .unwrap();
    assert_eq!(retry.number, Some(7));
    assert_eq!(q.reconcile(2, 3, 7, &provider).unwrap(), id);
    assert!(matches!(
        q.finish_discovery_number(&work, at + 2 + RETRY_SECONDS),
        Err(SetupError::Conflict)
    ));
    q.finish_discovery_number(&retry, at + 2 + RETRY_SECONDS)
        .unwrap();
    assert!(pending(&q).is_empty());
}

#[test]
fn moving_pages_repeat_walk_and_duplicates_preserve_unique_pending_hints() {
    let f = Fixture::new();
    let q = f.queue();
    let at = now().unwrap();
    let first = q.claim_discovery_at(at).unwrap().unwrap();
    store(&q, &first, &[1, 2], at).unwrap();
    sql(
        &q,
        "UPDATE app_pr_discovery_pending SET available_at=9223372036854775807",
    );
    let second = q.claim_discovery_at(at + 1).unwrap().unwrap();
    // Page overlap is legal during movement; #2 remains exactly one hint.
    store(&q, &second, &[2, 3], at + 1).unwrap();
    assert_eq!(pending(&q), vec![1, 2, 3]);
    sql(
        &q,
        "UPDATE app_pr_discovery_pending SET available_at=9223372036854775807",
    );
    let repeated = q.claim_discovery_at(at + 2).unwrap().unwrap();
    assert!(matches!(
        store(&q, &repeated, &[1, 2], at + 2),
        Err(SetupError::Conflict)
    ));
    q.defer_discovery(&repeated, &SetupError::Conflict, at + 2)
        .unwrap();
    let restarted = q
        .claim_discovery_at(at + 2 + RETRY_SECONDS)
        .unwrap()
        .unwrap();
    assert_eq!(restarted.page, 1);
    assert_eq!(pending(&q), vec![1, 2, 3]);
}

#[test]
fn bounded_pending_capacity_rolls_back_whole_page_and_checkpoint() {
    let f = Fixture::new();
    let q = f.queue();
    let at = now().unwrap();
    let work = q.claim_discovery_at(at).unwrap().unwrap();
    sql(
        &q,
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) INSERT INTO app_pr_discovery_pending(app_id,installation_id,repository_id,pr_number,available_at) SELECT 1,2,3,x,9223372036854775807 FROM n",
    );
    assert!(matches!(
        store(&q, &work, &[1001, 1002], at),
        Err(SetupError::Capacity)
    ));
    assert_eq!(pending(&q).len(), 1000);
    q.defer_discovery(&work, &SetupError::Capacity, at).unwrap();
    sql(
        &q,
        "DELETE FROM app_pr_discovery_pending WHERE pr_number<=2",
    );
    let retry = q.claim_discovery_at(at + RETRY_SECONDS).unwrap().unwrap();
    assert_eq!(retry.page, 1);
    store(&q, &retry, &[1001, 1002], at + RETRY_SECONDS).unwrap();
    assert_eq!(pending(&q).len(), 1000);
}

#[test]
fn inactive_and_sticky_revoked_grants_produce_no_discovery_work() {
    let f = Fixture::new();
    let q = f.queue();
    let at = now().unwrap();
    sql(&q, "UPDATE app_execution_grants SET enabled=0");
    assert!(q.claim_discovery_at(at).unwrap().is_none());
    sql(
        &q,
        "UPDATE app_execution_grants SET enabled=1; INSERT INTO app_repository_blocks(app_id,installation_id,repository_id,reason,changed_at_s) VALUES(1,2,3,'revoked',1)",
    );
    assert!(q.claim_discovery_at(at + 1).unwrap().is_none());
}

#[test]
fn malformed_pages_never_advance_checkpoint_or_admit_partial_numbers() {
    let f = Fixture::new();
    let q = f.queue();
    let at = now().unwrap();
    let work = q.claim_discovery_at(at).unwrap().unwrap();
    assert!(matches!(
        store(&q, &work, &[1, 1, 2], at),
        Err(SetupError::Invalid)
    ));
    assert!(pending(&q).is_empty());
    q.defer_discovery(&work, &SetupError::Invalid, at).unwrap();
    let retry = q.claim_discovery_at(at + RETRY_SECONDS).unwrap().unwrap();
    assert_eq!(retry.page, 1);
    store(&q, &retry, &[1, 2], at + RETRY_SECONDS).unwrap();
    assert_eq!(pending(&q), vec![1, 2]);
}

#[test]
fn repository_and_installation_turns_are_fair_even_for_unusable_grants() {
    let f = Fixture::new();
    let q = f.queue();
    // The copied grants intentionally have no matching exact publication
    // consent. They exercise denied candidate fairness without provider IO.
    sql(&q,"INSERT INTO app_setup_repositories(app_id,installation_id,repository_id,owner,name,selected_by_user_id,selected_at_s) SELECT app_id,2,5,owner,name,selected_by_user_id,selected_at_s FROM app_setup_repositories WHERE repository_id=3;
        INSERT INTO app_setup_repositories(app_id,installation_id,repository_id,owner,name,selected_by_user_id,selected_at_s) SELECT app_id,4,7,owner,name,selected_by_user_id,selected_at_s FROM app_setup_repositories WHERE repository_id=3;
        INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) SELECT app_id,2,5,revision,actor_id,confirmed_at_s,document FROM app_setup_scopes WHERE repository_id=3;
        INSERT INTO app_setup_scopes(app_id,installation_id,repository_id,revision,actor_id,confirmed_at_s,document) SELECT app_id,4,7,revision,actor_id,confirmed_at_s,document FROM app_setup_scopes WHERE repository_id=3;
        INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) SELECT app_id,2,5,scope_revision,actor_id,accepted_at_s,context FROM app_architecture_acceptances WHERE repository_id=3;
        INSERT INTO app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision,actor_id,accepted_at_s,context) SELECT app_id,4,7,scope_revision,actor_id,accepted_at_s,context FROM app_architecture_acceptances WHERE repository_id=3;
        INSERT INTO app_execution_grants(app_id,installation_id,repository_id,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy,enabled) SELECT app_id,2,5,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy,1 FROM app_execution_grants WHERE repository_id=3;
        INSERT INTO app_execution_grants(app_id,installation_id,repository_id,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy,enabled) SELECT app_id,4,7,scope_revision,consent_digest,engine_digest,engine_metadata_digest,policy,1 FROM app_execution_grants WHERE repository_id=3;");
    let at = now().unwrap();
    let first = q.claim_discovery_at(at).unwrap().unwrap();
    assert_eq!((first.installation, first.repository), (2, 3));
    assert!(q.claim_discovery_at(at).unwrap().is_none());
    assert!(q.claim_discovery_at(at).unwrap().is_none());
    let storage = q.storage.lock().unwrap();
    let turns: (i64,i64) = storage.connection.query_row("SELECT (SELECT turn FROM app_pr_discovery_cursors WHERE installation_id=4 AND repository_id=7),(SELECT turn FROM app_pr_discovery_cursors WHERE installation_id=2 AND repository_id=5)",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(turns.0 < turns.1); // Other installation before second repository.
}

#[test]
fn repeated_inventory_leaves_tracked_jobs_to_repair_but_rediscovers_drafts() {
    let f = Fixture::new();
    let q = f.queue();
    let provider = Provider(f.request.input.comparison.clone());
    q.reconcile(2, 3, 7, &provider).unwrap();
    let mut comparison = f.request.input.comparison.clone();
    comparison.draft = true;
    let draft = Provider(comparison);
    assert!(q.reconcile(2, 3, 8, &draft).unwrap().is_none());
    let at = now().unwrap();
    let first = q.claim_discovery_at(at).unwrap().unwrap();
    store(&q, &first, &[7, 8], at).unwrap();
    assert_eq!(pending(&q), vec![8]);
    let work = q.claim_discovery_at(at + 1).unwrap().unwrap();
    assert_eq!(work.number, Some(8));
    assert!(q.reconcile(2, 3, 8, &draft).unwrap().is_none());
    q.finish_discovery_number(&work, at + 1).unwrap();
    let end = q.claim_discovery_at(at + 2).unwrap().unwrap();
    store(&q, &end, &[], at + 2).unwrap();
    let repeat = q
        .claim_discovery_at(at + 2 + SWEEP_SECONDS)
        .unwrap()
        .unwrap();
    store(&q, &repeat, &[7, 8], at + 2 + SWEEP_SECONDS).unwrap();
    assert_eq!(pending(&q), vec![8]);
}

#[test]
fn jobs_under_a_previous_engine_cannot_suppress_initial_discovery() {
    let f = Fixture::new();
    let q = f.queue();
    let mut old = f.request.input.clone();
    old.engine_digest = "d".repeat(64);
    q.storage.lock().unwrap().connection.execute(
        "INSERT INTO app_analysis_jobs(id,app_id,installation_id,repository_id,pr_number,consent_digest,input,state,created_at,available_at) VALUES(?1,1,2,3,7,?2,?3,'pending',1,1)",
        params![old.id().unwrap(),old.consent_digest,serde_json::to_string(&old).unwrap()],
    ).unwrap();
    let at = now().unwrap();
    let work = q.claim_discovery_at(at).unwrap().unwrap();
    store(&q, &work, &[7], at).unwrap();
    assert_eq!(pending(&q), vec![7]);
}
