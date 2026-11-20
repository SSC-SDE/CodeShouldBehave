//! `csb` — local, network-free behavioral review.
//!
//! The command reads Git objects only. It never checks out, builds, runs hooks,
//! contacts a provider or executes reviewed code.

use flowdiff::{
    Action, Cancellation, ComparisonMode, Grant, PathScope, PrincipalId, RepositoryId,
    ReviewRequest, ScopeRecord, ScopeVersion, SecurityDomain, analyze_review, csb_serve, csb_site,
    is_worktree_clean, prepare_local_context_access, resolve_to_exact_sha,
};
use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const EXIT_OK: u8 = 0;
const EXIT_INCOMPLETE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_GIT: u8 = 3;
const EXIT_INTERNAL: u8 = 4;

struct CliError {
    code: u8,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_USAGE,
            message: message.into(),
        }
    }
    fn git(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_GIT,
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: EXIT_INTERNAL,
            message: message.into(),
        }
    }
}

fn help() -> &'static str {
    "csb — local behavioral Git review\n\n\
Usage:\n\
  csb review --repo <path> --base <commit-sha> --head <commit-sha> --out-dir <new-dir>\n\
             [--merge-base] [--context <scope.json>] [--cache-dir <path>]\n\
             [--design off|auto] [--design-context <accepted-context.json>]\n\
             [--disable-hld] [--disable-lld]\n\
             [--allow-dirty] [--summary check|markdown|json]\n\
  csb serve --dir <review-dir> [--host <addr>] [--port <port>] [--open]\n\
  csb verify --dir <review-dir>\n\
  csb explain --dir <review-dir> --boundary <hld-change-id> --model <id>\n\
              [--provider ollama|openai] [--endpoint <url>]\n\
              [--key-file <path>] [--send] [--out <prefix>]\n\
  csb version\n\
  csb help\n\n\
Review:\n\
  --base, --head       Exact 40-character commit IDs. HEAD, branch names, tags\n\
                       and abbreviated IDs are rejected.\n\
  --out-dir            New or empty directory. Existing output is never overwritten.\n\
  --merge-base         Compare the merge base of base/head with head.\n\
  --context            Explicit scope manifest for additional local evidence.\n\
  --summary            Stdout format: check (default), markdown or json.\n\
  --design auto        Derive provisional HLD/LLD from the committed Cargo\n\
                       workspace. Components are workspace-derived, not\n\
                       human-confirmed, and are labeled provisional.\n\
  --design-context     Use a bounded accepted-context JSON as confirmed design\n\
                       input (requires --features server).\n\
  --disable-hld        Skip HLD generation (also disables LLD).\n\
  --disable-lld        Skip LLD generation.\n\
  --allow-dirty        Skip the clean-worktree check (analysis still reads\n\
                       committed objects only).\n\n\
Exit codes:\n\
  0 complete review · 1 incomplete coverage · 2 usage · 3 Git · 4 internal ·\n\
  5 AI/provider failure\n\n\
review, serve and verify never contact the network. `explain --send` contacts\n\
only the selected AI provider. The command never installs packages or executes\n\
reviewed code. Serve the output directory with `csb serve`; this is required\n\
when a browser refuses local file access.\n"
}

fn review_help() -> &'static str {
    "csb review — analyze one exact commit pair\n\n\
Usage: csb review --repo <path> --base <sha> --head <sha> --out-dir <new-dir>\n\
                   [--merge-base] [--context <scope.json>] [--cache-dir <path>]\n\
                   [--design off|auto] [--design-context <accepted-context.json>]\n\
                   [--disable-hld] [--disable-lld]\n\
                   [--allow-dirty] [--summary check|markdown|json]\n"
}

fn serve_help() -> &'static str {
    "csb serve — serve a generated review over local HTTP\n\n\
Usage: csb serve --dir <review-dir> [--host <addr>] [--port <port>] [--open]\n\
Defaults: host 127.0.0.1, port 8080.\n"
}

