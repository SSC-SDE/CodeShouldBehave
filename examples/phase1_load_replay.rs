use flowdiff::{
    Action, Cancellation, ComparisonMode, FetchLimits, FetchRequest, Grant, JobLane, PathScope,
    PrincipalId, RepositoryId, RestrictedFetcher, ReviewRequest, Scheduler, ScopeRecord,
    ScopeVersion, SecurityDomain, Storage, analyze_review, authorize, render_markdown,
};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Barrier},
    thread,
    time::Instant,
};

const PRS: usize = 100;
const REVISIONS: usize = 5;
const REPOSITORIES: usize = 5;
const REVIEWER_SESSIONS: usize = 25;
const INTERACTIONS_PER_SESSION: usize = 4;
const BURST_UPDATES: usize = 20;
const HOT_PR_UPDATES: usize = 10;
const SIMULATED_WINDOW_MS: u64 = 8 * 60 * 60 * 1_000;

struct FaultFetcher(&'static str);

struct CacheReplay {
    cold_analyzed: usize,
    warm_reused: usize,
    mixed_analyzed: usize,
    mixed_reused: usize,
    cold_ms: u64,
    warm_ms: u64,
    mixed_ms: u64,
    clean_rebuild_match: bool,
}

impl RestrictedFetcher for FaultFetcher {
    fn fetch(&self, _request: &FetchRequest) -> Result<(), String> {
        Err(self.0.into())
    }
}

fn commit(value: usize) -> String {
    format!("{value:040x}")
}

fn percentile(samples: &mut [u64], percentile: usize) -> u64 {
    samples.sort_unstable();
    let index = (samples.len() * percentile).div_ceil(100).saturating_sub(1);
    samples[index.min(samples.len().saturating_sub(1))]
}

fn directory_bytes(path: &Path) -> Result<u64, String> {
    let mut total = 0_u64;
    let mut pending = vec![path.to_path_buf()];
    let mut entries = 0_usize;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("cannot measure {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("cannot measure output: {error}"))?;
            entries += 1;
            if entries > 100_000 {
                return Err("replay output exceeds measurement entry bound".into());
            }
            let metadata = entry
                .file_type()
                .map_err(|error| format!("cannot inspect replay output: {error}"))?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total
                    .checked_add(
                        entry
                            .metadata()
                            .map_err(|error| format!("cannot inspect replay file: {error}"))?
                            .len(),
                    )
                    .ok_or("replay disk measurement overflow")?;
            }
        }
    }
    Ok(total)
}

fn parse_cpu_time(value: &str) -> Result<u64, String> {
    let (days, clock) = value
        .split_once('-')
        .map_or((0_u64, value), |(days, clock)| {
            (days.parse::<u64>().unwrap_or(u64::MAX), clock)
        });
    if days == u64::MAX {
        return Err("cannot parse process CPU days".into());
    }
    let fields = clock.split(':').collect::<Vec<_>>();
    let (hours, minutes, seconds) = match fields.as_slice() {
        [minutes, seconds] => ("0", *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return Err("cannot parse process CPU time".into()),
    };
    let hours = hours
        .parse::<u64>()
        .map_err(|_| "cannot parse process CPU hours")?;
    let minutes = minutes
        .parse::<u64>()
        .map_err(|_| "cannot parse process CPU minutes")?;
    let seconds = seconds
        .parse::<f64>()
        .map_err(|_| "cannot parse process CPU seconds")?;
    Ok((((days * 24 + hours) * 60 + minutes) * 60_000) + (seconds * 1_000.0) as u64)
}

fn process_usage() -> Result<(u64, u64), String> {
    let process = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-o", "time=", "-p", &process])
        .output()
        .map_err(|error| format!("cannot sample process resources: {error}"))?;
    if !output.status.success() {
        return Err("process resource sampler failed".into());
    }
    let line = String::from_utf8_lossy(&output.stdout);
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2 {
        return Err("process resource sampler returned an unexpected format".into());
    }
    let rss_kib = fields[0]
        .parse::<u64>()
        .map_err(|_| "cannot parse process RSS")?;
    Ok((rss_kib * 1024, parse_cpu_time(fields[1])?))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("cannot persist {}: {error}", path.display()))
}

