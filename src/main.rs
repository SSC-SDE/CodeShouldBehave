use flowdiff::{
    Action, ArtifactStorage, Cancellation, ComparisonMode, FileArtifactStorage, Grant,
    IndexRequest, PathScope, PrincipalId, RepositoryId, ReviewRequest, ScopeRecord, ScopeVersion,
    SecurityDomain, analyze_index, analyze_review, demo_review, prepare_local_context_access,
    render_html, render_markdown,
};
use std::{collections::BTreeSet, env, path::PathBuf, process::ExitCode};

struct Options {
    repository: PathBuf,
    baseline: String,
    proposal: String,
    output: PathBuf,
    artifact: Option<PathBuf>,
    github_repository: Option<String>,
    check_draft: Option<PathBuf>,
    dashboard_origin: Option<String>,
    pr_number: Option<u64>,
    format: String,
    comparison: ComparisonMode,
    context_manifest: Option<PathBuf>,
    onboarding_context: Option<PathBuf>,
    internal_artifact: bool,
    disable_hld: bool,
    disable_lld: bool,
    cache_dir: PathBuf,
}

fn help() -> &'static str {
    "flowdiff — see where code changes its flow\n\nUsage:\n  flowdiff review --base <ref> [--head <ref>] [--repo <path>]\n                  [--output <file>] [--format html|markdown] [--merge-base] [--context <scope-file>] [--cache-dir <path>] [--artifact <json-file>]\n  flowdiff index --repo <path> --base <commit-or-local-ref> [--cache-dir <path>] [--output <file>]\n  flowdiff demo [--output <file>] [--format html|markdown]\n\nConfirmed primary scope (server feature): --onboarding-context <accepted-context.json>. This is trusted operator input, not remote authorization. --internal-artifact retains export policy while writing private host artifacts; do not serve the output files.\nHLD generation (server feature): confirmed component context enables source-backed HLD artifacts. --disable-hld disables new HLD for this comparison and creates a distinct review identity; it does not relabel structural sketches as LLD.\nLLD generation (server feature): confirmed context and HLD enable a scoped Rust LLD catalog and retained syntax input; function details are generated on demand. --disable-lld disables new LLD indexing with a distinct review identity.\nOffline check draft (server feature): --github-repository owner/name --pr-number N --dashboard-origin https://dashboard.example.com --github-check-draft check.json. No GitHub write occurs.\nDefaults: repo ., head HEAD, format html, output review.html (or review.md).\nFor PRs use --merge-base to compare the common ancestor with the head.\nThe --baseline and --proposal flags alias --base and --head. Architecture indexes reuse a local .flowdiff-cache directory.\nFetch PR refs yourself; Flowdiff never contacts the network or runs reviewed code.\nOutput must be a new file; existing files and symlinks are never overwritten.\n"
}

