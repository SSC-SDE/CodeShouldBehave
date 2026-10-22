use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::IpAddr,
    sync::{Arc, Mutex},
};

use crate::{
    ComparisonAccessProvider, ContextPin, ExactComparisonRequest, ExactSourceReader,
    ExportCommands, GraphDirection, PathScope, RepositoryId, ReviewAccessProvider, RevisionSide,
    ScopeChangeRequest, ScopeRecord, ScopeVersion, SecurityDomain, ServiceApi,
    ServiceAuthenticator, ServiceCommands, SnapshotId, Storage,
};

const MAX_HTTP_BODY: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum HttpOperation {
    CreateOnboarding {
        request: Value,
    },
    ReadOnboarding {
        id: String,
        domain: String,
    },
    MutateOnboarding {
        id: String,
        action: String,
        request: Value,
    },
    WorkspacePreferences {
        request: Option<Value>,
    },
    AiCapability,
    ListPrs {
        limit: usize,
        cursor: Option<String>,
    },
    ListRevisions {
        provider_pr_id: String,
        repository_id: i64,
        limit: usize,
        cursor: Option<String>,
    },
    CreateReview {
        request: Value,
    },
    PreviewScope {
        review_id: String,
        request: Value,
    },
    SaveScope {
        review_id: String,
        request: Value,
    },
    ReviewDetails {
        review_id: String,
    },
    ReviewProgress {
        review_id: String,
    },
    ChangedFiles {
        review_id: String,
        limit: usize,
        cursor: Option<String>,
    },
    Graph {
        review_id: String,
        query: Value,
    },
    Source {
        review_id: String,
        query: Value,
    },
    Findings {
        review_id: String,
        limit: usize,
        cursor: Option<String>,
    },
    ReviewAnnotations {
        review_id: String,
    },
    ReviewAnnotationAudit {
        review_id: String,
    },
    WriteReviewAnnotation {
        review_id: String,
        request: Value,
    },
    Job {
        job_id: String,
    },
    CreateExport {
        request: Value,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpFailure {
    Malformed,
    Authentication,
    DeniedOrMissing,
    Conflict,
    TooLarge,
    Unavailable,
    Internal,
}

/// Transport boundary implemented by the service host. Inputs are already
/// bounded and authenticated at the HTTP syntax layer; implementations must
/// still reauthorize current grants in the core for every operation.
pub trait HttpBackend: Send + Sync + 'static {
    fn execute(&self, bearer: &str, operation: HttpOperation) -> Result<Value, HttpFailure>;
}

pub struct ServiceHost<A, P, R> {
    storage: Mutex<Storage>,
    authenticator: A,
    access_provider: P,
    source_reader: R,
}

impl<A, P, R> ServiceHost<A, P, R> {
    pub fn new(storage: Storage, authenticator: A, access_provider: P, source_reader: R) -> Self {
        Self {
            storage: Mutex::new(storage),
            authenticator,
            access_provider,
            source_reader,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryPayload {
    security_domain: String,
    provider: String,
    owner: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewAnnotationPayload {
    id: String,
    visibility: String,
    kind: String,
    target_path: Option<String>,
    target_key: String,
    body: String,
    status: String,
    expected_version: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewPayload {
    repository: RepositoryPayload,
    provider_pr_id: String,
    generation: i64,
    selected_baseline: String,
    proposal: String,
    target_branch: String,
    state: String,
    priority: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportPayload {
    review_id: String,
    format: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextPayload {
    name: String,
    repository: RepositoryPayload,
    commit: String,
    include: Vec<String>,
    exclude: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScopePayload {
    primary_repository: RepositoryPayload,
    primary_include: Vec<String>,
    primary_exclude: Vec<String>,
    contexts: Vec<ContextPayload>,
    contracts: Vec<String>,
    export_allowed: bool,
    model_destinations: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeSavePayload {
    provider_pr_id: String,
    generation: i64,
    priority: i64,
    scope: ScopePayload,
}

fn repository(payload: RepositoryPayload) -> Result<RepositoryId, HttpFailure> {
    RepositoryId::new(
        SecurityDomain::new(payload.security_domain).map_err(|_| HttpFailure::Malformed)?,
        payload.provider,
        payload.owner,
        payload.name,
    )
    .map_err(|_| HttpFailure::Malformed)
}

pub(crate) fn scope(payload: ScopePayload) -> Result<ScopeRecord, HttpFailure> {
    if payload.contexts.len() > 12
        || payload.contracts.len() > 100
        || payload.model_destinations.len() > 32
    {
        return Err(HttpFailure::TooLarge);
    }
    let mut contexts = Vec::with_capacity(payload.contexts.len());
    for context in payload.contexts {
        contexts.push(ContextPin {
            name: context.name,
            repository: repository(context.repository)?,
            commit: SnapshotId::new(context.commit).map_err(|_| HttpFailure::Malformed)?,
            paths: PathScope::new(context.include, context.exclude)
                .map_err(|_| HttpFailure::Malformed)?,
        });
    }
    Ok(ScopeRecord {
        version: ScopeVersion::new("scope-preview").map_err(|_| HttpFailure::Malformed)?,
        primary_repository: repository(payload.primary_repository)?,
        primary_paths: PathScope::new(payload.primary_include, payload.primary_exclude)
            .map_err(|_| HttpFailure::Malformed)?,
        contexts,
        contracts: payload.contracts.into_iter().collect(),
        export_allowed: payload.export_allowed,
        model_destinations: payload.model_destinations.into_iter().collect(),
    })
}

impl<A, P, R> HttpBackend for ServiceHost<A, P, R>
where
    A: ServiceAuthenticator + Send + Sync + 'static,
    P: ReviewAccessProvider + ComparisonAccessProvider + Send + Sync + 'static,
    R: ExactSourceReader + Send + Sync + 'static,
{
    fn execute(&self, bearer: &str, operation: HttpOperation) -> Result<Value, HttpFailure> {
        let mut storage = self.storage.lock().map_err(|_| HttpFailure::Unavailable)?;
        match operation {
            HttpOperation::WorkspacePreferences { request } => {
                let principal = self
                    .authenticator
                    .authenticate(bearer)
                    .map_err(map_core_error)?;
                match request {
                    None => crate::workspace_preferences::read(&storage, &principal),
                    Some(value) => {
                        crate::workspace_preferences::write(&mut storage, &principal, &value)
                    }
                }
                .map_err(|error| {
                    if error == "Preference conflict" {
                        HttpFailure::Conflict
                    } else if error.starts_with("Invalid") {
                        HttpFailure::Malformed
                    } else {
                        HttpFailure::Unavailable
                    }
                })
            }
            HttpOperation::AiCapability => {
                self.authenticator
                    .authenticate(bearer)
                    .map_err(map_core_error)?;
                Ok(
                    json!({"api_version":"v1","contract_version":crate::AI_CONTRACT_VERSION,"capability":"disabled","dispatch_available":false,"credential_input_available":false}),
                )
            }
            HttpOperation::CreateReview { request } => {
                let request: ReviewPayload =
                    serde_json::from_value(request).map_err(|_| HttpFailure::Malformed)?;
                let repository = RepositoryId::new(
                    SecurityDomain::new(request.repository.security_domain)
                        .map_err(|_| HttpFailure::Malformed)?,
                    request.repository.provider,
                    request.repository.owner,
                    request.repository.name,
                )
                .map_err(|_| HttpFailure::Malformed)?;
                let receipt =
                    ServiceCommands::new(&mut storage, &self.authenticator, &self.access_provider)
                        .create_exact_comparison(
                            bearer,
                            &ExactComparisonRequest {
                                repository,
                                provider_pr_id: request.provider_pr_id,
                                generation: request.generation,
                                selected_baseline: request.selected_baseline,
                                proposal: request.proposal,
                                target_branch: request.target_branch,
                                state: request.state,
                                priority: request.priority,
                            },
                        )
                        .map_err(map_core_error)?;
                Ok(json!({
                    "api_version": receipt.api_version,
                    "review_id": receipt.review_id,
                    "job_id": format!("analysis:{}", receipt.job_id),
                    "created": receipt.created,
                }))
            }
            HttpOperation::CreateExport { request } => {
                let request: ExportPayload =
                    serde_json::from_value(request).map_err(|_| HttpFailure::Malformed)?;
                if request.format != "markdown-v1" {
                    return Err(HttpFailure::Malformed);
                }
                let receipt = ExportCommands::new(
                    &mut storage,
                    &self.authenticator,
                    &self.access_provider,
                    &self.source_reader,
                )
                .create_markdown_export(bearer, &request.review_id)
                .map_err(map_core_error)?;
                Ok(json!({
                    "api_version": receipt.api_version,
                    "job_id": receipt.job_id,
                    "review_id": receipt.review_id,
                    "state": receipt.state,
                    "artifact_digest": receipt.artifact_digest,
                }))
            }
            HttpOperation::SaveScope { review_id, request } => {
                let request: ScopeSavePayload =
                    serde_json::from_value(request).map_err(|_| HttpFailure::Malformed)?;
                let receipt =
                    ServiceCommands::new(&mut storage, &self.authenticator, &self.access_provider)
                        .replace_review_scope(
                            bearer,
                            ScopeChangeRequest {
                                review_id,
                                provider_pr_id: request.provider_pr_id,
                                generation: request.generation,
                                priority: request.priority,
                                proposed_scope: scope(request.scope)?,
                            },
                        )
                        .map_err(map_core_error)?;
                Ok(
                    json!({"api_version":receipt.api_version,"review_id":receipt.review_id,"job_id":format!("analysis:{}",receipt.job_id),"created":receipt.created}),
                )
            }
            HttpOperation::PreviewScope { review_id, request } => {
                let proposed: ScopePayload =
                    serde_json::from_value(request).map_err(|_| HttpFailure::Malformed)?;
                let api = ServiceApi::new(
                    &storage,
                    &self.authenticator,
                    &self.access_provider,
                    &self.source_reader,
                );
                let delta = api
                    .preview_scope(bearer, &review_id, &scope(proposed)?)
                    .map_err(map_core_error)?;
                Ok(
                    json!({"api_version":"v1","review_id":review_id,"changes_identity":delta.changes_identity(),"added_contexts":delta.added_contexts,"removed_contexts":delta.removed_contexts,"changed_contexts":delta.changed_contexts,"primary_paths_changed":delta.primary_paths_changed,"contracts_changed":delta.contracts_changed,"export_policy_changed":delta.export_policy_changed,"model_destinations_changed":delta.model_destinations_changed,"added_files":delta.added_files,"removed_files":delta.removed_files,"file_preview_truncated":delta.file_preview_truncated}),
                )
            }
            HttpOperation::ReviewAnnotations { review_id } => {
                let commands = ExportCommands::new(
                    &mut storage,
                    &self.authenticator,
                    &self.access_provider,
                    &self.source_reader,
                );
                let items = commands
                    .review_annotations(bearer, &review_id)
                    .map_err(map_core_error)?;
                let delta = commands
                    .review_continuity_delta(bearer, &review_id)
                    .map_err(map_core_error)?;
                Ok(
                    json!({"api_version":"v1","review_id":review_id,"items":items.into_iter().map(annotation_json).collect::<Vec<_>>(),
                        "delta":{"prior_review_id":delta.prior_review_id,"prior_proposal":delta.prior_proposal,"changed_paths":delta.changed_paths,"truncated":delta.truncated} }),
                )
            }
            HttpOperation::ReviewAnnotationAudit { review_id } => {
                let items = ExportCommands::new(
                    &mut storage,
                    &self.authenticator,
                    &self.access_provider,
                    &self.source_reader,
                )
                .review_annotation_audit(bearer, &review_id)
                .map_err(map_core_error)?;
                Ok(
                    json!({"api_version":"v1","review_id":review_id,"items":items.into_iter().map(|item| json!({
                    "id":item.id,"annotation_id":item.annotation_id,"actor_principal":item.actor_principal,
                    "action":item.action,"from_version":item.from_version,"to_version":item.to_version,"created_at_ms":item.created_at_ms
                })).collect::<Vec<_>>() }),
                )
            }
            HttpOperation::WriteReviewAnnotation { review_id, request } => {
                let request: ReviewAnnotationPayload =
                    serde_json::from_value(request).map_err(|_| HttpFailure::Malformed)?;
                let item = ExportCommands::new(
                    &mut storage,
                    &self.authenticator,
                    &self.access_provider,
                    &self.source_reader,
                )
                .write_review_annotation(
                    bearer,
                    &review_id,
                    &crate::ReviewAnnotationWrite {
                        id: request.id,
                        visibility: request.visibility,
                        kind: request.kind,
                        target_path: request.target_path,
                        target_key: request.target_key,
                        body: request.body,
                        status: request.status,
                        expected_version: request.expected_version,
                    },
                )
                .map_err(map_core_error)?;
                Ok(annotation_json(item))
            }
            operation => {
                let api = ServiceApi::new(
                    &storage,
                    &self.authenticator,
                    &self.access_provider,
                    &self.source_reader,
                );
                execute_read(&api, bearer, operation)
            }
        }
    }
}

type Backend = Arc<dyn HttpBackend>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    repository_id: Option<i64>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQuery {
    side: String,
    symbol: Option<String>,
    path: Option<String>,
    parent: Option<String>,
    direction: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceQuery {
    side: String,
    path: String,
    blob: String,
    start_line: usize,
    end_line: usize,
}

fn execute_read<A, P, R>(
    api: &ServiceApi<'_, A, P, R>,
    bearer: &str,
    operation: HttpOperation,
) -> Result<Value, HttpFailure>
where
    A: ServiceAuthenticator,
    P: ReviewAccessProvider,
    R: ExactSourceReader,
{
    match operation {
        HttpOperation::ListPrs { limit, cursor } => {
            let page = api
                .list_prs(bearer, limit, cursor.as_deref())
                .map_err(map_core_error)?;
            Ok(json!({
                "api_version": page.api_version,
                "items": page.items.into_iter().map(pr_json).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor,
            }))
        }
        HttpOperation::ListRevisions {
            provider_pr_id,
            repository_id,
            limit,
            cursor,
        } => {
            let page = api
                .list_revisions(
                    bearer,
                    repository_id,
                    &provider_pr_id,
                    limit,
                    cursor.as_deref(),
                )
                .map_err(map_core_error)?;
            Ok(json!({
                "api_version": page.api_version,
                "items": page.items.into_iter().map(pr_json).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor,
            }))
        }
        HttpOperation::ReviewDetails { review_id } => {
            let details = api
                .review_details(bearer, &review_id)
                .map_err(map_core_error)?;
            Ok(json!({
                "api_version": details.api_version,
                "review_id": details.review_id,
                "selected_baseline": details.selected_baseline,
                "analysis_baseline": details.analysis_baseline,
                "proposal": details.proposal,
                "comparison_mode": details.comparison_mode,
                "scope_version": details.scope_version,
                "stale": details.stale,
                "capabilities": details.capabilities,
            }))
        }
        HttpOperation::ReviewProgress { review_id } => {
            let progress = api.progress(bearer, &review_id).map_err(map_core_error)?;
            Ok(progress_json(progress))
        }
        HttpOperation::ChangedFiles {
            review_id,
            limit,
            cursor,
        } => {
            let page = api
                .changed_files(bearer, &review_id, limit, cursor.as_deref())
                .map_err(map_core_error)?;
            Ok(json!({
                "api_version": page.api_version,
                "review_id": page.review_id,
                "items": page.items.into_iter().map(|file| json!({
                    "path": file.path,
                    "old_path": file.old_path,
                    "kind": file.kind,
                    "additions": file.additions,
                    "deletions": file.deletions,
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor,
            }))
        }
        HttpOperation::Findings {
            review_id,
            limit,
            cursor,
        } => {
            let page = api
                .findings(bearer, &review_id, limit, cursor.as_deref())
                .map_err(map_core_error)?;
            let items = page
                .items
                .into_iter()
                .map(|finding| {
                    json!({
                        "id": finding.id,
                        "kind": finding.kind,
                        "resolution": finding.resolution,
                        "confidence": finding.confidence,
                        "source_path": finding.source_path,
                        "source_start_line": finding.source_start_line,
                        "source_end_line": finding.source_end_line,
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "api_version": page.api_version,
                "review_id": page.review_id,
                "items": items,
                "next_cursor": page.next_cursor,
            }))
        }
        HttpOperation::Job { job_id } => {
            let job = api.job_details(bearer, &job_id).map_err(map_core_error)?;
            Ok(json!({
                "api_version": job.api_version,
                "job_id": job.job_id,
                "review_id": job.review_id,
                "kind": job.kind,
                "state": job.state,
                "attempt_count": job.attempt_count,
                "terminal_reason": job.terminal_reason,
                "artifact_digest": job.artifact_digest,
            }))
        }
        HttpOperation::Source { review_id, query } => {
            let query: SourceQuery =
                serde_json::from_value(query).map_err(|_| HttpFailure::Malformed)?;
            let span = api
                .source_span(
                    bearer,
                    &review_id,
                    parse_side(&query.side)?,
                    &query.path,
                    &query.blob,
                    query.start_line,
                    query.end_line,
                )
                .map_err(map_core_error)?;
            Ok(json!({
                "api_version": span.api_version,
                "review_id": span.review_id,
                "commit": span.commit,
                "blob_id": span.blob_id,
                "path": span.path,
                "start_line": span.start_line,
                "end_line": span.end_line,
                "text": span.text,
            }))
        }
        HttpOperation::Graph { review_id, query } => {
            let query: GraphQuery =
                serde_json::from_value(query).map_err(|_| HttpFailure::Malformed)?;
            let side = parse_side(&query.side)?;
            let limit = query.limit.unwrap_or(100);
            match (query.symbol, query.path) {
                (Some(symbol), None) if query.parent.is_none() => {
                    let direction = match query.direction.as_deref().unwrap_or("outgoing") {
                        "incoming" => GraphDirection::Incoming,
                        "outgoing" => GraphDirection::Outgoing,
                        _ => return Err(HttpFailure::Malformed),
                    };
                    let page = api
                        .graph_neighborhood(
                            bearer,
                            &review_id,
                            side,
                            &symbol,
                            direction,
                            limit,
                            query.cursor.as_deref(),
                        )
                        .map_err(map_core_error)?;
                    let edges = page
                        .edges
                        .into_iter()
                        .map(|edge| {
                            json!({
                                "id": edge.id,
                                "stable_key": edge.stable_key,
                                "from_symbol": edge.from_symbol,
                                "to_symbol": edge.to_symbol,
                                "kind": edge.kind,
                                "confidence": edge.confidence,
                                "resolution": edge.resolution,
                                "source_path": edge.source_path,
                            })
                        })
                        .collect::<Vec<_>>();
                    Ok(
                        json!({ "api_version": "v1", "kind": "neighborhood", "edges": edges,
                        "next_cursor": page.next_cursor, "omitted": page.omitted, "coverage": page.coverage }),
                    )
                }
                (None, Some(path)) if query.parent.is_none() && query.direction.is_none() => {
                    let page = api
                        .graph_symbols(
                            bearer,
                            &review_id,
                            side,
                            &path,
                            limit,
                            query.cursor.as_deref(),
                        )
                        .map_err(map_core_error)?;
                    let blob_id = api
                        .source_blob_selector(bearer, &review_id, side, &path)
                        .map_err(map_core_error)?;
                    let symbols = page
                        .symbols
                        .into_iter()
                        .map(|symbol| {
                            json!({
                                "stable_key": symbol.stable_key,
                                "correspondence_key": symbol.correspondence_key,
                                "path": symbol.path,
                                "kind": symbol.kind,
                                "name": symbol.name,
                                "start_line": symbol.start_line,
                                "start_column": symbol.start_column,
                                "end_line": symbol.end_line,
                                "end_column": symbol.end_column,
                                "blob_id": blob_id,
                            })
                        })
                        .collect::<Vec<_>>();
                    Ok(
                        json!({ "api_version": "v1", "kind": "symbols", "symbols": symbols,
                        "next_cursor": page.next_cursor }),
                    )
                }
                (None, None) if query.direction.is_none() => {
                    let page = api
                        .graph_children(
                            bearer,
                            &review_id,
                            side,
                            query.parent.as_deref(),
                            limit,
                            query.cursor.as_deref(),
                        )
                        .map_err(map_core_error)?;
                    Ok(
                        json!({ "api_version": "v1", "kind": "children", "paths": page.paths,
                        "next_cursor": page.next_cursor }),
                    )
                }
                _ => Err(HttpFailure::Malformed),
            }
        }
        HttpOperation::WorkspacePreferences { .. }
        | HttpOperation::AiCapability
        | HttpOperation::CreateOnboarding { .. }
        | HttpOperation::ReadOnboarding { .. }
        | HttpOperation::MutateOnboarding { .. }
        | HttpOperation::CreateReview { .. }
        | HttpOperation::CreateExport { .. }
        | HttpOperation::PreviewScope { .. }
        | HttpOperation::SaveScope { .. } => Err(HttpFailure::Internal),
        HttpOperation::ReviewAnnotations { .. }
        | HttpOperation::ReviewAnnotationAudit { .. }
        | HttpOperation::WriteReviewAnnotation { .. } => Err(HttpFailure::Internal),
    }
}

fn parse_side(side: &str) -> Result<RevisionSide, HttpFailure> {
    match side {
        "baseline" => Ok(RevisionSide::Baseline),
        "proposal" => Ok(RevisionSide::Proposal),
        _ => Err(HttpFailure::Malformed),
    }
}

fn pr_json(item: crate::PrSummary) -> Value {
    json!({
        "repository_id": item.repository_id,
        "repository": {
            "security_domain": item.security_domain,
            "provider": item.provider,
            "owner": item.repository_owner,
            "name": item.repository_name,
        },
        "provider_pr_id": item.provider_pr_id,
        "generation": item.generation,
        "review_id": item.review_id,
        "base_commit": item.base_commit,
        "head_commit": item.head_commit,
        "target_branch": item.target_branch,
        "state": item.state,
        "priority": item.priority,
        "observed_at_ms": item.observed_at_ms,
    })
}

fn annotation_json(item: crate::ReviewAnnotation) -> Value {
    json!({"id":item.id,"review_id":item.review_id,"proposal_commit":item.proposal_commit,
        "owner_principal":item.owner_principal,"visibility":item.visibility,"kind":item.kind,
        "target_path":item.target_path,"target_key":item.target_key,"body":item.body,
        "status":item.status,"version":item.version,"updated_at_ms":item.updated_at_ms})
}

fn progress_json(progress: crate::ReviewProgress) -> Value {
    json!({
        "api_version": progress.api_version,
        "review_id": progress.review_id,
        "job_state": progress.job_state,
        "attempt_count": progress.attempt_count,
        "terminal_reason": progress.terminal_reason,
        "coverage": progress.coverage,
        "stale": progress.stale,
    })
}

fn map_core_error(error: String) -> HttpFailure {
    let lower = error.to_ascii_lowercase();
    if lower.contains("authentication") || lower.contains("session") {
        HttpFailure::Authentication
    } else if lower.contains("access denied")
        || lower.contains("outside the authorized")
        || lower.contains("unavailable")
        || lower.contains("absent from")
    {
        HttpFailure::DeniedOrMissing
    } else if lower.contains("stale") || lower.contains("conflict") {
        HttpFailure::Conflict
    } else if lower.contains("too large") || lower.contains("limit") || lower.contains("excessive")
    {
        HttpFailure::TooLarge
    } else if lower.contains("invalid") || lower.contains("must") {
        HttpFailure::Malformed
    } else {
        HttpFailure::Internal
    }
}

pub fn router(backend: Arc<dyn HttpBackend>) -> Router {
    Router::new()
        .route("/workspace", get(workspace))
        .route("/assets/workspace.js", get(workspace_js))
        .route("/assets/workspace.css", get(workspace_css))
        .route("/assets/inter.ttf", get(inter_font))
        .route("/assets/jetbrains-mono.ttf", get(jetbrains_font))
        .route("/assets/mermaid.js", get(mermaid_js))
        .route("/v1/ai/capability", get(ai_capability))
        .route(
            "/v1/me/workspace-preferences",
            get(read_workspace_preferences).put(write_workspace_preferences),
        )
        .route("/assets/workspace-preferences.js", get(preferences_js))
        .route("/assets/workspace-registry.json", get(preferences_registry))
        .route("/onboarding", get(onboarding_page))
        .route("/assets/onboarding.js", get(onboarding_js))
        .route("/assets/onboarding.css", get(onboarding_css))
        .route("/v1/onboarding", post(create_onboarding))
        .route("/v1/onboarding/{id}", get(read_onboarding))
        .route("/v1/onboarding/{id}/{action}", post(mutate_onboarding))
        .route("/v1/prs", get(list_prs))
        .route("/v1/prs/{provider_pr_id}/revisions", get(list_revisions))
        .route("/v1/reviews", post(create_review))
        .route("/v1/reviews/{review_id}", get(review_details))
        .route("/v1/reviews/{review_id}/progress", get(review_progress))
        .route("/v1/reviews/{review_id}/files", get(changed_files))
        .route("/v1/reviews/{review_id}/scope/preview", post(preview_scope))
        .route("/v1/reviews/{review_id}/scope", put(save_scope))
        .route("/v1/reviews/{review_id}/graph", get(graph))
        .route("/v1/reviews/{review_id}/source", get(source))
        .route("/v1/reviews/{review_id}/findings", get(findings))
        .route(
            "/v1/reviews/{review_id}/annotations",
            get(review_annotations).put(write_review_annotation),
        )
        .route(
            "/v1/reviews/{review_id}/annotations/audit",
            get(review_annotation_audit),
        )
        .route("/v1/jobs/{job_id}", get(job))
        .route("/v1/exports", post(create_export))
        .layer(DefaultBodyLimit::max(MAX_HTTP_BODY))
        .with_state(backend)
}

async fn workspace() -> Html<&'static str> {
    Html(include_str!("workspace.html"))
}

async fn onboarding_page() -> Html<&'static str> {
    Html(include_str!("onboarding.html"))
}
async fn onboarding_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("onboarding.css"),
    )
}
async fn onboarding_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("onboarding.js"),
    )
}
async fn create_onboarding(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::CreateOnboarding { request },
    )
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnboardingQuery {
    security_domain: String,
}
async fn read_onboarding(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<OnboardingQuery>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::ReadOnboarding {
            id,
            domain: query.security_domain,
        },
    )
    .await
}
async fn mutate_onboarding(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path((id, action)): Path<(String, String)>,
    Json(request): Json<Value>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::MutateOnboarding {
            id,
            action,
            request,
        },
    )
    .await
}

async fn workspace_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("workspace.js"),
    )
}

async fn workspace_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("workspace.css"),
    )
}

async fn inter_font() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "font/ttf")],
        include_bytes!("fonts/inter.ttf").as_slice(),
    )
}
async fn jetbrains_font() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "font/ttf")],
        include_bytes!("fonts/jetbrains-mono.ttf").as_slice(),
    )
}

