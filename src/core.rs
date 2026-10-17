use crate::{
    Action, Grant, PrincipalId, ReviewId, ReviewIdentityInput, ScopeRecord, analysis::Review,
    architecture, authorize, context, git,
};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

// Parser-quality evidence is now stable across cold/warm analysis. New identities
// keep corrected evidence separate from artifacts produced by the old cache.
pub(crate) const REVIEW_RULE_VERSION: &str = "flowdiff-review-v5";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotId(String);

impl SnapshotId {
    pub fn new(commit: String) -> Result<Self, String> {
        if matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(commit.to_ascii_lowercase()))
        } else {
            Err("Snapshot ID must be a 40- or 64-character hexadecimal commit ID".into())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeVersion(String);

impl ScopeVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err("Scope version must contain 1–128 printable characters".into());
        }
        Ok(Self(value))
    }

    pub fn local_default() -> Self {
        Self("local-unversioned".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComparisonMode {
    #[default]
    Exact,
    MergeBase,
}

#[derive(Clone, Debug)]
pub struct ReviewRequest {
    pub repository: PathBuf,
    pub baseline: String,
    pub proposal: String,
    pub comparison: ComparisonMode,
    pub principal: PrincipalId,
    pub scope: ScopeRecord,
    pub current_grants: Vec<Grant>,
    pub context_manifest: Option<PathBuf>,
    pub cache_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct IndexRequest {
    pub repository: PathBuf,
    pub snapshot: String,
    pub principal: PrincipalId,
    pub scope: ScopeRecord,
    pub current_grants: Vec<Grant>,
    pub cache_dir: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub indexed: usize,
    pub unsupported: usize,
    pub excluded: usize,
    pub failed: usize,
    pub deferred: usize,
    pub enumeration_complete: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn checkpoint(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err("Analysis cancelled".into())
        } else {
            Ok(())
        }
    }
}

pub struct AnalysisResult {
    pub review_id: ReviewId,
    pub selected_baseline: SnapshotId,
    pub baseline: SnapshotId,
    pub proposal: SnapshotId,
    pub scope_version: ScopeVersion,
    pub coverage: Coverage,
    pub review: Review,
    principal: PrincipalId,
    scope: ScopeRecord,
}

impl AnalysisResult {
    pub fn scope(&self) -> &ScopeRecord {
        &self.scope
    }
    pub fn authorize(
        &self,
        action: Action,
        current_grants: &[Grant],
        path: Option<&str>,
    ) -> Result<(), String> {
        authorize(action, &self.principal, &self.scope, current_grants, path)
    }
}

pub trait ReviewEngine {
    fn review(
        &self,
        request: &ReviewRequest,
        cancellation: &Cancellation,
    ) -> Result<AnalysisResult, String>;
}

pub trait RepositoryAccess {
    fn resolve_snapshot(&self, repository: &Path, reference: &str) -> Result<SnapshotId, String>;
}

/// Credentialed mirror synchronization is an adapter boundary separate from
/// read-only object access. Implementations must validate provider ownership
/// and the configured remote before fetching, then return an immutable commit.
pub trait MirrorUpdater {
    fn update_mirror(
        &self,
        repository: &crate::RepositoryId,
        mirror: &Path,
        configured_remote: &str,
        reference: &str,
    ) -> Result<SnapshotId, String>;
}

pub trait ArtifactStorage {
    fn write_new(&self, path: &Path, bytes: &[u8]) -> Result<(), String>;
}

pub trait Clock {
    fn unix_millis(&self) -> u64;
}

#[derive(Default)]
pub struct LocalReviewEngine;

#[derive(Default)]
pub struct LocalGitRepository;

impl RepositoryAccess for LocalGitRepository {
    fn resolve_snapshot(&self, repository: &Path, reference: &str) -> Result<SnapshotId, String> {
        SnapshotId::new(git::resolve(repository, reference)?)
    }
}

impl ReviewEngine for LocalReviewEngine {
    fn review(
        &self,
        request: &ReviewRequest,
        cancellation: &Cancellation,
    ) -> Result<AnalysisResult, String> {
        cancellation.checkpoint()?;
        authorize(
            Action::Index,
            &request.principal,
            &request.scope,
            &request.current_grants,
            None,
        )?;
        let selected_baseline =
            LocalGitRepository.resolve_snapshot(&request.repository, &request.baseline)?;
        let mut review = git::review(request, cancellation)?;
        cancellation.checkpoint()?;
        let baseline = SnapshotId::new(review.base.clone())?;
        let proposal = SnapshotId::new(review.head.clone())?;
        let review_id = ReviewId::from_input(&ReviewIdentityInput {
            repository: request.scope.primary_repository.clone(),
            selected_baseline: selected_baseline.clone(),
            analysis_baseline: baseline.clone(),
            proposal: proposal.clone(),
            comparison: request.comparison,
            scope: request.scope.clone(),
            parser_version: crate::parser::RUST_PARSER_VERSION.into(),
            resolver_version: "flowdiff-name-candidate-v1".into(),
            rule_version: REVIEW_RULE_VERSION.into(),
        })?;
        review.architecture = architecture::build(&architecture::BuildRequest {
            repo: &request.repository,
            base: &review.base,
            head: &review.head,
            cache: &request.cache_dir,
            security_domain: request.scope.primary_repository.security_domain().as_str(),
            paths: &request.scope.primary_paths,
            principal: &request.principal,
            scope: &request.scope,
            current_grants: &request.current_grants,
            cancellation,
        })?;
        let change_set = crate::incremental::plan(
            baseline.as_str(),
            proposal.as_str(),
            &review.architecture,
            false,
            false,
        );
        review.architecture.after = crate::incremental::apply(
            &review.architecture.before,
            &review.architecture.after,
            &change_set,
        )?;
        review.change_set = Some(change_set);
        cancellation.checkpoint()?;
        if let Some(path) = &request.context_manifest {
            review.context = context::load(
                path,
                &review.files,
                &request.principal,
                &request.scope,
                &request.current_grants,
            )?;
        }
        cancellation.checkpoint()?;

        let coverage = Coverage {
            indexed: review.architecture.analyzed + review.architecture.reused,
            excluded: review.architecture.excluded,
            unsupported: review.architecture.unsupported,
            failed: review.architecture.failed,
            deferred: review.architecture.deferred,
            enumeration_complete: review.architecture.enumeration_complete,
        };
        Ok(AnalysisResult {
            review_id,
            selected_baseline,
            baseline,
            proposal,
            scope_version: request.scope.version.clone(),
            coverage,
            review,
            principal: request.principal.clone(),
            scope: request.scope.clone(),
        })
    }
}

pub fn analyze_review(
    request: &ReviewRequest,
    cancellation: &Cancellation,
) -> Result<AnalysisResult, String> {
    LocalReviewEngine.review(request, cancellation)
}

pub fn analyze_index(
    request: &IndexRequest,
    cancellation: &Cancellation,
) -> Result<AnalysisResult, String> {
    analyze_review(
        &ReviewRequest {
            repository: request.repository.clone(),
            baseline: request.snapshot.clone(),
            proposal: request.snapshot.clone(),
            comparison: ComparisonMode::Exact,
            principal: request.principal.clone(),
            scope: request.scope.clone(),
            current_grants: request.current_grants.clone(),
            context_manifest: None,
            cache_dir: request.cache_dir.clone(),
        },
        cancellation,
    )
}

#[derive(Default)]
pub struct FileArtifactStorage;

impl ArtifactStorage for FileArtifactStorage {
    fn write_new(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| {
                format!(
                    "Cannot create {}: {error}. Choose a new output path.",
                    path.display()
                )
            })?;
        file.write_all(bytes)
            .map_err(|error| format!("Cannot write report: {error}"))
    }
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_snapshot_and_scope_identifiers() {
        assert!(SnapshotId::new("a".repeat(40)).is_ok());
        assert!(SnapshotId::new("not-a-commit".into()).is_err());
        assert_eq!(ScopeVersion::new("scope-v1").unwrap().as_str(), "scope-v1");
        assert!(ScopeVersion::new("bad\nvalue").is_err());
    }

    #[test]
    fn cancellation_is_cloneable_and_observable() {
        let cancellation = Cancellation::default();
        let worker_view = cancellation.clone();
        cancellation.cancel();
        assert!(worker_view.is_cancelled());
        assert_eq!(worker_view.checkpoint(), Err("Analysis cancelled".into()));
    }

    #[test]
    fn storage_failure_is_injectable_without_analysis() {
        struct FailingStorage;
        impl ArtifactStorage for FailingStorage {
            fn write_new(&self, _: &Path, _: &[u8]) -> Result<(), String> {
                Err("injected storage failure".into())
            }
        }
        assert_eq!(
            FailingStorage.write_new(Path::new("unused"), b"report"),
            Err("injected storage failure".into())
        );
    }

    #[test]
    fn clock_is_injectable() {
        struct FixedClock;
        impl Clock for FixedClock {
            fn unix_millis(&self) -> u64 {
                42
            }
        }
        assert_eq!(FixedClock.unix_millis(), 42);
    }

    #[test]
    fn repository_failure_is_injectable() {
        struct DeniedRepository;
        impl RepositoryAccess for DeniedRepository {
            fn resolve_snapshot(&self, _: &Path, _: &str) -> Result<SnapshotId, String> {
                Err("injected repository denial".into())
            }
        }
        assert_eq!(
            DeniedRepository.resolve_snapshot(Path::new("unused"), "HEAD"),
            Err("injected repository denial".into())
        );
    }
}
