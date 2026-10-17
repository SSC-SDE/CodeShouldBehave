use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "flowdiff-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q"]);
        git(&path, &["config", "user.name", "Flowdiff test"]);
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
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn run(repo: &Repo, output: &Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_flowdiff"))
        .args(["review", "--repo"])
        .arg(&repo.0)
        .args(["--base", "HEAD~1", "--output"])
        .arg(output)
        .args(extra)
        .output()
        .unwrap()
}

#[cfg(feature = "server")]
fn confirmed_context(repo: &Repo, commit: &str) -> serde_json::Value {
    use flowdiff::{PathScope, RepositoryId, ScopeRecord, ScopeVersion, SecurityDomain};
    use sha2::{Digest, Sha256};
    let scope = ScopeRecord {
        version: ScopeVersion::new("onboarding-test").unwrap(),
        primary_repository: RepositoryId::new(
            SecurityDomain::new("local").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap(),
        primary_paths: PathScope::new(vec!["src".into()], vec!["src/rejected".into()]).unwrap(),
        contexts: vec![],
        contracts: Default::default(),
        export_allowed: false,
        model_destinations: Default::default(),
    };
    let mut hash = Sha256::new();
    hash.update(b"csb-accepted-context-v1:sensitive-path-policy-v1");
    hash.update(scope.canonical_bytes());
    let fingerprint = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let value = serde_json::json!({"version":"csb-accepted-context-v1","session_id":"confirmed-test","session_revision":7,
        "repository":{"security_domain":"local","provider":"github","owner":"owner","name":"repo"},
        "commit":commit,"input_fingerprint":"a".repeat(64),"proposal_fingerprint":"b".repeat(64),
        "accepted_scope_fingerprint":fingerprint,"scope_version":"onboarding-test","export_allowed":false,
        "include":["src"],"exclude":["src/rejected"],"contexts":[],"contracts":[],"sensitive_paths_excluded":true,
        "usable":true,"confirmation":{"actor":"github-user:99","confirmed_at_ms":1234,"input_fingerprint":"a".repeat(64),
        "proposal_fingerprint":"b".repeat(64),"known_gaps_accepted":true}});
    fs::write(repo.0.join("accepted.json"), value.to_string()).unwrap();
    value
}

#[cfg(feature = "server")]
#[test]
fn drift_handoff_binds_selected_proposal_and_excludes_unapproved_evidence() {
    let repo = Repo::new();
    fs::create_dir_all(repo.0.join("src")).unwrap();
    fs::write(repo.0.join("src/api.rs"), "pub fn run() { old(); }\n").unwrap();
    repo.commit("before");
    let baseline = git(&repo.0, &["rev-parse", "HEAD"]);
    fs::write(repo.0.join("src/api.rs"), "pub fn run() { new(); }\n").unwrap();
    repo.commit("after");
    let proposal = git(&repo.0, &["rev-parse", "HEAD"]);
    let mut context = confirmed_context(&repo, &baseline);
    context["drift"] = serde_json::json!({"version":"csb-onboarding-drift-v1",
        "session_id":context["session_id"],"session_revision":context["session_revision"],
        "source_commit":baseline,"target_commit":proposal,"input_fingerprint":context["input_fingerprint"],
        "proposal_fingerprint":context["proposal_fingerprint"],"accepted_scope_fingerprint":context["accepted_scope_fingerprint"],
        "status":"unchanged","reuse_allowed":true,"reconfirmation_required":false,
        "affected_components":[],"gaps":["Not PR impact or runtime proof"],"defaults_changed":false,"scope_expanded":false});
    let context_path = repo.0.join("accepted.json");
    let invoke = |context: &serde_json::Value, index: usize| {
        fs::write(&context_path, context.to_string()).unwrap();
        let artifact = repo.0.join(format!("drift-{index}.json"));
        let output = repo.0.join(format!("drift-{index}.html"));
        let result = run(
            &repo,
            &output,
            &[
                "--github-repository",
                "owner/repo",
                "--onboarding-context",
                context_path.to_str().unwrap(),
                "--internal-artifact",
                "--artifact",
                artifact.to_str().unwrap(),
            ],
        );
        (result, artifact, output)
    };
    let (result, artifact, _) = invoke(&context, 0);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(artifact).unwrap()).unwrap();
    assert_eq!(saved["onboarding_context"]["drift"], context["drift"]);
    let mut changed = context.clone();
    changed["drift"]["status"] = "changed".into();
    changed["drift"]["reuse_allowed"] = false.into();
    changed["drift"]["reconfirmation_required"] = true.into();
    changed["drift"]["affected_components"] = serde_json::json!([{"component_id":"api","evidence":[{
        "repository_index":0,"path":"src/api.rs","blob":git(&repo.0,&["rev-parse","HEAD:src/api.rs"]),
        "side":"target","commit":proposal,"start_line":1,"end_line":1,"producer_version":"csb-onboarding-drift-v1"}]}]);
    let (result, artifact, _) = invoke(&changed, 4);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let changed_artifact: serde_json::Value =
        serde_json::from_slice(&fs::read(artifact).unwrap()).unwrap();
    assert_ne!(saved["review_id"], changed_artifact["review_id"]);
    let mut wrong_target = context.clone();
    wrong_target["drift"]["target_commit"] = baseline.clone().into();
    let mut expanded = context.clone();
    expanded["drift"]["scope_expanded"] = true.into();
    let mut leak = changed;
    leak["drift"]["affected_components"][0]["evidence"][0]["path"] =
        "src/rejected/private.rs".into();
    for (index, invalid) in [wrong_target, expanded, leak].iter().enumerate() {
        let (result, artifact, output) = invoke(invalid, index + 1);
        assert!(!result.status.success());
        assert!(!artifact.exists());
        assert!(!output.exists());
    }
}

