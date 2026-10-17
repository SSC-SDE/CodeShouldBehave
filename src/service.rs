use crate::{
    Action, ComparisonMode, Grant, GraphDirection, GraphPage, GraphPathPage, GraphSymbolPage,
    JobLane, PrincipalId, ReviewId, ReviewIdentityInput, Scheduler, ScopeRecord, SnapshotId,
    Storage, authorize, authorize_repository,
};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::RwLock,
    time::{SystemTime, UNIX_EPOCH},
};

pub const SERVICE_API_VERSION: &str = "v1";
const MAX_PAGE: usize = 200;
const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_EXPORT_BYTES: usize = 2 * 1024 * 1024;
const MAX_INVENTORY_SCAN: usize = 10_000;

#[derive(Clone, Debug)]
struct SessionRecord {
    principal: PrincipalId,
    expires_at_ms: u64,
}

/// Process-local organizational sessions. The registry keeps only token
/// digests; token issuance and upstream identity verification stay in the
/// deployment's authentication adapter.
#[derive(Default)]
pub struct SessionRegistry {
    sessions: RwLock<BTreeMap<[u8; 32], SessionRecord>>,
}

impl SessionRegistry {
    pub fn register(
        &self,
        token: &str,
        principal: PrincipalId,
        expires_at_ms: u64,
    ) -> Result<(), String> {
        validate_session_token(token)?;
        if expires_at_ms <= now_ms()? {
            return Err("Session expiry must be in the future".into());
        }
        self.sessions
            .write()
            .map_err(|_| "Session store is unavailable")?
            .insert(
                token_digest(token),
                SessionRecord {
                    principal,
                    expires_at_ms,
                },
            );
        Ok(())
    }

    pub fn revoke(&self, token: &str) -> Result<bool, String> {
        validate_session_token(token)?;
        Ok(self
            .sessions
            .write()
            .map_err(|_| "Session store is unavailable")?
            .remove(&token_digest(token))
            .is_some())
    }
}

#[derive(Clone, Debug)]
pub struct ReviewAccess {
    pub principal: PrincipalId,
    pub scope: ScopeRecord,
    pub current_grants: Vec<Grant>,
}

pub trait ServiceAuthenticator {
    fn authenticate(&self, bearer_token: &str) -> Result<PrincipalId, String>;
}

impl ServiceAuthenticator for SessionRegistry {
    fn authenticate(&self, bearer_token: &str) -> Result<PrincipalId, String> {
        validate_session_token(bearer_token)?;
        let digest = token_digest(bearer_token);
        let now = now_ms()?;
        let mut sessions = self
            .sessions
            .write()
            .map_err(|_| "Session store is unavailable")?;
        let Some(session) = sessions.get(&digest) else {
            return Err("Authentication required".into());
        };
        if session.expires_at_ms <= now {
            sessions.remove(&digest);
            return Err("Authentication required".into());
        }
        Ok(session.principal.clone())
    }
}

pub trait ReviewAccessProvider {
    fn access(&self, principal: &PrincipalId, review_id: &str) -> Result<ReviewAccess, String>;
}

pub trait ComparisonAccessProvider {
    fn comparison_access(
        &self,
        principal: &PrincipalId,
        repository: &crate::RepositoryId,
    ) -> Result<ReviewAccess, String>;
}

#[derive(Clone, Debug)]
pub struct ExactComparisonRequest {
    pub repository: crate::RepositoryId,
    pub provider_pr_id: String,
    pub generation: i64,
    pub selected_baseline: String,
    pub proposal: String,
    pub target_branch: String,
    pub state: String,
    pub priority: i64,
}

