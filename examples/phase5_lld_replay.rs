//! Real deterministic HLD/LLD publication with separate lazy flow requests.
//! This local single-worker driver is not a hosted/concurrent-capacity claim.
#[cfg(not(feature = "server"))]
fn main() {
    eprintln!("phase5_lld_replay requires --features server");
    std::process::exit(2);
}

#[cfg(feature = "server")]
fn main() {
    if let Err(error) = replay::run() {
        eprintln!("Phase 5 HLD/LLD replay failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(feature = "server")]
mod replay {
    use flowdiff::design::{
        ConfirmedDesignContext, DesignDelta, DesignOutcome, DesignVersion, compare_hld,
        generate_hld,
    };
    use flowdiff::lld::{
        LLD_IR_VERSION, LldDelta, LldFunctionDetail, LldSnapshot, build_lld_detail, compare_lld,
        compare_lld_detail, generate_lld, lld_source,
    };
    use flowdiff::{
        Action, Cancellation, ClaimedJob, ComparisonMode, ComponentDecision, ComponentProposal,
        EvidenceConfidence, Grant, JobLane, PathScope, PrincipalId, RepositoryId, ReviewRequest,
        Scheduler, ScopeRecord, ScopeVersion, SecurityDomain, SnapshotId, Storage, analyze_review,
        authorize,
    };
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs::{self, OpenOptions},
        io::Write,
        path::{Path, PathBuf},
        process::Command,
        time::Instant,
    };
    const PRS: usize = 100;
    const REVISIONS: usize = 5;
    const BURST: usize = 20;
    const REPOSITORIES: usize = 5;
    const SEEDS: [u64; 3] = [104729, 130363, 155921];
    const SIMULATED_WINDOW_MS: u64 = 8 * 60 * 60 * 1000;
    #[derive(Clone, Serialize, Deserialize)]
    struct Case {
        pr: usize,
        repository: usize,
        baseline: String,
        heads: Vec<String>,
        root: String,
    }
    struct Job {
        case: Case,
        revision: usize,
        head: String,
        virtual_ms: u64,
    }
    #[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct Bundle {
        hld: DesignVersion,
        lld: LldSnapshot,
    }
    struct DurableQueue {
        storage: Storage,
        repositories: BTreeMap<usize, i64>,
        scopes: BTreeMap<usize, i64>,
        clock: Instant,
    }
    impl DurableQueue {
        fn new(root: &Path, cases: &[Case]) -> Result<Self, String> {
            let mut storage = Storage::open(&root.join("queue.db"), &root.join("artifacts"))?;
            let mut repositories = BTreeMap::new();
            let mut scopes = BTreeMap::new();
            for case in cases {
                let (scope, _, _, _) = request(case)?;
                let repository = storage.register_repository(
                    scope.primary_repository.security_domain().as_str(),
                    scope.primary_repository.provider(),
                    scope.primary_repository.owner(),
                    scope.primary_repository.name(),
                )?;
                let id = storage.register_scope(
                    repository,
                    scope.version.as_str(),
                    &scope.canonical_bytes(),
                )?;
                repositories.insert(case.pr, repository);
                scopes.insert(case.pr, id);
            }
            Ok(Self {
                storage,
                repositories,
                scopes,
                clock: Instant::now(),
            })
        }
        fn now(&self) -> u64 {
            1000 + self.clock.elapsed().as_millis() as u64
        }
        fn enqueue(
            &mut self,
            case: &Case,
            revision: usize,
            head: &str,
        ) -> Result<(String, Instant), String> {
            let now = self.now();
            let review = format!("lld-pr-{}-r{revision}-{}", case.pr, &head[..12]);
            let provider = format!("pr-{}", case.pr);
            let mut scheduler = Scheduler::new(&mut self.storage);
            scheduler.register_review(
                &review,
                self.repositories[&case.pr],
                self.scopes[&case.pr],
                &case.baseline,
                &case.baseline,
                head,
                "exact",
                now,
            )?;
            if !scheduler.observe_pr(
                self.repositories[&case.pr],
                &provider,
                revision as i64,
                &review,
                head,
                &case.baseline,
                now,
            )? {
                return Err("Durable event generation was unexpectedly duplicate/stale".into());
            }
            scheduler.enqueue(
                &review,
                "lld",
                100,
                JobLane::Warm,
                &provider,
                revision as i64,
                now,
            )?;
            Ok((review, Instant::now()))
        }
        fn claim(&mut self) -> Result<ClaimedJob, String> {
            let now = self.now();
            Scheduler::new(&mut self.storage)
                .claim("lld-real-replay-worker", JobLane::Warm, 1, 60_000, now)?
                .ok_or("No real HLD/LLD replay job was claimable".into())
        }
        fn publish(
            &mut self,
            job: &ClaimedJob,
            baseline: &Bundle,
            proposal: &Bundle,
            delta: &DesignDelta,
            lld_delta: &LldDelta,
        ) -> Result<String, String> {
            Scheduler::new(&mut self.storage).checkpoint(job)?;
            let payload=serde_json::to_vec(&json!({"version":"csb-lld-durable-replay-result-v1","review_id":job.review_id,"status":if supported_catalog(proposal){"catalog_published_local_details_pending"}else{"partial_catalog_published"},"baseline_hld":baseline.hld,"proposal_hld":proposal.hld,"hld_delta":delta,"baseline_lld_catalog":baseline.lld.catalog,"proposal_lld_catalog":proposal.lld.catalog,"lld_delta":lld_delta,"local_detail_generation":"Separate lazy requests after catalog publication; complete function bodies are private IR, never this result payload"})).map_err(|error|error.to_string())?;
            let artifact = self.storage.publish_artifact(
                proposal.hld.repository.security_domain().as_str(),
                "csb-lld-durable-replay-result-v1",
                &payload,
            )?;
            let now = self.now();
            Scheduler::new(&mut self.storage).complete(
                job,
                "lld-real-replay-worker",
                &artifact.digest,
                now,
            )?;
            let current = Scheduler::new(&mut self.storage).current_result(
                job.repository_id,
                &job.provider_pr_id,
                "lld",
            )?;
            if current.as_ref() != Some(&(job.review_id.clone(), artifact.digest.clone())) {
                return Err(
                    "Current result does not reference the actual typed HLD/LLD catalog artifact"
                        .into(),
                );
            }
            Ok(artifact.digest)
        }
        fn validate_current(&mut self, cases: &[Case]) -> Result<serde_json::Value, String> {
            let now = self.now();
            let stats = Scheduler::new(&mut self.storage).stats(now)?;
            if stats.queued != 0
                || stats.running != 0
                || stats.failed != 0
                || stats.total_attempts != 520
            {
                return Err("Durable HLD/LLD replay terminal queue oracle failed".into());
            }
            for case in cases {
                let revision = if case.pr < BURST { 6 } else { 5 };
                let head = &case.heads[revision - 1];
                let expected = format!("lld-pr-{}-r{revision}-{}", case.pr, &head[..12]);
                let current = Scheduler::new(&mut self.storage)
                    .current_result(
                        self.repositories[&case.pr],
                        &format!("pr-{}", case.pr),
                        "lld",
                    )?
                    .ok_or("Final PR has no published HLD result")?;
                if current.0 != expected {
                    return Err("Historical HLD result became current".into());
                }
            }
            Ok(
                json!({"queued":stats.queued,"running":stats.running,"failed":stats.failed,"actual_typed_artifact_completions":stats.total_attempts,"final_current_prs":cases.len()}),
            )
        }
    }
    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())
    }
    fn write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
        fs::create_dir_all(path.parent().ok_or("No parent")?).map_err(|error| error.to_string())?;
        fs::write(path, bytes).map_err(|error| error.to_string())
    }
    fn git(root: &Path, args: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .args([
                "--no-pager",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "protocol.allow=never",
            ])
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!("Synthetic Git {:?} failed", args.first()));
        }
        String::from_utf8(output.stdout)
            .map(|text| text.trim().into())
            .map_err(|error| error.to_string())
    }
    fn init(root: &Path) -> Result<(), String> {
        fs::create_dir(root).map_err(|error| error.to_string())?;
        git(root, &["init", "-q"])?;
        git(root, &["config", "user.name", "Synthetic HLD/LLD replay"])?;
        git(root, &["config", "user.email", "replay@example.invalid"])?;
        Ok(())
    }
    fn commit(root: &Path, message: &str) -> Result<String, String> {
        git(root, &["add", "--all"])?;
        git(root, &["commit", "-q", "-m", message])?;
        git(root, &["rev-parse", "HEAD"])
    }
    fn fixture(root: &Path, pr: usize, revision: usize) -> Result<(), String> {
        let folder = root.join(format!("pr-{pr:03}"));
        let dependency = if revision == 3 {
            "[dependencies]\nserde='1'\n"
        } else {
            ""
        };
        write(
            &folder.join("Cargo.toml"),
            format!("[package]\nname='fixture-pr-{pr}'\nversion='0.1.0'\n{dependency}"),
        )?;
        let ty = if revision == 2 { "i64" } else { "i32" };
        let body = match revision {
            1 => format!("if value >= {pr} {{ value }} else {{ 0 }}"),
            5 => "opaque!(value)".into(),
            6 => format!("while value > {pr} {{ return value; }} 0"),
            _ => format!("if value > {pr} {{ value }} else {{ 0 }}"),
        };
        let source = format!(
            "pub struct Request{pr} {{ pub id:{ty} }}\npub fn handle_{pr}(request:Request{pr})->{ty} {{ helper_{pr}(request.id as i32) as {ty} }}\nfn helper_{pr}(value:i32)->i32 {{ {body} }}\nstruct Left{pr};\nstruct Right{pr};\nimpl Left{pr} {{ fn same(&self)->i32 {{ 10 }} }}\nimpl Right{pr} {{ fn same(&self)->i32 {{ 20 }} }}\n"
        );
        let source = if revision == 4 {
            format!(
                "// Formatting-only fixture\n{}",
                source
                    .replace("pub id:", "pub id: ")
                    .replace(" { ", " {\n    ")
                    .replace(" }", "\n}")
            )
        } else {
            source
        };
        write(&folder.join("src/lib.rs"), source)?;
        write(
            &folder.join(".env"),
            "LLD_SECRET=CSB_LLD_REPLAY_NEVER_DISCLOSE",
        )?;
        write(
            &folder.join("private/excluded.rs"),
            "pub struct CSB_LLD_PRIVATE_NEVER_DISCLOSE;",
        )?;
        if revision == 5 {
            write(
                &folder.join("migrations/001.sql"),
                "CREATE TABLE request (id BIGINT PRIMARY KEY);\n",
            )?;
        }
        Ok(())
    }
    fn corpus(output: &Path) -> Result<Vec<Case>, String> {
        let mut cases = Vec::new();
        for repository in 0..REPOSITORIES {
            let root = output.join(format!("repo-{repository}"));
            init(&root)?;
            for pr in (repository..PRS).step_by(REPOSITORIES) {
                fixture(&root, pr, 0)?;
            }
            let baseline = commit(&root, "All distinct PR baseline fixtures")?;
            for pr in (repository..PRS).step_by(REPOSITORIES) {
                git(&root, &["checkout", "-q", "--detach", &baseline])?;
                let mut heads = Vec::new();
                for revision in 1..=REVISIONS + usize::from(pr < BURST) {
                    git(&root, &["checkout", "-q", "--detach", &baseline])?;
                    fixture(&root, pr, revision)?;
                    heads.push(commit(&root, &format!("PR {pr} revision {revision}"))?);
                }
                cases.push(Case {
                    pr,
                    repository,
                    baseline: baseline.clone(),
                    heads,
                    root: format!("pr-{pr:03}"),
                });
            }
        }
        cases.sort_by_key(|case| case.pr);
        Ok(cases)
    }
    fn request(
        case: &Case,
    ) -> Result<(ScopeRecord, ConfirmedDesignContext, PrincipalId, Vec<Grant>), String> {
        let repository = RepositoryId::new(
            SecurityDomain::new("synthetic-phase5-replay")?,
            "local-git",
            "fixture",
            format!("repo-{}", case.repository),
        )?;
        let paths = PathScope::new(
            vec![case.root.clone()],
            vec![format!("{}/private", case.root)],
        )?;
        let scope = ScopeRecord {
            version: ScopeVersion::new(format!("hld-fixture-pr-{}", case.pr))?,
            primary_repository: repository.clone(),
            primary_paths: paths.clone(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let context = ConfirmedDesignContext {
            accepted_commit: SnapshotId::new(case.baseline.clone())?,
            session_id: format!("synthetic-pr-{}", case.pr),
            session_revision: 1,
            input_fingerprint: sha(format!("input-pr-{}", case.pr).as_bytes()),
            proposal_fingerprint: sha(format!("interpretation-pr-{}", case.pr).as_bytes()),
            components: vec![ComponentProposal {
                id: format!("component-pr-{}", case.pr),
                name: format!("PR {} Rust library", case.pr),
                role: "Rust library".into(),
                repository_index: 0,
                roots: vec![case.root.clone()],
                stack_candidates: vec![],
                signals: vec![],
                evidence: vec![],
                confidence: EvidenceConfidence::UserSupplied,
                contradictions: vec![],
                decision: ComponentDecision::Corrected,
            }],
            providers: vec![],
            integrations: vec![],
            gaps: vec![],
        };
        let principal = PrincipalId::new("synthetic-lld-reviewer")?;
        let grants = vec![Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source]),
            paths,
        }];
        Ok((scope, context, principal, grants))
    }
    fn design(output: &Path, case: &Case, commit: &str) -> Result<Bundle, String> {
        let (scope, context, principal, grants) = request(case)?;
        authorize(Action::Source, &principal, &scope, &grants, None)?;
        let root = output.join(format!("repo-{}", case.repository));
        let snapshot = SnapshotId::new(commit.into())?;
        let hld = generate_hld(&root, &snapshot, &scope, &context, &Cancellation::default())?;
        let lld = generate_lld(
            &root,
            &snapshot,
            &scope,
            &context,
            &hld,
            &Cancellation::default(),
        )?;
        Ok(Bundle { hld, lld })
    }
    fn supported_catalog(bundle: &Bundle) -> bool {
        bundle.hld.coverage.complete
            && bundle.lld.catalog.coverage.inventory_complete
            && bundle.lld.catalog.coverage.structure_complete
    }
    fn publication_access(case: &Case) -> Result<(), String> {
        let (scope, _, principal, grants) = request(case)?;
        authorize(Action::Graph, &principal, &scope, &grants, None)?;
        authorize(Action::Source, &principal, &scope, &grants, None)
    }
    fn random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
    fn jobs(cases: &[Case], seed: u64) -> Vec<Job> {
        let mut jobs = Vec::new();
        let mut state = seed;
        for revision in 1..=REVISIONS {
            let mut order = (0..PRS).collect::<Vec<_>>();
            for index in (1..PRS).rev() {
                let other = random(&mut state) as usize % (index + 1);
                order.swap(index, other);
            }
            for (position, index) in order.into_iter().enumerate() {
                let case = cases[index].clone();
                let ordinal = (revision - 1) * PRS + position;
                jobs.push(Job {
                    head: case.heads[revision - 1].clone(),
                    case,
                    revision,
                    virtual_ms: ordinal as u64 * SIMULATED_WINDOW_MS / (PRS * REVISIONS) as u64,
                });
            }
        }
        jobs
    }
    fn percentile(values: &[u64], p: usize) -> u64 {
        if values.is_empty() {
            return 0;
        }
        let mut values = values.to_vec();
        values.sort_unstable();
        values[(values.len() * p)
            .div_ceil(100)
            .saturating_sub(1)
            .min(values.len() - 1)]
    }
    fn micros(start: Instant) -> u64 {
        start.elapsed().as_micros().max(1) as u64
    }
    fn resources() -> serde_json::Value {
        let pid = std::process::id().to_string();
        let output = Command::new("ps")
            .args(["-o", "rss=", "-o", "time=", "-p", &pid])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                json!({"sampler":"ps process RSS KiB / cumulative process CPU clock","raw":String::from_utf8_lossy(&output.stdout).trim()})
            }
            _ => json!({"sampler":"unavailable"}),
        }
    }
    fn persist_bundle(output: &Path, bundle: &Bundle) -> Result<(), String> {
        let bytes = serde_json::to_vec(bundle).map_err(|error| error.to_string())?;
        let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        if text.contains("CSB_LLD_REPLAY_NEVER_DISCLOSE")
            || text.contains("CSB_LLD_PRIVATE_NEVER_DISCLOSE")
        {
            return Err("Sensitive/excluded sentinel appeared in immutable LLD input".into());
        }
        let path = output
            .join("designs")
            .join(format!("{}.json", bundle.lld.catalog.id));
        if path.exists() {
            if fs::read(&path).map_err(|error| error.to_string())? != bytes {
                return Err("Immutable catalog ID changed its pinned input/facts".into());
            }
            Ok(())
        } else {
            write_new(&path, &bytes)
        }
    }
    fn oracle(
        revision: usize,
        baseline: &Bundle,
        proposal: &Bundle,
    ) -> Result<(DesignDelta, LldDelta), String> {
        let hld = compare_hld(&baseline.hld, &proposal.hld)?;
        let lld = compare_lld(&baseline.lld.catalog, &proposal.lld.catalog);
        let expected_hld = if matches!(revision, 2 | 3 | 5) {
            DesignOutcome::ChangeDetected
        } else {
            DesignOutcome::NoChangeDetectedWithinScope
        };
        let expected_lld = if revision == 4 {
            "incomplete"
        } else {
            "change_detected"
        };
        if hld.outcome != expected_hld
            || lld.outcome != expected_lld
            || supported_catalog(proposal) != (revision != 5)
        {
            return Err(format!(
                "Predeclared revision {revision} oracle failed: HLD {:?}, LLD {}, supported {}, gaps {:?}",
                hld.outcome,
                lld.outcome,
                supported_catalog(proposal),
                proposal.lld.catalog.coverage.gaps
            ));
        }
        if revision == 4 && !lld.changes.is_empty() {
            return Err("Formatting-only code acquired a fabricated LLD change".into());
        }
        if revision == 3
            && lld.changes.iter().any(|change| {
                change
                    .proposal_entry_id
                    .as_ref()
                    .and_then(|id| {
                        proposal
                            .lld
                            .catalog
                            .entries
                            .iter()
                            .find(|entry| entry.id == *id)
                    })
                    .is_none_or(|entry| entry.kind != "configuration")
            })
        {
            return Err("Dependency-only edit fabricated a Rust declaration/body change".into());
        }
        let same = proposal
            .lld
            .catalog
            .entries
            .iter()
            .filter(|entry| entry.kind == "function" && entry.name == "same")
            .collect::<Vec<_>>();
        if same.len() != 2
            || same
                .iter()
                .map(|entry| &entry.logical_key)
                .collect::<BTreeSet<_>>()
                .len()
                != 2
        {
            return Err("Same-named methods lost their qualified container identity".into());
        }
        if proposal
            .lld
            .inputs
            .iter()
            .any(|input| input.version != LLD_IR_VERSION)
        {
            return Err("Lazy input parser/profile version mismatch".into());
        }
        Ok((hld, lld))
    }
    fn selected_detail(
        bundle: &Bundle,
        case: &Case,
        revision: usize,
    ) -> Result<LldFunctionDetail, String> {
        let name = if revision == 2 {
            format!("handle_{}", case.pr)
        } else {
            format!("helper_{}", case.pr)
        };
        let selected = bundle
            .lld
            .catalog
            .entries
            .iter()
            .find(|entry| entry.kind == "function" && entry.name == name)
            .ok_or("Selected lazy function is missing")?;
        build_lld_detail(&bundle.lld, &selected.id, &Cancellation::default())
    }
    fn lazy_oracle(
        baseline: &Bundle,
        proposal: &Bundle,
        case: &Case,
        revision: usize,
    ) -> Result<serde_json::Value, String> {
        let started = Instant::now();
        let before = selected_detail(baseline, case, revision)?;
        let after = selected_detail(proposal, case, revision)?;
        let delta = compare_lld_detail(Some(&before), Some(&after));
        let expected = if matches!(revision, 3 | 4) {
            "no_change_detected_within_scope"
        } else {
            "change_detected"
        };
        if delta.outcome != expected || after.graph.complete != (revision != 5) {
            return Err(format!(
                "Lazy revision {revision} local graph oracle failed: {} / complete {} / {:?}",
                delta.outcome, after.graph.complete, after.graph.limitations
            ));
        }
        if before.graph.nodes.len() > 200
            || after.graph.nodes.len() > 200
            || before.graph.edges.len() > 400
            || after.graph.edges.len() > 400
        {
            return Err("Local graph bounds were exceeded".into());
        }
        if after.coverage.runtime_validation != "unavailable" {
            return Err("Syntax graph acquired a runtime validation claim".into());
        }
        let detail_us = micros(started);
        let source_started = Instant::now();
        let span = |detail: &LldFunctionDetail| -> Result<_, String> {
            let evidence_id = detail
                .graph
                .nodes
                .iter()
                .find(|node| {
                    node.operation == "condition" || node.operation == "unsupported_region"
                })
                .map(|node| &node.evidence_id)
                .or_else(|| detail.entry.evidence_ids.first())
                .ok_or("Lazy source has no evidence ID")?;
            detail
                .evidence
                .iter()
                .find(|evidence| evidence.id == *evidence_id)
                .cloned()
                .ok_or("Lazy source evidence is not bound to selected detail".into())
        };
        let before_evidence = span(&before)?;
        let after_evidence = span(&after)?;
        let before_source = lld_source(&baseline.lld, &before_evidence)?;
        let after_source = lld_source(&proposal.lld, &after_evidence)?;
        if before_source.is_empty()
            || after_source.is_empty()
            || before_source.len() > 64 * 1024
            || after_source.len() > 64 * 1024
            || before_evidence.commit != baseline.lld.catalog.snapshot
            || after_evidence.commit != proposal.lld.catalog.snapshot
        {
            return Err("Lazy exact-source identity/bounds oracle failed".into());
        }
        Ok(
            json!({"lazy_detail_pair_us":detail_us,"native_source_pair_us":micros(source_started),"selected_name":after.entry.name,"before_detail_id":before.id,"after_detail_id":after.id,"before_fingerprint":before.behavior_fingerprint,"after_fingerprint":after.behavior_fingerprint,"before_source_evidence":before_evidence.id,"after_source_evidence":after_evidence.id,"exact_source_identity_and_bounds":true,"outcome":delta.outcome,"local_graph_complete":after.graph.complete,"nodes":after.graph.nodes.len(),"edges":after.graph.edges.len(),"runtime_validation":after.coverage.runtime_validation,"limitations":delta.limitations,"parser_ir_version":LLD_IR_VERSION}),
        )
    }
    fn cache_probe(output: &Path, case: &Case, run: &Path) -> Result<serde_json::Value, String> {
        let (scope, _context, principal, grants) = request(case)?;
        let request = |cache: &str| ReviewRequest {
            repository: output.join(format!("repo-{}", case.repository)),
            baseline: case.baseline.clone(),
            proposal: case.heads[0].clone(),
            comparison: ComparisonMode::Exact,
            principal: principal.clone(),
            scope: scope.clone(),
            current_grants: grants.clone(),
            context_manifest: None,
            cache_dir: run.join(cache),
        };
        let cold = analyze_review(&request("shared-cache"), &Cancellation::default())?;
        let warm = analyze_review(&request("shared-cache"), &Cancellation::default())?;
        let clean = analyze_review(&request("clean-cache"), &Cancellation::default())?;
        if cold.review.architecture.analyzed == 0
            || warm.review.architecture.analyzed != 0
            || warm.review.architecture.reused == 0
            || cold.coverage != warm.coverage
            || warm.coverage != clean.coverage
            || cold.review_id != warm.review_id
            || warm.review_id != clean.review_id
        {
            return Err("Actual foundation syntax-cache replay failed".into());
        }
        Ok(
            json!({"scope":"Foundation syntax-cache probe only; not an HLD generation cache","cold_analyzed":cold.review.architecture.analyzed,"warm_analyzed":warm.review.architecture.analyzed,"warm_reused":warm.review.architecture.reused,"clean_analyzed":clean.review.architecture.analyzed,"coverage_and_review_identity_match":true}),
        )
    }
    fn run_seed(output: &Path, cases: &[Case], seed: u64) -> Result<serde_json::Value, String> {
        let run = output.join(format!("seed-{seed}"));
        fs::create_dir(&run).map_err(|error| error.to_string())?;
        let mut queue = DurableQueue::new(&run, cases)?;
        let mut raw = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(run.join("samples.jsonl"))
            .map_err(|error| error.to_string())?;
        let mut baselines = BTreeMap::<usize, Bundle>::new();
        let mut times = vec![];
        let mut lazy_times = vec![];
        let mut source_times = vec![];
        let mut burst_times = vec![];
        let mut baseline_times = vec![];
        let mut clean_samples = vec![];
        let mut resource_samples = vec![];
        let mut supported = 0;
        let mut partial = 0;
        let started = Instant::now();
        for job in jobs(cases, seed) {
            publication_access(&job.case)?;
            let (review_id, accepted) = queue.enqueue(&job.case, job.revision, &job.head)?;
            let claimed = queue.claim()?;
            let queue_wait_us = micros(accepted);
            if claimed.review_id != review_id {
                return Err("Durable worker claimed an unexpected PR generation".into());
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = baselines.entry(job.case.pr)
            {
                let cold = Instant::now();
                let bundle = design(output, &job.case, &job.case.baseline)?;
                baseline_times.push(micros(cold));
                persist_bundle(output, &bundle)?;
                entry.insert(bundle);
            }
            let generated = Instant::now();
            let proposal = design(output, &job.case, &job.head)?;
            let generation_us = micros(generated);
            let compared = Instant::now();
            let (hld, lld) = oracle(job.revision, &baselines[&job.case.pr], &proposal)?;
            let comparison_us = micros(compared);
            persist_bundle(output, &proposal)?;
            publication_access(&job.case)?;
            let digest =
                queue.publish(&claimed, &baselines[&job.case.pr], &proposal, &hld, &lld)?;
            let publication_us = micros(accepted);
            times.push(publication_us);
            if supported_catalog(&proposal) {
                supported += 1;
            } else {
                partial += 1;
            }
            // These are separate viewer requests after durable catalog publication.
            // They are never included in the critical catalog generation time.
            let lazy = lazy_oracle(&baselines[&job.case.pr], &proposal, &job.case, job.revision)?;
            lazy_times.push(
                lazy["lazy_detail_pair_us"]
                    .as_u64()
                    .ok_or("Missing lazy timing")?,
            );
            source_times.push(
                lazy["native_source_pair_us"]
                    .as_u64()
                    .ok_or("Missing native source timing")?,
            );
            let record = json!({"pr":job.case.pr,"revision":job.revision,"baseline":job.case.baseline,"proposal":job.head,"virtual_arrival_ms":job.virtual_ms,"durable_review_id":review_id,"queue_wait_us":queue_wait_us,"catalog_publication_us":publication_us,"typed_artifact_digest":digest,"hld_and_lld_catalog_generation_us":generation_us,"comparison_us":comparison_us,"catalog_supported_profile":supported_catalog(&proposal),"catalog_status":if supported_catalog(&proposal){"local_details_pending"}else{"partial"},"hld_outcome":hld.outcome,"lld_catalog_outcome":lld.outcome,"proposal_catalog_id":proposal.lld.catalog.id,"coverage":proposal.lld.catalog.coverage,"lazy_after_publication":lazy});
            writeln!(raw, "{record}").map_err(|error| error.to_string())?;
            if job.case.pr < 4 {
                let clean_start = Instant::now();
                let rebuilt = design(output, &job.case, &job.head)?;
                let clean_us = micros(clean_start);
                let read_start = Instant::now();
                let bytes = fs::read(
                    output
                        .join("designs")
                        .join(format!("{}.json", proposal.lld.catalog.id)),
                )
                .map_err(|error| error.to_string())?;
                let cached: Bundle =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                let read_us = micros(read_start);
                if rebuilt != proposal
                    || cached != proposal
                    || compare_lld(&baselines[&job.case.pr].lld.catalog, &rebuilt.lld.catalog)
                        != lld
                    || selected_detail(&cached, &job.case, job.revision)?
                        != selected_detail(&rebuilt, &job.case, job.revision)?
                {
                    return Err(
                        "Clean generation/persisted warm private IR/lazy detail/delta mismatch"
                            .into(),
                    );
                }
                clean_samples.push(json!({"pr":job.case.pr,"revision":job.revision,"clean_hld_lld_generation_us":clean_us,"persisted_private_ir_read_us":read_us,"exact_catalog_input_detail_delta_match":true}));
            }
            if (supported + partial) % 100 == 0 {
                resource_samples.push(resources());
            }
        }
        // Enqueue the entire burst before one worker drains it. Lazy requests
        // happen after all burst catalogs publish, keeping them off this queue.
        let mut burst_accepted = BTreeMap::new();
        for case in cases.iter().take(BURST) {
            let (id, accepted) = queue.enqueue(case, 6, &case.heads[5])?;
            burst_accepted.insert(id, (case, accepted));
        }
        let mut burst_proposals = vec![];
        for _ in 0..BURST {
            let claimed = queue.claim()?;
            let (case, accepted) = burst_accepted
                .get(&claimed.review_id)
                .ok_or("Unknown burst input")?;
            let queue_wait_us = micros(*accepted);
            let generated = Instant::now();
            let proposal = design(output, case, &case.heads[5])?;
            let generation_us = micros(generated);
            let (hld, lld) = oracle(6, &baselines[&case.pr], &proposal)?;
            persist_bundle(output, &proposal)?;
            publication_access(case)?;
            let digest = queue.publish(&claimed, &baselines[&case.pr], &proposal, &hld, &lld)?;
            let publication_us = micros(*accepted);
            burst_times.push(publication_us);
            supported += 1;
            writeln!(raw,"{}",json!({"pr":case.pr,"revision":6,"durable_review_id":claimed.review_id,"baseline":case.baseline,"proposal":case.heads[5],"virtual_arrival_ms":SIMULATED_WINDOW_MS,"queue_wait_us":queue_wait_us,"burst_queue_inclusive_catalog_publication_us":publication_us,"generation_us":generation_us,"typed_artifact_digest":digest,"hld_outcome":hld.outcome,"lld_catalog_outcome":lld.outcome,"catalog_status":"local_details_pending"})).map_err(|error|error.to_string())?;
            burst_proposals.push((*case, proposal));
        }
        for (case, proposal) in burst_proposals {
            let lazy = lazy_oracle(&baselines[&case.pr], &proposal, case, 6)?;
            lazy_times.push(
                lazy["lazy_detail_pair_us"]
                    .as_u64()
                    .ok_or("Missing timing")?,
            );
            source_times.push(
                lazy["native_source_pair_us"]
                    .as_u64()
                    .ok_or("Missing native source timing")?,
            );
            writeln!(raw,"{}",json!({"pr":case.pr,"revision":6,"separate_lazy_after_entire_burst_publication":lazy})).map_err(|error|error.to_string())?;
        }
        raw.sync_all().map_err(|error| error.to_string())?;
        if supported != 420 || partial != 100 {
            return Err("Predeclared catalog coverage-count oracle failed".into());
        }
        resource_samples.push(resources());
        if percentile(&times, 95) > 300_000_000
            || percentile(&burst_times, 95) > 300_000_000
            || percentile(&lazy_times, 95) > 300_000
            || percentile(&source_times, 95) > 500_000
        {
            return Err("Declared local publication/detail/source timing target failed; raw samples are retained".into());
        }
        let cache = cache_probe(output, &cases[(seed as usize) % PRS], &run)?;
        let terminal = queue.validate_current(cases)?;
        let summary = json!({"seed":seed,"status":"passed_real_durable_local_catalog_and_separate_lazy_profile","primary_prs":PRS,"revisions_per_pr":REVISIONS,"primary_jobs":PRS*REVISIONS,"burst_jobs":BURST,"virtual_window_ms":SIMULATED_WINDOW_MS,"virtual_schedule":"Five ordered revision waves with seeded PR permutations; arrival times virtual; no real-time pacing","actual_worker_slots":1,"catalogs_with_supported_inventory":supported,"partial_catalogs":partial,"all_function_details_in_catalog":"Not generated; selected lazy functions only; matching catalogs without local detail remain incomplete","historical_revisions_processed_before_successor_arrival":true,"catalog_publication_samples":times.len(),"catalog_publication_p95_us":percentile(&times,95),"catalog_publication_max_us":times.iter().max(),"baseline_pair_generation_p95_us":percentile(&baseline_times,95),"burst_queue_inclusive_samples":burst_times.len(),"burst_queue_inclusive_catalog_publication_p95_us":percentile(&burst_times,95),"lazy_before_after_selected_function_samples":lazy_times.len(),"lazy_detail_pair_p95_us":percentile(&lazy_times,95),"lazy_detail_pair_max_us":lazy_times.iter().max(),"native_source_pairs":source_times.len(),"native_source_pair_p95_us":percentile(&source_times,95),"timing_targets_local_only":{"catalog_publication_p95_max_us":300_000_000,"lazy_detail_pair_p95_max_us":300_000,"native_source_pair_p95_max_us":500_000},"total_elapsed_us":micros(started),"resource_samples":resource_samples,"clean_and_persisted_private_ir_samples":clean_samples,"foundation_syntax_cache_probe":cache,"durable_queue_terminal":terminal,"configured_debounce_ms":0,"arrival_profile":"Explicit local manual-review Scheduler intake; synthetic provider metadata; no external fetch","production_cache_claim":"No production LLD generation cache measured; repeated baseline objects are harness-local memoization; persisted exact IR/detail equality checked separately","excluded_from_timing":["Fixture Git preparation","Provider fetch/intake/debounce","Browser usable diagram/source latency","Network/hosting","Concurrent users/workers","Child process cumulative RSS/CPU"]});
        write_new(
            &run.join("report.json"),
            &serde_json::to_vec_pretty(&summary).map_err(|error| error.to_string())?,
        )?;
        Ok(summary)
    }
    fn lifecycle(output: &Path) -> Result<serde_json::Value, String> {
        let root = output.join("lifecycle-repo");
        init(&root)?;
        fixture(&root, 0, 0)?;
        let accepted = commit(&root, "Pinned accepted source")?;
        let case = Case {
            pr: 0,
            repository: 99,
            baseline: accepted.clone(),
            heads: vec![],
            root: "pr-000".into(),
        };
        let (scope, context, principal, grants) = request(&case)?;
        let generate = |commit: &str| -> Result<Bundle, String> {
            let snapshot = SnapshotId::new(commit.into())?;
            let hld = generate_hld(&root, &snapshot, &scope, &context, &Cancellation::default())?;
            let lld = generate_lld(
                &root,
                &snapshot,
                &scope,
                &context,
                &hld,
                &Cancellation::default(),
            )?;
            Ok(Bundle { hld, lld })
        };
        let baseline = generate(&accepted)?;
        fixture(&root, 0, 1)?;
        let reviewed_head = commit(&root, "Private condition change only")?;
        let proposal = generate(&reviewed_head)?;
        let (hld, lld) = oracle(1, &baseline, &proposal)?;
        lazy_oracle(&baseline, &proposal, &case, 1)?;
        git(&root, &["branch", "synthetic-feature", &reviewed_head])?;
        git(&root, &["checkout", "-q", "--detach", &accepted])?;
        write(
            &root.join("README.md"),
            "Unrelated target branch advancement\n",
        )?;
        let target = commit(&root, "Target advanced outside approved context")?;
        git(
            &root,
            &[
                "merge",
                "--no-ff",
                "-m",
                "Actual synthetic merge",
                "synthetic-feature",
            ],
        )?;
        let merged = git(&root, &["rev-parse", "HEAD"])?;
        let merged_bundle = generate(&merged)?;
        if merged == reviewed_head
            || merged_bundle.lld.catalog.snapshot.as_str() != merged
            || merged_bundle.lld.catalog.id == proposal.lld.catalog.id
        {
            return Err("Actual merged commit was replaced with reviewed head identity".into());
        }
        git(
            &root,
            &["update-ref", "refs/heads/synthetic-feature", &target],
        )?;
        if generate(&reviewed_head)? != proposal {
            return Err(
                "Force-updated ref altered immutable catalog/source/detail identity".into(),
            );
        }
        let missing = generate(&"f".repeat(40))?;
        if missing.lld.catalog.coverage.inventory_complete
            || compare_lld(&baseline.lld.catalog, &missing.lld.catalog).outcome
                == "no_change_detected_within_scope"
        {
            return Err("Missing exact object returned false clean LLD".into());
        }
        if authorize(Action::Graph, &principal, &scope, &[], None).is_ok()
            || authorize(Action::Source, &principal, &scope, &[], None).is_ok()
        {
            return Err("Revoked access checkpoint did not deny graph/source".into());
        }
        authorize(Action::Graph, &principal, &scope, &grants, None)?;
        let cancellation = Cancellation::default();
        cancellation.cancel();
        if generate_lld(
            &root,
            &SnapshotId::new(reviewed_head.clone())?,
            &scope,
            &context,
            &proposal.hld,
            &cancellation,
        )
        .is_ok()
        {
            return Err("Pre-cancelled LLD generation continued".into());
        }
        let private_path = proposal
            .lld
            .catalog
            .entries
            .iter()
            .any(|entry| entry.path.contains("private") || entry.path.ends_with(".env"));
        if private_path {
            return Err("Excluded private/secret path entered LLD catalog".into());
        }
        Ok(
            json!({"accepted_commit":accepted,"reviewed_head":reviewed_head,"advanced_target":target,"actual_merge_commit":merged,"hld_private_only_outcome":hld.outcome,"lld_private_only_outcome":lld.outcome,"actual_merge_catalog_identity":true,"force_updated_ref_preserves_pinned_input_and_catalog":true,"missing_object_cannot_be_clean":true,"revoked_shared_access_graph_and_source_denied":true,"pre_cancelled_generation_denied":true,"excluded_private_and_secret_paths_absent":true,"not_run_here":["HTTP viewer/provider/staged-write fences","Accepted default mutation","Live provider/fork/push delivery","Concurrent PR/context navigation","Browser Mermaid/source repaint"]}),
        )
    }
    fn directory_usage(path: &Path) -> Result<(u64, u64), String> {
        let mut bytes = 0;
        let mut files = 0;
        for item in fs::read_dir(path).map_err(|error| error.to_string())? {
            let item = item.map_err(|error| error.to_string())?;
            let metadata = item.file_type().map_err(|error| error.to_string())?;
            if metadata.is_dir() {
                let (child_bytes, child_files) = directory_usage(&item.path())?;
                bytes += child_bytes;
                files += child_files;
            } else if metadata.is_file() {
                bytes += item.metadata().map_err(|error| error.to_string())?.len();
                files += 1;
            }
        }
        Ok((bytes, files))
    }
    pub fn run() -> Result<(), String> {
        let output = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .ok_or("usage: phase5_lld_replay NEW_OUTPUT_DIRECTORY")?;
        if output.exists() {
            return Err("Refusing to overwrite existing replay output".into());
        }
        fs::create_dir(&output).map_err(|error| error.to_string())?;
        fs::create_dir(output.join("designs")).map_err(|error| error.to_string())?;
        let prepared = Instant::now();
        let cases = corpus(&output)?;
        let corpus_bytes = serde_json::to_vec(&cases).map_err(|error| error.to_string())?;
        write_new(&output.join("corpus.json"), &corpus_bytes)?;
        let corpus_ms = prepared.elapsed().as_millis();
        let lifecycle = lifecycle(&output)?;
        let mut reports = Vec::new();
        for seed in SEEDS {
            reports.push(run_seed(&output, &cases, seed)?);
            println!(
                "seed {seed}: 500 distinct PR revisions + 20 burst HLD/LLD catalogs and lazy flow requests passed"
            );
        }
        let binary = std::env::current_exe().map_err(|error| error.to_string())?;
        let binary_hash = sha(&fs::read(binary).map_err(|error| error.to_string())?);
        let source_files = [
            "Cargo.toml",
            "Cargo.lock",
            "src/lld.rs",
            "src/lld/catalog.rs",
            "src/lld/blob.rs",
            "src/lld/diff.rs",
            "src/lld/flow.rs",
            "src/lld/render.rs",
            "src/parser.rs",
            "src/core.rs",
            "src/design.rs",
            "src/design/generate.rs",
            "src/design/diff.rs",
            "src/design/render.rs",
            "src/design/wire.rs",
            "src/scheduler.rs",
            "src/storage.rs",
            "examples/phase5_lld_replay.rs",
        ];
        let mut source_digests = BTreeMap::new();
        for file in source_files {
            source_digests.insert(
                file,
                sha(&fs::read(file).map_err(|error| error.to_string())?),
            );
        }
        let (output_bytes, output_files) = directory_usage(&output)?;
        let summary = json!({"schema":"codeshouldbehave-phase5-lld-replay-v1","status":"passed_real_durable_local_catalog_and_lazy_flow_profile_public_capacity_not_claimed","distinct_pr_fixtures":100,"revisions_per_pr":5,"unique_proposal_commits":cases.iter().flat_map(|case|case.heads.iter()).collect::<BTreeSet<_>>().len(),"repositories":5,"seed_runs":SEEDS,"corpus_manifest_sha256":sha(&corpus_bytes),"fixture_preparation_ms":corpus_ms,"binary_sha256":binary_hash,"source_file_sha256_at_report":source_digests,"hardware_parallelism":std::thread::available_parallelism().map_or(1,usize::from),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"output_directory_bytes_before_final_report":output_bytes,"output_file_count_before_final_report":output_files,"resource_measurement":"Sparse parent-process ps RSS/CPU samples only; not peak, child totals, or concurrent capacity","analysis_profile":"Actual durable Scheduler intake/claim/fenced publication of typed HLD/LLD catalog artifacts; one worker; accelerated virtual eight-hour order; real twenty-job queued burst; separate lazy immutable-input local-flow requests","scope_receipts":"Explicit synthetic typed context; no real browser/GitHub acceptance","parser_ir_version":LLD_IR_VERSION,"oracles":{"revision_1":"private condition changed: no observed HLD change, known LLD change","revision_2":"public struct/signature change: known HLD and LLD change","revision_3":"dependency-only: known HLD and LLD configuration change; selected function local graph unchanged","revision_4":"format/comments only: no observed HLD change or normalized LLD change; lazy detail equal","revision_5":"unsupported private macro + schema addition: known changes with partial graph/architecture","burst_revision_6":"private while/early-return change: no observed HLD change, known LLD change","same_named_methods":"Two same-named methods retain distinct qualified identities","supported_catalogs_per_run":420,"partial_catalogs_per_run":100},"lifecycle":lifecycle,"runs":reports,"required_capacity_gaps":["No live provider fetch/intake/debounce measurement","No browser twenty-five-session graph/source replay here","No production LLD generation computation-cache benchmark","No deployed concurrent-user/worker capacity acceptance"]});
        write_new(
            &output.join("report.json"),
            serde_json::to_vec_pretty(&summary)
                .map_err(|error| error.to_string())?
                .as_slice(),
        )?;
        println!("Report: {}", output.join("report.json").display());
        Ok(())
    }
}