#[cfg(feature = "server")]
#[test]
fn accepted_context_filters_future_pr_files_and_preserves_interpretation_identity() {
    let repo = Repo::new();
    fs::create_dir_all(repo.0.join("src/rejected")).unwrap();
    fs::create_dir_all(repo.0.join("outside")).unwrap();
    fs::write(repo.0.join("src/accepted.rs"), "fn run() { old(); }\n").unwrap();
    repo.commit("accepted baseline");
    let baseline = git(&repo.0, &["rev-parse", "HEAD"]);
    let mut context = confirmed_context(&repo, &baseline);
    fs::write(repo.0.join("src/accepted.rs"), "fn run() { new(); }\n").unwrap();
    fs::write(
        repo.0.join("src/rejected/no.rs"),
        "fn EXCLUDED_SENTINEL() {}\n",
    )
    .unwrap();
    fs::write(repo.0.join("outside/no.rs"), "fn OUTSIDE_SENTINEL() {}\n").unwrap();
    fs::write(repo.0.join("src/.env"), "TOKEN=SECRET_SENTINEL\n").unwrap();
    fs::write(repo.0.join("src/key.pem"), "SECRET_SENTINEL\n").unwrap();
    repo.commit("new proposal with excluded paths");
    let proposal = git(&repo.0, &["rev-parse", "HEAD"]);
    let context_path = repo.0.join("accepted.json");
    let artifact = repo.0.join("review.json");
    let result = run(
        &repo,
        &repo.0.join("review.html"),
        &[
            "--github-repository",
            "owner/repo",
            "--onboarding-context",
            context_path.to_str().unwrap(),
            "--internal-artifact",
            "--artifact",
            artifact.to_str().unwrap(),
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let serialized = fs::read_to_string(artifact).unwrap();
    let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(value["scope_manifest"]["export_allowed"], false);
    assert_eq!(value["proposal"], proposal);
    assert_eq!(value["onboarding_context"]["accepted_commit"], baseline);
    assert_eq!(
        value["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["src/accepted.rs"]
    );
    for sentinel in [
        "EXCLUDED_SENTINEL",
        "OUTSIDE_SENTINEL",
        "SECRET_SENTINEL",
        "src/rejected/no.rs",
        "outside/no.rs",
        "src/.env",
        "src/key.pem",
    ] {
        assert!(!serialized.contains(sentinel), "leaked {sentinel}");
    }
    context["proposal_fingerprint"] = serde_json::json!("c".repeat(64));
    context["confirmation"]["proposal_fingerprint"] = context["proposal_fingerprint"].clone();
    fs::write(&context_path, context.to_string()).unwrap();
    let other = repo.0.join("other.json");
    assert!(
        run(
            &repo,
            &repo.0.join("other.html"),
            &[
                "--github-repository",
                "owner/repo",
                "--onboarding-context",
                context_path.to_str().unwrap(),
                "--internal-artifact",
                "--artifact",
                other.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    let other: serde_json::Value = serde_json::from_slice(&fs::read(other).unwrap()).unwrap();
    assert_ne!(value["review_id"], other["review_id"]);
    let export = run(
        &repo,
        &repo.0.join("export.md"),
        &[
            "--format",
            "markdown",
            "--github-repository",
            "owner/repo",
            "--onboarding-context",
            context_path.to_str().unwrap(),
        ],
    );
    assert!(!export.status.success());
    assert!(!repo.0.join("export.md").exists());
    assert!(String::from_utf8_lossy(&export.stderr).contains("Export"));
}

#[cfg(feature = "server")]
#[test]
fn accepted_context_rejects_changed_scope_receipts_and_unavailable_related_mirrors() {
    let repo = Repo::new();
    fs::write(repo.0.join("code.rs"), "fn run() {}\n").unwrap();
    repo.commit("before");
    let baseline = git(&repo.0, &["rev-parse", "HEAD"]);
    let context = confirmed_context(&repo, &baseline);
    fs::write(repo.0.join("code.rs"), "fn run() { next(); }\n").unwrap();
    repo.commit("after");
    let mutations = [
        ("include", serde_json::json!(["."])),
        ("usable", serde_json::json!(false)),
        ("commit", serde_json::json!("HEAD")),
        ("sensitive_paths_excluded", serde_json::json!(false)),
        ("contexts", serde_json::json!([{"repository":"other"}])),
    ];
    for (index, (key, changed)) in mutations.into_iter().enumerate() {
        let mut invalid = context.clone();
        invalid[key] = changed;
        let path = repo.0.join(format!("invalid-{index}.json"));
        fs::write(&path, invalid.to_string()).unwrap();
        let report = repo.0.join(format!("invalid-{index}.html"));
        let artifact = repo.0.join(format!("invalid-{index}-artifact.json"));
        let result = run(
            &repo,
            &report,
            &[
                "--github-repository",
                "owner/repo",
                "--onboarding-context",
                path.to_str().unwrap(),
                "--internal-artifact",
                "--artifact",
                artifact.to_str().unwrap(),
            ],
        );
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("requires confirmed context"));
        assert!(!report.exists());
    }
}

#[cfg(feature = "server")]
#[test]
fn offline_check_draft_keeps_real_commit_identity_and_does_not_claim_hld_lld() {
    let repo = Repo::new();
    fs::write(repo.0.join("code.rs"), "fn run() { old(); }\n").unwrap();
    repo.commit("before");
    let baseline = git(&repo.0, &["rev-parse", "HEAD"]);
    fs::write(repo.0.join("code.rs"), "fn run() { new(); }\n").unwrap();
    repo.commit("after");
    let proposal = git(&repo.0, &["rev-parse", "HEAD"]);
    let report = repo.0.join("review.md");
    let check = repo.0.join("check.json");
    let artifact = repo.0.join("artifact.json");
    let extra = [
        "--format",
        "markdown",
        "--github-repository",
        "owner/repo",
        "--pr-number",
        "35",
        "--dashboard-origin",
        "https://csb.example.com",
        "--github-check-draft",
        check.to_str().unwrap(),
        "--artifact",
        artifact.to_str().unwrap(),
    ];
    let output = run(&repo, &report, &extra);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&fs::read(&check).unwrap()).unwrap();
    let evidence: serde_json::Value =
        serde_json::from_slice(&fs::read(&artifact).unwrap()).unwrap();
    assert_eq!(evidence["selected_baseline"], baseline);
    assert_eq!(payload["head_sha"], proposal);
    assert_eq!(payload["conclusion"], "neutral");
    assert_eq!(
        payload["details_url"],
        format!(
            "https://csb.example.com/workspace#review=pr-35-{}&layout=architecture&side=diff",
            evidence["review_id"].as_str().unwrap()
        )
    );
    let serialized = payload.to_string();
    assert!(!serialized.contains("code.rs"));
    assert!(!serialized.contains("fn run"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("no check was published"));
    let untouched = fs::read(&check).unwrap();
    assert!(
        !run(&repo, &repo.0.join("other.md"), &extra)
            .status
            .success()
    );
    assert_eq!(fs::read(&check).unwrap(), untouched);
}

#[test]
fn reviews_real_commits_without_running_diff_helpers_or_touching_worktree() {
    let repo = Repo::new();
    fs::write(repo.0.join("code.rs"), "fn run() {\n old();\n}\n").unwrap();
    repo.commit("before");
    fs::write(
        repo.0.join("code.rs"),
        "fn run() {\n if ready() { new(); }\n}\n",
    )
    .unwrap();
    // Literal pathspecs must not interpret this filename as a pattern.
    fs::write(repo.0.join(":(glob)*.rs"), "fn added() {}\n").unwrap();
    fs::write(repo.0.join("binary.bin"), [0, 1, 2, 3]).unwrap();
    repo.commit("after");
    fs::write(
        repo.0.join("code.rs"),
        "uncommitted content must stay untouched",
    )
    .unwrap();
    git(&repo.0, &["config", "diff.external", "false"]);
    let out = repo.0.join("report.html");
    let result = run(&repo, &out, &[]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = fs::read_to_string(out).unwrap();
    assert!(report.contains("ready()"));
    assert!(report.contains("Binary content omitted"));
    assert!(!report.contains("uncommitted content must stay untouched"));
    assert_eq!(
        fs::read_to_string(repo.0.join("code.rs")).unwrap(),
        "uncommitted content must stay untouched"
    );
}

#[test]
fn refuses_overwrite_and_invalid_refs() {
    let repo = Repo::new();
    fs::write(repo.0.join("a.rs"), "fn a() {}\n").unwrap();
    repo.commit("a");
    fs::write(repo.0.join("a.rs"), "fn b() {}\n").unwrap();
    repo.commit("b");
    let out = repo.0.join("existing");
    fs::write(&out, "keep me").unwrap();
    assert!(!run(&repo, &out, &[]).status.success());
    assert_eq!(fs::read_to_string(&out).unwrap(), "keep me");
    assert!(
        !run(&repo, &repo.0.join("bad.html"), &["--base", "--help"])
            .status
            .success()
    );
    assert!(!repo.0.join("bad.html").exists());
}

#[test]
fn merge_base_excludes_target_branch_only_changes() {
    let repo = Repo::new();
    fs::write(repo.0.join("a.rs"), "fn a() {}\n").unwrap();
    repo.commit("root");
    let root = git(&repo.0, &["rev-parse", "HEAD"]);
    fs::write(repo.0.join("target.rs"), "fn target_only() {}\n").unwrap();
    repo.commit("target");
    let target = git(&repo.0, &["rev-parse", "HEAD"]);
    git(&repo.0, &["checkout", "--detach", &root]);
    fs::write(repo.0.join("pr.rs"), "fn feature() {}\n").unwrap();
    repo.commit("feature");
    let out = repo.0.join("review.md");
    let result = run(
        &repo,
        &out,
        &["--base", &target, "--merge-base", "--format", "markdown"],
    );
    assert!(result.status.success());
    let report = fs::read_to_string(out).unwrap();
    assert!(report.contains("pr.rs"));
    assert!(!report.contains("target.rs"));
}

#[test]
fn context_reads_only_selected_committed_paths_across_sources() {
    let repo = Repo::new();
    fs::write(repo.0.join("api.rs"), "pub fn checkout() { old(); }\n").unwrap();
    repo.commit("before");
    fs::write(repo.0.join("api.rs"), "pub fn checkout() { new(); }\n").unwrap();
    repo.commit("after");
    let consumer = Repo::new();
    fs::create_dir_all(consumer.0.join("src/private")).unwrap();
    fs::create_dir_all(consumer.0.join("src-other")).unwrap();
    fs::create_dir_all(consumer.0.join("contracts")).unwrap();
    fs::write(
        consumer.0.join("src/client.rs"),
        "pub fn order() { checkout(); }\n",
    )
    .unwrap();
    fs::write(
        consumer.0.join("src/private/key.rs"),
        "EXCLUDED_SECRET_SENTINEL",
    )
    .unwrap();
    fs::write(
        consumer.0.join("src-other/hidden.rs"),
        "OUTSIDE_SCOPE_SENTINEL",
    )
    .unwrap();
    fs::write(
        consumer.0.join("contracts/api.md"),
        "CONTRACT_EVIDENCE_SENTINEL\n</textarea><script>bad()</script>",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("private/key.rs", consumer.0.join("src/link.rs")).unwrap();
    consumer.commit("context snapshot");
    fs::write(
        consumer.0.join("src/client.rs"),
        "UNCOMMITTED_CONTEXT_SENTINEL",
    )
    .unwrap();
    let scope = repo.0.join("review.scope");
    fs::write(&scope, format!("repository\tConsumer\t{}\tHEAD\tsrc/\tsrc/private\nservice\tContract\t{}\tHEAD\tcontracts/api.md\t-\n",consumer.0.display(),consumer.0.display())).unwrap();
    let output = repo.0.join("context.html");
    let result = run(&repo, &output, &["--context", scope.to_str().unwrap()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = fs::read_to_string(output).unwrap();
    assert!(report.contains("possible caller of checkout()"));
    assert!(report.contains("CONTRACT_EVIDENCE_SENTINEL"));
    assert!(!report.contains("EXCLUDED_SECRET_SENTINEL"));
    assert!(!report.contains("OUTSIDE_SCOPE_SENTINEL"));
    assert!(!report.contains("UNCOMMITTED_CONTEXT_SENTINEL"));
    assert!(!report.contains("<script>bad()"));
    assert!(report.contains("symlink or submodule omitted") || !cfg!(unix));
    let md_output = repo.0.join("context.md");
    assert!(
        run(
            &repo,
            &md_output,
            &["--context", scope.to_str().unwrap(), "--format", "markdown"]
        )
        .status
        .success()
    );
    let markdown = fs::read_to_string(md_output).unwrap();
    assert!(markdown.contains("### Consumer"));
    assert!(markdown.contains("### Contract"));
    assert!(!markdown.contains("EXCLUDED_SECRET_SENTINEL"));
}

#[test]
fn invalid_context_fails_closed_without_writing_report() {
    let repo = Repo::new();
    fs::write(repo.0.join("a.rs"), "fn a() {}\n").unwrap();
    repo.commit("before");
    fs::write(repo.0.join("a.rs"), "fn b() {}\n").unwrap();
    repo.commit("after");
    let scope = repo.0.join("review.scope");
    fs::write(&scope, "repository\tInvalid\t.\tHEAD\t../outside\t-\n").unwrap();
    let output = repo.0.join("context.html");
    assert!(
        !run(&repo, &output, &["--context", scope.to_str().unwrap()])
            .status
            .success()
    );
    assert!(!output.exists());
}

#[test]
fn architecture_index_reuses_cache_and_finds_unchanged_callers_against_exact_pr_ref() {
    let repo = Repo::new();
    fs::create_dir_all(repo.0.join("src")).unwrap();
    fs::create_dir_all(repo.0.join("clients")).unwrap();
    fs::write(repo.0.join("src/library.rs"), "pub fn old_api() {}\n").unwrap();
    fs::write(
        repo.0.join("clients/unchanged.rs"),
        "pub fn caller() { old_api(); }\n",
    )
    .unwrap();
    repo.commit("baseline");
    git(
        &repo.0,
        &["update-ref", "refs/remotes/origin/pr/12", "HEAD"],
    );
    let cache = repo.0.join("cache");
    let baseline = repo.0.join("baseline.html");
    let first = Command::new(env!("CARGO_BIN_EXE_flowdiff"))
        .args(["index", "--repo"])
        .arg(&repo.0)
        .args(["--baseline", "refs/remotes/origin/pr/12", "--cache-dir"])
        .arg(&cache)
        .arg("--output")
        .arg(&baseline)
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("2 blobs analyzed, 0 cached"));
    fs::write(repo.0.join("src/library.rs"), "pub fn new_api() {}\n").unwrap();
    repo.commit("proposal");
    let output = repo.0.join("comparison.html");
    let run_review = |output: &Path| {
        Command::new(env!("CARGO_BIN_EXE_flowdiff"))
            .args(["review", "--repo"])
            .arg(&repo.0)
            .args([
                "--baseline",
                "refs/remotes/origin/pr/12",
                "--proposal",
                "HEAD",
                "--cache-dir",
            ])
            .arg(&cache)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap()
    };
    let reviewed = run_review(&output);
    assert!(
        reviewed.status.success(),
        "{}",
        String::from_utf8_lossy(&reviewed.stderr)
    );
    assert!(String::from_utf8_lossy(&reviewed.stdout).contains("1 blobs analyzed, 3 cached"));
    let html = fs::read_to_string(&output).unwrap();
    assert!(html.contains("clients/unchanged.rs"));
    assert!(
        html.contains(
            "&quot;name&quot;:&quot;old_api&quot;,&quot;status&quot;:&quot;removed&quot;"
        )
    );
    let repeated = run_review(&repo.0.join("repeat.html"));
    assert!(repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stdout).contains("0 blobs analyzed, 4 cached"));
}

#[test]
fn repository_reader_reports_unusual_binary_oversized_and_symlink_entries() {
    let repo = Repo::new();
    fs::write(repo.0.join("line\nbreak.rs"), "fn unusual() {}\n").unwrap();
    fs::write(repo.0.join("binary.rs"), [0, 1, 2, 3]).unwrap();
    fs::write(repo.0.join("oversized.rs"), vec![b'x'; 512 * 1024 + 1]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("line\nbreak.rs", repo.0.join("linked.rs")).unwrap();
    repo.commit("reader edge cases");
    let output = repo.0.join("reader.html");
    let cache = repo.0.join("cache");
    let result = Command::new(env!("CARGO_BIN_EXE_flowdiff"))
        .args(["index", "--repo"])
        .arg(&repo.0)
        .args(["--baseline", "HEAD", "--cache-dir"])
        .arg(&cache)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = fs::read_to_string(output).unwrap();
    assert!(report.contains("binary symbols omitted"));
    assert!(report.contains("exceeds the 512 KiB reader limit; deferred"));
    assert!(report.contains("line#10;break.rs") || report.contains("line\\nbreak.rs"));
    if cfg!(unix) {
        assert!(report.contains("symlink/submodule not indexed"));
    }
}

#[test]
fn repository_reader_resumes_cached_chunks_after_a_missing_object() {
    let repo = Repo::new();
    for index in 0..140 {
        fs::write(
            repo.0.join(format!("file-{index:03}.rs")),
            format!("fn function_{index:03}() {{}}\n"),
        )
        .unwrap();
    }
    repo.commit("batch corpus");
    let missing_oid = git(&repo.0, &["rev-parse", "HEAD:file-139.rs"]);
    let object_path = repo
        .0
        .join(".git/objects")
        .join(&missing_oid[..2])
        .join(&missing_oid[2..]);
    let saved_object = repo.0.join("saved-object");
    fs::rename(&object_path, &saved_object).unwrap();

    let cache = repo.0.join("cache");
    let first_output = repo.0.join("partial.html");
    let first = Command::new(env!("CARGO_BIN_EXE_flowdiff"))
        .args(["index", "--repo"])
        .arg(&repo.0)
        .args(["--baseline", "HEAD", "--cache-dir"])
        .arg(&cache)
        .arg("--output")
        .arg(&first_output)
        .output()
        .unwrap();
    assert!(first.status.success());
    assert!(
        fs::read_to_string(first_output)
            .unwrap()
            .contains("Git object is missing; indexing failed")
    );

    fs::rename(&saved_object, &object_path).unwrap();
    let resumed_output = repo.0.join("resumed.html");
    let resumed = Command::new(env!("CARGO_BIN_EXE_flowdiff"))
        .args(["index", "--repo"])
        .arg(&repo.0)
        .args(["--baseline", "HEAD", "--cache-dir"])
        .arg(&cache)
        .arg("--output")
        .arg(&resumed_output)
        .output()
        .unwrap();
    assert!(resumed.status.success());
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("1 blobs analyzed, 139 cached"));
}

#[test]
fn mirror_read_lease_pins_and_releases_exact_commits() {
    let repo = Repo::new();
    fs::write(repo.0.join("leased.rs"), "fn leased() {}\n").unwrap();
    repo.commit("leased commit");
    let head = git(&repo.0, &["rev-parse", "HEAD"]);
    {
        let _lease =
            flowdiff::MirrorReadLease::acquire(&repo.0, std::slice::from_ref(&head)).unwrap();
        let refs = git(
            &repo.0,
            &[
                "for-each-ref",
                "--format=%(objectname)",
                "refs/codeshouldbehave/read-leases/",
            ],
        );
        assert_eq!(refs, head);
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo.0)
        .args([
            "for-each-ref",
            "--format=%(objectname)",
            "refs/codeshouldbehave/read-leases/",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}

#[cfg(feature = "server")]
#[test]
fn typed_artifact_binds_exact_commits_blobs_deltas_and_excludes_sensitive_paths() {
    let repo = Repo::new();
    fs::write(repo.0.join("a.rs"), "pub fn run() { old(); }\n").unwrap();
    fs::write(repo.0.join("removed.rs"), "pub fn gone() {}\n").unwrap();
    fs::write(repo.0.join(".env.example"), "SECRET=BEFORE_SECRET_SENTINEL").unwrap();
    repo.commit("base");
    let base = git(&repo.0, &["rev-parse", "HEAD"]);
    fs::write(repo.0.join("a.rs"), "pub fn run() { new(); }\n").unwrap();
    fs::remove_file(repo.0.join("removed.rs")).unwrap();
    fs::write(repo.0.join("added.rs"), "pub fn added() {}\n").unwrap();
    fs::write(repo.0.join(".env.example"), "SECRET=AFTER_SECRET_SENTINEL").unwrap();
    repo.commit("head");
    let head = git(&repo.0, &["rev-parse", "HEAD"]);
    fs::write(repo.0.join("a.rs"), "WORKTREE_SENTINEL").unwrap();
    let artifact = repo.0.join("artifact.json");
    let out = repo.0.join("review.html");
    let result = run(&repo, &out, &["--artifact", artifact.to_str().unwrap()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = fs::read_to_string(artifact).unwrap();
    let value: serde_json::Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(value["artifact_version"], "csb-review-artifact-v1");
    assert_eq!(value["baseline"], base);
    assert_eq!(value["proposal"], head);
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert!(files.iter().any(|file| file["path"] == "added.rs"
        && file["kind"] == "added"
        && file["additions"] == 1));
    assert!(files.iter().any(|file| file["path"] == "removed.rs"
        && file["kind"] == "deleted"
        && file["deletions"] == 1));
    let node = value["architecture"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["path"] == "a.rs")
        .unwrap();
    assert_eq!(node["blobAfter"], git(&repo.0, &["rev-parse", "HEAD:a.rs"]));
    assert_eq!(value["hld_status"], "unavailable");
    for forbidden in [
        "BEFORE_SECRET_SENTINEL",
        "AFTER_SECRET_SENTINEL",
        "WORKTREE_SENTINEL",
        ".env.example",
    ] {
        assert!(!bytes.contains(forbidden));
    }
    assert!(value["coverage"]["excluded"].as_u64().unwrap() > 0);
}