async fn mermaid_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../docs/prototypes/vendor/mermaid.tiny-12.1.0.js"),
    )
}

/// Serve only on an already-bound loopback socket. External exposure requires
/// a separate, reviewed deployment adapter with TLS and trusted-proxy controls.
pub async fn serve_loopback(
    listener: tokio::net::TcpListener,
    backend: Arc<dyn HttpBackend>,
) -> Result<(), String> {
    let address = listener
        .local_addr()
        .map_err(|error| format!("Cannot inspect service listener: {error}"))?;
    if !is_loopback(address.ip()) {
        return Err("Step 1.11 service listener must be loopback-only".into());
    }
    axum::serve(listener, router(backend))
        .await
        .map_err(|error| format!("HTTP service failed: {error}"))
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

async fn list_prs(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::ListPrs {
            limit: query.limit.unwrap_or(100),
            cursor: query.cursor,
        },
    )
    .await
}

async fn ai_capability(State(backend): State<Backend>, headers: HeaderMap) -> Response {
    dispatch(backend, headers, HttpOperation::AiCapability).await
}

async fn list_revisions(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(provider_pr_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::ListRevisions {
            provider_pr_id,
            repository_id: query.repository_id.unwrap_or(-1),
            limit: query.limit.unwrap_or(100),
            cursor: query.cursor,
        },
    )
    .await
}