fn git(path: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("cannot start synthetic Git command: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "synthetic Git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

fn commit_fixture(repository: &Path, message: &str) -> Result<String, String> {
    git(repository, &["add", "--all"])?;
    git(
        repository,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-q",
            "-m",
            message,
        ],
    )?;
    git(repository, &["rev-parse", "HEAD"])
}

fn analysis_request(
    repository: &Path,
    cache_dir: PathBuf,
    baseline: &str,
    proposal: &str,
) -> Result<ReviewRequest, String> {
    let principal = PrincipalId::new("synthetic-cache-reviewer")?;
    let repository_id = RepositoryId::new(
        SecurityDomain::new("synthetic-cache")?,
        "local-git",
        "load-fixture",
        "cache-repository",
    )?;
    let scope = ScopeRecord {
        version: ScopeVersion::new("cache-v1")?,
        primary_repository: repository_id.clone(),
        primary_paths: PathScope::all(),
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: false,
        model_destinations: BTreeSet::new(),
    };
    Ok(ReviewRequest {
        repository: repository.into(),
        baseline: baseline.into(),
        proposal: proposal.into(),
        comparison: ComparisonMode::Exact,
        principal: principal.clone(),
        scope,
        current_grants: vec![Grant {
            principal,
            repository: repository_id,
            actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source]),
            paths: PathScope::all(),
        }],
        context_manifest: None,
        cache_dir,
    })
}

