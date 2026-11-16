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
             [--allow-dirty] [--summary check|markdown|json]\n\
  csb serve --dir <review-dir> [--host <addr>] [--port <port>] [--open]\n\
  csb verify --dir <review-dir>\n\
  csb version\n\
  csb help\n\n\
Review:\n\
  --base, --head       Exact 40-character commit IDs. HEAD, branch names, tags\n\
                       and abbreviated IDs are rejected.\n\
  --out-dir            New or empty directory. Existing output is never overwritten.\n\
  --merge-base         Compare the merge base of base/head with head.\n\
  --context            Explicit scope manifest for additional local evidence.\n\
  --summary            Stdout format: check (default), markdown or json.\n\
  --allow-dirty        Skip the clean-worktree check (analysis still reads\n\
                       committed objects only).\n\n\
Exit codes:\n\
  0 complete review · 1 incomplete coverage · 2 usage · 3 Git · 4 internal\n\n\
The command never contacts the network, installs packages or executes reviewed\n\
code. Serve the output directory with `csb serve`; this is required when a\n\
browser refuses local file access.\n"
}

fn review_help() -> &'static str {
    "csb review — analyze one exact commit pair\n\n\
Usage: csb review --repo <path> --base <sha> --head <sha> --out-dir <new-dir>\n\
                   [--merge-base] [--context <scope.json>] [--cache-dir <path>]\n\
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

    let manifest = csb_site::emit(&options.out, &result, None).map_err(CliError::internal)?;

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

fn run() -> Result<u8, CliError> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    let rest = args.collect::<Vec<_>>();
    match command.as_str() {
        "review" => review(rest),
        "serve" => serve(rest),
        "verify" => verify(rest),
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