fn verify_help() -> &'static str {
    "csb verify — recheck every artifact hash in a review directory\n\n\
Usage: csb verify --dir <review-dir>\n"
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SummaryFormat {
    Check,
    Markdown,
    Json,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DesignMode {
    Off,
    Auto,
    Context(PathBuf),
}

struct ReviewOptions {
    repository: PathBuf,
    base: String,
    head: String,
    out: PathBuf,
    comparison: ComparisonMode,
    context: Option<PathBuf>,
    cache: Option<PathBuf>,
    allow_dirty: bool,
    summary: SummaryFormat,
    design: DesignMode,
    disable_hld: bool,
    disable_lld: bool,
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, CliError> {
    args.next()
        .ok_or_else(|| CliError::usage(format!("Missing value for {flag}")))
}

fn parse_review(arguments: Vec<String>) -> Result<Option<ReviewOptions>, CliError> {
    let mut repository: Option<PathBuf> = None;
    let mut base = None;
    let mut head = None;
    let mut out = None;
    let mut comparison = ComparisonMode::Exact;
    let mut context = None;
    let mut cache = None;
    let mut allow_dirty = false;
    let mut summary = SummaryFormat::Check;
    let mut design = DesignMode::Off;
    let mut disable_hld = false;
    let mut disable_lld = false;
    let mut args = arguments.into_iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                print!("{}", review_help());
                return Ok(None);
            }
            "--merge-base" => comparison = ComparisonMode::MergeBase,
            "--allow-dirty" => allow_dirty = true,
            "--repo" => repository = Some(PathBuf::from(value(&mut args, "--repo")?)),
            "--base" | "--baseline" => base = Some(value(&mut args, &flag)?),
            "--head" | "--proposal" => head = Some(value(&mut args, &flag)?),
            "--out-dir" => out = Some(PathBuf::from(value(&mut args, "--out-dir")?)),
            "--context" => context = Some(PathBuf::from(value(&mut args, "--context")?)),
            "--cache-dir" => cache = Some(PathBuf::from(value(&mut args, "--cache-dir")?)),
            "--design" => {
                design = match value(&mut args, "--design")?.as_str() {
                    "off" => DesignMode::Off,
                    "auto" => DesignMode::Auto,
                    other => {
                        return Err(CliError::usage(format!(
                            "--design must be off or auto (got {other}); use --design-context for a confirmed file"
                        )));
                    }
                }
            }
            "--design-context" => {
                design = DesignMode::Context(PathBuf::from(value(&mut args, "--design-context")?));
            }
            "--disable-hld" => disable_hld = true,
            "--disable-lld" => disable_lld = true,
            "--summary" => {
                summary = match value(&mut args, "--summary")?.as_str() {
                    "check" => SummaryFormat::Check,
                    "markdown" => SummaryFormat::Markdown,
                    "json" => SummaryFormat::Json,
                    other => {
                        return Err(CliError::usage(format!(
                            "--summary must be check, markdown or json (got {other})"
                        )));
                    }
                }
            }
            other => {
                return Err(CliError::usage(format!(
                    "Unknown review option: {other}\n{}",
                    review_help()
                )));
            }
        }
    }
    Ok(Some(ReviewOptions {
        repository: repository.ok_or_else(|| CliError::usage("review requires --repo <path>"))?,
        base: base.ok_or_else(|| CliError::usage("review requires --base <commit-sha>"))?,
        head: head.ok_or_else(|| CliError::usage("review requires --head <commit-sha>"))?,
        out: out.ok_or_else(|| CliError::usage("review requires --out-dir <new-dir>"))?,
        comparison,
        context,
        cache,
        allow_dirty,
        summary,
        design,
        disable_hld,
        disable_lld,
    }))
}

fn exact_id(flag: &str, value: &str) -> Result<(), CliError> {
    let normalized = value.to_ascii_lowercase();
    if normalized.len() != 40 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::usage(format!(
            "{flag} must be an exact 40-character hexadecimal commit ID; symbolic refs, tags and abbreviated IDs are rejected"
        )));
    }
    Ok(())
}

fn default_cache(repository: &Path) -> PathBuf {
    let canonical = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.to_path_buf());
    let digest = csb_site::sha256(canonical.to_string_lossy().as_bytes());
    env::temp_dir().join("csb-cache").join(&digest[..16])
}

#[cfg(not(feature = "server"))]
fn build_design(
    options: &ReviewOptions,
    _: &Path,
    _: &flowdiff::AnalysisResult,
    _: &[Grant],
    _: &Cancellation,
) -> Result<Option<csb_site::DesignArtifacts>, CliError> {
    let _ = (options.disable_hld, options.disable_lld);
    if options.design == DesignMode::Off {
        Ok(None)
    } else {
        Err(CliError::usage(
            "--design requires a build with --features server",
        ))
    }
}