#[derive(Clone, Debug)]
pub struct ScopeChangeRequest {
    pub review_id: String,
    pub provider_pr_id: String,
    pub generation: i64,
    pub priority: i64,
    pub proposed_scope: ScopeRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparisonReceipt {
    pub api_version: &'static str,
    pub review_id: String,
    pub job_id: i64,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportReceipt {
    pub api_version: &'static str,
    pub job_id: String,
    pub review_id: String,
    pub state: String,
    pub artifact_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobDetails {
    pub api_version: &'static str,
    pub job_id: String,
    pub review_id: String,
    pub kind: String,
    pub state: String,
    pub attempt_count: usize,
    pub terminal_reason: Option<String>,
    pub artifact_digest: Option<String>,
}

pub struct ServiceCommands<'a, A, P> {
    storage: &'a mut Storage,
    authenticator: &'a A,
    access_provider: &'a P,
}

impl<'a, A: ServiceAuthenticator, P: ComparisonAccessProvider + ReviewAccessProvider>
    ServiceCommands<'a, A, P>
{
    pub fn new(storage: &'a mut Storage, authenticator: &'a A, access_provider: &'a P) -> Self {
        Self {
            storage,
            authenticator,
            access_provider,
        }
    }

    pub fn create_exact_comparison(
        &mut self,
        bearer: &str,
        request: &ExactComparisonRequest,
    ) -> Result<ComparisonReceipt, String> {
        validate_label("provider PR identity", &request.provider_pr_id)?;
        validate_label("target branch", &request.target_branch)?;
        if request.generation < 0
            || !matches!(request.state.as_str(), "open" | "draft")
            || !(0..=1_000).contains(&request.priority)
        {
            return Err("Invalid comparison scheduling fields".into());
        }
        let selected_baseline = SnapshotId::new(request.selected_baseline.clone())?;
        let proposal = SnapshotId::new(request.proposal.clone())?;
        let principal = self.authenticator.authenticate(bearer)?;
        let access = self
            .access_provider
            .comparison_access(&principal, &request.repository)?;
        if access.principal != principal || access.scope.primary_repository != request.repository {
            return Err("Comparison access does not match the requested repository".into());
        }
        authorize(
            Action::Index,
            &principal,
            &access.scope,
            &access.current_grants,
            None,
        )?;
        let (repository_id, scope_id): (i64, i64) = self
            .storage
            .connection
            .query_row(
                "SELECT repo.id,scope.id FROM repositories repo
                 JOIN scopes scope ON scope.repository_id=repo.id
                 WHERE repo.security_domain=?1 AND repo.provider=?2 AND repo.owner=?3
                   AND repo.name=?4 AND scope.version=?5",
                params![
                    request.repository.security_domain().as_str(),
                    request.repository.provider(),
                    request.repository.owner(),
                    request.repository.name(),
                    access.scope.version.as_str(),
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Repository or immutable scope is unavailable".to_owned())?;
        let review_id = ReviewId::from_input(&ReviewIdentityInput {
            repository: request.repository.clone(),
            selected_baseline: selected_baseline.clone(),
            analysis_baseline: selected_baseline.clone(),
            proposal: proposal.clone(),
            comparison: ComparisonMode::Exact,
            scope: access.scope,
            parser_version: crate::parser::RUST_PARSER_VERSION.into(),
            resolver_version: "flowdiff-name-candidate-v1".into(),
            rule_version: crate::core::REVIEW_RULE_VERSION.into(),
        })?;
        let now = now_ms()?;
        {
            let mut scheduler = Scheduler::new(self.storage);
            scheduler.register_review(
                review_id.as_str(),
                repository_id,
                scope_id,
                selected_baseline.as_str(),
                selected_baseline.as_str(),
                proposal.as_str(),
                "exact",
                now,
            )?;
            scheduler.observe_pr(
                repository_id,
                &request.provider_pr_id,
                request.generation,
                review_id.as_str(),
                proposal.as_str(),
                selected_baseline.as_str(),
                now,
            )?;
        }
        let current_matches: i64 = self
            .storage
            .connection
            .query_row(
                "SELECT count(*) FROM pr_heads WHERE repository_id=?1 AND provider_pr_id=?2
             AND generation=?3 AND current_review_id=?4 AND head_commit=?5 AND base_commit=?6",
                params![
                    repository_id,
                    request.provider_pr_id,
                    request.generation,
                    review_id.as_str(),
                    proposal.as_str(),
                    selected_baseline.as_str()
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot verify current comparison: {error}"))?;
        if current_matches != 1 {
            return Err("Comparison generation is stale or conflicts with current PR state".into());
        }
        self.storage.connection.execute(
            "INSERT OR IGNORE INTO pr_revisions(repository_id,provider_pr_id,generation,review_id,
             base_commit,head_commit,target_branch,state,observed_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![repository_id,request.provider_pr_id,request.generation,review_id.as_str(),
                selected_baseline.as_str(),proposal.as_str(),request.target_branch,request.state,
                i64::try_from(now).map_err(|_| "Timestamp is too large")?],
        ).map_err(|error| format!("Cannot record comparison revision: {error}"))?;
        let revision_matches: i64 = self
            .storage
            .connection
            .query_row(
                "SELECT count(*) FROM pr_revisions WHERE repository_id=?1 AND provider_pr_id=?2
             AND generation=?3 AND review_id=?4 AND base_commit=?5 AND head_commit=?6
             AND target_branch=?7 AND state=?8",
                params![
                    repository_id,
                    request.provider_pr_id,
                    request.generation,
                    review_id.as_str(),
                    selected_baseline.as_str(),
                    proposal.as_str(),
                    request.target_branch,
                    request.state
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot verify comparison revision: {error}"))?;
        if revision_matches != 1 {
            return Err("Comparison generation conflicts with an existing revision".into());
        }
        let (job_id, created) = Scheduler::new(self.storage).enqueue(
            review_id.as_str(),
            "analyze",
            request.priority,
            JobLane::Warm,
            &request.provider_pr_id,
            request.generation,
            now,
        )?;
        Ok(ComparisonReceipt {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.as_str().into(),
            job_id,
            created,
        })
    }

    pub fn replace_review_scope(
        &mut self,
        bearer: &str,
        request: ScopeChangeRequest,
    ) -> Result<ComparisonReceipt, String> {
        validate_label("review identity", &request.review_id)?;
        validate_label("provider PR identity", &request.provider_pr_id)?;
        if request.generation < 0 || !(0..=1_000).contains(&request.priority) {
            return Err("Invalid scope scheduling fields".into());
        }
        let principal = self.authenticator.authenticate(bearer)?;
        let access = self
            .access_provider
            .access(&principal, &request.review_id)?;
        if access.principal != principal {
            return Err("Scope access principal does not match the session".into());
        }
        access.scope.diff(&request.proposed_scope)?;
        authorize_repository(
            Action::Index,
            &principal,
            &request.proposed_scope.primary_repository,
            &access.current_grants,
            None,
            &request.proposed_scope.primary_paths,
        )?;
        for context in &request.proposed_scope.contexts {
            authorize_repository(
                Action::Index,
                &principal,
                &context.repository,
                &access.current_grants,
                None,
                &context.paths,
            )?;
        }
        let (repository_id, selected, analysis, proposal, comparison): (i64, String, String, String, String) =
            self.storage.connection.query_row(
                "SELECT repository_id,selected_base_commit,analysis_base_commit,proposal_commit,comparison_mode FROM reviews WHERE id=?1",
                [&request.review_id],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
            ).map_err(|_| "Review is unavailable".to_owned())?;
        let canonical = request.proposed_scope.canonical_bytes();
        let digest = Sha256::digest(&canonical);
        let version = format!("scope-{}", hex_digest(&digest));
        let mut proposed = request.proposed_scope;
        proposed.version = crate::ScopeVersion::new(version.clone())?;
        let scope_id = self
            .storage
            .register_scope(repository_id, &version, &canonical)?;
        for (position, context) in proposed.contexts.iter().enumerate() {
            let context_repository_id: i64 = self.storage.connection.query_row(
                "SELECT id FROM repositories WHERE security_domain=?1 AND provider=?2 AND owner=?3 AND name=?4",
                params![context.repository.security_domain().as_str(),context.repository.provider(),context.repository.owner(),context.repository.name()],
                |row| row.get(0),
            ).map_err(|_| "Selected context repository is unavailable".to_owned())?;
            self.storage.connection.execute(
                "INSERT OR IGNORE INTO context_revisions(scope_id,position,repository_id,commit_id,path_scope) VALUES (?1,?2,?3,?4,?5)",
                params![scope_id,position as i64,context_repository_id,context.commit.as_str(),canonical_paths(&context.paths)],
            ).map_err(|error| format!("Cannot persist context revision: {error}"))?;
        }
        let review_id = ReviewId::from_input(&ReviewIdentityInput {
            repository: proposed.primary_repository.clone(),
            selected_baseline: SnapshotId::new(selected.clone())?,
            analysis_baseline: SnapshotId::new(analysis.clone())?,
            proposal: SnapshotId::new(proposal.clone())?,
            comparison: if comparison == "exact" {
                ComparisonMode::Exact
            } else {
                ComparisonMode::MergeBase
            },
            scope: proposed,
            parser_version: crate::parser::RUST_PARSER_VERSION.into(),
            resolver_version: "flowdiff-name-candidate-v1".into(),
            rule_version: crate::core::REVIEW_RULE_VERSION.into(),
        })?;
        let now = now_ms()?;
        let mut scheduler = Scheduler::new(self.storage);
        scheduler.register_review(
            review_id.as_str(),
            repository_id,
            scope_id,
            &selected,
            &analysis,
            &proposal,
            &comparison,
            now,
        )?;
        scheduler.replace_current_scope(
            repository_id,
            &request.provider_pr_id,
            request.generation,
            &request.review_id,
            review_id.as_str(),
            now,
        )?;
        let (job_id, created) = scheduler.enqueue(
            review_id.as_str(),
            "analyze",
            request.priority,
            JobLane::Warm,
            &request.provider_pr_id,
            request.generation,
            now,
        )?;
        Ok(ComparisonReceipt {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.as_str().into(),
            job_id,
            created,
        })
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn markdown_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| if character == '`' { '\'' } else { character })
        .collect()
}

fn canonical_paths(paths: &crate::PathScope) -> Vec<u8> {
    let mut value = String::new();
    for path in paths.includes() {
        value.push_str("+\t");
        value.push_str(path);
        value.push('\n');
    }
    for path in paths.excludes() {
        value.push_str("-\t");
        value.push_str(path);
        value.push('\n');
    }
    value.into_bytes()
}

pub struct ExportCommands<'a, A, P, R> {
    storage: &'a mut Storage,
    authenticator: &'a A,
    access_provider: &'a P,
    source_reader: &'a R,
}

impl<'a, A: ServiceAuthenticator, P: ReviewAccessProvider, R: ExactSourceReader>
    ExportCommands<'a, A, P, R>
{
    pub fn new(
        storage: &'a mut Storage,
        authenticator: &'a A,
        access_provider: &'a P,
        source_reader: &'a R,
    ) -> Self {
        Self {
            storage,
            authenticator,
            access_provider,
            source_reader,
        }
    }

    pub fn review_annotations(
        &self,
        bearer: &str,
        review_id: &str,
    ) -> Result<Vec<ReviewAnnotation>, String> {
        let principal = self.authenticator.authenticate(bearer)?;
        let access = self.access_provider.access(&principal, review_id)?;
        if access.principal != principal {
            return Err("Session principal does not match review access".into());
        }
        authorize(
            Action::Graph,
            &principal,
            &access.scope,
            &access.current_grants,
            None,
        )?;
        let mut statement = self.storage.connection.prepare(
            "SELECT id,review_id,proposal_commit,owner_principal,visibility,kind,target_path,target_key,body,status,version,updated_at_ms
             FROM review_annotations WHERE review_id=?1 AND (owner_principal=?2 OR visibility='shared')
             ORDER BY updated_at_ms,id"
        ).map_err(|error| format!("Cannot prepare review annotations: {error}"))?;
        let rows = statement
            .query_map(params![review_id, access.principal.as_str()], |row| {
                Ok(ReviewAnnotation {
                    id: row.get(0)?,
                    review_id: row.get(1)?,
                    proposal_commit: row.get(2)?,
                    owner_principal: row.get(3)?,
                    visibility: row.get(4)?,
                    kind: row.get(5)?,
                    target_path: row.get(6)?,
                    target_key: row.get(7)?,
                    body: row.get(8)?,
                    status: row.get(9)?,
                    version: row.get(10)?,
                    updated_at_ms: row.get(11)?,
                })
            })
            .map_err(|error| format!("Cannot query review annotations: {error}"))?;
        let mut visible = Vec::new();
        for row in rows {
            let annotation =
                row.map_err(|error| format!("Cannot decode review annotation: {error}"))?;
            if let Some(path) = annotation.target_path.as_deref() {
                authorize(
                    Action::Graph,
                    &access.principal,
                    &access.scope,
                    &access.current_grants,
                    Some(path),
                )?;
            }
            visible.push(annotation);
        }
        Ok(visible)
    }

    pub fn write_review_annotation(
        &self,
        bearer: &str,
        review_id: &str,
        request: &ReviewAnnotationWrite,
    ) -> Result<ReviewAnnotation, String> {
        if request.id.is_empty()
            || request.id.len() > 128
            || request.target_key.is_empty()
            || request.target_key.len() > 1024
            || request.body.len() > 16 * 1024
            || request.expected_version < 0
            || !matches!(request.visibility.as_str(), "private" | "shared")
            || !matches!(
                request.kind.as_str(),
                "reviewed" | "bookmark" | "question" | "note" | "handoff"
            )
            || !matches!(request.status.as_str(), "active" | "resolved" | "outdated")
        {
            return Err("Invalid review annotation".into());
        }
        let action = if request.visibility == "shared" {
            Action::Export
        } else {
            Action::Graph
        };
        let principal = self.authenticator.authenticate(bearer)?;
        let access = self.access_provider.access(&principal, review_id)?;
        if access.principal != principal {
            return Err("Session principal does not match review access".into());
        }
        authorize(
            action,
            &principal,
            &access.scope,
            &access.current_grants,
            request.target_path.as_deref(),
        )?;
        if request.kind == "handoff" {
            if request.visibility != "shared" {
                return Err("A reviewer handoff must be shared".into());
            }
            let target = PrincipalId::new(request.body.clone())?;
            let target_access = self.access_provider.access(&target, review_id)?;
            if target_access.principal != target {
                return Err("Handoff reviewer is not authorized".into());
            }
            authorize(
                Action::Graph,
                &target,
                &target_access.scope,
                &target_access.current_grants,
                request.target_path.as_deref(),
            )?;
        }
        let proposal: String = self
            .storage
            .connection
            .query_row(
                "SELECT proposal_commit FROM reviews WHERE id=?1",
                [review_id],
                |row| row.get(0),
            )
            .map_err(|_| "Review is unavailable".to_string())?;
        let now = i64::try_from(now_ms()?).map_err(|_| "Clock is out of range")?;
        let transaction = self
            .storage
            .connection
            .unchecked_transaction()
            .map_err(|error| format!("Cannot begin annotation update: {error}"))?;
        let next_version = request.expected_version + 1;
        let changed = if request.expected_version == 0 {
            transaction.execute(
                "INSERT OR IGNORE INTO review_annotations(id,review_id,proposal_commit,owner_principal,visibility,kind,target_path,target_key,body,status,version,created_at_ms,updated_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?11)",
                params![request.id,review_id,proposal,access.principal.as_str(),request.visibility,request.kind,request.target_path,request.target_key,request.body,request.status,now],
            )
        } else {
            transaction.execute(
                "UPDATE review_annotations SET visibility=?1,body=?2,status=?3,version=?4,updated_at_ms=?5
                 WHERE id=?6 AND review_id=?7 AND proposal_commit=?8 AND owner_principal=?9 AND version=?10 AND kind=?11 AND target_key=?12",
                params![request.visibility,request.body,request.status,next_version,now,request.id,review_id,proposal,access.principal.as_str(),request.expected_version,request.kind,request.target_key],
            )
        }.map_err(|error| format!("Cannot write review annotation: {error}"))?;
        if changed != 1 {
            return Err(
                "Review annotation changed concurrently or conflicts with this snapshot".into(),
            );
        }
        let audit_action = if request.expected_version == 0 {
            "created"
        } else if request.kind == "handoff" {
            "handed_off"
        } else if request.status == "resolved" {
            "resolved"
        } else {
            "updated"
        };
        transaction.execute(
            "INSERT INTO review_annotation_audit(annotation_id,review_id,actor_principal,action,from_version,to_version,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![request.id,review_id,access.principal.as_str(),audit_action,if request.expected_version == 0 { None } else { Some(request.expected_version) },next_version,now],
        ).map_err(|error| format!("Cannot audit review annotation: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit review annotation: {error}"))?;
        self.review_annotations(bearer, review_id)?
            .into_iter()
            .find(|item| item.id == request.id)
            .ok_or_else(|| "Written review annotation is unavailable".into())
    }

    pub fn review_annotation_audit(
        &self,
        bearer: &str,
        review_id: &str,
    ) -> Result<Vec<ReviewAnnotationAudit>, String> {
        let principal = self.authenticator.authenticate(bearer)?;
        let access = self.access_provider.access(&principal, review_id)?;
        if access.principal != principal {
            return Err("Session principal does not match review access".into());
        }
        authorize(
            Action::Graph,
            &principal,
            &access.scope,
            &access.current_grants,
            None,
        )?;
        let mut statement = self.storage.connection.prepare(
            "SELECT audit.id,audit.annotation_id,audit.actor_principal,audit.action,audit.from_version,audit.to_version,audit.created_at_ms,annotation.target_path
             FROM review_annotation_audit audit JOIN review_annotations annotation ON annotation.id=audit.annotation_id
             WHERE audit.review_id=?1 AND (annotation.owner_principal=?2 OR annotation.visibility='shared') ORDER BY audit.id"
        ).map_err(|error| format!("Cannot prepare annotation audit: {error}"))?;
        let rows = statement
            .query_map(params![review_id, principal.as_str()], |row| {
                Ok((
                    ReviewAnnotationAudit {
                        id: row.get(0)?,
                        annotation_id: row.get(1)?,
                        actor_principal: row.get(2)?,
                        action: row.get(3)?,
                        from_version: row.get(4)?,
                        to_version: row.get(5)?,
                        created_at_ms: row.get(6)?,
                    },
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|error| format!("Cannot query annotation audit: {error}"))?;
        let mut visible = Vec::new();
        for row in rows {
            let (audit, path) =
                row.map_err(|error| format!("Cannot decode annotation audit: {error}"))?;
            authorize(
                Action::Graph,
                &principal,
                &access.scope,
                &access.current_grants,
                path.as_deref(),
            )?;
            visible.push(audit);
        }
        Ok(visible)
    }

    pub fn review_continuity_delta(
        &self,
        bearer: &str,
        review_id: &str,
    ) -> Result<ReviewContinuityDelta, String> {
        let principal = self.authenticator.authenticate(bearer)?;
        let access = self.access_provider.access(&principal, review_id)?;
        if access.principal != principal {
            return Err("Session principal does not match review access".into());
        }
        authorize(
            Action::Graph,
            &principal,
            &access.scope,
            &access.current_grants,
            None,
        )?;
        let prior: Option<(String,String,Option<i64>,Option<i64>)> = self.storage.connection.query_row(
            "SELECT previous.review_id,previous.head_commit,prior_review.proposal_snapshot_id,current_review.proposal_snapshot_id
             FROM pr_revisions current
             JOIN pr_revisions previous ON previous.repository_id=current.repository_id AND previous.provider_pr_id=current.provider_pr_id AND previous.generation<current.generation
             JOIN reviews prior_review ON prior_review.id=previous.review_id
             JOIN reviews current_review ON current_review.id=current.review_id
             WHERE current.review_id=?1 AND EXISTS (SELECT 1 FROM review_annotations annotation WHERE annotation.review_id=previous.review_id AND annotation.owner_principal=?2)
             ORDER BY previous.generation DESC LIMIT 1",
            params![review_id,principal.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))
        ).optional().map_err(|error| format!("Cannot find prior reviewer snapshot: {error}"))?;
        let Some((prior_review_id, prior_proposal, Some(prior_snapshot), Some(current_snapshot))) =
            prior
        else {
            return Ok(ReviewContinuityDelta {
                prior_review_id: None,
                prior_proposal: None,
                changed_paths: vec![],
                truncated: false,
            });
        };
        let mut statement=self.storage.connection.prepare(
            "WITH paths(path) AS (SELECT path FROM files WHERE snapshot_id=?1 UNION SELECT path FROM files WHERE snapshot_id=?2)
             SELECT paths.path FROM paths LEFT JOIN files old ON old.snapshot_id=?1 AND old.path=paths.path LEFT JOIN files new ON new.snapshot_id=?2 AND new.path=paths.path
             WHERE old.path IS NULL OR new.path IS NULL OR COALESCE(old.blob_id,'')<>COALESCE(new.blob_id,'') ORDER BY paths.path LIMIT 501"
        ).map_err(|error| format!("Cannot prepare reviewer snapshot delta: {error}"))?;
        let rows = statement
            .query_map(params![prior_snapshot, current_snapshot], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("Cannot query reviewer snapshot delta: {error}"))?;
        let mut changed_paths = Vec::new();
        for row in rows {
            let path =
                row.map_err(|error| format!("Cannot decode reviewer snapshot delta: {error}"))?;
            if authorize(
                Action::Graph,
                &principal,
                &access.scope,
                &access.current_grants,
                Some(&path),
            )
            .is_ok()
            {
                changed_paths.push(path);
            }
        }
        let truncated = changed_paths.len() > 500;
        changed_paths.truncate(500);
        Ok(ReviewContinuityDelta {
            prior_review_id: Some(prior_review_id),
            prior_proposal: Some(prior_proposal),
            changed_paths,
            truncated,
        })
    }

    pub fn create_markdown_export(
        &mut self,
        bearer: &str,
        review_id: &str,
    ) -> Result<ExportReceipt, String> {
        let principal = self.authenticator.authenticate(bearer)?;
        let access = self.access_provider.access(&principal, review_id)?;
        if access.principal != principal {
            return Err("Session principal does not match export access".into());
        }
        authorize(
            Action::Export,
            &principal,
            &access.scope,
            &access.current_grants,
            None,
        )?;
        if let Some(receipt) =
            load_export_receipt(self.storage, review_id, principal.as_str(), "markdown-v1")?
        {
            return Ok(receipt);
        }
        let bytes = ServiceApi::new(
            &*self.storage,
            self.authenticator,
            self.access_provider,
            self.source_reader,
        )
        .export_summary(bearer, review_id)?;
        let artifact = self.storage.publish_artifact(
            access.scope.primary_repository.security_domain().as_str(),
            "markdown-v1",
            &bytes,
        )?;
        let now = i64::try_from(now_ms()?).map_err(|_| "Timestamp is too large")?;
        self.storage
            .connection
            .execute(
                "INSERT INTO export_jobs(review_id,requested_by,format_version,state,artifact_id,
             created_at_ms,completed_at_ms) VALUES (?1,?2,'markdown-v1','ready',?3,?4,?4)",
                params![review_id, principal.as_str(), artifact.id, now],
            )
            .map_err(|error| format!("Cannot persist export job: {error}"))?;
        load_export_receipt(self.storage, review_id, principal.as_str(), "markdown-v1")?
            .ok_or("Export job was not persisted".into())
    }
}

fn load_export_receipt(
    storage: &Storage,
    review_id: &str,
    principal: &str,
    format_version: &str,
) -> Result<Option<ExportReceipt>, String> {
    storage
        .connection
        .query_row(
            "SELECT e.id,e.state,a.digest FROM export_jobs e
         LEFT JOIN artifact_refs a ON a.id=e.artifact_id
         WHERE e.review_id=?1 AND e.requested_by=?2 AND e.format_version=?3",
            params![review_id, principal, format_version],
            |row| {
                Ok(ExportReceipt {
                    api_version: SERVICE_API_VERSION,
                    job_id: format!("export:{}", row.get::<_, i64>(0)?),
                    review_id: review_id.into(),
                    state: row.get(1)?,
                    artifact_digest: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Cannot read export job: {error}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevisionSide {
    Baseline,
    Proposal,
}

pub struct SourceBlob {
    pub blob_id: String,
    pub bytes: Vec<u8>,
}

pub trait ExactSourceReader {
    fn read_blob(
        &self,
        repository_id: i64,
        commit: &str,
        path: &str,
        maximum_bytes: usize,
    ) -> Result<SourceBlob, String>;

    /// Lists committed paths without reading blob contents. Implementations
    /// must resolve only the supplied immutable commit and enforce the bound.
    fn list_paths(
        &self,
        repository_id: i64,
        commit: &str,
        maximum_paths: usize,
    ) -> Result<(Vec<String>, bool), String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrSummary {
    pub repository_id: i64,
    pub security_domain: String,
    pub provider: String,
    pub repository_owner: String,
    pub repository_name: String,
    pub provider_pr_id: String,
    pub generation: i64,
    pub review_id: Option<String>,
    pub base_commit: String,
    pub head_commit: String,
    pub target_branch: String,
    pub state: String,
    pub priority: i64,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrPage {
    pub api_version: &'static str,
    pub items: Vec<PrSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewProgress {
    pub api_version: &'static str,
    pub review_id: String,
    pub job_state: Option<String>,
    pub attempt_count: usize,
    pub terminal_reason: Option<String>,
    pub coverage: Vec<(String, usize)>,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewDetails {
    pub api_version: &'static str,
    pub review_id: String,
    pub selected_baseline: String,
    pub analysis_baseline: String,
    pub proposal: String,
    pub comparison_mode: String,
    pub scope_version: String,
    pub stale: bool,
    pub capabilities: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionPage {
    pub api_version: &'static str,
    pub items: Vec<PrSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FindingSummary {
    pub id: i64,
    pub kind: String,
    pub resolution: String,
    pub confidence: f64,
    pub source_path: String,
    pub source_start_line: Option<i64>,
    pub source_end_line: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FindingPage {
    pub api_version: &'static str,
    pub review_id: String,
    pub items: Vec<FindingSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFileSummary {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: String,
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFilePage {
    pub api_version: &'static str,
    pub review_id: String,
    pub items: Vec<ChangedFileSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpanResponse {
    pub api_version: &'static str,
    pub review_id: String,
    pub commit: String,
    pub blob_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAnnotation {
    pub id: String,
    pub review_id: String,
    pub proposal_commit: String,
    pub owner_principal: String,
    pub visibility: String,
    pub kind: String,
    pub target_path: Option<String>,
    pub target_key: String,
    pub body: String,
    pub status: String,
    pub version: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAnnotationWrite {
    pub id: String,
    pub visibility: String,
    pub kind: String,
    pub target_path: Option<String>,
    pub target_key: String,
    pub body: String,
    pub status: String,
    pub expected_version: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAnnotationAudit {
    pub id: i64,
    pub annotation_id: String,
    pub actor_principal: String,
    pub action: String,
    pub from_version: Option<i64>,
    pub to_version: i64,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewContinuityDelta {
    pub prior_review_id: Option<String>,
    pub prior_proposal: Option<String>,
    pub changed_paths: Vec<String>,
    pub truncated: bool,
}

pub struct ServiceApi<'a, A, P, R> {
    storage: &'a Storage,
    authenticator: &'a A,
    access_provider: &'a P,
    source_reader: &'a R,
}

impl<'a, A: ServiceAuthenticator, P: ReviewAccessProvider, R: ExactSourceReader>
    ServiceApi<'a, A, P, R>
{
    pub fn new(
        storage: &'a Storage,
        authenticator: &'a A,
        access_provider: &'a P,
        source_reader: &'a R,
    ) -> Self {
        Self {
            storage,
            authenticator,
            access_provider,
            source_reader,
        }
    }

    pub fn list_prs(
        &self,
        bearer: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<PrPage, String> {
        let principal = self.authenticator.authenticate(bearer)?;
        bounded_limit(limit)?;
        let cursor_binding = format!("prs:{}", principal.as_str());
        let after = decode_opaque_cursor(cursor, "pr1", &cursor_binding)?;
        let mut visible = Vec::new();
        let mut scan_after = after;
        let mut scanned = 0;
        loop {
            let batch = self.pr_revision_batch(scan_after, 256)?;
            if batch.is_empty() {
                break;
            }
            for (position, item) in &batch {
                scan_after = *position;
                scanned += 1;
                let Some(review_id) = item.review_id.as_deref() else {
                    continue;
                };
                if self
                    .authorize_review_for_principal(&principal, review_id, Action::Graph, None)
                    .is_ok()
                {
                    visible.push((*position, item.clone()));
                }
                if visible.len() > limit {
                    break;
                }
            }
            if visible.len() > limit || batch.len() < 256 {
                break;
            }
            if scanned >= MAX_INVENTORY_SCAN {
                return Err("Authorized inventory scan limit exceeded; narrow the query".into());
            }
        }
        let has_more = visible.len() > limit;
        visible.truncate(limit);
        let next_cursor = has_more
            .then(|| encode_opaque_cursor("pr1", &cursor_binding, visible.last().unwrap().0));
        Ok(PrPage {
            api_version: SERVICE_API_VERSION,
            items: visible.into_iter().map(|(_, item)| item).collect(),
            next_cursor,
        })
    }

    pub fn progress(&self, bearer: &str, review_id: &str) -> Result<ReviewProgress, String> {
        let access = self.review_access(bearer, review_id, Action::Graph, None)?;
        self.progress_for_access(review_id, &access)
    }

    pub fn review_details(&self, bearer: &str, review_id: &str) -> Result<ReviewDetails, String> {
        let access = self.review_access(bearer, review_id, Action::Graph, None)?;
        let (selected_baseline, analysis_baseline, proposal, comparison_mode): (
            String,
            String,
            String,
            String,
        ) = self
            .storage
            .connection
            .query_row(
                "SELECT selected_base_commit,analysis_base_commit,proposal_commit,comparison_mode
                 FROM reviews WHERE id=?1",
                [review_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|_| "Review is unavailable".to_owned())?;
        let mut capabilities = Vec::new();
        for (action, label) in [
            (Action::Graph, "graph"),
            (Action::Source, "source"),
            (Action::Export, "export"),
            (Action::SubmitModel, "submit_model"),
        ] {
            if authorize(
                action,
                &access.principal,
                &access.scope,
                &access.current_grants,
                None,
            )
            .is_ok()
            {
                capabilities.push(label);
            }
        }
        let stale = self.progress_for_access(review_id, &access)?.stale;
        Ok(ReviewDetails {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.into(),
            selected_baseline,
            analysis_baseline,
            proposal,
            comparison_mode,
            scope_version: access.scope.version.as_str().into(),
            stale,
            capabilities,
        })
    }

    pub fn preview_scope(
        &self,
        bearer: &str,
        review_id: &str,
        proposed: &ScopeRecord,
    ) -> Result<crate::ScopeDelta, String> {
        let access = self.review_access(bearer, review_id, Action::Graph, None)?;
        let mut delta = access.scope.diff(proposed)?;
        authorize_repository(
            Action::Index,
            &access.principal,
            &proposed.primary_repository,
            &access.current_grants,
            None,
            &proposed.primary_paths,
        )?;
        for context in &proposed.contexts {
            authorize_repository(
                Action::Index,
                &access.principal,
                &context.repository,
                &access.current_grants,
                None,
                &context.paths,
            )?;
        }
        let proposal_commit: String = self
            .storage
            .connection
            .query_row(
                "SELECT proposal_commit FROM reviews WHERE id=?1",
                [review_id],
                |row| row.get(0),
            )
            .map_err(|_| "Review is unavailable".to_owned())?;
        let (current_files, current_truncated) =
            self.scope_file_set(&access.scope, &proposal_commit)?;
        let (proposed_files, proposed_truncated) =
            self.scope_file_set(proposed, &proposal_commit)?;
        const MAX_FILE_CHANGES: usize = 1_000;
        let all_added = proposed_files
            .difference(&current_files)
            .cloned()
            .collect::<Vec<_>>();
        let all_removed = current_files
            .difference(&proposed_files)
            .cloned()
            .collect::<Vec<_>>();
        delta.file_preview_truncated = current_truncated
            || proposed_truncated
            || all_added.len() > MAX_FILE_CHANGES
            || all_removed.len() > MAX_FILE_CHANGES;
        delta.added_files = all_added.into_iter().take(MAX_FILE_CHANGES).collect();
        delta.removed_files = all_removed.into_iter().take(MAX_FILE_CHANGES).collect();
        Ok(delta)
    }

    fn scope_file_set(
        &self,
        scope: &ScopeRecord,
        primary_commit: &str,
    ) -> Result<(std::collections::BTreeSet<String>, bool), String> {
        const MAX_CONTEXT_PATHS: usize = 20_000;
        let mut files = std::collections::BTreeSet::new();
        let mut truncated = false;
        let primary_id = self.repository_row_id(&scope.primary_repository)?;
        let (paths, clipped) =
            self.source_reader
                .list_paths(primary_id, primary_commit, MAX_CONTEXT_PATHS)?;
        truncated |= clipped;
        for path in paths {
            if scope.primary_paths.allows(&path) {
                files.insert(format!("primary:{path}"));
            }
        }
        for context in &scope.contexts {
            let repository_id = self.repository_row_id(&context.repository)?;
            let (paths, clipped) = self.source_reader.list_paths(
                repository_id,
                context.commit.as_str(),
                MAX_CONTEXT_PATHS,
            )?;
            truncated |= clipped;
            for path in paths {
                if context.paths.allows(&path) {
                    files.insert(format!("{}:{path}", context.name));
                }
            }
        }
        Ok((files, truncated))
    }

    fn repository_row_id(&self, repository: &crate::RepositoryId) -> Result<i64, String> {
        self.storage.connection.query_row(
            "SELECT id FROM repositories WHERE security_domain=?1 AND provider=?2 AND owner=?3 AND name=?4",
            rusqlite::params![repository.security_domain().as_str(), repository.provider(), repository.owner(), repository.name()],
            |row| row.get(0),
        ).map_err(|_| "Repository context is unavailable".to_owned())
    }

    pub fn list_revisions(
        &self,
        bearer: &str,
        repository_id: i64,
        provider_pr_id: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<RevisionPage, String> {
        validate_label("provider PR identity", provider_pr_id)?;
        bounded_limit(limit)?;
        let principal = self.authenticator.authenticate(bearer)?;
        if repository_id <= 0 {
            return Err("Invalid repository identity".into());
        }
        let cursor_binding = format!(
            "revisions:{}:{repository_id}:{provider_pr_id}",
            principal.as_str()
        );
        let after = decode_opaque_cursor(cursor, "rev1", &cursor_binding)?;
        let mut visible = Vec::new();
        let mut scan_after = after;
        let mut scanned = 0;
        loop {
            let batch = self.revision_batch(repository_id, provider_pr_id, scan_after, 256)?;
            if batch.is_empty() {
                break;
            }
            for (position, item) in &batch {
                scan_after = *position;
                scanned += 1;
                let Some(review_id) = item.review_id.as_deref() else {
                    continue;
                };
                if self
                    .authorize_review_for_principal(&principal, review_id, Action::Graph, None)
                    .is_ok()
                {
                    visible.push((*position, item.clone()));
                }
                if visible.len() > limit {
                    break;
                }
            }
            if visible.len() > limit || batch.len() < 256 {
                break;
            }
            if scanned >= MAX_INVENTORY_SCAN {
                return Err("Authorized revision scan limit exceeded; narrow the query".into());
            }
        }
        let has_more = visible.len() > limit;
        visible.truncate(limit);
        let next_cursor = has_more
            .then(|| encode_opaque_cursor("rev1", &cursor_binding, visible.last().unwrap().0));
        Ok(RevisionPage {
            api_version: SERVICE_API_VERSION,
            items: visible.into_iter().map(|(_, item)| item).collect(),
            next_cursor,
        })
    }

    pub fn findings(
        &self,
        bearer: &str,
        review_id: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<FindingPage, String> {
        bounded_limit(limit)?;
        let access = self.review_access(bearer, review_id, Action::Graph, None)?;
        let snapshot = self.snapshot_id(review_id, RevisionSide::Proposal)?;
        let cursor_binding = format!(
            "findings:{review_id}:{snapshot}:{}:{}",
            access.principal.as_str(),
            access.scope.version.as_str()
        );
        let after = decode_opaque_cursor(cursor, "finding1", &cursor_binding)?;
        let mut items = Vec::new();
        let mut scan_after = after;
        let mut scanned = 0;
        loop {
            let batch = self.finding_batch(snapshot, scan_after, 256)?;
            if batch.is_empty() {
                break;
            }
            for finding in &batch {
                scan_after = finding.id;
                scanned += 1;
                if access.scope.primary_paths.allows(&finding.source_path) {
                    items.push(finding.clone());
                }
                if items.len() > limit {
                    break;
                }
            }
            if items.len() > limit || batch.len() < 256 {
                break;
            }
            if scanned >= MAX_INVENTORY_SCAN {
                return Err("Scoped finding scan limit exceeded; narrow the query".into());
            }
        }
        let has_more = items.len() > limit;
        items.truncate(limit);
        let next_cursor = has_more.then(|| {
            encode_opaque_cursor(
                "finding1",
                &cursor_binding,
                items.last().expect("nonempty page").id,
            )
        });
        Ok(FindingPage {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.into(),
            items,
            next_cursor,
        })
    }

    pub fn changed_files(
        &self,
        bearer: &str,
        review_id: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<ChangedFilePage, String> {
        bounded_limit(limit)?;
        let access = self.review_access(bearer, review_id, Action::Graph, None)?;
        let change_set: Option<String> = self
            .storage
            .connection
            .query_row(
                "SELECT change_set_id FROM reviews WHERE id=?1",
                [review_id],
                |row| row.get(0),
            )
            .map_err(|_| "Review is unavailable".to_owned())?;
        let Some(change_set) = change_set else {
            return Ok(ChangedFilePage {
                api_version: SERVICE_API_VERSION,
                review_id: review_id.into(),
                items: Vec::new(),
                next_cursor: None,
            });
        };
        let binding = format!(
            "files:{review_id}:{change_set}:{}:{}",
            access.principal.as_str(),
            access.scope.version.as_str()
        );
        let after = decode_opaque_cursor(cursor, "file1", &binding)?;
        let mut statement = self
            .storage
            .connection
            .prepare(
                "SELECT rowid,kind,path,old_path FROM change_paths
                 WHERE change_set_id=?1 AND rowid>?2 ORDER BY rowid LIMIT ?3",
            )
            .map_err(|error| format!("Cannot prepare changed files: {error}"))?;
        let rows = statement
            .query_map(
                params![change_set, after, MAX_INVENTORY_SCAN as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .map_err(|error| format!("Cannot query changed files: {error}"))?;
        let mut visible = Vec::new();
        for row in rows {
            let (position, kind, path, old_path) =
                row.map_err(|error| format!("Cannot decode changed file: {error}"))?;
            if access.scope.primary_paths.allows(&path) {
                let old_path = old_path.filter(|value| access.scope.primary_paths.allows(value));
                visible.push((
                    position,
                    ChangedFileSummary {
                        path,
                        old_path,
                        kind,
                        additions: None,
                        deletions: None,
                    },
                ));
            }
        }
        let has_more = visible.len() > limit;
        visible.truncate(limit);
        let next_cursor = has_more.then(|| {
            encode_opaque_cursor("file1", &binding, visible.last().expect("nonempty page").0)
        });
        Ok(ChangedFilePage {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.into(),
            items: visible.into_iter().map(|(_, item)| item).collect(),
            next_cursor,
        })
    }

    pub fn job_details(&self, bearer: &str, job_id: &str) -> Result<JobDetails, String> {
        if let Some(id) = job_id.strip_prefix("analysis:") {
            let id = parse_positive_id(id)?;
            let (review_id, kind, state, attempts, reason): (
                String,
                String,
                String,
                i64,
                Option<String>,
            ) = self
                .storage
                .connection
                .query_row(
                    "SELECT review_id,kind,state,attempt_count,terminal_reason FROM jobs WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .map_err(|_| "Job is unavailable".to_owned())?;
            self.review_access(bearer, &review_id, Action::Graph, None)?;
            return Ok(JobDetails {
                api_version: SERVICE_API_VERSION,
                job_id: job_id.into(),
                review_id,
                kind,
                state,
                attempt_count: usize::try_from(attempts).unwrap_or(usize::MAX),
                terminal_reason: reason,
                artifact_digest: None,
            });
        }
        if let Some(id) = job_id.strip_prefix("export:") {
            let id = parse_positive_id(id)?;
            let (review_id, state, reason, digest): (
                String,
                String,
                Option<String>,
                Option<String>,
            ) = self
                .storage
                .connection
                .query_row(
                    "SELECT e.review_id,e.state,e.failure_reason,a.digest FROM export_jobs e
                     LEFT JOIN artifact_refs a ON a.id=e.artifact_id WHERE e.id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|_| "Job is unavailable".to_owned())?;
            self.review_access(bearer, &review_id, Action::Export, None)?;
            return Ok(JobDetails {
                api_version: SERVICE_API_VERSION,
                job_id: job_id.into(),
                review_id,
                kind: "export".into(),
                state,
                attempt_count: 1,
                terminal_reason: reason,
                artifact_digest: digest,
            });
        }
        Err("Invalid job identity".into())
    }

    fn progress_for_access(
        &self,
        review_id: &str,
        access: &ReviewAccess,
    ) -> Result<ReviewProgress, String> {
        let (state, attempts, reason): (Option<String>, i64, Option<String>) = self.storage.connection.query_row(
            "SELECT (SELECT state FROM jobs WHERE review_id=?1 ORDER BY id DESC LIMIT 1),
                    coalesce((SELECT sum(attempt_count) FROM jobs WHERE review_id=?1),0),
                    (SELECT terminal_reason FROM jobs WHERE review_id=?1 AND terminal_reason IS NOT NULL ORDER BY id DESC LIMIT 1)",
            [review_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))
        ).map_err(|error| format!("Cannot read review progress: {error}"))?;
        let snapshot: Option<i64> = self
            .storage
            .connection
            .query_row(
                "SELECT proposal_snapshot_id FROM reviews WHERE id=?1",
                [review_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Cannot read review snapshot: {error}"))?
            .flatten();
        let mut coverage = std::collections::BTreeMap::new();
        if let Some(snapshot) = snapshot {
            for file in self.storage.load_file_progress(snapshot)? {
                if access.scope.primary_paths.allows(&file.path) {
                    *coverage.entry(file.coverage).or_insert(0) += 1;
                }
            }
        }
        let stale: i64 = self
            .storage
            .connection
            .query_row(
                "SELECT count(*) FROM pr_revisions rev
                 JOIN pr_heads h ON h.repository_id=rev.repository_id
                    AND h.provider_pr_id=rev.provider_pr_id
                 WHERE rev.review_id=?1 AND h.current_review_id IS NOT ?1",
                [review_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot inspect review staleness: {error}"))?;
        Ok(ReviewProgress {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.into(),
            job_state: state,
            attempt_count: attempts as usize,
            terminal_reason: reason,
            coverage: coverage.into_iter().collect(),
            stale: stale > 0,
        })
    }

    pub fn graph_children(
        &self,
        bearer: &str,
        review_id: &str,
        side: RevisionSide,
        parent: Option<&str>,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphPathPage, String> {
        let access = self.review_access(bearer, review_id, Action::Graph, parent)?;
        let snapshot = self.snapshot_id(review_id, side)?;
        self.storage
            .graph_children(snapshot, parent, &access.scope.primary_paths, limit, cursor)
    }

    pub fn graph_symbols(
        &self,
        bearer: &str,
        review_id: &str,
        side: RevisionSide,
        path: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphSymbolPage, String> {
        let access = self.review_access(bearer, review_id, Action::Graph, Some(path))?;
        let snapshot = self.snapshot_id(review_id, side)?;
        self.storage
            .graph_symbols(snapshot, path, &access.scope.primary_paths, limit, cursor)
    }

    /// Return the immutable blob selector required for an exact, bounded source
    /// request. This deliberately follows the same current-grant and path-scope
    /// checks as graph navigation; callers still reauthorize when reading it.
    pub fn source_blob_selector(
        &self,
        bearer: &str,
        review_id: &str,
        side: RevisionSide,
        path: &str,
    ) -> Result<String, String> {
        self.review_access(bearer, review_id, Action::Graph, Some(path))?;
        let snapshot = self.snapshot_id(review_id, side)?;
        self.storage
            .connection
            .query_row(
                "SELECT blob_id FROM files WHERE snapshot_id=?1 AND path=?2",
                params![snapshot, path],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|_| "Graph file is unavailable".to_owned())?
            .ok_or_else(|| "Graph file is unavailable".to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn graph_neighborhood(
        &self,
        bearer: &str,
        review_id: &str,
        side: RevisionSide,
        symbol: &str,
        direction: GraphDirection,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphPage, String> {
        let access = self.review_access(bearer, review_id, Action::Graph, None)?;
        let snapshot = self.snapshot_id(review_id, side)?;
        self.storage.graph_neighborhood(
            snapshot,
            symbol,
            direction,
            &access.scope.primary_paths,
            limit,
            cursor,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn source_span(
        &self,
        bearer: &str,
        review_id: &str,
        side: RevisionSide,
        path: &str,
        expected_blob: &str,
        start_line: usize,
        end_line: usize,
    ) -> Result<SourceSpanResponse, String> {
        validate_path(path)?;
        if start_line == 0 || end_line < start_line || end_line - start_line + 1 > 200 {
            return Err("Invalid or excessive source line range".into());
        }
        let access = self.review_access(bearer, review_id, Action::Source, Some(path))?;
        if !access.scope.primary_paths.allows(path) {
            return Err("Source path is outside review scope".into());
        }
        let (repository_id, commit) = self.review_revision(review_id, side)?;
        let snapshot_id = self.snapshot_id(review_id, side)?;
        let recorded_blob: Option<String> = self
            .storage
            .connection
            .query_row(
                "SELECT blob_id FROM files WHERE snapshot_id=?1 AND path=?2",
                params![snapshot_id, path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Cannot verify source identity: {error}"))?
            .flatten();
        let recorded_blob =
            recorded_blob.ok_or("Source path is absent from the selected snapshot")?;
        if expected_blob != recorded_blob {
            return Err("Requested blob identity does not match the selected snapshot".into());
        }
        let blob = self
            .source_reader
            .read_blob(repository_id, &commit, path, MAX_SOURCE_BYTES)?;
        if blob.blob_id != expected_blob {
            return Err("Source blob identity does not match the selected revision".into());
        }
        if blob.bytes.len() > MAX_SOURCE_BYTES || blob.bytes.contains(&0) {
            return Err("Source response is binary or too large".into());
        }
        let source =
            std::str::from_utf8(&blob.bytes).map_err(|_| "Source response is not UTF-8")?;
        let lines = source.lines().collect::<Vec<_>>();
        if start_line > lines.len() || end_line > lines.len() {
            return Err("Source range exceeds the selected blob".into());
        }
        let text = lines[start_line - 1..end_line].join("\n");
        if text.len() > MAX_SOURCE_BYTES {
            return Err("Source response is too large".into());
        }
        Ok(SourceSpanResponse {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.into(),
            commit,
            blob_id: blob.blob_id,
            path: path.into(),
            start_line,
            end_line,
            text,
        })
    }

    pub fn export_summary(&self, bearer: &str, review_id: &str) -> Result<Vec<u8>, String> {
        let access = self.review_access(bearer, review_id, Action::Export, None)?;
        let progress = self.progress_for_access(review_id, &access)?;
        let details = self.review_details_for_access(review_id, &access)?;
        let mut output = format!(
            "# CodeShouldBehave review {review_id}\n\nAPI: {}\nState: {}\nStale: {}\nScope version: {}\nSelected baseline: `{}`\nAnalysis baseline: `{}`\nProposal: `{}`\nComparison: {}\n\n## Authorized context\n\n- Primary: `{}/{}/{}`\n",
            SERVICE_API_VERSION,
            progress.job_state.as_deref().unwrap_or("not-scheduled"),
            progress.stale,
            access.scope.version.as_str(),
            details.selected_baseline,
            details.analysis_baseline,
            details.proposal,
            details.comparison_mode,
            access.scope.primary_repository.provider(),
            access.scope.primary_repository.owner(),
            access.scope.primary_repository.name(),
        );
        output.push_str("  - Includes: ");
        output.push_str(
            &access
                .scope
                .primary_paths
                .includes()
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push_str("\n  - Excludes: ");
        output.push_str(
            &access
                .scope
                .primary_paths
                .excludes()
                .collect::<Vec<_>>()
                .join(", "),
        );
        output.push('\n');
        for context in &access.scope.contexts {
            authorize_repository(
                Action::Export,
                &access.principal,
                &context.repository,
                &access.current_grants,
                None,
                &context.paths,
            )?;
            output.push_str(&format!(
                "- Context {}: `{}/{}/{}` at `{}`\n",
                markdown_text(&context.name),
                context.repository.provider(),
                context.repository.owner(),
                context.repository.name(),
                context.commit.as_str(),
            ));
        }
        if !access.scope.contracts.is_empty() {
            output.push_str("- Versioned contracts: ");
            output.push_str(
                &access
                    .scope
                    .contracts
                    .iter()
                    .map(|item| markdown_text(item))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push('\n');
        }
        output.push_str("\n## Coverage\n");
        for (state, count) in progress.coverage {
            output.push_str(&format!("- {state}: {count}\n"));
        }
        output.push_str("\n## Selected structural evidence\n");
        let snapshot = self
            .storage
            .connection
            .query_row(
                "SELECT proposal_snapshot_id FROM reviews WHERE id=?1",
                [review_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|_| "Review is unavailable".to_owned())?;
        if let Some(snapshot) = snapshot {
            let mut after = 0_i64;
            let mut scanned = 0_usize;
            let mut included = 0_usize;
            loop {
                let batch = self.finding_batch(snapshot, after, 256)?;
                if batch.is_empty() {
                    break;
                }
                for finding in &batch {
                    after = finding.id;
                    scanned += 1;
                    if access.scope.primary_paths.allows(&finding.source_path)
                        && authorize_repository(
                            Action::Export,
                            &access.principal,
                            &access.scope.primary_repository,
                            &access.current_grants,
                            Some(&finding.source_path),
                            &access.scope.primary_paths,
                        )
                        .is_ok()
                    {
                        output.push_str(&format!(
                            "- `{}` — {} / {}; confidence {:.2}; inference only; lines {}–{}\n",
                            markdown_text(&finding.source_path),
                            markdown_text(&finding.kind),
                            markdown_text(&finding.resolution),
                            finding.confidence,
                            finding
                                .source_start_line
                                .map_or_else(|| "unknown".into(), |v| v.to_string()),
                            finding
                                .source_end_line
                                .map_or_else(|| "unknown".into(), |v| v.to_string()),
                        ));
                        included += 1;
                    }
                    if scanned >= MAX_INVENTORY_SCAN {
                        break;
                    }
                }
                if batch.len() < 256 || scanned >= MAX_INVENTORY_SCAN {
                    break;
                }
            }
            if included == 0 {
                output.push_str("- No authorized structural findings.\n");
            }
            if scanned >= MAX_INVENTORY_SCAN {
                output.push_str("- Evidence scan truncated at the documented limit.\n");
            }
        } else {
            output.push_str("- Structural evidence unavailable: proposal snapshot is not ready.\n");
        }
        output.push_str("\nGenerated explanations are not included. Structural findings are inspectable inference, not runtime proof.\n");
        if output.len() > MAX_EXPORT_BYTES {
            return Err("Export exceeds response limit".into());
        }
        Ok(output.into_bytes())
    }

    fn review_details_for_access(
        &self,
        review_id: &str,
        access: &ReviewAccess,
    ) -> Result<ReviewDetails, String> {
        let (selected_baseline, analysis_baseline, proposal, comparison_mode): (String, String, String, String) = self.storage.connection.query_row(
            "SELECT selected_base_commit,analysis_base_commit,proposal_commit,comparison_mode FROM reviews WHERE id=?1",
            [review_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).map_err(|_| "Review is unavailable".to_owned())?;
        Ok(ReviewDetails {
            api_version: SERVICE_API_VERSION,
            review_id: review_id.into(),
            selected_baseline,
            analysis_baseline,
            proposal,
            comparison_mode,
            scope_version: access.scope.version.as_str().into(),
            stale: false,
            capabilities: Vec::new(),
        })
    }

    fn pr_revision_batch(&self, after: i64, limit: usize) -> Result<Vec<(i64, PrSummary)>, String> {
        let mut statement = self.storage.connection.prepare(
            "SELECT rev.rowid,rev.repository_id,repo.security_domain,repo.provider,repo.owner,repo.name,
                    rev.provider_pr_id,rev.generation,rev.review_id,rev.base_commit,rev.head_commit,
                    rev.target_branch,rev.state,coalesce((SELECT max(priority) FROM jobs WHERE review_id=rev.review_id),0),rev.observed_at_ms
             FROM pr_revisions rev
             JOIN pr_heads head ON head.repository_id=rev.repository_id
                AND head.provider_pr_id=rev.provider_pr_id AND head.generation=rev.generation
             JOIN repositories repo ON repo.id=rev.repository_id
             WHERE rev.rowid>?1 ORDER BY rev.rowid LIMIT ?2",
        ).map_err(|error| format!("Cannot prepare PR listing: {error}"))?;
        statement
            .query_map(params![after, limit as i64], |row| {
                Ok((
                    row.get(0)?,
                    PrSummary {
                        repository_id: row.get(1)?,
                        security_domain: row.get(2)?,
                        provider: row.get(3)?,
                        repository_owner: row.get(4)?,
                        repository_name: row.get(5)?,
                        provider_pr_id: row.get(6)?,
                        generation: row.get(7)?,
                        review_id: row.get(8)?,
                        base_commit: row.get(9)?,
                        head_commit: row.get(10)?,
                        target_branch: row.get(11)?,
                        state: row.get(12)?,
                        priority: row.get(13)?,
                        observed_at_ms: row.get(14)?,
                    },
                ))
            })
            .map_err(|error| format!("Cannot query PR listing: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode PR listing: {error}"))
    }

    fn revision_batch(
        &self,
        repository_id: i64,
        provider_pr_id: &str,
        after: i64,
        limit: usize,
    ) -> Result<Vec<(i64, PrSummary)>, String> {
        let mut statement = self
            .storage
            .connection
            .prepare(
                "SELECT rev.rowid,rev.repository_id,repo.security_domain,repo.provider,repo.owner,repo.name,
                    rev.provider_pr_id,rev.generation,rev.review_id,rev.base_commit,rev.head_commit,
                    rev.target_branch,rev.state,coalesce((SELECT max(priority) FROM jobs WHERE review_id=rev.review_id),0),rev.observed_at_ms
             FROM pr_revisions rev JOIN repositories repo ON repo.id=rev.repository_id
             WHERE rev.repository_id=?1 AND rev.provider_pr_id=?2 AND rev.rowid>?3 ORDER BY rev.rowid LIMIT ?4",
            )
            .map_err(|error| format!("Cannot prepare revision listing: {error}"))?;
        statement
            .query_map(
                params![repository_id, provider_pr_id, after, limit as i64],
                |row| {
                    Ok((
                        row.get(0)?,
                        PrSummary {
                            repository_id: row.get(1)?,
                            security_domain: row.get(2)?,
                            provider: row.get(3)?,
                            repository_owner: row.get(4)?,
                            repository_name: row.get(5)?,
                            provider_pr_id: row.get(6)?,
                            generation: row.get(7)?,
                            review_id: row.get(8)?,
                            base_commit: row.get(9)?,
                            head_commit: row.get(10)?,
                            target_branch: row.get(11)?,
                            state: row.get(12)?,
                            priority: row.get(13)?,
                            observed_at_ms: row.get(14)?,
                        },
                    ))
                },
            )
            .map_err(|error| format!("Cannot query revisions: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode revision: {error}"))
    }

    fn finding_batch(
        &self,
        snapshot: i64,
        after: i64,
        limit: usize,
    ) -> Result<Vec<FindingSummary>, String> {
        let mut statement = self
            .storage
            .connection
            .prepare(
                "SELECT id,kind,resolution,confidence,source_path,source_start_line,source_end_line
             FROM edges WHERE snapshot_id=?1 AND id>?2 AND resolution!='resolved'
             ORDER BY id LIMIT ?3",
            )
            .map_err(|error| format!("Cannot prepare findings: {error}"))?;
        statement
            .query_map(params![snapshot, after, limit as i64], |row| {
                Ok(FindingSummary {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    resolution: row.get(2)?,
                    confidence: row.get(3)?,
                    source_path: row.get(4)?,
                    source_start_line: row.get(5)?,
                    source_end_line: row.get(6)?,
                })
            })
            .map_err(|error| format!("Cannot query findings: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot decode findings: {error}"))
    }

    fn review_access(
        &self,
        bearer: &str,
        review_id: &str,
        action: Action,
        path: Option<&str>,
    ) -> Result<ReviewAccess, String> {
        if review_id.is_empty() || review_id.len() > 128 {
            return Err("Invalid review identity".into());
        }
        let principal = self.authenticator.authenticate(bearer)?;
        self.authorize_review_for_principal(&principal, review_id, action, path)
    }

    fn authorize_review_for_principal(
        &self,
        principal: &PrincipalId,
        review_id: &str,
        action: Action,
        path: Option<&str>,
    ) -> Result<ReviewAccess, String> {
        if review_id.is_empty() || review_id.len() > 128 {
            return Err("Invalid review identity".into());
        }
        let access = self.access_provider.access(principal, review_id)?;
        if access.principal != *principal {
            return Err("Session principal does not match review access".into());
        }
        authorize(
            action,
            principal,
            &access.scope,
            &access.current_grants,
            path,
        )?;
        let repository_matches: i64 = self
            .storage
            .connection
            .query_row(
                "SELECT count(*) FROM reviews r JOIN repositories repo ON repo.id=r.repository_id
             WHERE r.id=?1 AND repo.security_domain=?2 AND repo.provider=?3
               AND repo.owner=?4 AND repo.name=?5",
                params![
                    review_id,
                    access.scope.primary_repository.security_domain().as_str(),
                    access.scope.primary_repository.provider(),
                    access.scope.primary_repository.owner(),
                    access.scope.primary_repository.name(),
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot bind review authorization: {error}"))?;
        if repository_matches != 1 {
            return Err("Review is outside the authorized repository".into());
        }
        Ok(access)
    }

    fn snapshot_id(&self, review_id: &str, side: RevisionSide) -> Result<i64, String> {
        let column = match side {
            RevisionSide::Baseline => "baseline_snapshot_id",
            RevisionSide::Proposal => "proposal_snapshot_id",
        };
        self.storage
            .connection
            .query_row(
                &format!("SELECT {column} FROM reviews WHERE id=?1"),
                [review_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|_| "Review is unavailable".to_owned())?
            .ok_or("Review snapshot is not ready".into())
    }

    fn review_revision(
        &self,
        review_id: &str,
        side: RevisionSide,
    ) -> Result<(i64, String), String> {
        let column = match side {
            RevisionSide::Baseline => "analysis_base_commit",
            RevisionSide::Proposal => "proposal_commit",
        };
        self.storage
            .connection
            .query_row(
                &format!("SELECT repository_id,{column} FROM reviews WHERE id=?1"),
                [review_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Review is unavailable".to_owned())
    }
}

fn bounded_limit(limit: usize) -> Result<(), String> {
    if limit == 0 || limit > MAX_PAGE {
        Err("Page limit must be between 1 and 200".into())
    } else {
        Ok(())
    }
}
fn validate_label(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value.chars().any(char::is_control)
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace)
    {
        Err(format!("Invalid {kind}"))
    } else {
        Ok(())
    }
}
fn parse_positive_id(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or("Invalid opaque identity".into())
}
fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.contains(['\0', '\\'])
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".." | ".git"))
    {
        Err("Invalid repository-relative source path".into())
    } else {
        Ok(())
    }
}
fn decode_opaque_cursor(cursor: Option<&str>, prefix: &str, binding: &str) -> Result<i64, String> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let value = cursor
        .strip_prefix(&format!("{prefix}:"))
        .ok_or("Invalid cursor")?;
    let (encoded, tag) = value.split_once(':').ok_or("Invalid cursor")?;
    if encoded.len() != 16 || tag.len() != 16 {
        return Err("Invalid cursor".into());
    }
    let encoded = u64::from_str_radix(encoded, 16).map_err(|_| "Invalid cursor")?;
    let position = i64::try_from(encoded ^ cursor_mask(binding)).map_err(|_| "Invalid cursor")?;
    if tag != cursor_tag(prefix, binding, position) {
        return Err("Invalid cursor".into());
    }
    Ok(position)
}
fn encode_opaque_cursor(prefix: &str, binding: &str, value: i64) -> String {
    let encoded = (value as u64) ^ cursor_mask(binding);
    format!(
        "{prefix}:{encoded:016x}:{}",
        cursor_tag(prefix, binding, value)
    )
}
fn cursor_mask(binding: &str) -> u64 {
    let digest = Sha256::digest(format!("flowdiff-cursor-mask-v1\0{binding}").as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"))
}
fn cursor_tag(prefix: &str, binding: &str, value: i64) -> String {
    let digest =
        Sha256::digest(format!("flowdiff-cursor-tag-v1\0{prefix}\0{binding}\0{value}").as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_session_token(token: &str) -> Result<(), String> {
    if token.len() < 32 || token.len() > 4096 || token.chars().any(char::is_control) {
        Err("Invalid session token".into())
    } else {
        Ok(())
    }
}

fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn now_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(u128::from(u64::MAX)) as u64)
        .map_err(|_| "System clock is before the Unix epoch".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        JobLane, PathScope, RepositoryId, Scheduler, ScopeVersion, SecurityDomain, StoredEdge,
        StoredFileProgress,
    };
    use std::{
        collections::BTreeSet,
        fs,
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const REVIEW: &str = "review-service-contract";

    struct AccessState {
        principal: PrincipalId,
        scope: ScopeRecord,
        grants: RwLock<Vec<Grant>>,
    }

    impl ReviewAccessProvider for AccessState {
        fn access(
            &self,
            principal: &PrincipalId,
            _review_id: &str,
        ) -> Result<ReviewAccess, String> {
            if principal != &self.principal {
                return Err("Access unavailable".into());
            }
            Ok(ReviewAccess {
                principal: principal.clone(),
                scope: self.scope.clone(),
                current_grants: self
                    .grants
                    .read()
                    .map_err(|_| "Grant store is unavailable")?
                    .clone(),
            })
        }
    }

    impl ComparisonAccessProvider for AccessState {
        fn comparison_access(
            &self,
            principal: &PrincipalId,
            repository: &RepositoryId,
        ) -> Result<ReviewAccess, String> {
            if principal != &self.principal || repository != &self.scope.primary_repository {
                return Err("Access unavailable".into());
            }
            Ok(ReviewAccess {
                principal: principal.clone(),
                scope: self.scope.clone(),
                current_grants: self.grants.read().unwrap().clone(),
            })
        }
    }

    struct Reader {
        calls: AtomicUsize,
        requests: Mutex<Vec<(i64, String, String)>>,
    }

    impl Reader {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl ExactSourceReader for Reader {
        fn read_blob(
            &self,
            repository_id: i64,
            commit: &str,
            path: &str,
            maximum_bytes: usize,
        ) -> Result<SourceBlob, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.requests
                .lock()
                .unwrap()
                .push((repository_id, commit.into(), path.into()));
            let (blob_id, bytes) = if commit.bytes().all(|byte| byte == b'a') {
                ("blob-before", b"one\ntwo\nthree\n".to_vec())
            } else {
                ("blob-after", b"one\nchanged\nthree\n".to_vec())
            };
            assert!(bytes.len() <= maximum_bytes);
            Ok(SourceBlob {
                blob_id: blob_id.into(),
                bytes,
            })
        }

        fn list_paths(
            &self,
            _: i64,
            _: &str,
            maximum_paths: usize,
        ) -> Result<(Vec<String>, bool), String> {
            let paths = vec![
                "README.md".into(),
                "src/lib.rs".into(),
                "src/private/key.rs".into(),
            ];
            assert!(paths.len() <= maximum_paths);
            Ok((paths, false))
        }
    }

    struct Fixture {
        storage: Storage,
        auth: SessionRegistry,
        access: AccessState,
        reader: Reader,
        repository_id: i64,
        baseline: String,
        proposal: String,
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "flowdiff-service-{name}-{}-{}",
            std::process::id(),
            now_ms().unwrap()
        ))
    }

    fn fixture(name: &str) -> Fixture {
        let root = temp_path(name);
        fs::create_dir_all(&root).unwrap();
        let mut storage =
            Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
        let repository = RepositoryId::new(
            SecurityDomain::new("tenant-a").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let repository_id = storage
            .register_repository("tenant-a", "github", "owner", "repo")
            .unwrap();
        let scope_id = storage
            .register_scope(repository_id, "scope-service", b"scope-service-canonical")
            .unwrap();
        let baseline = "a".repeat(40);
        let proposal = "b".repeat(40);
        let (baseline_snapshot, _) = storage
            .begin_snapshot(repository_id, &baseline, scope_id, "service-test")
            .unwrap();
        let (proposal_snapshot, _) = storage
            .begin_snapshot(repository_id, &proposal, scope_id, "service-test")
            .unwrap();
        for (snapshot, blob) in [
            (baseline_snapshot, "blob-before"),
            (proposal_snapshot, "blob-after"),
        ] {
            storage
                .record_file_chunk(
                    snapshot,
                    &[
                        StoredFileProgress {
                            path: "src/lib.rs".into(),
                            blob_id: Some(blob.into()),
                            coverage: "indexed".into(),
                            detail: None,
                        },
                        StoredFileProgress {
                            path: "src/private/secret.rs".into(),
                            blob_id: Some("secret-blob".into()),
                            coverage: "excluded".into(),
                            detail: None,
                        },
                    ],
                    true,
                )
                .unwrap();
            storage
                .record_graph_paths(snapshot, &["src/lib.rs".into()])
                .unwrap();
        }
        storage
            .record_graph_chunk(
                proposal_snapshot,
                &[],
                &[
                    StoredEdge {
                        stable_key: "unresolved-call".into(),
                        from_symbol: None,
                        to_symbol: None,
                        kind: "call".into(),
                        confidence: 0.4,
                        resolution: "unresolved".into(),
                        source_path: "src/lib.rs".into(),
                        source_span: Some((2, 1, 2, 8)),
                        provenance: b"test".to_vec(),
                    },
                    StoredEdge {
                        stable_key: "EXCLUDED_EXPORT_SENTINEL".into(),
                        from_symbol: None,
                        to_symbol: None,
                        kind: "secret-call".into(),
                        confidence: 1.0,
                        resolution: "resolved".into(),
                        source_path: "src/private/EXCLUDED_EXPORT_SENTINEL.rs".into(),
                        source_span: Some((1, 1, 1, 20)),
                        provenance: b"excluded-sentinel".to_vec(),
                    },
                ],
            )
            .unwrap();
        {
            let mut scheduler = Scheduler::new(&mut storage);
            scheduler
                .register_review(
                    REVIEW,
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
                .observe_pr(repository_id, "42", 1, REVIEW, &proposal, &baseline, 2)
                .unwrap();
            scheduler
                .enqueue(REVIEW, "analyze", 100, JobLane::Warm, "42", 1, 3)
                .unwrap();
        }
        storage
            .connection
            .execute(
                "UPDATE reviews SET baseline_snapshot_id=?1,proposal_snapshot_id=?2 WHERE id=?3",
                params![baseline_snapshot, proposal_snapshot, REVIEW],
            )
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO pr_revisions(repository_id,provider_pr_id,generation,review_id,base_commit,head_commit,target_branch,state,observed_at_ms)
                 VALUES (?1,'42',1,?2,?3,?4,'main','open',4)",
                params![repository_id, REVIEW, baseline, proposal],
            )
            .unwrap();

        let principal = PrincipalId::new("reviewer@example.com").unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("scope-service").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::new(vec!["src".into()], vec!["src/private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: true,
            model_destinations: BTreeSet::new(),
        };
        let grant = Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source, Action::Export]),
            paths: PathScope::all(),
        };
        let auth = SessionRegistry::default();
        auth.register(TOKEN, principal.clone(), now_ms().unwrap() + 60_000)
            .unwrap();
        Fixture {
            storage,
            auth,
            access: AccessState {
                principal,
                scope,
                grants: RwLock::new(vec![grant]),
            },
            reader: Reader::new(),
            repository_id,
            baseline,
            proposal,
        }
    }

    #[test]
    fn sessions_store_only_digests_and_revoke_immediately() {
        let registry = SessionRegistry::default();
        let principal = PrincipalId::new("person").unwrap();
        registry
            .register(TOKEN, principal.clone(), now_ms().unwrap() + 10_000)
            .unwrap();
        assert_eq!(registry.authenticate(TOKEN).unwrap(), principal);
        assert!(registry.revoke(TOKEN).unwrap());
        assert_eq!(
            registry.authenticate(TOKEN).unwrap_err(),
            "Authentication required"
        );
        assert!(!format!("{:?}", registry.sessions.read().unwrap()).contains(TOKEN));
    }

    #[test]
    fn review_annotations_are_snapshot_bound_private_audited_and_conflict_safe() {
        let mut fixture = fixture("review-annotations");
        fixture.storage.connection.execute_batch(&format!(
            "INSERT INTO reviews(id,repository_id,selected_base_commit,analysis_base_commit,proposal_commit,comparison_mode,scope_id,baseline_snapshot_id,proposal_snapshot_id,created_at_ms)
             SELECT 'review-prior',repository_id,selected_base_commit,analysis_base_commit,selected_base_commit,comparison_mode,scope_id,baseline_snapshot_id,baseline_snapshot_id,0 FROM reviews WHERE id='{REVIEW}';
             INSERT INTO pr_revisions(repository_id,provider_pr_id,generation,review_id,base_commit,head_commit,target_branch,state,observed_at_ms)
             VALUES ({},'42',0,'review-prior','{}','{}','main','open',0);
             INSERT INTO review_annotations(id,review_id,proposal_commit,owner_principal,visibility,kind,target_path,target_key,body,status,version,created_at_ms,updated_at_ms)
             VALUES ('prior-note','review-prior','{}','reviewer@example.com','private','reviewed','src/lib.rs','src/lib.rs','','active',1,0,0);",
            fixture.repository_id,fixture.baseline,fixture.baseline,fixture.baseline
        )).unwrap();
        let private = ReviewAnnotationWrite {
            id: "annotation-private".into(),
            visibility: "private".into(),
            kind: "question".into(),
            target_path: Some("src/lib.rs".into()),
            target_key: "src/lib.rs:2".into(),
            body: "Does the caller handle this branch?".into(),
            status: "active".into(),
            expected_version: 0,
        };
        let created = ExportCommands::new(
            &mut fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .write_review_annotation(TOKEN, REVIEW, &private)
        .unwrap();
        assert_eq!(created.proposal_commit, fixture.proposal);
        assert_eq!(created.version, 1);
        let delta = ExportCommands::new(
            &mut fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .review_continuity_delta(TOKEN, REVIEW)
        .unwrap();
        assert_eq!(delta.prior_review_id.as_deref(), Some("review-prior"));
        assert_eq!(delta.changed_paths, ["src/lib.rs"]);

        let mut update = private.clone();
        update.expected_version = 1;
        update.status = "resolved".into();
        let resolved = ExportCommands::new(
            &mut fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .write_review_annotation(TOKEN, REVIEW, &update)
        .unwrap();
        assert_eq!(resolved.version, 2);
        assert!(
            ExportCommands::new(
                &mut fixture.storage,
                &fixture.auth,
                &fixture.access,
                &fixture.reader
            )
            .write_review_annotation(TOKEN, REVIEW, &update)
            .unwrap_err()
            .contains("concurrently")
        );
        let audit_count: i64 = fixture.storage.connection.query_row(
            "SELECT count(*) FROM review_annotation_audit WHERE annotation_id='annotation-private'", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(audit_count, 2);
        let audit = ExportCommands::new(
            &mut fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .review_annotation_audit(TOKEN, REVIEW)
        .unwrap();
        assert_eq!(
            audit
                .iter()
                .map(|item| item.action.as_str())
                .collect::<Vec<_>>(),
            ["created", "resolved"]
        );
        let handoff = ReviewAnnotationWrite {
            id: "handoff-authorized".into(),
            visibility: "shared".into(),
            kind: "handoff".into(),
            target_path: Some("src/lib.rs".into()),
            target_key: "handoff:reviewer@example.com".into(),
            body: "reviewer@example.com".into(),
            status: "active".into(),
            expected_version: 0,
        };
        assert_eq!(
            ExportCommands::new(
                &mut fixture.storage,
                &fixture.auth,
                &fixture.access,
                &fixture.reader
            )
            .write_review_annotation(TOKEN, REVIEW, &handoff)
            .unwrap()
            .kind,
            "handoff"
        );
        let mut denied_handoff = handoff.clone();
        denied_handoff.id = "handoff-denied".into();
        denied_handoff.target_key = "handoff:unknown@example.com".into();
        denied_handoff.body = "unknown@example.com".into();
        assert!(
            ExportCommands::new(
                &mut fixture.storage,
                &fixture.auth,
                &fixture.access,
                &fixture.reader
            )
            .write_review_annotation(TOKEN, REVIEW, &denied_handoff)
            .is_err()
        );

        let other_principal = PrincipalId::new("other@example.com").unwrap();
        let other_token = "fedcba9876543210fedcba9876543210";
        let other_auth = SessionRegistry::default();
        other_auth
            .register(
                other_token,
                other_principal.clone(),
                now_ms().unwrap() + 60_000,
            )
            .unwrap();
        let other_access = AccessState {
            principal: other_principal.clone(),
            scope: fixture.access.scope.clone(),
            grants: RwLock::new(vec![Grant {
                principal: other_principal,
                repository: fixture.access.scope.primary_repository.clone(),
                actions: BTreeSet::from([Action::Graph, Action::Export]),
                paths: PathScope::all(),
            }]),
        };
        let visible = ExportCommands::new(
            &mut fixture.storage,
            &other_auth,
            &other_access,
            &fixture.reader,
        )
        .review_annotations(other_token, REVIEW)
        .unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "handoff-authorized");
    }

    #[test]
    fn service_cursors_are_opaque_tamper_evident_and_access_bound() {
        let cursor = encode_opaque_cursor("pr1", "prs:reviewer-a", 42);
        assert!(!cursor.contains(":42"));
        assert_eq!(
            decode_opaque_cursor(Some(&cursor), "pr1", "prs:reviewer-a").unwrap(),
            42
        );
        assert!(decode_opaque_cursor(Some(&cursor), "pr1", "prs:reviewer-b").is_err());
        let mut tampered = cursor.into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'0' { b'1' } else { b'0' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert!(decode_opaque_cursor(Some(&tampered), "pr1", "prs:reviewer-a").is_err());
    }

    #[test]
    fn exact_source_is_scoped_and_bound_to_snapshot_blob() {
        let fixture = fixture("source");
        let api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        );
        let before = api
            .source_span(
                TOKEN,
                REVIEW,
                RevisionSide::Baseline,
                "src/lib.rs",
                "blob-before",
                2,
                2,
            )
            .unwrap();
        let after = api
            .source_span(
                TOKEN,
                REVIEW,
                RevisionSide::Proposal,
                "src/lib.rs",
                "blob-after",
                2,
                2,
            )
            .unwrap();
        assert_eq!(
            (before.commit, before.text),
            (fixture.baseline, "two".into())
        );
        assert_eq!(
            (after.commit, after.text),
            (fixture.proposal, "changed".into())
        );
        assert!(
            api.source_span(
                TOKEN,
                REVIEW,
                RevisionSide::Proposal,
                "../secret",
                "x",
                1,
                1
            )
            .is_err()
        );
        assert!(
            api.source_span(
                TOKEN,
                REVIEW,
                RevisionSide::Proposal,
                "src/private/secret.rs",
                "secret-blob",
                1,
                1,
            )
            .is_err()
        );
        assert!(
            api.source_span(
                TOKEN,
                REVIEW,
                RevisionSide::Proposal,
                "src/lib.rs",
                "caller-chosen-blob",
                1,
                1,
            )
            .is_err()
        );
        assert_eq!(fixture.reader.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn current_grants_and_exact_repository_are_rechecked() {
        let fixture = fixture("authorization");
        let api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        );
        assert_eq!(api.list_prs(TOKEN, 1, None).unwrap().items.len(), 1);
        fixture.access.grants.write().unwrap().clear();
        assert!(
            api.graph_children(TOKEN, REVIEW, RevisionSide::Proposal, None, 10, None)
                .is_err()
        );
        assert!(api.export_summary(TOKEN, REVIEW).is_err());

        let wrong_repository = RepositoryId::new(
            SecurityDomain::new("tenant-a").unwrap(),
            "github",
            "owner",
            "different-repo",
        )
        .unwrap();
        let mut wrong_scope = fixture.access.scope.clone();
        wrong_scope.primary_repository = wrong_repository.clone();
        let wrong_access = AccessState {
            principal: fixture.access.principal.clone(),
            scope: wrong_scope,
            grants: RwLock::new(vec![Grant {
                principal: fixture.access.principal.clone(),
                repository: wrong_repository,
                actions: BTreeSet::from([Action::Graph]),
                paths: PathScope::all(),
            }]),
        };
        let wrong_api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &wrong_access,
            &fixture.reader,
        );
        assert!(wrong_api.progress(TOKEN, REVIEW).is_err());
    }

    #[test]
    fn changed_files_are_ordered_and_scope_filtered() {
        let fixture = fixture("changed-files");
        fixture.storage.connection.execute(
            "INSERT INTO change_sets(id,repository_id,baseline_snapshot_id,proposal_snapshot_id,baseline_commit,proposal_commit,resolution_strategy,created_at_ms)
             SELECT 'changes-1',?1,b.id,p.id,?2,?3,'local',1 FROM snapshots b,snapshots p
             WHERE b.repository_id=?1 AND b.commit_id=?2 AND p.repository_id=?1 AND p.commit_id=?3",
            params![fixture.repository_id, fixture.baseline, fixture.proposal],
        ).unwrap();
        for (position, kind, path, old_path) in [
            (0, "modified", "src/lib.rs", None),
            (1, "added", "src/new.rs", None),
            (2, "modified", "src/private/secret.rs", None),
            (3, "renamed", "src/moved.rs", Some("src/private/old.rs")),
        ] {
            fixture.storage.connection.execute(
                "INSERT INTO change_paths(change_set_id,position,kind,path,old_path,parse_required) VALUES ('changes-1',?1,?2,?3,?4,1)",
                params![position, kind, path, old_path],
            ).unwrap();
        }
        fixture
            .storage
            .connection
            .execute(
                "UPDATE reviews SET change_set_id='changes-1' WHERE id=?1",
                [REVIEW],
            )
            .unwrap();
        let api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        );
        let first = api.changed_files(TOKEN, REVIEW, 2, None).unwrap();
        assert_eq!(
            first
                .items
                .iter()
                .map(|item| (item.kind.as_str(), item.path.as_str()))
                .collect::<Vec<_>>(),
            vec![("modified", "src/lib.rs"), ("added", "src/new.rs"),]
        );
        let second = api
            .changed_files(TOKEN, REVIEW, 2, first.next_cursor.as_deref())
            .unwrap();
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].path, "src/moved.rs");
        assert_eq!(second.items[0].old_path, None);
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn inventory_pagination_and_request_limits_are_explicit() {
        let fixture = fixture("pagination");
        fixture
            .storage
            .connection
            .execute(
                "INSERT INTO pr_revisions(repository_id,provider_pr_id,generation,review_id,base_commit,head_commit,target_branch,state,observed_at_ms)
                 VALUES (?1,'43',1,?2,?3,?4,'main','draft',5)",
                params![fixture.repository_id, REVIEW, fixture.baseline, fixture.proposal],
            )
            .unwrap();
        fixture.storage.connection.execute(
            "INSERT INTO pr_heads(repository_id,provider_pr_id,generation,head_commit,base_commit,observed_at_ms,current_review_id)
             VALUES (?1,'43',1,?2,?3,5,?4)",
            params![fixture.repository_id, fixture.proposal, fixture.baseline, REVIEW],
        ).unwrap();
        let api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        );
        let first = api.list_prs(TOKEN, 1, None).unwrap();
        assert_eq!(first.items.len(), 1);
        let second = api
            .list_prs(TOKEN, 1, first.next_cursor.as_deref())
            .unwrap();
        assert_eq!(second.items.len(), 1);
        assert_ne!(
            first.items[0].provider_pr_id,
            second.items[0].provider_pr_id
        );
        assert!(api.list_prs(TOKEN, 201, None).is_err());
        assert!(
            api.source_span(
                TOKEN,
                REVIEW,
                RevisionSide::Proposal,
                "src/lib.rs",
                "blob-after",
                1,
                201,
            )
            .is_err()
        );
    }

    #[test]
    fn details_revisions_and_findings_preserve_exact_identity() {
        let fixture = fixture("details");
        let api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        );
        let details = api.review_details(TOKEN, REVIEW).unwrap();
        assert_eq!(details.selected_baseline, fixture.baseline);
        assert_eq!(details.analysis_baseline, fixture.baseline);
        assert_eq!(details.proposal, fixture.proposal);
        assert_eq!(details.comparison_mode, "exact");
        assert_eq!(details.capabilities, vec!["graph", "source", "export"]);
        assert!(!details.stale);

        let revisions = api
            .list_revisions(TOKEN, fixture.repository_id, "42", 10, None)
            .unwrap();
        assert_eq!(revisions.items.len(), 1);
        assert_eq!(revisions.items[0].review_id.as_deref(), Some(REVIEW));
        assert_eq!(revisions.items[0].repository_owner, "owner");
        assert_eq!(revisions.items[0].repository_name, "repo");
        assert_eq!(revisions.items[0].priority, 100);
        let findings = api.findings(TOKEN, REVIEW, 10, None).unwrap();
        assert_eq!(findings.items.len(), 1);
        assert_eq!(findings.items[0].resolution, "unresolved");
        assert_eq!(findings.items[0].source_path, "src/lib.rs");
    }

    #[test]
    fn scope_preview_lists_only_paths_crossing_the_boundary() {
        let fixture = fixture("scope-file-preview");
        let api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        );
        let mut proposed = fixture.access.scope.clone();
        proposed.primary_paths = PathScope::all();
        let delta = api.preview_scope(TOKEN, REVIEW, &proposed).unwrap();
        assert_eq!(
            delta.added_files,
            vec!["primary:README.md", "primary:src/private/key.rs"]
        );
        assert!(delta.removed_files.is_empty());
        assert!(!delta.file_preview_truncated);
    }

    #[test]
    fn exact_comparison_creation_is_authorized_and_idempotent() {
        let mut fixture = fixture("create-comparison");
        let request = ExactComparisonRequest {
            repository: fixture.access.scope.primary_repository.clone(),
            provider_pr_id: "99".into(),
            generation: 1,
            selected_baseline: fixture.baseline.clone(),
            proposal: "c".repeat(40),
            target_branch: "main".into(),
            state: "draft".into(),
            priority: 100,
        };
        let mut commands =
            ServiceCommands::new(&mut fixture.storage, &fixture.auth, &fixture.access);
        let first = commands.create_exact_comparison(TOKEN, &request).unwrap();
        let second = commands.create_exact_comparison(TOKEN, &request).unwrap();
        assert_eq!(first.review_id, second.review_id);
        assert_eq!(first.job_id, second.job_id);
        assert!(first.created);
        assert!(!second.created);
        let mut conflicting = request;
        conflicting.state = "open".into();
        assert!(
            commands
                .create_exact_comparison(TOKEN, &conflicting)
                .is_err()
        );
    }

    #[test]
    fn scope_change_persists_new_identity_and_supersedes_old_work() {
        let mut fixture = fixture("scope-change");
        let mut proposed = fixture.access.scope.clone();
        proposed.primary_paths = PathScope::new(vec!["src/lib.rs".into()], vec![]).unwrap();
        let receipt = ServiceCommands::new(&mut fixture.storage, &fixture.auth, &fixture.access)
            .replace_review_scope(
                TOKEN,
                ScopeChangeRequest {
                    review_id: REVIEW.into(),
                    provider_pr_id: "42".into(),
                    generation: 1,
                    priority: 90,
                    proposed_scope: proposed,
                },
            )
            .unwrap();
        assert_ne!(receipt.review_id, REVIEW);
        let current: String = fixture.storage.connection.query_row(
            "SELECT current_review_id FROM pr_heads WHERE repository_id=?1 AND provider_pr_id='42'",
            [fixture.repository_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(current, receipt.review_id);
        let old_state: String = fixture
            .storage
            .connection
            .query_row(
                "SELECT state FROM jobs WHERE review_id=?1",
                [REVIEW],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_state, "superseded");
        let (scope_count, new_job): (i64, i64) = fixture
            .storage
            .connection
            .query_row(
                "SELECT (SELECT count(*) FROM scopes WHERE repository_id=?1),
                    (SELECT count(*) FROM jobs WHERE review_id=?2 AND state='queued')",
                params![fixture.repository_id, receipt.review_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((scope_count, new_job), (2, 1));
    }

    #[test]
    fn export_jobs_are_durable_idempotent_and_reauthorized() {
        let mut fixture = fixture("export-job");
        let summary = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .export_summary(TOKEN, REVIEW)
        .unwrap();
        let summary = String::from_utf8(summary).unwrap();
        assert!(summary.contains("Selected baseline:"));
        assert!(summary.contains("src/lib.rs"));
        assert!(summary.contains("inference only"));
        assert!(!summary.contains("EXCLUDED_EXPORT_SENTINEL"));
        let first = ExportCommands::new(
            &mut fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .create_markdown_export(TOKEN, REVIEW)
        .unwrap();
        let second = ExportCommands::new(
            &mut fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .create_markdown_export(TOKEN, REVIEW)
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.state, "ready");
        assert!(first.artifact_digest.is_some());

        let api = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        );
        let job = api.job_details(TOKEN, &first.job_id).unwrap();
        assert_eq!(job.review_id, REVIEW);
        assert_eq!(job.kind, "export");
        assert_eq!(job.artifact_digest, first.artifact_digest);
        fixture.access.grants.write().unwrap().clear();
        assert!(api.job_details(TOKEN, &first.job_id).is_err());
    }

    #[test]
    fn export_evidence_respects_current_path_grants() {
        let fixture = fixture("export-current-path-grant");
        fixture.access.grants.write().unwrap()[0].paths =
            PathScope::new(vec!["README.md".into()], vec![]).unwrap();
        let summary = ServiceApi::new(
            &fixture.storage,
            &fixture.auth,
            &fixture.access,
            &fixture.reader,
        )
        .export_summary(TOKEN, REVIEW)
        .unwrap();
        let summary = String::from_utf8(summary).unwrap();
        assert!(!summary.contains("src/lib.rs"));
        assert!(!summary.contains("EXCLUDED_EXPORT_SENTINEL"));
        assert!(summary.contains("No authorized structural findings"));
    }
}
