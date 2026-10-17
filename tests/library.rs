use flowdiff::{
    Action, Cancellation, ComparisonMode, Grant, PathScope, PrincipalId, RepositoryId,
    ReviewRequest, ScopeRecord, ScopeVersion, SecurityDomain, analyze_review,
    prepare_local_context_access, render_markdown,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

fn access() -> (PrincipalId, ScopeRecord, Vec<Grant>) {
    access_for("library-test-domain", PathScope::all())
}

fn access_for(domain: &str, paths: PathScope) -> (PrincipalId, ScopeRecord, Vec<Grant>) {
    let principal = PrincipalId::new("library-test-user").unwrap();
    let repository = RepositoryId::new(
        SecurityDomain::new(domain).unwrap(),
        "local-git",
        "tests",
        "fixture",
    )
    .unwrap();
    let scope = ScopeRecord {
        version: ScopeVersion::new("library-test-v1").unwrap(),
        primary_repository: repository.clone(),
        primary_paths: paths,
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: false,
        model_destinations: BTreeSet::new(),
    };
    let grants = vec![Grant {
        principal: principal.clone(),
        repository,
        actions: BTreeSet::from([Action::Index, Action::Source]),
        paths: PathScope::all(),
    }];
    (principal, scope, grants)
}

fn request(
    repo: &Repo,
    baseline: &str,
    proposal: &str,
    cache_dir: PathBuf,
    access: (PrincipalId, ScopeRecord, Vec<Grant>),
) -> ReviewRequest {
    ReviewRequest {
        repository: repo.0.clone(),
        baseline: baseline.into(),
        proposal: proposal.into(),
        comparison: ComparisonMode::Exact,
        principal: access.0,
        scope: access.1,
        current_grants: access.2,
        context_manifest: None,
        cache_dir,
    }
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Repo(PathBuf);

impl Repo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "flowdiff-library-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q"]);
        git(&path, &["config", "user.name", "Flowdiff library test"]);
        git(&path, &["config", "user.email", "test@example.invalid"]);
        Self(path)
    }

    fn commit(&self, message: &str) {
        git(&self.0, &["add", "--all"]);
        git(
            &self.0,
            &["-c", "core.hooksPath=/dev/null", "commit", "-qm", message],
        );
    }
}

#[test]
fn scope_blocks_excluded_source_before_review_and_cache_publication() {
    let repo = Repo::new();
    fs::create_dir_all(repo.0.join("src/private")).unwrap();
    fs::write(repo.0.join("src/allowed.rs"), "fn allowed() { old(); }\n").unwrap();
    fs::write(
        repo.0.join("src/private/secret.rs"),
        "fn EXCLUDED_SENTINEL() { old(); }\n",
    )
    .unwrap();
    repo.commit("baseline");
    let baseline = git(&repo.0, &["rev-parse", "HEAD"]);
    fs::write(repo.0.join("src/allowed.rs"), "fn allowed() { new(); }\n").unwrap();
    fs::write(
        repo.0.join("src/private/secret.rs"),
        "fn EXCLUDED_SENTINEL() { changed(); }\n",
    )
    .unwrap();
    repo.commit("proposal");
    let proposal = git(&repo.0, &["rev-parse", "HEAD"]);
    let access = access_for(
        "scope-test-domain",
        PathScope::new(vec!["src".into()], vec!["src/private".into()]).unwrap(),
    );
    let result = analyze_review(
        &request(&repo, &baseline, &proposal, repo.0.join("cache"), access),
        &Cancellation::default(),
    )
    .unwrap();

    let rendered = render_markdown(&result.review);
    assert!(rendered.contains("src/allowed.rs"));
    assert!(!rendered.contains("EXCLUDED_SENTINEL"));
    assert!(
        result
            .review
            .architecture
            .after
            .iter()
            .all(|node| node.path != "src/private/secret.rs")
    );
    assert!(result.coverage.excluded >= 2);
}