async fn create_review(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    dispatch(backend, headers, HttpOperation::CreateReview { request }).await
}

async fn preview_scope(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    Json(request): Json<Value>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::PreviewScope { review_id, request },
    )
    .await
}

async fn save_scope(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    Json(request): Json<Value>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::SaveScope { review_id, request },
    )
    .await
}

async fn review_details(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
) -> Response {
    dispatch(backend, headers, HttpOperation::ReviewDetails { review_id }).await
}

async fn review_progress(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::ReviewProgress { review_id },
    )
    .await
}

async fn changed_files(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::ChangedFiles {
            review_id,
            limit: query.limit.unwrap_or(100),
            cursor: query.cursor,
        },
    )
    .await
}

async fn graph(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    Query(query): Query<GraphQuery>,
) -> Response {
    let query = json!({
        "side": query.side,
        "symbol": query.symbol,
        "path": query.path,
        "parent": query.parent,
        "direction": query.direction,
        "limit": query.limit.unwrap_or(100),
        "cursor": query.cursor,
    });
    dispatch(backend, headers, HttpOperation::Graph { review_id, query }).await
}

async fn source(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    Query(query): Query<SourceQuery>,
) -> Response {
    let query = json!({
        "side": query.side,
        "path": query.path,
        "blob": query.blob,
        "start_line": query.start_line,
        "end_line": query.end_line,
    });
    dispatch(backend, headers, HttpOperation::Source { review_id, query }).await
}

