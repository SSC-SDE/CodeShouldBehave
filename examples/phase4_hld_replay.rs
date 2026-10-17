//! Additive deterministic HLD replay. It deliberately does not represent a
//! hosted ingestion, debounce, browser or concurrent-capacity benchmark.
#[cfg(not(feature = "server"))]
fn main() {
    eprintln!("phase4_hld_replay requires --features server");
    std::process::exit(2);
}

#[cfg(feature = "server")]
fn main() {
    if let Err(error) = replay::run() {
        eprintln!("Phase 4 HLD replay failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(feature = "server")]
mod replay {
    use flowdiff::design::{
        ConfirmedDesignContext, DesignDelta, DesignOutcome, DesignVersion, compare_hld,
        generate_hld,
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
            let review = format!("hld-pr-{}-r{revision}-{}", case.pr, &head[..12]);
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
                "hld",
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
                .claim("hld-real-replay-worker", JobLane::Warm, 1, 60_000, now)?
                .ok_or("No real HLD replay job was claimable".into())
        }
        fn publish(
            &mut self,
            job: &ClaimedJob,
            baseline: &DesignVersion,
            proposal: &DesignVersion,
            delta: &DesignDelta,
        ) -> Result<String, String> {
            Scheduler::new(&mut self.storage).checkpoint(job)?;
            let payload=serde_json::to_vec(&json!({"version":"csb-hld-durable-replay-result-v1","review_id":job.review_id,"status":if proposal.coverage.complete{"ready"}else{"partial"},"baseline":baseline,"proposal":proposal,"delta":delta})).map_err(|error|error.to_string())?;
            let artifact = self.storage.publish_artifact(
                proposal.repository.security_domain().as_str(),
                "csb-hld-durable-replay-result-v1",
                &payload,
            )?;
            let now = self.now();
            Scheduler::new(&mut self.storage).complete(
                job,
                "hld-real-replay-worker",
                &artifact.digest,
                now,
            )?;
            let current = Scheduler::new(&mut self.storage).current_result(
                job.repository_id,
                &job.provider_pr_id,
                "hld",
            )?;
            if current.as_ref() != Some(&(job.review_id.clone(), artifact.digest.clone())) {
                return Err(
                    "Current result does not reference the actual typed HLD artifact".into(),
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
                return Err("Durable HLD replay terminal queue oracle failed".into());
            }
            for case in cases {
                let revision = if case.pr < BURST { 6 } else { 5 };
                let head = &case.heads[revision - 1];
                let expected = format!("hld-pr-{}-r{revision}-{}", case.pr, &head[..12]);
                let current = Scheduler::new(&mut self.storage)
                    .current_result(
                        self.repositories[&case.pr],
                        &format!("pr-{}", case.pr),
                        "hld",
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
        git(root, &["config", "user.name", "Synthetic HLD replay"])?;
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
        let manifest = format!(
            "[package]\nname='fixture-pr-{pr}'\nversion='0.1.0'\n[dependencies]\nreqwest='0.12'\n{}",
            if revision >= 3 { "lapin='3'\n" } else { "" }
        );
        write(&folder.join("Cargo.toml"), manifest)?;
        let ty = if revision >= 2 { "u64" } else { "i32" };
        let body = if revision == 0 {
            pr
        } else {
            pr + 100 + revision
        };
        let source = format!(
            "use reqwest::Client;\n{}pub struct Request{pr} {{ pub id:{ty} }}\npub fn handle_{pr}(request:Request{pr})->{ty} {{ request.id + helper_{pr}() }}\nfn helper_{pr}()->{ty} {{ {body} }}\n",
            if revision >= 3 {
                "use lapin::Channel;\n"
            } else {
                ""
            }
        );
        write(&folder.join("src/lib.rs"), source)?;
        write(
            &folder.join(".env"),
            "HLD_SECRET=CSB_HLD_REPLAY_NEVER_DISCLOSE",
        )?;
        write(
            &folder.join("private/excluded.rs"),
            "pub struct CSB_HLD_PRIVATE_NEVER_DISCLOSE;",
        )?;
        if revision >= 4 {
            write(
                &folder.join("vercel.json"),
                "{\"framework\":\"vite\",\"outputDirectory\":\"dist\"}",
            )?;
        }
        if revision >= 5 {
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
            SecurityDomain::new("synthetic-phase4-replay")?,
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
        let principal = PrincipalId::new("synthetic-hld-reviewer")?;
        let grants = vec![Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source]),
            paths,
        }];
        Ok((scope, context, principal, grants))
    }
    fn design(output: &Path, case: &Case, commit: &str) -> Result<DesignVersion, String> {
        let (scope, context, principal, grants) = request(case)?;
        authorize(Action::Source, &principal, &scope, &grants, None)?;
        generate_hld(
            &output.join(format!("repo-{}", case.repository)),
            &SnapshotId::new(commit.into())?,
            &scope,
            &context,
            &Cancellation::default(),
        )
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
    fn persist_design(output: &Path, version: &DesignVersion) -> Result<(), String> {
        let bytes = serde_json::to_vec(version).map_err(|error| error.to_string())?;
        let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        if text.contains("CSB_HLD_REPLAY_NEVER_DISCLOSE")
            || text.contains("CSB_HLD_PRIVATE_NEVER_DISCLOSE")
        {
            return Err("Sensitive/excluded sentinel appeared in design".into());
        }
        let path = output.join("designs").join(format!("{}.json", version.id));
        if path.exists() {
            if fs::read(&path).map_err(|error| error.to_string())? != bytes {
                return Err("Immutable design ID changed facts".into());
            }
            Ok(())
        } else {
            write_new(&path, &bytes)
        }
    }
    fn oracle(
        revision: usize,
        baseline: &DesignVersion,
        proposal: &DesignVersion,
    ) -> Result<DesignDelta, String> {
        let delta = compare_hld(baseline, proposal)?;
        let expected = if revision == 1 {
            DesignOutcome::NoChangeDetectedWithinScope
        } else {
            DesignOutcome::ChangeDetected
        };
        if delta.outcome != expected || proposal.coverage.complete != (revision < 5) {
            return Err(format!(
                "Predeclared revision {revision} oracle failed: {:?} / gaps {:?}",
                delta.outcome, proposal.coverage.gaps
            ));
        }
        Ok(delta)
    }
    fn cache_probe(output: &Path, case: &Case, run: &Path) -> Result<serde_json::Value, String> {
        let (scope, _context, principal, grants) = request(case)?;
        let request = |cache: &str| ReviewRequest {
            repository: output.join(format!("repo-{}", case.repository)),
            baseline: case.baseline.clone(),
            proposal: case.heads[2].clone(),
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
        let run = output.join(format!("run-{seed}"));
        fs::create_dir(&run).map_err(|error| error.to_string())?;
        let started = Instant::now();
        let mut raw = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(run.join("jobs.jsonl"))
            .map_err(|error| error.to_string())?;
        let mut baselines = BTreeMap::new();
        let mut queue = DurableQueue::new(&run, cases)?;
        let mut times = Vec::new();
        let mut ready_times = Vec::new();
        let mut partial_times = Vec::new();
        let mut ready = 0;
        let mut partial = 0;
        let mut baseline_times = Vec::new();
        let mut resource_samples = vec![resources()];
        let mut clean_samples = Vec::new();
        for job in jobs(cases, seed) {
            let (review_id, accepted) = queue.enqueue(&job.case, job.revision, &job.head)?;
            let claimed = queue.claim()?;
            let queue_wait_us = micros(accepted);
            if claimed.review_id != review_id {
                return Err("Durable worker claimed an unexpected PR generation".into());
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = baselines.entry(job.case.pr)
            {
                let cold = Instant::now();
                let version = design(output, &job.case, &job.case.baseline)?;
                baseline_times.push(micros(cold));
                persist_design(output, &version)?;
                entry.insert(version);
            }
            let generated = Instant::now();
            let proposed = design(output, &job.case, &job.head)?;
            let generation_us = micros(generated);
            let compared = Instant::now();
            let delta = oracle(job.revision, &baselines[&job.case.pr], &proposed)?;
            let comparison_us = micros(compared);
            persist_design(output, &proposed)?;
            publication_access(&job.case)?;
            let artifact_digest =
                queue.publish(&claimed, &baselines[&job.case.pr], &proposed, &delta)?;
            if proposed.coverage.complete {
                ready += 1;
            } else {
                partial += 1;
            }
            let publication_us = micros(accepted);
            times.push(publication_us);
            if proposed.coverage.complete {
                ready_times.push(publication_us);
            } else {
                partial_times.push(publication_us);
            }
            let record = json!({"pr":job.case.pr,"revision":job.revision,"baseline":job.case.baseline,"proposal":job.head,"virtual_arrival_ms":job.virtual_ms,"durable_review_id":review_id,"queue_wait_us":queue_wait_us,"actual_durable_acceptance_to_typed_result_publication_us":publication_us,"typed_artifact_digest":artifact_digest,"hld_generation_us":generation_us,"comparison_us":comparison_us,"status":if proposed.coverage.complete{"ready"}else{"partial"},"outcome":delta.outcome,"baseline_design_id":baselines[&job.case.pr].id,"proposal_design_id":proposed.id,"coverage":proposed.coverage});
            writeln!(raw, "{record}").map_err(|error| error.to_string())?;
            if job.case.pr < 4 {
                let cold = Instant::now();
                let rebuilt = design(output, &job.case, &job.head)?;
                let clean_us = micros(cold);
                let disk = Instant::now();
                let bytes = fs::read(output.join("designs").join(format!("{}.json", proposed.id)))
                    .map_err(|error| error.to_string())?;
                let cached: DesignVersion =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                let read_us = micros(disk);
                if rebuilt != proposed
                    || cached != proposed
                    || compare_hld(&baselines[&job.case.pr], &rebuilt)? != delta
                {
                    return Err("Clean HLD generation / persisted warm read / delta schema+coverage mismatch".into());
                }
                clean_samples.push(json!({"pr":job.case.pr,"revision":job.revision,"clean_generation_us":clean_us,"persisted_read_us":read_us,"exact_design_delta_and_coverage_match":true}));
            }
            if (ready + partial) % 100 == 0 {
                resource_samples.push(resources());
            }
        }
        // Burst events are accepted together and drained with one worker; the
        // real queue wait is included. There is no simulated real-time sleep.
        let mut burst_accepted = BTreeMap::new();
        for case in cases.iter().take(BURST) {
            let (id, accepted) = queue.enqueue(case, 6, &case.heads[5])?;
            burst_accepted.insert(id, (case, accepted));
        }
        let mut burst_times = Vec::new();
        for _ in 0..BURST {
            let claimed = queue.claim()?;
            let (case, accepted) = burst_accepted
                .get(&claimed.review_id)
                .ok_or("Burst claim references an unknown typed input")?;
            let queue_wait_us = micros(*accepted);
            let started = Instant::now();
            let proposed = design(output, case, &case.heads[5])?;
            let generation_us = micros(started);
            let delta = oracle(6, &baselines[&case.pr], &proposed)?;
            persist_design(output, &proposed)?;
            publication_access(case)?;
            let artifact_digest =
                queue.publish(&claimed, &baselines[&case.pr], &proposed, &delta)?;
            partial += 1;
            let publication_us = micros(*accepted);
            burst_times.push(publication_us);
            let record = json!({"pr":case.pr,"revision":6,"baseline":case.baseline,"proposal":case.heads[5],"virtual_arrival_ms":SIMULATED_WINDOW_MS,"durable_review_id":claimed.review_id,"queue_wait_us":queue_wait_us,"actual_burst_queue_inclusive_publication_us":publication_us,"typed_artifact_digest":artifact_digest,"hld_generation_us":generation_us,"status":"partial","outcome":delta.outcome,"proposal_design_id":proposed.id});
            writeln!(raw, "{record}").map_err(|error| error.to_string())?;
        }
        raw.sync_all().map_err(|error| error.to_string())?;
        if ready != 400 || partial != 120 {
            return Err("Predeclared terminal status count oracle failed".into());
        }
        let cache = cache_probe(output, &cases[(seed as usize) % PRS], &run)?;
        let terminal = queue.validate_current(cases)?;
        let summary = json!({"seed":seed,"status":"passed_real_durable_hld_local_profile_not_public_capacity_acceptance","primary_prs":PRS,"revisions_per_pr":REVISIONS,"primary_jobs":PRS*REVISIONS,"burst_jobs":BURST,"simulated_window_ms":SIMULATED_WINDOW_MS,"virtual_schedule":"Five ordered revision waves with seeded PR permutations; virtual arrival timestamps retained; events processed immediately without pacing","actual_worker_slots":1,"ready_hld_jobs":ready,"partial_hld_jobs":partial,"superseded":0,"cancelled":0,"expected_final_current_prs":100,"expected_final_current_partial":100,"historical_revisions_processed_before_successor_arrival":true,"durable_event_to_publication_sample_count":times.len(),"durable_event_to_publication_p95_us":percentile(&times,95),"durable_event_to_publication_max_us":times.iter().max(),"baseline_generation_p95_us":percentile(&baseline_times,95),"burst_queue_inclusive_samples":burst_times.len(),"burst_queue_inclusive_p95_us":percentile(&burst_times,95),"total_elapsed_us":micros(started),"resources":resource_samples,"clean_and_persisted_replay_samples":clean_samples,"foundation_syntax_cache_probe":cache,"durable_queue_terminal":terminal,"hld_generation_cache":"No production HLD generation cache exists; all proposed designs regenerate. Reused baseline objects are harness-local memoization, not measured production cache reuse","excluded_from_timing":["Fixture Git preparation","Provider fetch/intake and configured production debounce","Browser graph/source rendering","Network/provider hosting","Concurrent users/workers","Subprocess cumulative RSS/CPU"]});
        let mut summary = summary;
        summary["configured_debounce_ms"] = json!(0);
        summary["arrival_profile"] = json!(
            "Local explicit manual-review requests through durable Scheduler; fixture provider metadata supplied directly; no external fetch"
        );
        summary["ready_primary_publication"] = json!({"samples":ready_times.len(),"p95_us":percentile(&ready_times,95),"max_us":ready_times.iter().max()});
        summary["partial_primary_publication"] = json!({"samples":partial_times.len(),"p95_us":percentile(&partial_times,95),"max_us":partial_times.iter().max()});
        summary["partial_burst_publication"] = json!({"samples":burst_times.len(),"p95_us":percentile(&burst_times,95),"max_us":burst_times.iter().max()});
        write_new(
            &run.join("report.json"),
            serde_json::to_vec_pretty(&summary)
                .map_err(|error| error.to_string())?
                .as_slice(),
        )?;
        Ok(summary)
    }
    fn lifecycle(output: &Path) -> Result<serde_json::Value, String> {
        let root = output.join("lifecycle-repo");
        init(&root)?;
        fixture(&root, 0, 0)?;
        let accepted = commit(&root, "Accepted source")?;
        let mut case = Case {
            pr: 0,
            repository: 99,
            baseline: accepted.clone(),
            heads: vec![],
            root: "pr-000".into(),
        };
        // The generator path is explicit; lifecycle source is an independent
        // inert fixture with the same approved typed context.
        let (scope, context, principal, grants) = request(&case)?;
        let generate = |commit: &str| {
            generate_hld(
                &root,
                &SnapshotId::new(commit.into())?,
                &scope,
                &context,
                &Cancellation::default(),
            )
        };
        let accepted_design = generate(&accepted)?;
        fixture(&root, 0, 2)?;
        let baseline = commit(&root, "Pre-existing public contract drift")?;
        let observed = generate(&baseline)?;
        fixture(&root, 0, 1)?;
        // Keep the changed public contract while changing only a private body.
        write(
            &root.join("pr-000/src/lib.rs"),
            "use reqwest::Client;\npub struct Request0 {pub id:u64}\npub fn handle_0(request:Request0)->u64 {request.id+helper_0()}\nfn helper_0()->u64 {999}\n",
        )?;
        let head = commit(&root, "PR private body only")?;
        let proposal = generate(&head)?;
        if compare_hld(&accepted_design, &observed)?.outcome != DesignOutcome::ChangeDetected
            || compare_hld(&observed, &proposal)?.outcome
                != DesignOutcome::NoChangeDetectedWithinScope
        {
            return Err("Accepted drift was misattributed to private-only PR".into());
        }
        git(&root, &["branch", "synthetic-feature", &head])?;
        git(&root, &["checkout", "-q", "--detach", &baseline])?;
        write(&root.join("README.md"), "Unrelated target documentation\n")?;
        let target = commit(&root, "Target advanced")?;
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
        let merged_design = generate(&merged)?;
        if merged == head
            || merged_design.snapshot.as_str() != merged
            || merged_design.id == proposal.id
        {
            return Err("Actual merge snapshot was replaced by reviewed head".into());
        }
        git(
            &root,
            &["update-ref", "refs/heads/synthetic-feature", &target],
        )?;
        if proposal.snapshot.as_str() != head || generate(&head)? != proposal {
            return Err("Force-updated ref altered immutable historical design".into());
        }
        let missing = generate(&"f".repeat(40))?;
        if missing.coverage.complete || missing.coverage.enumeration_complete {
            return Err("Missing object incorrectly returned complete HLD".into());
        }
        case.heads = vec![head.clone(), merged.clone()];
        let separate = generate(&target)?;
        if separate.id == proposal.id || separate.id == merged_design.id {
            return Err("Stacked/concurrent snapshot design identities collapsed".into());
        }
        let revoked = authorize(Action::Graph, &principal, &scope, &[], None).is_err();
        authorize(Action::Graph, &principal, &scope, &grants, None)?;
        if !revoked {
            return Err("Revocation checkpoint failed".into());
        }
        let cancel = Cancellation::default();
        cancel.cancel();
        if generate_hld(
            &root,
            &SnapshotId::new(head.clone())?,
            &scope,
            &context,
            &cancel,
        )
        .is_ok()
        {
            return Err("Cancelled generation continued".into());
        }
        write(
            &root.join("pr-000/gateway/Cargo.toml"),
            "[package]\nname='gateway'\nversion='0.1.0'\n",
        )?;
        write(
            &root.join("pr-000/gateway/src/lib.rs"),
            "pub trait GatewayBoundary {fn send(&self)->u64;}\n",
        )?;
        let boundary_commit = commit(&root, "Explicit new confirmed component boundary")?;
        let mut boundary_context = context.clone();
        boundary_context.accepted_commit = SnapshotId::new(boundary_commit.clone())?;
        boundary_context.session_revision = 2;
        boundary_context.proposal_fingerprint = sha(b"explicit-confirmed-new-boundary");
        boundary_context.components.push(ComponentProposal {
            id: "explicit-gateway-boundary".into(),
            name: "Gateway component boundary".into(),
            role: "User-confirmed Rust library boundary".into(),
            repository_index: 0,
            roots: vec!["pr-000/gateway".into()],
            stack_candidates: vec![],
            signals: vec![],
            evidence: vec![],
            confidence: EvidenceConfidence::UserSupplied,
            contradictions: vec![],
            decision: ComponentDecision::Corrected,
        });
        let boundary_design = generate_hld(
            &root,
            &SnapshotId::new(boundary_commit.clone())?,
            &scope,
            &boundary_context,
            &Cancellation::default(),
        )?;
        if compare_hld(&merged_design, &boundary_design)?.outcome != DesignOutcome::ChangeDetected {
            return Err(
                "New explicit component boundary was not observed as a qualified HLD change".into(),
            );
        }
        Ok(
            json!({"preexisting_drift_separate_from_private_pr":true,"accepted_commit":accepted,"observed_baseline_commit":baseline,"reviewed_head":head,"advanced_target":target,"actual_merge_commit":merged,"actual_merge_snapshot_identity":true,"force_updated_ref_preserves_historical_exact_commit":true,"missing_object_partial":true,"independent_snapshot_designs":true,"revoked_shared_authorization_checkpoint_denied":true,"pre_cancelled_generation_denied":true,"new_explicit_confirmed_boundary_commit":boundary_commit,"new_explicit_boundary_hld_change":true,"accepted_design_default_unchanged":"No acceptance/default store used here","not_run_here":["Authenticated HTTP cache/default revocation","Publication generation fences","Host new-HLD kill switch","UI stale response discard","Authenticated design acceptance transaction","True combined stacked PR merge analysis"]}),
        )
    }
    pub fn run() -> Result<(), String> {
        let output = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .ok_or("usage: phase4_hld_replay NEW_OUTPUT_DIRECTORY")?;
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
            println!("seed {seed}: 500 distinct PR revisions + 20 burst HLD analyses passed");
        }
        let binary = std::env::current_exe().map_err(|error| error.to_string())?;
        let binary_hash = sha(&fs::read(binary).map_err(|error| error.to_string())?);
        let source_files = [
            "Cargo.toml",
            "Cargo.lock",
            "src/design.rs",
            "src/design/generate.rs",
            "src/design/diff.rs",
            "src/design/render.rs",
            "src/design/wire.rs",
            "src/scheduler.rs",
            "src/storage.rs",
            "examples/phase4_hld_replay.rs",
        ];
        let mut source_digests = BTreeMap::new();
        for file in source_files {
            source_digests.insert(
                file,
                sha(&fs::read(file).map_err(|error| error.to_string())?),
            );
        }
        let summary = json!({"schema":"codeshouldbehave-phase4-hld-replay-v1","status":"passed_real_durable_local_profile_public_capacity_gate_not_claimed","distinct_pr_fixtures":100,"revisions_per_pr":5,"unique_proposal_commits":cases.iter().flat_map(|case|case.heads.iter()).collect::<BTreeSet<_>>().len(),"repositories":5,"seed_runs":SEEDS,"corpus_manifest_sha256":sha(&corpus_bytes),"fixture_preparation_ms":corpus_ms,"binary_sha256":binary_hash,"source_file_sha256_at_report":source_digests,"hardware_parallelism":std::thread::available_parallelism().map_or(1,usize::from),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"analysis_profile":"Actual durable Scheduler intake/claim/fenced publication of real typed HLD artifacts; one actual worker; seeded accelerated virtual eight-hour ordering; separate real 20-job single-worker burst","scope_receipts":"Synthetic explicit typed context; not a browser/provider authorization or real accepted HLD baseline","oracles":{"revision_1":"private body only: no change within complete supported profile","revision_2":"public contract: observed HLD change","revision_3":"package SDK dependency: observed HLD change; no running queue inferred","revision_4":"static Vercel mapping: observed HLD change; no live hosting inferred","revision_5_and_burst":"schema presence: observed HLD change with partial ownership/compatibility coverage","ready_per_run":400,"partial_per_run":120},"lifecycle":lifecycle,"runs":reports,"required_capacity_gaps":["Configured production provider fetch/intake/debounce is not exercised by this local fixture driver","No 25-session browser graph/source latency replay in this harness","No production HLD computation-cache reuse measurement","No deployment-host or concurrent-user capacity acceptance"]});
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