#[test]
fn security_domains_do_not_share_architecture_cache_records() {
    let repo = Repo::new();
    fs::write(repo.0.join("service.rs"), "fn service() {}\n").unwrap();
    repo.commit("snapshot");
    let commit = git(&repo.0, &["rev-parse", "HEAD"]);
    let cache = repo.0.join("cache");
    let first = analyze_review(
        &request(
            &repo,
            &commit,
            &commit,
            cache.clone(),
            access_for("tenant-a", PathScope::all()),
        ),
        &Cancellation::default(),
    )
    .unwrap();
    let second = analyze_review(
        &request(
            &repo,
            &commit,
            &commit,
            cache,
            access_for("tenant-b", PathScope::all()),
        ),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(first.review.architecture.analyzed, 1);
    assert_eq!(second.review.architecture.analyzed, 1);
    assert_eq!(second.review.architecture.reused, 0);
    assert_ne!(first.review_id, second.review_id);
}

#[test]
fn context_revision_must_match_the_authorized_pin() {
    let repo = Repo::new();
    fs::write(repo.0.join("service.rs"), "fn service() {}\n").unwrap();
    repo.commit("primary");
    let commit = git(&repo.0, &["rev-parse", "HEAD"]);

    let context = Repo::new();
    fs::write(context.0.join("client.rs"), "fn client() { service(); }\n").unwrap();
    context.commit("authorized context");
    let manifest = repo.0.join("review.scope");
    fs::write(
        &manifest,
        format!(
            "repository\tConsumer\t{}\tHEAD\t.\t-\n",
            context.0.display()
        ),
    )
    .unwrap();

    let (principal, mut scope, mut grants) = access_for("context-test", PathScope::all());
    let (pins, context_grants) = prepare_local_context_access(
        &manifest,
        scope.primary_repository.security_domain(),
        &principal,
    )
    .unwrap();
    scope.contexts = pins;
    grants.extend(context_grants);

    fs::write(
        context.0.join("client.rs"),
        "fn client() { changed_after_authorization(); }\n",
    )
    .unwrap();
    context.commit("moved context head");

    let mut review = request(
        &repo,
        &commit,
        &commit,
        repo.0.join("cache"),
        (principal, scope, grants),
    );
    review.context_manifest = Some(manifest);
    let result = analyze_review(&review, &Cancellation::default());
    assert!(matches!(result, Err(error) if error.contains("resolved commit does not match")));
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().into()
}

#[test]
fn library_analyzes_without_spawning_the_cli_or_scraping_stdout() {
    let repo = Repo::new();
    fs::write(repo.0.join("service.rs"), "fn serve() { old(); }\n").unwrap();
    repo.commit("baseline");
    let baseline = git(&repo.0, &["rev-parse", "HEAD"]);
    fs::write(
        repo.0.join("service.rs"),
        "fn serve() { if ready() { new(); } }\n",
    )
    .unwrap();
    repo.commit("proposal");
    let proposal = git(&repo.0, &["rev-parse", "HEAD"]);

    let (principal, scope, current_grants) = access();
    let result = analyze_review(
        &ReviewRequest {
            repository: repo.0.clone(),
            baseline: baseline.clone(),
            proposal: proposal.clone(),
            comparison: ComparisonMode::Exact,
            principal,
            scope,
            current_grants,
            context_manifest: None,
            cache_dir: repo.0.join("cache"),
        },
        &Cancellation::default(),
    )
    .unwrap();

    assert_eq!(result.baseline.as_str(), baseline);
    assert_eq!(result.proposal.as_str(), proposal);
    assert_eq!(result.scope_version.as_str(), "library-test-v1");
    assert!(result.coverage.enumeration_complete);
    assert_eq!(result.review.files.len(), 1);
    assert!(render_markdown(&result.review).contains("ready()"));
    assert!(result.authorize(Action::Export, &[], None).is_err());
}

#[test]
fn current_grant_paths_narrow_immutable_scope_before_source_reads() {
    let repo = Repo::new();
    fs::create_dir_all(repo.0.join("allowed")).unwrap();
    fs::create_dir_all(repo.0.join("revoked")).unwrap();
    fs::write(repo.0.join("allowed/code.rs"), "fn allowed() {}\n").unwrap();
    fs::write(
        repo.0.join("revoked/secret.rs"),
        "fn REVOKED_SENTINEL() {}\n",
    )
    .unwrap();
    repo.commit("snapshot");
    let commit = git(&repo.0, &["rev-parse", "HEAD"]);
    let (principal, scope, mut grants) = access_for("grant-test", PathScope::all());
    grants[0].paths = PathScope::new(vec!["allowed".into()], vec![]).unwrap();
    let result = analyze_review(
        &request(
            &repo,
            &commit,
            &commit,
            repo.0.join("cache"),
            (principal, scope, grants),
        ),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(result.review.architecture.after.len(), 1);
    assert_eq!(result.review.architecture.after[0].path, "allowed/code.rs");
    assert!(!render_markdown(&result.review).contains("REVOKED_SENTINEL"));
}

#[test]
fn scope_version_changes_identity_and_revocation_blocks_later_access() {
    let repo = Repo::new();
    fs::write(repo.0.join("service.rs"), "fn service() {}\n").unwrap();
    repo.commit("snapshot");
    let commit = git(&repo.0, &["rev-parse", "HEAD"]);
    let first_access = access_for("identity-test", PathScope::all());
    let first = analyze_review(
        &request(&repo, &commit, &commit, repo.0.join("cache"), first_access),
        &Cancellation::default(),
    )
    .unwrap();

    let (principal, mut scope, grants) = access_for("identity-test", PathScope::all());
    scope.version = ScopeVersion::new("scope-v2").unwrap();
    let second = analyze_review(
        &request(
            &repo,
            &commit,
            &commit,
            repo.0.join("cache"),
            (principal, scope, grants),
        ),
        &Cancellation::default(),
    )
    .unwrap();

    assert_ne!(first.review_id, second.review_id);
    assert!(
        first
            .authorize(Action::Source, &[], Some("service.rs"))
            .is_err()
    );
    assert!(first.authorize(Action::Graph, &[], None).is_err());
}

#[test]
fn cancelled_library_request_performs_no_repository_access() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let (principal, scope, current_grants) = access();
    let result = analyze_review(
        &ReviewRequest {
            repository: PathBuf::from("/path/that/must/not/be/read"),
            baseline: "HEAD".into(),
            proposal: "HEAD".into(),
            comparison: ComparisonMode::Exact,
            principal,
            scope,
            current_grants,
            context_manifest: None,
            cache_dir: PathBuf::from("/path/that/must/not/be/written"),
        },
        &cancellation,
    );
    assert!(matches!(result, Err(error) if error == "Analysis cancelled"));
}