async fn findings(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::Findings {
            review_id,
            limit: query.limit.unwrap_or(100),
            cursor: query.cursor,
        },
    )
    .await
}

async fn review_annotations(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::ReviewAnnotations { review_id },
    )
    .await
}

async fn review_annotation_audit(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::ReviewAnnotationAudit { review_id },
    )
    .await
}

async fn write_review_annotation(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    Json(request): Json<Value>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::WriteReviewAnnotation { review_id, request },
    )
    .await
}

async fn job(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Response {
    dispatch(backend, headers, HttpOperation::Job { job_id }).await
}

async fn create_export(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    dispatch(backend, headers, HttpOperation::CreateExport { request }).await
}

async fn dispatch(backend: Backend, headers: HeaderMap, operation: HttpOperation) -> Response {
    let bearer = match bearer(&headers) {
        Ok(token) => token,
        Err(failure) => return failure_response(failure),
    };
    let result = tokio::task::spawn_blocking(move || backend.execute(&bearer, operation)).await;
    match result {
        Ok(Ok(value)) => json_response(StatusCode::OK, value),
        Ok(Err(failure)) => failure_response(failure),
        Err(_) => failure_response(HttpFailure::Internal),
    }
}

fn bearer(headers: &HeaderMap) -> Result<String, HttpFailure> {
    for name in [
        "forwarded",
        "x-forwarded-user",
        "x-forwarded-email",
        "x-remote-user",
    ] {
        if headers.contains_key(name) {
            return Err(HttpFailure::Authentication);
        }
    }
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(HttpFailure::Authentication)?;
    let token = value
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty() && token.len() <= 4096)
        .ok_or(HttpFailure::Authentication)?;
    Ok(token.to_owned())
}