fn parse() -> Result<Option<(String, Options)>, String> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "--help".into());
    if matches!(command.as_str(), "--help" | "-h" | "help") {
        print!("{}", help());
        return Ok(None);
    }
    if command == "--version" {
        println!("flowdiff {}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }
    if command != "review" && command != "demo" && command != "index" {
        return Err(format!("Unknown command: {command}\n{}", help()));
    }
    let mut options = Options {
        repository: ".".into(),
        baseline: String::new(),
        proposal: "HEAD".into(),
        output: PathBuf::new(),
        artifact: None,
        github_repository: None,
        check_draft: None,
        dashboard_origin: None,
        pr_number: None,
        format: "html".into(),
        comparison: ComparisonMode::Exact,
        context_manifest: None,
        onboarding_context: None,
        internal_artifact: false,
        disable_hld: false,
        disable_lld: false,
        cache_dir: ".flowdiff-cache".into(),
    };
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            print!("{}", help());
            return Ok(None);
        }
        if flag == "--merge-base" {
            options.comparison = ComparisonMode::MergeBase;
            continue;
        }
        if flag == "--internal-artifact" {
            options.internal_artifact = true;
            continue;
        }
        if flag == "--disable-lld" {
            options.disable_lld = true;
            continue;
        }
        if flag == "--disable-hld" {
            options.disable_hld = true;
            continue;
        }
        if !matches!(
            flag.as_str(),
            "--github-repository"
                | "--github-check-draft"
                | "--dashboard-origin"
                | "--pr-number"
                | "--artifact"
                | "--repo"
                | "--base"
                | "--head"
                | "--output"
                | "--format"
                | "--context"
                | "--onboarding-context"
                | "--cache-dir"
                | "--baseline"
                | "--proposal"
        ) {
            return Err(format!("Unknown option: {flag}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {flag}"))?;
        match flag.as_str() {
            "--github-repository" => options.github_repository = Some(value),
            "--github-check-draft" => options.check_draft = Some(value.into()),
            "--dashboard-origin" => options.dashboard_origin = Some(value),
            "--pr-number" => {
                options.pr_number = Some(value.parse().map_err(|_| "Invalid PR number")?)
            }
            "--artifact" => options.artifact = Some(value.into()),
            "--repo" => options.repository = value.into(),
            "--base" | "--baseline" => options.baseline = value,
            "--head" | "--proposal" => options.proposal = value,
            "--output" => options.output = value.into(),
            "--format" => options.format = value,
            "--context" => options.context_manifest = Some(value.into()),
            "--onboarding-context" => options.onboarding_context = Some(value.into()),
            "--cache-dir" => options.cache_dir = value.into(),
            _ => unreachable!(),
        }
    }
    if !matches!(options.format.as_str(), "html" | "markdown") {
        return Err("Format must be html or markdown".into());
    }
    if options.output.as_os_str().is_empty() {
        options.output = if options.format == "html" {
            "review.html"
        } else {
            "review.md"
        }
        .into();
    }
    if command != "demo" && options.baseline.is_empty() {
        return Err("review/index requires --base <commit-or-local-ref>".into());
    }
    let check_options = options.check_draft.is_some()
        || options.dashboard_origin.is_some()
        || options.pr_number.is_some();
    if check_options
        && (command != "review"
            || options.check_draft.is_none()
            || options.dashboard_origin.is_none()
            || options.pr_number.is_none()
            || options.github_repository.is_none())
    {
        return Err("Check drafts require review, --github-repository, --github-check-draft, --dashboard-origin and --pr-number together".into());
    }
    #[cfg(not(feature = "server"))]
    if check_options {
        return Err("Check drafts require a build with --features server".into());
    }
    if options.onboarding_context.is_some()
        && (command != "review"
            || options.github_repository.is_none()
            || options.context_manifest.is_some())
    {
        return Err("Confirmed onboarding context requires a GitHub review and cannot be combined with an unconfirmed context manifest".into());
    }
    #[cfg(not(feature = "server"))]
    if options.onboarding_context.is_some() {
        return Err("Confirmed onboarding context requires --features server".into());
    }
    if options.internal_artifact
        && (options.onboarding_context.is_none()
            || options.artifact.is_none()
            || options.format != "html"
            || options.check_draft.is_some())
    {
        return Err("Internal artifact mode requires confirmed context, a typed artifact and HTML private output".into());
    }
    if (options.disable_hld || options.disable_lld)
        && (command != "review" || options.onboarding_context.is_none())
    {
        return Err("Disabling HLD generation requires a confirmed-context review".into());
    }
    Ok(Some((command, options)))
}