fn semantic_markdown(review: &flowdiff::Review) -> String {
    render_markdown(review)
        .lines()
        .map(|line| {
            if line.contains(" blobs analyzed, ") && line.ends_with(" reused from cache.") {
                "<cache accounting omitted>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn replay_analysis_caches(output: &Path) -> Result<CacheReplay, String> {
    let repository = output.join("cache-fixture");
    let source = repository.join("src");
    fs::create_dir_all(&source).map_err(|error| format!("cannot create cache fixture: {error}"))?;
    git(&repository, &["init", "-q"])?;
    git(
        &repository,
        &["config", "user.name", "CodeShouldBehave replay"],
    )?;
    git(
        &repository,
        &["config", "user.email", "replay@example.invalid"],
    )?;
    for index in 0..40 {
        fs::write(
            source.join(format!("module_{index}.rs")),
            format!("pub fn module_{index}() {{ shared_{index}(); }}\n"),
        )
        .map_err(|error| format!("cannot write cache fixture: {error}"))?;
    }
    let baseline = commit_fixture(&repository, "baseline")?;
    for index in 0..10 {
        fs::write(
            source.join(format!("module_{index}.rs")),
            format!("pub fn module_{index}() {{ changed_{index}(); shared_{index}(); }}\n"),
        )
        .map_err(|error| format!("cannot update cache fixture: {error}"))?;
    }
    let first_proposal = commit_fixture(&repository, "first proposal")?;
    let shared_cache = output.join("analysis-cache");
    let cold_started = Instant::now();
    let cold = analyze_review(
        &analysis_request(
            &repository,
            shared_cache.clone(),
            &baseline,
            &first_proposal,
        )?,
        &Cancellation::default(),
    )?;
    let cold_ms = cold_started.elapsed().as_millis() as u64;
    let warm_started = Instant::now();
    let warm = analyze_review(
        &analysis_request(
            &repository,
            shared_cache.clone(),
            &baseline,
            &first_proposal,
        )?,
        &Cancellation::default(),
    )?;
    let warm_ms = warm_started.elapsed().as_millis() as u64;
    for index in 10..15 {
        fs::write(
            source.join(format!("module_{index}.rs")),
            format!("pub fn module_{index}() {{ mixed_{index}(); shared_{index}(); }}\n"),
        )
        .map_err(|error| format!("cannot update mixed-cache fixture: {error}"))?;
    }
    let mixed_proposal = commit_fixture(&repository, "mixed proposal")?;
    let mixed_started = Instant::now();
    let mixed = analyze_review(
        &analysis_request(&repository, shared_cache, &baseline, &mixed_proposal)?,
        &Cancellation::default(),
    )?;
    let mixed_ms = mixed_started.elapsed().as_millis() as u64;
    let clean = analyze_review(
        &analysis_request(
            &repository,
            output.join("clean-analysis-cache"),
            &baseline,
            &mixed_proposal,
        )?,
        &Cancellation::default(),
    )?;
    let clean_rebuild_match = semantic_markdown(&mixed.review) == semantic_markdown(&clean.review)
        && mixed.coverage == clean.coverage
        && mixed.review_id == clean.review_id;
    let cold_analyzed = cold.review.architecture.analyzed;
    let warm_reused = warm.review.architecture.reused;
    let mixed_analyzed = mixed.review.architecture.analyzed;
    let mixed_reused = mixed.review.architecture.reused;
    if cold_analyzed == 0
        || warm.review.architecture.analyzed != 0
        || warm_reused == 0
        || mixed_analyzed == 0
        || mixed_reused == 0
        || !clean_rebuild_match
    {
        return Err(format!(
            "cache or clean-rebuild replay assertion failed: cold_analyzed={cold_analyzed}, warm_analyzed={}, warm_reused={warm_reused}, mixed_analyzed={mixed_analyzed}, mixed_reused={mixed_reused}, clean_rebuild_match={clean_rebuild_match}",
            warm.review.architecture.analyzed
        ));
    }
    Ok(CacheReplay {
        cold_analyzed,
        warm_reused,
        mixed_analyzed,
        mixed_reused,
        cold_ms,
        warm_ms,
        mixed_ms,
        clean_rebuild_match,
    })
}

fn measure_lock_contention(
    database: &Path,
    artifacts: &Path,
    repository_id: i64,
) -> Result<(u64, u64), String> {
    let barrier = Arc::new(Barrier::new(REVIEWER_SESSIONS));
    let mut writers = Vec::new();
    for index in 0..REVIEWER_SESSIONS {
        let mut storage = Storage::open(database, artifacts)?;
        let barrier = Arc::clone(&barrier);
        writers.push(thread::spawn(move || -> Result<u64, String> {
            barrier.wait();
            let started = Instant::now();
            storage.register_scope(
                repository_id,
                &format!("lock-replay-{index}"),
                format!("include=src/lock-{index}").as_bytes(),
            )?;
            Ok(started.elapsed().as_micros().max(1) as u64)
        }));
    }
    let mut samples = Vec::new();
    for writer in writers {
        samples.push(writer.join().map_err(|_| "lock replay writer panicked")??);
    }
    let maximum = samples.iter().copied().max().unwrap_or(0);
    let p95 = percentile(&mut samples, 95);
    Ok((p95, maximum))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("phase1 load replay failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: phase1_load_replay NEW_OUTPUT_DIRECTORY")?;
    if output.exists() {
        return Err(format!("refusing to overwrite {}", output.display()));
    }
    fs::create_dir(&output)
        .map_err(|error| format!("cannot create {}: {error}", output.display()))?;
    let database = output.join("replay.db");
    let artifacts = output.join("artifacts");
    let started = Instant::now();
    let (initial_rss_bytes, initial_cpu_ms) = process_usage()?;
    let mut peak_rss_bytes = initial_rss_bytes;
    let cache_replay = replay_analysis_caches(&output)?;
    peak_rss_bytes = peak_rss_bytes.max(process_usage()?.0);
    let mut storage = Storage::open(&database, &artifacts)?;
    let mut repositories = Vec::new();
    for index in 0..REPOSITORIES {
        let repository = storage.register_repository(
            "synthetic",
            "fake",
            "load-fixture",
            &format!("repository-{index}"),
        )?;
        let scope = storage.register_scope(repository, "load-v1", b"include=src")?;
        repositories.push((repository, scope));
    }

    let mut latest_generation = vec![0_i64; PRS];
    let mut latest_review = vec![String::new(); PRS];
    let mut latest_accepted_at = vec![Instant::now(); PRS];
    let setup_started = Instant::now();
    for pr in 0..PRS {
        let (repository, scope) = repositories[pr % REPOSITORIES];
        for revision in 1..=REVISIONS {
            let ordinal = pr * REVISIONS + revision;
            let now = (ordinal as u64 * SIMULATED_WINDOW_MS) / (PRS * REVISIONS) as u64;
            let review = format!("pr-{pr}-revision-{revision}");
            let provider_pr = format!("pr-{pr}");
            let base = commit(pr + 1);
            let proposal = commit(10_000 + ordinal);
            let mut scheduler = Scheduler::new(&mut storage);
            scheduler.register_review(
                &review, repository, scope, &base, &base, &proposal, "exact", now,
            )?;
            scheduler.observe_pr(
                repository,
                &provider_pr,
                revision as i64,
                &review,
                &proposal,
                &base,
                now,
            )?;
            scheduler.enqueue(
                &review,
                "analyze",
                100,
                JobLane::Warm,
                &provider_pr,
                revision as i64,
                now,
            )?;
            latest_generation[pr] = revision as i64;
            latest_review[pr] = review;
            latest_accepted_at[pr] = Instant::now();
        }
    }
    let primary_setup_ms = setup_started.elapsed().as_millis() as u64;

    let burst_started = Instant::now();
    for pr in 0..BURST_UPDATES {
        let (repository, scope) = repositories[pr % REPOSITORIES];
        let generation = latest_generation[pr] + 1;
        let review = format!("pr-{pr}-burst-{generation}");
        let base = commit(pr + 1);
        let proposal = commit(20_000 + pr);
        let mut scheduler = Scheduler::new(&mut storage);
        scheduler.register_review(
            &review,
            repository,
            scope,
            &base,
            &base,
            &proposal,
            "exact",
            SIMULATED_WINDOW_MS,
        )?;
        scheduler.observe_pr(
            repository,
            &format!("pr-{pr}"),
            generation,
            &review,
            &proposal,
            &base,
            SIMULATED_WINDOW_MS,
        )?;
        scheduler.enqueue(
            &review,
            "analyze",
            200,
            JobLane::Warm,
            &format!("pr-{pr}"),
            generation,
            SIMULATED_WINDOW_MS,
        )?;
        latest_generation[pr] = generation;
        latest_review[pr] = review;
        latest_accepted_at[pr] = Instant::now();
    }
    for update in 0..HOT_PR_UPDATES {
        let pr = PRS - 1;
        let (repository, scope) = repositories[pr % REPOSITORIES];
        let generation = latest_generation[pr] + 1;
        let review = format!("pr-{pr}-hot-{generation}");
        let base = commit(pr + 1);
        let proposal = commit(30_000 + update);
        let mut scheduler = Scheduler::new(&mut storage);
        scheduler.register_review(
            &review,
            repository,
            scope,
            &base,
            &base,
            &proposal,
            "exact",
            SIMULATED_WINDOW_MS + update as u64,
        )?;
        scheduler.observe_pr(
            repository,
            &format!("pr-{pr}"),
            generation,
            &review,
            &proposal,
            &base,
            SIMULATED_WINDOW_MS + update as u64,
        )?;
        scheduler.enqueue(
            &review,
            "analyze",
            300,
            JobLane::Warm,
            &format!("pr-{pr}"),
            generation,
            SIMULATED_WINDOW_MS + update as u64,
        )?;
        latest_generation[pr] = generation;
        latest_review[pr] = review;
        latest_accepted_at[pr] = Instant::now();
    }
    let burst_setup_ms = burst_started.elapsed().as_millis() as u64;

    let duplicate_rejected = !Scheduler::new(&mut storage).observe_pr(
        repositories[0].0,
        "pr-0",
        latest_generation[0],
        &latest_review[0],
        &commit(20_000),
        &commit(1),
        SIMULATED_WINDOW_MS + 100,
    )?;
    let delayed_rejected = !Scheduler::new(&mut storage).observe_pr(
        repositories[0].0,
        "pr-0",
        1,
        "pr-0-revision-1",
        &commit(10_001),
        &commit(1),
        SIMULATED_WINDOW_MS + 101,
    )?;

    // Exercise provider failures while the synthetic queue is active. These
    // adapters perform no network access and keep credentials out of fixtures.
    let fault_request = FetchRequest {
        repository_id: repositories[0].0,
        mirror: output.join("fake-provider-mirror"),
        configured_remote: "fake-origin".into(),
        base_commit: commit(1),
        head_commit: commit(40_000),
        limits: FetchLimits::default(),
    };
    let provider_throttle_observed = FaultFetcher("provider throttled: retry after 60 seconds")
        .fetch(&fault_request)
        .is_err_and(|reason| reason.contains("throttled"));
    let provider_credential_revocation_observed = FaultFetcher("provider credentials revoked")
        .fetch(&fault_request)
        .is_err_and(|reason| reason.contains("credentials revoked"));

    // Recheck authorization after removing the grant while work is still
    // queued. This demonstrates the bounded authorization checkpoint without
    // embedding repository source or a real credential in the replay.
    let principal = PrincipalId::new("synthetic-reviewer")?;
    let logical_repository = RepositoryId::new(
        SecurityDomain::new("synthetic")?,
        "fake",
        "load-fixture",
        "repository-0",
    )?;
    let access_scope = ScopeRecord {
        version: ScopeVersion::new("load-v1")?,
        primary_repository: logical_repository.clone(),
        primary_paths: PathScope::all(),
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: false,
        model_destinations: BTreeSet::new(),
    };
    let grants = vec![Grant {
        principal: principal.clone(),
        repository: logical_repository,
        actions: BTreeSet::from([Action::Graph, Action::Source]),
        paths: PathScope::all(),
    }];
    authorize(
        Action::Source,
        &principal,
        &access_scope,
        &grants,
        Some("src/lib.rs"),
    )?;
    let access_revocation_denied = authorize(
        Action::Source,
        &principal,
        &access_scope,
        &[],
        Some("src/lib.rs"),
    )
    .is_err();

    // Replace one current review with a narrower immutable scope after its old
    // job starts running. The scheduler must fence the old job before admitting
    // the replacement at the same provider generation.
    let scope_pr = PRS - 1;
    let scope_repository = repositories[scope_pr % REPOSITORIES].0;
    let replacement_scope =
        storage.register_scope(scope_repository, "load-v2", b"include=src/module-0")?;
    let old_scope_review = latest_review[scope_pr].clone();
    let new_scope_review = format!("pr-{scope_pr}-scope-v2");
    let base = commit(scope_pr + 1);
    let proposal = commit(30_000 + HOT_PR_UPDATES - 1);
    let scope_now = SIMULATED_WINDOW_MS + 102;
    let claimed_old_scope_job = Scheduler::new(&mut storage)
        .claim("scope-change-worker", JobLane::Warm, 4, 60_000, scope_now)?
        .ok_or("scope-change replay could not claim active work")?;
    if claimed_old_scope_job.review_id != old_scope_review {
        return Err("scope-change replay claimed an unexpected review".into());
    }
    let mut scheduler = Scheduler::new(&mut storage);
    scheduler.register_review(
        &new_scope_review,
        scope_repository,
        replacement_scope,
        &base,
        &base,
        &proposal,
        "exact",
        scope_now,
    )?;
    scheduler.replace_current_scope(
        scope_repository,
        &format!("pr-{scope_pr}"),
        latest_generation[scope_pr],
        &old_scope_review,
        &new_scope_review,
        scope_now,
    )?;
    scheduler.enqueue(
        &new_scope_review,
        "analyze",
        400,
        JobLane::Warm,
        &format!("pr-{scope_pr}"),
        latest_generation[scope_pr],
        scope_now,
    )?;
    latest_review[scope_pr] = new_scope_review;
    latest_accepted_at[scope_pr] = Instant::now();
    let scope_change_fenced_old_job = Scheduler::new(&mut storage)
        .checkpoint(&claimed_old_scope_job)
        .is_err();
    if !provider_throttle_observed
        || !provider_credential_revocation_observed
        || !access_revocation_denied
        || !scope_change_fenced_old_job
    {
        return Err("one or more active-work fault assertions failed".into());
    }

    drop(storage);
    let (sqlite_write_p95_us, sqlite_write_max_us) =
        measure_lock_contention(&database, &artifacts, repositories[0].0)?;
    let mut storage = Storage::open(&database, &artifacts)?;
    peak_rss_bytes = peak_rss_bytes.max(process_usage()?.0);

    let graph_commit = commit(90_000);
    let (snapshot, _) = storage.begin_snapshot(
        repositories[0].0,
        &graph_commit,
        repositories[0].1,
        "load-v1",
    )?;
    let graph_paths = (0..100)
        .map(|index| format!("src/module-{index}/file.rs"))
        .collect::<Vec<_>>();
    for chunk in graph_paths.chunks(100) {
        storage.record_graph_paths(snapshot, chunk)?;
    }
    drop(storage);

    let coordinator_restart = Instant::now();
    let mut storage = Storage::open(&database, &artifacts)?;
    let coordinator_restart_ms = coordinator_restart.elapsed().as_millis() as u64;

    let session_barrier = Arc::new(Barrier::new(REVIEWER_SESSIONS));
    let mut sessions = Vec::new();
    for _ in 0..REVIEWER_SESSIONS {
        let database = database.clone();
        let artifacts = artifacts.clone();
        let barrier = Arc::clone(&session_barrier);
        sessions.push(thread::spawn(move || -> Result<Vec<u64>, String> {
            let storage = Storage::open(&database, &artifacts)?;
            let scope = PathScope::all();
            barrier.wait();
            let mut samples = Vec::new();
            for _ in 0..INTERACTIONS_PER_SESSION {
                let started = Instant::now();
                let page = storage.graph_children(snapshot, None, &scope, 100, None)?;
                if page.paths.is_empty() {
                    return Err("graph replay returned no paths".into());
                }
                samples.push(started.elapsed().as_micros().max(1) as u64);
            }
            Ok(samples)
        }));
    }
    let mut graph_samples_us = Vec::new();
    for session in sessions {
        graph_samples_us.extend(session.join().map_err(|_| "reviewer session panicked")??);
    }

    let mut now = SIMULATED_WINDOW_MS + 10_000;
    let abandoned = Scheduler::new(&mut storage)
        .claim("crashed-worker", JobLane::Warm, 4, 60_000, now)?
        .ok_or("load replay could not claim crash fixture")?;
    drop(storage);
    now += 60_001;
    let mut storage = Storage::open(&database, &artifacts)?;
    let drain_started = Instant::now();
    let mut completed = 0usize;
    let mut recovered_expired_lease = false;
    let mut event_to_ready_samples_ms = Vec::new();
    loop {
        let claim =
            Scheduler::new(&mut storage).claim("replay-worker", JobLane::Warm, 4, 60_000, now)?;
        let Some(job) = claim else { break };
        recovered_expired_lease |= job.id == abandoned.id && job.attempt >= 2;
        Scheduler::new(&mut storage).complete(
            &job,
            "replay-worker",
            &format!("result-{}", job.id),
            now + 1,
        )?;
        let pr = job
            .provider_pr_id
            .strip_prefix("pr-")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value < PRS)
            .ok_or("replay job has an invalid synthetic PR identity")?;
        event_to_ready_samples_ms.push(latest_accepted_at[pr].elapsed().as_millis() as u64);
        completed += 1;
        now += 2;
    }
    let drain_ms = drain_started.elapsed().as_millis() as u64;
    let final_stats = Scheduler::new(&mut storage).stats(now)?;
    let graph_p95_us = percentile(&mut graph_samples_us, 95);
    let event_to_ready_p95_ms = percentile(&mut event_to_ready_samples_ms, 95);
    let event_to_ready_max_ms = event_to_ready_samples_ms.iter().copied().max().unwrap_or(0);
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let (final_rss_bytes, final_cpu_ms) = process_usage()?;
    peak_rss_bytes = peak_rss_bytes.max(final_rss_bytes);
    let cpu_time_ms = final_cpu_ms.saturating_sub(initial_cpu_ms);
    let disk_bytes = directory_bytes(&output)?;
    let parallelism = thread::available_parallelism().map_or(1, usize::from);
    let report = format!(
        concat!(
            "{{\n",
            "  \"schema\": \"codeshouldbehave-phase1-load-replay-v5\",\n",
            "  \"status\": \"harness_checkpoint_not_capacity_acceptance\",\n",
            "  \"seed\": 1,\n",
            "  \"hardware_parallelism\": {parallelism},\n",
            "  \"primary_prs\": {PRS},\n",
            "  \"revisions_per_pr\": {REVISIONS},\n",
            "  \"repositories\": {REPOSITORIES},\n",
            "  \"simulated_window_ms\": {SIMULATED_WINDOW_MS},\n",
            "  \"burst_updates\": {BURST_UPDATES},\n",
            "  \"hot_pr_updates\": {HOT_PR_UPDATES},\n",
            "  \"reviewer_sessions\": {REVIEWER_SESSIONS},\n",
            "  \"graph_interactions\": {interactions},\n",
            "  \"primary_setup_ms\": {primary_setup_ms},\n",
            "  \"burst_setup_ms\": {burst_setup_ms},\n",
            "  \"coordinator_restart_ms\": {coordinator_restart_ms},\n",
            "  \"drain_ms\": {drain_ms},\n",
            "  \"total_elapsed_ms\": {elapsed_ms},\n",
            "  \"graph_query_p95_us\": {graph_p95_us},\n",
            "  \"event_to_ready_samples\": {event_to_ready_samples},\n",
            "  \"event_to_ready_p95_ms\": {event_to_ready_p95_ms},\n",
            "  \"event_to_ready_max_ms\": {event_to_ready_max_ms},\n",
            "  \"sqlite_write_p95_us\": {sqlite_write_p95_us},\n",
            "  \"sqlite_write_max_us\": {sqlite_write_max_us},\n",
            "  \"initial_rss_bytes\": {initial_rss_bytes},\n",
            "  \"peak_sampled_rss_bytes\": {peak_rss_bytes},\n",
            "  \"final_rss_bytes\": {final_rss_bytes},\n",
            "  \"cpu_time_ms\": {cpu_time_ms},\n",
            "  \"disk_bytes_before_report\": {disk_bytes},\n",
            "  \"completed_current_jobs\": {completed},\n",
            "  \"final_queued\": {queued},\n",
            "  \"final_running\": {running},\n",
            "  \"final_failed\": {failed},\n",
            "  \"duplicate_generation_rejected\": {duplicate_rejected},\n",
            "  \"delayed_generation_rejected\": {delayed_rejected},\n",
            "  \"expired_lease_recovered\": {recovered_expired_lease},\n",
            "  \"provider_throttle_observed\": {provider_throttle_observed},\n",
            "  \"provider_credential_revocation_observed\": {provider_credential_revocation_observed},\n",
            "  \"access_revocation_denied\": {access_revocation_denied},\n",
            "  \"scope_change_fenced_old_job\": {scope_change_fenced_old_job},\n",
            "  \"cold_analyzed_blobs\": {cold_analyzed},\n",
            "  \"warm_reused_blobs\": {warm_reused},\n",
            "  \"mixed_analyzed_blobs\": {mixed_analyzed},\n",
            "  \"mixed_reused_blobs\": {mixed_reused},\n",
            "  \"cold_analysis_ms\": {cold_ms},\n",
            "  \"warm_analysis_ms\": {warm_ms},\n",
            "  \"mixed_analysis_ms\": {mixed_ms},\n",
            "  \"clean_rebuild_match\": {clean_rebuild_match},\n",
            "  \"pending_scenarios\": [\"source_preview_browser_timing\",\"retained_three_run_acceptance\"]\n",
            "}}\n"
        ),
        parallelism = parallelism,
        PRS = PRS,
        REVISIONS = REVISIONS,
        REPOSITORIES = REPOSITORIES,
        SIMULATED_WINDOW_MS = SIMULATED_WINDOW_MS,
        BURST_UPDATES = BURST_UPDATES,
        HOT_PR_UPDATES = HOT_PR_UPDATES,
        REVIEWER_SESSIONS = REVIEWER_SESSIONS,
        interactions = REVIEWER_SESSIONS * INTERACTIONS_PER_SESSION,
        primary_setup_ms = primary_setup_ms,
        burst_setup_ms = burst_setup_ms,
        coordinator_restart_ms = coordinator_restart_ms,
        drain_ms = drain_ms,
        elapsed_ms = elapsed_ms,
        graph_p95_us = graph_p95_us,
        event_to_ready_samples = event_to_ready_samples_ms.len(),
        event_to_ready_p95_ms = event_to_ready_p95_ms,
        event_to_ready_max_ms = event_to_ready_max_ms,
        sqlite_write_p95_us = sqlite_write_p95_us,
        sqlite_write_max_us = sqlite_write_max_us,
        initial_rss_bytes = initial_rss_bytes,
        peak_rss_bytes = peak_rss_bytes,
        final_rss_bytes = final_rss_bytes,
        cpu_time_ms = cpu_time_ms,
        disk_bytes = disk_bytes,
        completed = completed,
        queued = final_stats.queued,
        running = final_stats.running,
        failed = final_stats.failed,
        duplicate_rejected = duplicate_rejected,
        delayed_rejected = delayed_rejected,
        recovered_expired_lease = recovered_expired_lease,
        provider_throttle_observed = provider_throttle_observed,
        provider_credential_revocation_observed = provider_credential_revocation_observed,
        access_revocation_denied = access_revocation_denied,
        scope_change_fenced_old_job = scope_change_fenced_old_job,
        cold_analyzed = cache_replay.cold_analyzed,
        warm_reused = cache_replay.warm_reused,
        mixed_analyzed = cache_replay.mixed_analyzed,
        mixed_reused = cache_replay.mixed_reused,
        cold_ms = cache_replay.cold_ms,
        warm_ms = cache_replay.warm_ms,
        mixed_ms = cache_replay.mixed_ms,
        clean_rebuild_match = cache_replay.clean_rebuild_match,
    );
    write_new(&output.join("report.json"), report.as_bytes())?;
    println!("wrote {}", output.join("report.json").display());
    Ok(())
}
