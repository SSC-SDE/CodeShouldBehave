mod ai;
mod analysis;
mod architecture;
mod context;
mod core;
#[cfg(feature = "server")]
pub mod design;
#[cfg(feature = "server")]
pub mod design_storage;
mod git;
#[cfg(feature = "server")]
mod github_check;
#[cfg(feature = "server")]
mod http;
mod incremental;
#[cfg(feature = "server")]
pub mod lld;
mod model;
mod onboarding;
#[cfg(feature = "server")]
mod onboarding_detection;
#[cfg(feature = "server")]
mod onboarding_review;
mod onboarding_scan;
#[cfg(feature = "server")]
mod onboarding_service;
mod operations;
mod parser;
mod policy;
mod provider;
#[cfg(feature = "server")]
pub mod public_report;
mod report;
#[cfg(feature = "server")]
mod review_artifact;
mod rollout;
mod scheduler;
mod service;
mod snapshot;
mod storage;

pub use ai::{
    AI_CONTRACT_VERSION, DestinationPolicy, DisabledModelAdapter, DisclosureLevel, EvidenceBundle,
    EvidenceItem, FakeModelAdapter, ModelAdapter, ModelCapability, ModelRequest, ModelResult,
    build_evidence_bundle, redact_sensitive, validate_model_result,
};
pub use analysis::Review;
pub use core::{
    AnalysisResult, ArtifactStorage, Cancellation, Clock, ComparisonMode, Coverage,
    FileArtifactStorage, IndexRequest, LocalGitRepository, LocalReviewEngine, MirrorUpdater,
    RepositoryAccess, ReviewEngine, ReviewRequest, ScopeVersion, SnapshotId, SystemClock,
    analyze_index, analyze_review,
};
pub use git::MirrorReadLease;
#[cfg(feature = "server")]
pub use github_check::{ARCHITECTURE_CHECK_NAME, ArchitectureCheckDraft, CheckOutput, CheckTarget};
#[cfg(feature = "server")]
pub use http::{HttpBackend, HttpFailure, HttpOperation, ServiceHost, router, serve_loopback};
pub use incremental::{ChangeSet, InvalidationCause, PathChange, ResolutionStrategy};
pub use model::{
    ContextPin, PathScope, RepositoryId, ReviewId, ReviewIdentityInput, ScopeDelta, ScopeRecord,
    SecurityDomain,
};
pub use onboarding::{
    ComponentDecision, ComponentProposal, ComponentProviderMapping, ComponentSignal,
    EvidenceConfidence, IntegrationCandidate, IntegrationDecision, ONBOARDING_VERSION,
    OnboardingConfirmation, OnboardingEvidence, OnboardingQuestion, OnboardingSession,
    OnboardingState, ProviderVerification,
};
#[cfg(feature = "server")]
pub use onboarding_review::load_onboarding_review_context;
pub use onboarding_scan::{
    INVENTORY_VERSION, InventoryItem, OnboardingInventory, scan_chunk as scan_onboarding_chunk,
    sensitive_path,
};
#[cfg(feature = "server")]
pub use onboarding_service::{
    LocalOnboardingSource, OnboardingAccessProvider, OnboardingHost, OnboardingSource,
};
pub use operations::{
    OperationalController, OperationalLimits, OperationalMetrics, OperationalMode, Readiness,
    WorkerAction, WorkerLimit, WorkerResourceLimits, WorkerResourceUsage, WorkerSupervisor,
};
pub use policy::{Action, Grant, PrincipalId, authorize, authorize_repository};
pub use provider::{
    AllowedRepository, CommandRestrictedFetcher, FetchLimits, FetchRequest, IntakeOutcome,
    ProviderEvent, ProviderIntake, RestrictedFetcher,
};
pub use rollout::{
    FeatureSwitch, ObservationWindow, ReviewerSignals, RolloutControls, RolloutStage,
};
pub use scheduler::{ClaimedJob, JobLane, QueueStats, RetryClass, Scheduler};
pub use service::{
    ChangedFilePage, ChangedFileSummary, ComparisonAccessProvider, ComparisonReceipt,
    ExactComparisonRequest, ExactSourceReader, ExportCommands, ExportReceipt, FindingPage,
    FindingSummary, JobDetails, PrPage, PrSummary, ReviewAccess, ReviewAccessProvider,
    ReviewAnnotation, ReviewAnnotationAudit, ReviewAnnotationWrite, ReviewContinuityDelta,
    ReviewDetails, ReviewProgress, RevisionPage, RevisionSide, SERVICE_API_VERSION,
    ScopeChangeRequest, ServiceApi, ServiceAuthenticator, ServiceCommands, SessionRegistry,
    SourceBlob, SourceSpanResponse,
};
pub use storage::{
    AiAuditRecord, ArtifactGarbageCollection, ArtifactRef, ArtifactRetentionPolicy,
    ClosedReviewRetentionCandidate, ClosedReviewRetentionPlan, ClosedReviewRetentionPolicy,
    GraphDirection, GraphEdge, GraphPage, GraphPathPage, GraphSymbolPage, Reconciliation, Storage,
    StoredEdge, StoredFileProgress, StoredSymbol,
};

#[cfg(feature = "server")]
pub use review_artifact::ReviewArtifact;

pub fn render_html(result: &Review) -> String {
    report::html(result)
}

pub fn render_markdown(result: &Review) -> String {
    report::markdown(result)
}

pub fn demo_review() -> Review {
    analysis::demo()
}

pub fn prepare_local_context_access(
    manifest: &std::path::Path,
    security_domain: &SecurityDomain,
    principal: &PrincipalId,
) -> Result<(Vec<ContextPin>, Vec<Grant>), String> {
    context::prepare_local_access(manifest, security_domain, principal)
}