fn run() -> Result<(), String> {
    let Some((command, options)) = parse()? else {
        return Ok(());
    };
    let cancellation = Cancellation::default();
    let principal = PrincipalId::new("local-cli-user")?;
    let security_domain = SecurityDomain::new("local-cli")?;
    let repository_id = if let Some(slug) = &options.github_repository {
        let (owner, name) = slug
            .split_once('/')
            .ok_or("GitHub repository must be owner/name")?;
        if name.contains('/') {
            return Err("Invalid GitHub repository identity".into());
        }
        RepositoryId::new(security_domain.clone(), "github", owner, name)?
    } else {
        RepositoryId::new(
            security_domain.clone(),
            "local-git",
            "local-user",
            options
                .repository
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("selected-repository"),
        )?
    };
    let (contexts, context_grants) = if let Some(manifest) = &options.context_manifest {
        prepare_local_context_access(manifest, &security_domain, &principal)?
    } else {
        (vec![], vec![])
    };
    #[cfg(feature = "server")]
    let accepted_context = options
        .onboarding_context
        .as_ref()
        .map(|path| flowdiff::load_onboarding_review_context(path, &repository_id))
        .transpose()?;
    #[cfg(feature = "server")]
    let repository_id = if let Some((_, _, provenance)) = &accepted_context {
        RepositoryId::new(
            SecurityDomain::new(
                provenance["source_security_domain"]
                    .as_str()
                    .ok_or("Invalid accepted domain")?,
            )?,
            repository_id.provider(),
            repository_id.owner(),
            repository_id.name(),
        )?
    } else {
        repository_id
    };
    #[cfg(feature = "server")]
    let (primary_paths, scope_version) = accepted_context
        .as_ref()
        .map(|(paths, version, _)| {
            use sha2::{Digest, Sha256};
            let mut digest = Sha256::new();
            digest.update(b"csb-design-review-profile-v2");
            digest.update(version.as_str());
            digest.update(if options.disable_hld {
                "disabled"
            } else {
                flowdiff::design::HLD_GENERATOR_VERSION
            });
            digest.update(if options.disable_lld || options.disable_hld {
                "disabled"
            } else {
                flowdiff::lld::LLD_GENERATOR_VERSION
            });
            digest.update(flowdiff::lld::LLD_IR_VERSION);
            digest.update(flowdiff::lld::LLD_PROFILE);
            let key = digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            (
                paths.clone(),
                ScopeVersion::new(format!("onboarding-hld-{key}"))
                    .expect("bounded digest scope version"),
            )
        })
        .unwrap_or((PathScope::all(), ScopeVersion::local_default()));
    #[cfg(not(feature = "server"))]
    let (primary_paths, scope_version) = (PathScope::all(), ScopeVersion::local_default());
    #[cfg(feature = "server")]
    let export_allowed = accepted_context
        .as_ref()
        .is_none_or(|(_, _, provenance)| provenance["export_allowed"] == true);
    #[cfg(not(feature = "server"))]
    let export_allowed = true;
    let scope = ScopeRecord {
        version: scope_version,
        primary_repository: repository_id.clone(),
        primary_paths,
        contexts,
        contracts: BTreeSet::new(),
        export_allowed,
        model_destinations: BTreeSet::new(),
    };
    let mut current_grants = vec![Grant {
        principal: principal.clone(),
        repository: repository_id,
        actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source, Action::Export]),
        paths: PathScope::all(),
    }];
    current_grants.extend(context_grants);
    let result = match command.as_str() {
        "demo" => None,
        "index" => Some(analyze_index(
            &IndexRequest {
                repository: options.repository.clone(),
                snapshot: options.baseline.clone(),
                principal: principal.clone(),
                scope: scope.clone(),
                current_grants: current_grants.clone(),
                cache_dir: options.cache_dir.clone(),
            },
            &cancellation,
        )?),
        _ => Some(analyze_review(
            &ReviewRequest {
                repository: options.repository.clone(),
                baseline: options.baseline.clone(),
                proposal: options.proposal.clone(),
                comparison: options.comparison,
                principal,
                scope,
                current_grants: current_grants.clone(),
                context_manifest: options.context_manifest.clone(),
                cache_dir: options.cache_dir.clone(),
            },
            &cancellation,
        )?),
    };
    let demo;
    #[cfg(feature = "server")]
    if let (Some(result), Some((_, _, provenance))) = (&result, &accepted_context) {
        if provenance.get("drift").is_some()
            && provenance["drift"]["target_commit"].as_str() != Some(result.proposal.as_str())
        {
            return Err("Accepted context drift target differs from the selected proposal".into());
        }
    }
    if let Some(result) = &result {
        // The loopback host keeps these files private and serves authorized graph/source
        // responses. Persisting an internal artifact never enables the Export capability.
        result.authorize(
            if options.internal_artifact {
                Action::Graph
            } else {
                Action::Export
            },
            &current_grants,
            None,
        )?;
    }
    let review = if let Some(result) = &result {
        &result.review
    } else {
        demo = demo_review();
        &demo
    };
    let rendered = if options.format == "html" {
        render_html(review)
    } else {
        render_markdown(review)
    };
    #[cfg(feature = "server")]
    let artifact = if options.artifact.is_some() || options.check_draft.is_some() {
        let result = result
            .as_ref()
            .ok_or("Typed artifacts require an actual repository comparison")?;
        let mut artifact = flowdiff::ReviewArtifact::from_analysis(result);
        artifact.onboarding_context = accepted_context
            .as_ref()
            .map(|(_, _, provenance)| provenance.clone());
        if !options.disable_hld {
            if let Some(confirmed) = artifact
                .onboarding_context
                .as_ref()
                .and_then(|context| context.get("confirmed_context"))
                .cloned()
            {
                artifact.generate_hld(
                    result,
                    &options.repository,
                    &confirmed,
                    &current_grants,
                    &cancellation,
                )?;
                if !options.disable_lld {
                    artifact.generate_lld(
                        result,
                        &options.repository,
                        &confirmed,
                        &current_grants,
                        &cancellation,
                    )?;
                }
            }
        }
        Some(artifact)
    } else {
        None
    };
    #[cfg(feature = "server")]
    if let Some(path) = &options.check_draft {
        let result = result
            .as_ref()
            .ok_or("Check drafts require an actual repository comparison")?;
        let draft = artifact
            .as_ref()
            .ok_or("Missing typed check artifact")?
            .architecture_check_draft(&flowdiff::CheckTarget {
                repository: &result.scope().primary_repository,
                pr_number: options.pr_number.ok_or("Missing PR number")?,
                selected_baseline: result.selected_baseline.as_str(),
                head: result.proposal.as_str(),
                dashboard_origin: options
                    .dashboard_origin
                    .as_deref()
                    .ok_or("Missing dashboard origin")?,
            })?;
        let bytes = serde_json::to_vec(&draft).map_err(|_| "Cannot encode check draft")?;
        FileArtifactStorage.write_new(path, &bytes)?;
        println!(
            "Saved offline GitHub check draft {} · no check was published",
            path.display()
        );
    }
    if let Some(path) = &options.artifact {
        #[cfg(feature = "server")]
        {
            FileArtifactStorage.write_new(
                path,
                &artifact
                    .as_ref()
                    .ok_or("Missing typed review artifact")?
                    .to_json()?,
            )?;
        }
        #[cfg(not(feature = "server"))]
        {
            let _ = path;
            return Err("--artifact requires a build with --features server".into());
        }
    }
    FileArtifactStorage.write_new(&options.output, rendered.as_bytes())?;
    println!(
        "Reviewed {} files · {} flow edits · {} inferred impact links",
        review.files.len(),
        review.flow_edits(),
        review.impacts.len()
    );
    println!("Saved {}", options.output.display());
    println!(
        "Architecture: {} → {} files; {} blobs analyzed, {} cached",
        review.architecture.before.len(),
        review.architecture.after.len(),
        review.architecture.analyzed,
        review.architecture.reused
    );
    println!(
        "Loaded {} explicitly scoped context sources",
        review.context.len()
    );
    for warning in &review.warnings {
        eprintln!("Note: {warning}");
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flowdiff: {error}");
            ExitCode::FAILURE
        }
    }
}