#[cfg(feature = "server")]
fn build_design(
    options: &ReviewOptions,
    repository: &Path,
    result: &flowdiff::AnalysisResult,
    grants: &[Grant],
    cancellation: &Cancellation,
) -> Result<Option<csb_site::DesignArtifacts>, CliError> {
    if options.design == DesignMode::Off {
        return Ok(None);
    }
    if options.disable_hld && !options.disable_lld {
        return Err(CliError::usage(
            "--disable-hld requires --disable-lld because LLD needs an HLD parent",
        ));
    }
    if options.disable_hld && options.disable_lld {
        return Ok(None);
    }
    let confirmed = match &options.design {
        DesignMode::Off => unreachable!(),
        DesignMode::Auto => flowdiff::design::auto::provisional_context(
            repository,
            &result.proposal,
            result.scope(),
        )
        .map_err(CliError::usage)?,
        DesignMode::Context(path) => {
            let value = read_design_context(path)?;
            if value["commit"].as_str() != Some(result.proposal.as_str()) {
                return Err(CliError::usage(
                    "--design-context commit does not match the reviewed --head",
                ));
            }
            value
        }
    };
    let mut artifact = flowdiff::ReviewArtifact::from_analysis(result);
    if !options.disable_hld {
        artifact
            .generate_hld(result, repository, &confirmed, grants, cancellation)
            .map_err(CliError::internal)?;
    }
    if !options.disable_lld {
        artifact
            .generate_lld(result, repository, &confirmed, grants, cancellation)
            .map_err(CliError::internal)?;
    }
    Ok(Some(design_artifacts(&artifact)))
}