fn failure_response(failure: HttpFailure) -> Response {
    let (status, code) = match failure {
        HttpFailure::Malformed => (StatusCode::BAD_REQUEST, "invalid_request"),
        HttpFailure::Authentication => (StatusCode::UNAUTHORIZED, "authentication_required"),
        HttpFailure::DeniedOrMissing => (StatusCode::NOT_FOUND, "not_found"),
        HttpFailure::Conflict => (StatusCode::CONFLICT, "stale_or_conflicting_request"),
        HttpFailure::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
        HttpFailure::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable"),
        HttpFailure::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    };
    let mut response = json_response(status, json!({ "error": code, "api_version": "v1" }));
    if failure == HttpFailure::Authentication {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"codeshouldbehave\""),
        );
    }
    response
}

fn json_response(status: StatusCode, value: Value) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn preferences_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("workspace-preferences.js"),
    )
}
async fn preferences_registry() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        crate::workspace_preferences::REGISTRY,
    )
}
async fn read_workspace_preferences(
    State(backend): State<Backend>,
    headers: HeaderMap,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::WorkspacePreferences { request: None },
    )
    .await
}
async fn write_workspace_preferences(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Response {
    dispatch(
        backend,
        headers,
        HttpOperation::WorkspacePreferences {
            request: Some(value),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Action, Grant, JobLane, PathScope, PrincipalId, ReviewAccess, Scheduler, ScopeRecord,
        ScopeVersion, SessionRegistry, SourceBlob,
    };
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use std::{
        collections::BTreeSet,
        fs,
        path::PathBuf,
        sync::{Mutex, RwLock},
    };
    use tower::ServiceExt;

    #[derive(Default)]
    struct BackendMock {
        calls: Mutex<Vec<(String, HttpOperation)>>,
    }

    impl HttpBackend for BackendMock {
        fn execute(&self, bearer: &str, operation: HttpOperation) -> Result<Value, HttpFailure> {
            self.calls.lock().unwrap().push((bearer.into(), operation));
            Ok(json!({ "api_version": "v1" }))
        }
    }

    #[tokio::test]
    async fn requires_bearer_and_rejects_forwarded_identity() {
        let backend = Arc::new(BackendMock::default());
        let missing = router(backend.clone())
            .oneshot(Request::get("/v1/prs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        let spoofed = router(backend.clone())
            .oneshot(
                Request::get("/v1/prs")
                    .header(header::AUTHORIZATION, "Bearer valid-token")
                    .header("x-forwarded-user", "admin@example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(spoofed.status(), StatusCode::UNAUTHORIZED);
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn workspace_shell_contains_no_session_or_review_data() {
        let response = router(Arc::new(BackendMock::default()))
            .oneshot(Request::get("/workspace").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(
            BodyExt::collect(response.into_body())
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("Pull request inbox"));
        assert!(body.contains("Kept in memory only"));
        assert!(!body.contains("localStorage"));
        assert!(!body.contains("Authorization: Bearer"));
    }

    #[tokio::test]
    async fn workspace_serves_the_pinned_local_mermaid_renderer() {
        use sha2::{Digest, Sha256};
        let response = router(Arc::new(BackendMock::default()))
            .oneshot(
                Request::get("/assets/mermaid.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let digest = Sha256::digest(&body)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "143c1b9345c45963ff429da4bdcd9051316a43522ea9edd7ccb7a87ff947a804"
        );
    }

    #[tokio::test]
    async fn routes_bounded_versioned_requests_without_shared_caching() {
        let backend = Arc::new(BackendMock::default());
        let response = router(backend.clone())
            .oneshot(
                Request::get("/v1/reviews/review-1/findings?limit=20&cursor=finding1%3A4")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["api_version"],
            "v1"
        );
        assert_eq!(
            backend.calls.lock().unwrap()[0].1,
            HttpOperation::Findings {
                review_id: "review-1".into(),
                limit: 20,
                cursor: Some("finding1:4".into()),
            }
        );
    }

    #[tokio::test]
    async fn routes_changed_files_with_bound_pagination() {
        let backend = Arc::new(BackendMock::default());
        let response = router(backend.clone())
            .oneshot(
                Request::get("/v1/reviews/review-1/files?limit=25&cursor=file1%3A4")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            backend.calls.lock().unwrap()[0].1,
            HttpOperation::ChangedFiles {
                review_id: "review-1".into(),
                limit: 25,
                cursor: Some("file1:4".into()),
            }
        );
    }

    #[tokio::test]
    async fn routes_snapshot_bound_annotations_and_read_only_audit() {
        let backend = Arc::new(BackendMock::default());
        let get_response = router(backend.clone())
            .oneshot(
                Request::get("/v1/reviews/review-1/annotations")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let audit_response = router(backend.clone())
            .oneshot(
                Request::get("/v1/reviews/review-1/annotations/audit")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(audit_response.status(), StatusCode::OK);
        let put_response=router(backend.clone()).oneshot(Request::put("/v1/reviews/review-1/annotations")
            .header(header::AUTHORIZATION,"Bearer session-token").header(header::CONTENT_TYPE,"application/json")
            .body(Body::from(r#"{"id":"a","visibility":"private","kind":"question","target_path":"src/lib.rs","target_key":"src/lib.rs","body":"why","status":"active","expected_version":0}"#)).unwrap()).await.unwrap();
        assert_eq!(put_response.status(), StatusCode::OK);
        let calls = backend.calls.lock().unwrap();
        assert_eq!(
            calls[0].1,
            HttpOperation::ReviewAnnotations {
                review_id: "review-1".into()
            }
        );
        assert_eq!(
            calls[1].1,
            HttpOperation::ReviewAnnotationAudit {
                review_id: "review-1".into()
            }
        );
        assert!(
            matches!(calls[2].1,HttpOperation::WriteReviewAnnotation{ref review_id,..} if review_id=="review-1")
        );
    }

    #[tokio::test]
    async fn ai_capability_is_authenticated_and_has_no_dispatch_surface() {
        let backend = Arc::new(BackendMock::default());
        let response = router(backend.clone())
            .oneshot(
                Request::get("/v1/ai/capability")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            backend.calls.lock().unwrap()[0].1,
            HttpOperation::AiCapability
        );
    }

    #[tokio::test]
    async fn revision_history_is_bound_to_repository_identity() {
        let backend = Arc::new(BackendMock::default());
        let response = router(backend.clone())
            .oneshot(
                Request::get("/v1/prs/42/revisions?repository_id=7&limit=10")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            backend.calls.lock().unwrap()[0].1,
            HttpOperation::ListRevisions {
                provider_pr_id: "42".into(),
                repository_id: 7,
                limit: 10,
                cursor: None,
            }
        );
    }

    #[tokio::test]
    async fn scope_preview_and_save_routes_preserve_review_identity() {
        let backend = Arc::new(BackendMock::default());
        let scope = json!({
            "primary_repository":{"security_domain":"tenant","provider":"github","owner":"acme","name":"api"},
            "primary_include":["src"],"primary_exclude":["src/private"],"contexts":[],
            "contracts":["contracts/openapi.yaml"],"export_allowed":true,"model_destinations":[]
        });
        let preview = router(backend.clone())
            .oneshot(
                Request::post("/v1/reviews/review-1/scope/preview")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(scope.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK);
        let save = router(backend.clone())
            .oneshot(
                Request::put("/v1/reviews/review-1/scope")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"provider_pr_id":"42","generation":1,"priority":80,"scope":scope})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(save.status(), StatusCode::OK);
        let calls = backend.calls.lock().unwrap();
        assert!(
            matches!(&calls[0].1,HttpOperation::PreviewScope { review_id, .. } if review_id == "review-1")
        );
        assert!(
            matches!(&calls[1].1,HttpOperation::SaveScope { review_id, .. } if review_id == "review-1")
        );
    }

    #[tokio::test]
    async fn oversized_json_is_rejected_before_backend() {
        let backend = Arc::new(BackendMock::default());
        let response = router(backend.clone())
            .oneshot(
                Request::post("/v1/reviews")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        "{{\"data\":\"{}\"}}",
                        "x".repeat(MAX_HTTP_BODY)
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn denied_and_missing_share_one_nondisclosing_response() {
        struct Denied;
        impl HttpBackend for Denied {
            fn execute(&self, _: &str, _: HttpOperation) -> Result<Value, HttpFailure> {
                Err(HttpFailure::DeniedOrMissing)
            }
        }
        let response = router(Arc::new(Denied))
            .oneshot(
                Request::get("/v1/reviews/secret")
                    .header(header::AUTHORIZATION, "Bearer session-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"],
            "not_found"
        );
    }

    struct HostAccess {
        principal: PrincipalId,
        scope: ScopeRecord,
        grants: Arc<RwLock<Vec<Grant>>>,
    }

    impl ReviewAccessProvider for HostAccess {
        fn access(&self, principal: &PrincipalId, _: &str) -> Result<ReviewAccess, String> {
            if principal != &self.principal {
                return Err("Access unavailable".into());
            }
            Ok(ReviewAccess {
                principal: principal.clone(),
                scope: self.scope.clone(),
                current_grants: self.grants.read().unwrap().clone(),
            })
        }
    }

    impl ComparisonAccessProvider for HostAccess {
        fn comparison_access(
            &self,
            principal: &PrincipalId,
            repository: &RepositoryId,
        ) -> Result<ReviewAccess, String> {
            if principal != &self.principal || repository != &self.scope.primary_repository {
                return Err("Access unavailable".into());
            }
            self.access(principal, "")
        }
    }

    struct NoSource;
    impl ExactSourceReader for NoSource {
        fn read_blob(&self, _: i64, _: &str, _: &str, _: usize) -> Result<SourceBlob, String> {
            Err("Source unavailable".into())
        }

        fn list_paths(&self, _: i64, _: &str, _: usize) -> Result<(Vec<String>, bool), String> {
            Ok((Vec::new(), false))
        }
    }

    fn host_temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "flowdiff-http-host-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
    }

    #[tokio::test]
    async fn workspace_profiles_are_authenticated_isolated_and_conflict_checked() {
        let root = host_temp();
        fs::create_dir_all(&root).unwrap();
        let storage = Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
        let sessions = SessionRegistry::default();
        let principal = PrincipalId::new("github-user:100").unwrap();
        sessions
            .register(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                principal.clone(),
                u64::MAX,
            )
            .unwrap();
        sessions
            .register(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                PrincipalId::new("github-user:101").unwrap(),
                u64::MAX,
            )
            .unwrap();
        let repository = RepositoryId::new(
            SecurityDomain::new("local").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("scope").unwrap(),
            primary_repository: repository,
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let host = Arc::new(ServiceHost::new(
            storage,
            sessions,
            HostAccess {
                principal,
                scope,
                grants: Arc::new(RwLock::new(vec![])),
            },
            NoSource,
        ));
        let app = router(host);
        let endpoint = "/v1/me/workspace-preferences";
        let anonymous = app
            .clone()
            .oneshot(Request::get(endpoint).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
        let mut doc = crate::workspace_preferences::defaults();
        doc["defaultMode"] = json!("focus");
        let body = json!({"revision":0,"preferences":doc});
        for expected in [StatusCode::OK, StatusCode::CONFLICT] {
            let response = app
                .clone()
                .oneshot(
                    Request::put(endpoint)
                        .header(
                            header::AUTHORIZATION,
                            "Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        )
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
        for (token, mode) in [
            ("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "focus"),
            ("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "standard"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::get(endpoint)
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let data = response.into_body().collect().await.unwrap().to_bytes();
            let value: Value = serde_json::from_slice(&data).unwrap();
            assert_eq!(value["preferences"]["defaultMode"], mode);
        }
        let mut spoof = body;
        spoof["user_id"] = json!("github-user:100");
        let response = app
            .clone()
            .oneshot(
                Request::put(endpoint)
                    .header(
                        header::AUTHORIZATION,
                        "Bearer bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(spoof.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn concrete_host_persists_exports_and_rechecks_revocation() {
        let root = host_temp();
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut storage =
            Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
        let repository = RepositoryId::new(
            SecurityDomain::new("tenant-http").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let repository_id = storage
            .register_repository("tenant-http", "github", "owner", "repo")
            .unwrap();
        let scope_id = storage
            .register_scope(repository_id, "scope-http", b"scope-http")
            .unwrap();
        let baseline = "a".repeat(40);
        let proposal = "b".repeat(40);
        {
            let mut scheduler = Scheduler::new(&mut storage);
            scheduler
                .register_review(
                    "review-http",
                    repository_id,
                    scope_id,
                    &baseline,
                    &baseline,
                    &proposal,
                    "exact",
                    1,
                )
                .unwrap();
            scheduler
                .observe_pr(
                    repository_id,
                    "7",
                    1,
                    "review-http",
                    &proposal,
                    &baseline,
                    2,
                )
                .unwrap();
            scheduler
                .enqueue("review-http", "analyze", 10, JobLane::Warm, "7", 1, 3)
                .unwrap();
        }
        let principal = PrincipalId::new("http-reviewer").unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("scope-http").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: true,
            model_destinations: BTreeSet::new(),
        };
        let grants = Arc::new(RwLock::new(vec![Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Graph, Action::Export]),
            paths: PathScope::all(),
        }]));
        let sessions = SessionRegistry::default();
        sessions
            .register(
                "0123456789abcdef0123456789abcdef",
                principal.clone(),
                u64::MAX,
            )
            .unwrap();
        let host = Arc::new(ServiceHost::new(
            storage,
            sessions,
            HostAccess {
                principal,
                scope,
                grants: grants.clone(),
            },
            NoSource,
        ));
        let capability = router(host.clone())
            .oneshot(
                Request::get("/v1/ai/capability")
                    .header(
                        header::AUTHORIZATION,
                        "Bearer 0123456789abcdef0123456789abcdef",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let capability: Value =
            serde_json::from_slice(&capability.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(capability["capability"], "disabled");
        assert_eq!(capability["dispatch_available"], false);
        let export = router(host.clone())
            .oneshot(
                Request::post("/v1/exports")
                    .header(
                        header::AUTHORIZATION,
                        "Bearer 0123456789abcdef0123456789abcdef",
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"review_id":"review-http","format":"markdown-v1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(export.status(), StatusCode::OK);
        let body = export.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["state"], "ready");
        let job_id = body["job_id"].as_str().unwrap().to_owned();
        let job = router(host.clone())
            .oneshot(
                Request::get(format!("/v1/jobs/{job_id}"))
                    .header(
                        header::AUTHORIZATION,
                        "Bearer 0123456789abcdef0123456789abcdef",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(job.status(), StatusCode::OK);

        grants.write().unwrap().clear();
        let revoked = router(host)
            .oneshot(
                Request::get(format!("/v1/jobs/{job_id}"))
                    .header(
                        header::AUTHORIZATION,
                        "Bearer 0123456789abcdef0123456789abcdef",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::NOT_FOUND);
    }
}