#[cfg(feature = "server")]
fn read_design_context(path: &Path) -> Result<serde_json::Value, CliError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| {
        CliError::usage(format!(
            "{} is not a readable design context",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(CliError::usage(format!(
            "{} must be a regular design context file of at most 1 MiB",
            path.display()
        )));
    }
    let bytes = std::fs::read(path).map_err(|_| {
        CliError::usage(format!(
            "{} is not a readable design context",
            path.display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| CliError::usage("Design context is not valid JSON"))?;
    Ok(value.get("confirmed_context").cloned().unwrap_or(value))
}

#[cfg(feature = "server")]
fn design_artifacts(artifact: &flowdiff::ReviewArtifact) -> csb_site::DesignArtifacts {
    use csb_site::{DesignReport, DesignRow};
    let authority = artifact
        .hld
        .as_ref()
        .map(|hld| hld.authority.clone())
        .unwrap_or_else(|| "unavailable".into());
    let cap = |value: &str| value.chars().take(500).collect::<String>();
    let component = |binding: Option<&flowdiff::lld::LldParentBinding>| {
        binding
            .and_then(|binding| binding.component_id.clone())
            .unwrap_or_else(|| "—".into())
    };
    let hld_rows = artifact
        .hld
        .as_ref()
        .map(|hld| {
            hld.delta
                .changes
                .iter()
                .take(100)
                .map(|change| DesignRow {
                    kind: cap(&change.kind),
                    before: cap(change.before_name.as_deref().unwrap_or("—")),
                    after: cap(change.after_name.as_deref().unwrap_or("—")),
                    detail: cap(&change.reason),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let hld_questions = artifact
        .hld
        .as_ref()
        .map(|hld| {
            hld.delta
                .questions
                .iter()
                .take(50)
                .map(|item| cap(item))
                .collect()
        })
        .unwrap_or_default();
    let hld_gaps = artifact
        .hld
        .as_ref()
        .map(|hld| {
            hld.delta
                .coverage
                .gaps
                .iter()
                .take(50)
                .map(|item| cap(item))
                .collect()
        })
        .unwrap_or_default();
    let hld_mermaid = artifact
        .hld
        .as_ref()
        .map(|hld| hld.delta.mermaid.chars().take(256 * 1024).collect())
        .unwrap_or_default();
    let lld_rows = artifact
        .lld
        .as_ref()
        .map(|lld| {
            lld.delta
                .changes
                .iter()
                .take(100)
                .map(|change| DesignRow {
                    kind: cap(&change.kind),
                    before: cap(&component(change.before_parent.as_ref())),
                    after: cap(&component(change.after_parent.as_ref())),
                    detail: cap(&format!(
                        "{} · {}{}",
                        change.classification,
                        change.logical_key,
                        if change.limitations.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", change.limitations.join("; "))
                        }
                    )),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let lld_gaps = artifact
        .lld
        .as_ref()
        .map(|lld| {
            lld.delta
                .limitations
                .iter()
                .take(50)
                .map(|item| cap(item))
                .collect()
        })
        .unwrap_or_default();
    let mut markdown = format!(
        "# High-level design ({authority})\n\nHLD status: {}\nLLD status: {}\n\n",
        artifact.hld_status, artifact.lld_status
    );
    if let Some(hld) = &artifact.hld {
        markdown.push_str("## Baseline\n\n");
        markdown.push_str(&flowdiff::design::render_hld_markdown(&hld.baseline));
        markdown.push_str("\n\n## Proposal\n\n");
        markdown.push_str(&flowdiff::design::render_hld_markdown(&hld.proposal));
        markdown.push_str("\n\n## Delta\n\n```mermaid\n");
        markdown.push_str(&hld.delta.mermaid);
        markdown.push_str("\n```\n\n");
    }
    if !lld_rows.is_empty() {
        markdown.push_str(
            "## LLD changes\n\n| Change | Before | After | Detail |\n|---|---|---|---|\n",
        );
        for row in &lld_rows {
            markdown.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                row.kind, row.before, row.after, row.detail
            ));
        }
    }
    let json = serde_json::json!({
        "authority": authority.as_str(),
        "hld_status": artifact.hld_status,
        "lld_status": artifact.lld_status,
        "hld": &artifact.hld,
        "lld": &artifact.lld,
    });
    csb_site::DesignArtifacts {
        markdown,
        json,
        report: DesignReport {
            authority,
            hld_status: artifact.hld_status.into(),
            lld_status: artifact.lld_status.into(),
            hld_rows,
            hld_questions,
            hld_gaps,
            hld_mermaid,
            lld_rows,
            lld_gaps,
        },
    }
}

fn review(arguments: Vec<String>) -> Result<u8, CliError> {
    let Some(options) = parse_review(arguments)? else {
        return Ok(EXIT_OK);
    };
    let repository = &options.repository;
    if !repository.is_dir() {
        return Err(CliError::usage(format!(
            "{} is not a Git repository directory",
            repository.display()
        )));
    }
    let clean = is_worktree_clean(repository).map_err(|error| {
        CliError::usage(format!(
            "{} is not a readable Git repository: {error}",
            repository.display()
        ))
    })?;
    if !clean && !options.allow_dirty {
        return Err(CliError::usage(
            "the worktree is dirty. Commit or stash changes, or pass --allow-dirty. Analysis always reads committed objects only.",
        ));
    }
    exact_id("--base", &options.base)?;
    exact_id("--head", &options.head)?;
    let base = resolve_to_exact_sha(repository, &options.base).map_err(CliError::git)?;
    let head = resolve_to_exact_sha(repository, &options.head).map_err(CliError::git)?;
    if base == head {
        return Err(CliError::usage("--base and --head must differ"));
    }
    if options.out.exists() {
        if !options.out.is_dir() {
            return Err(CliError::usage(format!(
                "{} exists and is not a directory. Choose a new --out-dir.",
                options.out.display()
            )));
        }
        let empty = std::fs::read_dir(&options.out)
            .map_err(|error| {
                CliError::usage(format!("Cannot read {}: {error}", options.out.display()))
            })?
            .next()
            .is_none();
        if !empty {
            return Err(CliError::usage(format!(
                "{} already exists and is not empty. Choose a new --out-dir.",
                options.out.display()
            )));
        }
    }
    if let Some(context) = &options.context {
        if !context.is_file() {
            return Err(CliError::usage(format!(
                "{} is not a readable context manifest",
                context.display()
            )));
        }
    }

    let principal = PrincipalId::new("local-cli-user").map_err(CliError::internal)?;
    let security_domain = SecurityDomain::new("local-cli").map_err(CliError::internal)?;
    let repository_name = repository
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name != ".")
        .unwrap_or("selected-repository");
    let repository_id = RepositoryId::new(
        security_domain.clone(),
        "local-git",
        "local-user",
        repository_name,
    )
    .map_err(CliError::internal)?;
    let (contexts, context_grants) = if let Some(context) = &options.context {
        prepare_local_context_access(context, &security_domain, &principal)
            .map_err(CliError::usage)?
    } else {
        (vec![], vec![])
    };
    let scope = ScopeRecord {
        version: ScopeVersion::local_default(),
        primary_repository: repository_id.clone(),
        primary_paths: PathScope::all(),
        contexts,
        contracts: BTreeSet::new(),
        export_allowed: true,
        model_destinations: BTreeSet::new(),
    };
    let mut grants = vec![Grant {
        principal: principal.clone(),
        repository: repository_id,
        actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source, Action::Export]),
        paths: PathScope::all(),
    }];
    grants.extend(context_grants);
    let cache = options
        .cache
        .clone()
        .unwrap_or_else(|| default_cache(repository));
    let request = ReviewRequest {
        repository: repository.clone(),
        baseline: base,
        proposal: head,
        comparison: options.comparison,
        principal: principal.clone(),
        scope,
        current_grants: grants.clone(),
        context_manifest: options.context.clone(),
        cache_dir: cache,
    };
    let cancellation = Cancellation::default();
    let result = analyze_review(&request, &cancellation)
        .map_err(|error| CliError::git(format!("analysis failed: {error}")))?;
    result
        .authorize(Action::Export, &grants, None)
        .map_err(|error| CliError::internal(format!("export refused: {error}")))?;

    let design = build_design(&options, repository, &result, &grants, &cancellation)?;
    let manifest =
        csb_site::emit(&options.out, &result, design.as_ref()).map_err(CliError::internal)?;

    match options.summary {
        SummaryFormat::Check => println!("{}", csb_site::coverage_line(&result)),
        SummaryFormat::Markdown => print!("{}", csb_site::summary_markdown(&result)),
        SummaryFormat::Json => {
            let (kind, label) = csb_site::verdict(&result);
            let value = serde_json::json!({
                "verdict": {"kind": kind, "label": label},
                "coverage": {
                    "indexed": result.coverage.indexed,
                    "unsupported": result.coverage.unsupported,
                    "excluded": result.coverage.excluded,
                    "failed": result.coverage.failed,
                    "deferred": result.coverage.deferred,
                    "enumeration_complete": result.coverage.enumeration_complete,
                },
                "counts": {
                    "files": result.review.files.len(),
                    "flow_edits": result.review.flow_edits(),
                    "impacts": result.review.impacts.len(),
                },
                "manifest": {
                    "content_sha256": manifest.content_sha256,
                    "artifacts": manifest.files.len(),
                },
                "incomplete": manifest.incomplete,
            });
            println!(
                "{}",
                serde_json::to_string(&value)
                    .map_err(|error| CliError::internal(error.to_string()))?
            );
        }
    }
    eprintln!("csb: saved review site to {}", options.out.display());
    if manifest.incomplete {
        eprintln!("csb: coverage is incomplete; see summary.md and coverage warnings");
        Ok(EXIT_INCOMPLETE)
    } else {
        Ok(EXIT_OK)
    }
}

fn serve(arguments: Vec<String>) -> Result<u8, CliError> {
    let mut directory = None;
    let mut host = "127.0.0.1".to_string();
    let mut port = 8080;
    let mut open = false;
    let mut args = arguments.into_iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                print!("{}", serve_help());
                return Ok(EXIT_OK);
            }
            "--open" => open = true,
            "--dir" => directory = Some(PathBuf::from(value(&mut args, "--dir")?)),
            "--host" => host = value(&mut args, "--host")?,
            "--port" => {
                port = value(&mut args, "--port")?
                    .parse()
                    .map_err(|_| CliError::usage("--port must be a number between 0 and 65535"))?
            }
            other => {
                return Err(CliError::usage(format!(
                    "Unknown serve option: {other}\n{}",
                    serve_help()
                )));
            }
        }
    }
    let directory =
        directory.ok_or_else(|| CliError::usage("serve requires --dir <review-dir>"))?;
    csb_serve::serve(&directory, &host, port, open).map_err(CliError::usage)?;
    Ok(EXIT_OK)
}

fn verify(arguments: Vec<String>) -> Result<u8, CliError> {
    let mut directory = None;
    let mut args = arguments.into_iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                print!("{}", verify_help());
                return Ok(EXIT_OK);
            }
            "--dir" => directory = Some(PathBuf::from(value(&mut args, "--dir")?)),
            other => {
                return Err(CliError::usage(format!(
                    "Unknown verify option: {other}\n{}",
                    verify_help()
                )));
            }
        }
    }
    let directory =
        directory.ok_or_else(|| CliError::usage("verify requires --dir <review-dir>"))?;
    let manifest = csb_site::verify(&directory).map_err(|error| CliError {
        code: EXIT_INCOMPLETE,
        message: error,
    })?;
    println!(
        "OK {} artifacts · index {}",
        manifest.files.len(),
        manifest.content_sha256
    );
    if manifest.incomplete {
        eprintln!("csb: review coverage is marked incomplete");
    }
    Ok(EXIT_OK)
}

fn explain_help() -> &'static str {
    "csb explain — advisory AI over a verified review\n\n\
Usage: csb explain --dir <review-dir> --boundary <hld-change-id> --model <id>\n\
                   [--provider ollama|openai] [--endpoint <url>]\n\
                   [--key-file <path>] [--send] [--out <prefix>]\n\n\
Defaults: provider ollama, endpoint http://127.0.0.1:11434, preview only.\n\
--send is required to contact the provider. --out writes <prefix>.md/.json;\n\
the default is <review-dir>/ai.md and ai.json. Preview is ai-preview.json.\n\
OpenAI-compatible keys come from CSB_AI_API_KEY or a 0600 --key-file.\n"
}

#[cfg(not(feature = "ai"))]
fn explain(arguments: Vec<String>) -> Result<u8, CliError> {
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        print!("{}", explain_help());
        return Ok(EXIT_OK);
    }
    Err(CliError::usage(
        "csb explain requires a build with --features ai",
    ))
}

#[cfg(feature = "ai")]
fn explain(arguments: Vec<String>) -> Result<u8, CliError> {
    use flowdiff::csb_explain::{ExplainOptions, Provider};
    let mut dir = None;
    let mut boundary = None;
    let mut provider = Provider::Ollama;
    let mut model = None;
    let mut endpoint = None;
    let mut key_file = None;
    let mut send = false;
    let mut out = None;
    let mut args = arguments.into_iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                print!("{}", explain_help());
                return Ok(EXIT_OK);
            }
            "--dir" => dir = Some(PathBuf::from(value(&mut args, "--dir")?)),
            "--boundary" => boundary = Some(value(&mut args, "--boundary")?),
            "--provider" => {
                provider = Provider::parse(&value(&mut args, "--provider")?)
                    .map_err(|error| CliError::usage(error.message()))?
            }
            "--model" => model = Some(value(&mut args, "--model")?),
            "--endpoint" => endpoint = Some(value(&mut args, "--endpoint")?),
            "--key-file" => key_file = Some(PathBuf::from(value(&mut args, "--key-file")?)),
            "--send" => send = true,
            "--preview" => send = false,
            "--out" => out = Some(PathBuf::from(value(&mut args, "--out")?)),
            other => {
                return Err(CliError::usage(format!(
                    "Unknown explain option: {other}\n{}",
                    explain_help()
                )));
            }
        }
    }
    let options = ExplainOptions {
        dir: dir.ok_or_else(|| CliError::usage("explain requires --dir <review-dir>"))?,
        boundary: boundary
            .ok_or_else(|| CliError::usage("explain requires --boundary <hld-change-id>"))?,
        provider,
        model: model.ok_or_else(|| CliError::usage("explain requires --model <id>"))?,
        endpoint,
        key_file,
        send,
        out,
    };
    match flowdiff::csb_explain::run(&options) {
        Ok(outcome) => {
            eprintln!("csb: wrote AI preview to {}", outcome.preview.display());
            if let Some((markdown, json)) = outcome.explanation {
                eprintln!(
                    "csb: wrote advisory explanation to {} and {}",
                    markdown.display(),
                    json.display()
                );
            }
            Ok(EXIT_OK)
        }
        Err(error) => Err(CliError {
            code: error.code(),
            message: error.message().to_owned(),
        }),
    }
}

fn run() -> Result<u8, CliError> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    let rest = args.collect::<Vec<_>>();
    match command.as_str() {
        "review" => review(rest),
        "serve" => serve(rest),
        "verify" => verify(rest),
        "explain" => explain(rest),
        "version" | "--version" | "-V" => {
            println!("csb {}", env!("CARGO_PKG_VERSION"));
            Ok(EXIT_OK)
        }
        "help" | "--help" | "-h" => {
            print!("{}", help());
            Ok(EXIT_OK)
        }
        other => Err(CliError::usage(format!(
            "Unknown command: {other}\n{}",
            help()
        ))),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("csb: {}", error.message);
            ExitCode::from(error.code)
        }
    }
}
