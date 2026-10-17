//! Immutable generated HLD facts and separately reviewed user interpretations.
//! Current onboarding paths and grants are checked before every response; an
//! accepted receipt is never permission to read its creator's original scope.

use crate::design::{
    DesignConstraint, DesignCorrespondence, DesignDecisionRecord, DesignElementKind,
    DesignRelation, DesignResolution, DesignReviewState, DesignVersion, DesignView,
    validate_design_scope,
};
use crate::{
    Action, ContextPin, Grant, PathScope, PrincipalId, RepositoryId, ScopeRecord, ScopeVersion,
    SecurityDomain, SnapshotId, Storage, authorize_repository,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const DESIGN_REVIEW_VERSION: &str = "csb-design-review-v1";
const MAX_PAYLOAD: usize = 1024 * 1024;
const MAX_CORRECTIONS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignInterpretation {
    AsBuilt,
    Intended,
}
impl DesignInterpretation {
    fn key(self) -> &'static str {
        match self {
            Self::AsBuilt => "as_built",
            Self::Intended => "intended",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignCorrectionKind {
    Responsibility,
    Boundary,
    ContractOwnership,
    DataOwnership,
    MajorFlow,
    Constraint,
    Correspondence,
    DocumentReference,
    LldInvariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignConstraintKind {
    ProhibitedDependency,
    RequiredContract,
}

/// User intent only. This record has no evidence/resolution fields to forge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignCorrection {
    pub id: String,
    pub kind: DesignCorrectionKind,
    pub target_id: String,
    pub value: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub related_ids: Vec<String>,
    pub constraint_kind: Option<DesignConstraintKind>,
    pub reference_design_id: Option<String>,
    #[serde(default)]
    pub correspondence_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_evidence_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignDecision {
    pub actor: String,
    pub accepted_at_ms: i64,
    pub preview_revision: u64,
    pub input_fingerprint: String,
    pub known_gaps_accepted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignReview {
    pub version: String,
    pub id: String,
    pub design_id: String,
    pub principal: String,
    pub revision: u64,
    pub interpretation: DesignInterpretation,
    pub corrections: Vec<DesignCorrection>,
    pub state: DesignReviewState,
    pub input_fingerprint: String,
    pub confirmation: Option<DesignDecision>,
}

/// Lifecycle status is derived at retrieval, never written over accepted history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesignAccepted {
    pub accepted_id: String,
    pub design: DesignVersion,
    pub observed_design: DesignVersion,
    pub review: DesignReview,
    pub lifecycle_state: DesignReviewState,
    pub active_default: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryWire {
    security_domain: String,
    provider: String,
    owner: String,
    name: String,
}
impl RepositoryWire {
    fn new(repository: &RepositoryId) -> Self {
        Self {
            security_domain: repository.security_domain().as_str().into(),
            provider: repository.provider().into(),
            owner: repository.owner().into(),
            name: repository.name().into(),
        }
    }
    fn model(self) -> Result<RepositoryId, String> {
        RepositoryId::new(
            SecurityDomain::new(self.security_domain)?,
            self.provider,
            self.owner,
            self.name,
        )
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathsWire {
    include: Vec<String>,
    exclude: Vec<String>,
}
impl PathsWire {
    fn new(paths: &PathScope) -> Self {
        Self {
            include: paths.includes().map(str::to_owned).collect(),
            exclude: paths.excludes().map(str::to_owned).collect(),
        }
    }
    fn model(self) -> Result<PathScope, String> {
        if self.include.len() > 200 || self.exclude.len() > 200 {
            return Err("Design scope exceeds path limits".into());
        }
        PathScope::new(self.include, self.exclude)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextWire {
    name: String,
    repository: RepositoryWire,
    commit: String,
    paths: PathsWire,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeWire {
    version: String,
    repository: RepositoryWire,
    paths: PathsWire,
    contexts: Vec<ContextWire>,
    contracts: BTreeSet<String>,
    export_allowed: bool,
    model_destinations: BTreeSet<String>,
}
impl ScopeWire {
    fn new(scope: &ScopeRecord) -> Self {
        Self {
            version: scope.version.as_str().into(),
            repository: RepositoryWire::new(&scope.primary_repository),
            paths: PathsWire::new(&scope.primary_paths),
            contexts: scope
                .contexts
                .iter()
                .map(|pin| ContextWire {
                    name: pin.name.clone(),
                    repository: RepositoryWire::new(&pin.repository),
                    commit: pin.commit.as_str().into(),
                    paths: PathsWire::new(&pin.paths),
                })
                .collect(),
            contracts: scope.contracts.clone(),
            export_allowed: scope.export_allowed,
            model_destinations: scope.model_destinations.clone(),
        }
    }
    fn model(self) -> Result<ScopeRecord, String> {
        if self.contexts.len() > 12
            || self.contracts.len() > 100
            || self.model_destinations.len() > 100
        {
            return Err("Design scope exceeds context limits".into());
        }
        Ok(ScopeRecord {
            version: ScopeVersion::new(self.version)?,
            primary_repository: self.repository.model()?,
            primary_paths: self.paths.model()?,
            contexts: self
                .contexts
                .into_iter()
                .map(|pin| {
                    Ok(ContextPin {
                        name: pin.name,
                        repository: pin.repository.model()?,
                        commit: SnapshotId::new(pin.commit)?,
                        paths: pin.paths.model()?,
                    })
                })
                .collect::<Result<_, String>>()?,
            contracts: self.contracts,
            export_allowed: self.export_allowed,
            model_destinations: self.model_destinations,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DesignEnvelope {
    version: String,
    scope: ScopeWire,
    design: DesignVersion,
}

fn label(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err("Invalid design identifier".into())
    } else {
        Ok(())
    }
}
fn payload<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "Cannot encode design record")?;
    if bytes.len() > MAX_PAYLOAD {
        return Err("Design record exceeds 1 MiB".into());
    }
    Ok(bytes)
}
fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() > MAX_PAYLOAD {
        return Err("Stored design record exceeds 1 MiB".into());
    }
    serde_json::from_slice(bytes).map_err(|_| "Invalid stored design record".into())
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn accepted_scope_fingerprint(scope: &ScopeRecord) -> String {
    let mut hash = Sha256::new();
    hash.update(b"csb-accepted-context-v1:sensitive-path-policy-v1");
    hash.update(scope.canonical_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn context_id(context: &Value) -> Result<String, String> {
    Ok(format!(
        "{}:{}:{}",
        context["session_id"]
            .as_str()
            .ok_or("Invalid onboarding identity")?,
        context["session_revision"]
            .as_u64()
            .ok_or("Invalid onboarding revision")?,
        context["proposal_fingerprint"]
            .as_str()
            .ok_or("Invalid onboarding fingerprint")?
    ))
}
fn paths_from_context(value: &Value) -> Result<PathScope, String> {
    let paths: PathsWire = serde_json::from_value(
        serde_json::json!({"include":value["include"],"exclude":value["exclude"]}),
    )
    .map_err(|_| "Invalid accepted design scope")?;
    paths.model()
}
fn current_scope(context: &Value) -> Result<ScopeRecord, String> {
    let repository: RepositoryWire = serde_json::from_value(context["repository"].clone())
        .map_err(|_| "Invalid accepted repository")?;
    let contexts = context["contexts"]
        .as_array()
        .ok_or("Invalid accepted contexts")?
        .iter()
        .map(|pin| {
            let repository: RepositoryWire = serde_json::from_value(pin["repository"].clone())
                .map_err(|_| "Invalid accepted context repository")?;
            Ok(ContextPin {
                name: pin["name"]
                    .as_str()
                    .ok_or("Invalid accepted context name")?
                    .into(),
                repository: repository.model()?,
                commit: SnapshotId::new(
                    pin["commit"]
                        .as_str()
                        .ok_or("Invalid accepted context commit")?
                        .into(),
                )?,
                paths: paths_from_context(pin)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ScopeRecord {
        version: ScopeVersion::new(
            context["scope_version"]
                .as_str()
                .ok_or("Invalid accepted scope version")?,
        )?,
        primary_repository: repository.model()?,
        primary_paths: paths_from_context(context)?,
        contexts,
        contracts: BTreeSet::new(),
        export_allowed: context["export_allowed"].as_bool().unwrap_or(false),
        model_destinations: BTreeSet::new(),
    })
}
fn scope_visible(scope: &ScopeRecord, active: &ScopeRecord) -> Result<(), String> {
    if scope.primary_repository != active.primary_repository
        || !crate::onboarding::scope_is_covered(&scope.primary_paths, &active.primary_paths)
        || scope.contexts.iter().any(|old| {
            !active.contexts.iter().any(|current| {
                old.name == current.name
                    && old.repository == current.repository
                    && old.commit == current.commit
                    && crate::onboarding::scope_is_covered(&old.paths, &current.paths)
            })
        })
    {
        return Err("Design unavailable in the current accepted context".into());
    }
    Ok(())
}
fn authorize_scope(
    action: Action,
    scope: &ScopeRecord,
    principal: &PrincipalId,
    grants: &[Grant],
) -> Result<(), String> {
    if action == Action::Export && !scope.export_allowed {
        return Err("Export is disabled by the immutable design scope".into());
    }
    for (repository, paths) in std::iter::once((&scope.primary_repository, &scope.primary_paths))
        .chain(
            scope
                .contexts
                .iter()
                .map(|pin| (&pin.repository, &pin.paths)),
        )
    {
        authorize_repository(action, principal, repository, grants, None, paths)?;
        if !grants.iter().any(|grant| {
            grant.principal == *principal
                && grant.repository == *repository
                && grant.actions.contains(&action)
                && crate::onboarding::scope_is_covered(paths, &grant.paths)
        }) {
            return Err("Design exceeds current path grants".into());
        }
    }
    Ok(())
}
fn fingerprint(review: &DesignReview) -> Result<String, String> {
    Ok(digest(&payload(&(
        DESIGN_REVIEW_VERSION,
        &review.design_id,
        review.interpretation,
        &review.corrections,
    ))?))
}

fn validate_corrections(
    corrections: &[DesignCorrection],
    design: &DesignVersion,
    scope: &ScopeRecord,
    references: &BTreeMap<String, DesignVersion>,
) -> Result<(), String> {
    if corrections.len() > MAX_CORRECTIONS {
        return Err("Design corrections exceed 200 entries".into());
    }
    if let Some(lld) = &design.lld {
        let mut ids = BTreeSet::new();
        for correction in corrections {
            label(&correction.id)?;
            if correction.target_id != lld.entry_id
                || !ids.insert(&correction.id)
                || correction.value.is_empty()
                || correction.value.len() > 2048
                || correction.value.chars().any(char::is_control)
                || !correction.paths.is_empty()
                || correction.constraint_kind.is_some()
                || correction.document_evidence_id.is_some()
            {
                return Err("LLD assertions must target this exact bounded detail".into());
            }
            match correction.kind {
                DesignCorrectionKind::LldInvariant
                    if correction.related_ids.is_empty()
                        && correction.reference_design_id.is_none()
                        && correction.correspondence_targets.is_empty() => {}
                DesignCorrectionKind::Correspondence => {
                    let source = references
                        .get(
                            correction
                                .reference_design_id
                                .as_ref()
                                .ok_or("LLD correspondence needs a pinned reference")?,
                        )
                        .and_then(|design| design.lld.as_ref())
                        .ok_or("LLD correspondence source is unavailable")?;
                    let from = std::iter::once(&source.detail.entry)
                        .chain(&source.detail.declarations)
                        .collect::<Vec<_>>();
                    let to = std::iter::once(&lld.detail.entry)
                        .chain(&lld.detail.declarations)
                        .collect::<Vec<_>>();
                    if !matches!(
                        correction.value.as_str(),
                        "rename" | "move" | "split" | "merge"
                    ) || correction.related_ids.is_empty()
                        || correction.related_ids.len() > 100
                        || correction.correspondence_targets.is_empty()
                        || correction.correspondence_targets.len() > 100
                        || correction.related_ids.iter().any(|id| {
                            !from
                                .iter()
                                .any(|entry| entry.id == *id && entry.kind == lld.detail.entry.kind)
                        })
                        || correction.correspondence_targets.iter().any(|id| {
                            !to.iter()
                                .any(|entry| entry.id == *id && entry.kind == lld.detail.entry.kind)
                        })
                    {
                        return Err("LLD correspondence must name scoped exact declarations of compatible kinds".into());
                    }
                    if matches!(correction.value.as_str(), "rename" | "move")
                        && (correction.related_ids.len() != 1
                            || correction.correspondence_targets.len() != 1)
                    {
                        return Err("LLD rename/move requires one source and one target".into());
                    }
                    if (correction.value == "split"
                        && (correction.related_ids.len() != 1
                            || correction.correspondence_targets.len() < 2))
                        || (correction.value == "merge"
                            && (correction.related_ids.len() < 2
                                || correction.correspondence_targets.len() != 1))
                    {
                        return Err("LLD split/merge cardinality is invalid".into());
                    }
                    if correction.related_ids.iter().collect::<BTreeSet<_>>().len()
                        != correction.related_ids.len()
                        || correction
                            .correspondence_targets
                            .iter()
                            .collect::<BTreeSet<_>>()
                            .len()
                            != correction.correspondence_targets.len()
                    {
                        return Err("Duplicate LLD correspondence identities".into());
                    }
                }
                _ => {
                    return Err(
                        "Only typed invariant/correspondence assertions apply to LLD".into(),
                    );
                }
            }
        }
        return Ok(());
    }
    if corrections
        .iter()
        .any(|correction| correction.kind == DesignCorrectionKind::LldInvariant)
    {
        return Err("LLD invariants cannot target HLD".into());
    }
    let elements = design
        .elements
        .iter()
        .map(|element| element.id.as_str())
        .collect::<BTreeSet<_>>();
    let relations = design
        .relations
        .iter()
        .map(|relation| relation.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut ids = BTreeSet::new();
    for correction in corrections {
        label(&correction.id)?;
        label(&correction.target_id)?;
        if !ids.insert(&correction.id)
            || correction.value.is_empty()
            || correction.value.len() > 2048
            || correction.value.chars().any(char::is_control)
            || correction.paths.len() > 100
            || correction.related_ids.len() > 100
            || correction.correspondence_targets.len() > 100
        {
            return Err("Invalid or unbounded design correction".into());
        }
        let target_known = if correction.kind == DesignCorrectionKind::MajorFlow {
            relations.contains(correction.target_id.as_str())
                || elements.contains(correction.target_id.as_str())
        } else {
            elements.contains(correction.target_id.as_str())
        };
        let related_elements = if correction.kind == DesignCorrectionKind::Correspondence {
            if let Some(reference) = &correction.reference_design_id {
                label(reference)?;
                references
                    .get(reference)
                    .ok_or("Correspondence reference design unavailable")?
                    .elements
                    .iter()
                    .map(|element| element.id.as_str())
                    .collect::<BTreeSet<_>>()
            } else {
                elements.clone()
            }
        } else {
            elements.clone()
        };
        if !target_known
            || correction
                .related_ids
                .iter()
                .any(|id| !related_elements.contains(id.as_str()))
        {
            return Err("Design correction references an unknown element or relation".into());
        }
        if correction.kind != DesignCorrectionKind::Correspondence
            && (correction.reference_design_id.is_some()
                || !correction.correspondence_targets.is_empty())
        {
            return Err(
                "Reference design and targets belong only to correspondence assertions".into(),
            );
        }
        if correction.kind == DesignCorrectionKind::DocumentReference {
            if !correction.paths.is_empty()
                || !correction.related_ids.is_empty()
                || correction.constraint_kind.is_some()
            {
                return Err(
                    "Document decisions contain only a scoped candidate and reviewer intent".into(),
                );
            }
            let evidence = correction
                .document_evidence_id
                .as_ref()
                .ok_or("Document decision requires an exact candidate evidence ID")?;
            if !design
                .document_candidates
                .iter()
                .any(|candidate| candidate.evidence_id == *evidence)
            {
                return Err(
                    "Document decision references no candidate in this exact design version".into(),
                );
            }
        } else if correction.document_evidence_id.is_some() {
            return Err("Document evidence belongs only to an explicit document decision".into());
        }
        if correction.kind != DesignCorrectionKind::Boundary && !correction.paths.is_empty() {
            return Err("Only boundary corrections may contain paths".into());
        }
        if correction.kind == DesignCorrectionKind::Boundary {
            if correction.paths.is_empty() {
                return Err("A boundary assertion requires existing authorized paths".into());
            }
            // Assertions may narrow a generated component, never introduce a
            // new repository/path. New boundaries require new onboarding.
            let element = design
                .elements
                .iter()
                .find(|item| item.id == correction.target_id)
                .ok_or("Unknown correction target")?;
            let roots = PathScope::new(element.roots.clone(), vec![])?;
            for path in &correction.paths {
                PathScope::new(vec![path.clone()], vec![])?;
                let known = element.roots.contains(path)
                    || design.evidence.iter().any(|evidence| {
                        element.evidence_ids.contains(&evidence.id)
                            && (evidence.path == *path
                                || evidence
                                    .path
                                    .strip_prefix(path)
                                    .is_some_and(|suffix| suffix.starts_with('/')))
                    });
                if !known
                    || !roots.allows(path)
                    || !scope.primary_paths.allows(path)
                    || crate::sensitive_path(path)
                {
                    return Err("Boundary correction exceeds confirmed component paths".into());
                }
            }
        }
        if correction.kind == DesignCorrectionKind::Constraint {
            match correction.constraint_kind {
                Some(DesignConstraintKind::ProhibitedDependency)
                    if correction.related_ids.len() == 1 => {}
                Some(DesignConstraintKind::RequiredContract)
                    if correction.related_ids.len() == 1 => {}
                _ => return Err("A typed constraint requires one known related element".into()),
            }
        } else if correction.constraint_kind.is_some() {
            return Err("Constraint kind belongs only to a constraint assertion".into());
        }
        if correction.kind == DesignCorrectionKind::MajorFlow && correction.related_ids.len() != 2 {
            return Err("A major-flow assertion requires two known endpoint elements".into());
        }
        if correction.kind == DesignCorrectionKind::MajorFlow && correction.value.len() > 512 {
            return Err("Major-flow assertion exceeds its 512-byte display budget".into());
        }
        let component = |id: &str| {
            design
                .elements
                .iter()
                .any(|element| element.id == id && element.kind == DesignElementKind::Component)
        };
        if matches!(
            correction.kind,
            DesignCorrectionKind::Responsibility
                | DesignCorrectionKind::Boundary
                | DesignCorrectionKind::Constraint
                | DesignCorrectionKind::DocumentReference
        ) && !component(&correction.target_id)
        {
            return Err("Component interpretation requires a component target".into());
        }
        if correction.kind == DesignCorrectionKind::MajorFlow
            && correction.related_ids.iter().any(|id| !component(id))
        {
            return Err("Major flow endpoints require confirmed component identities".into());
        }
        if matches!(
            correction.kind,
            DesignCorrectionKind::ContractOwnership | DesignCorrectionKind::DataOwnership
        ) && (correction.related_ids.len() != 1 || !component(&correction.related_ids[0]))
        {
            return Err("Ownership requires exactly one confirmed component identity".into());
        }
        if correction.kind == DesignCorrectionKind::Constraint {
            let related = design
                .elements
                .iter()
                .find(|element| element.id == correction.related_ids[0])
                .ok_or("Unknown constraint target")?;
            if match correction.constraint_kind {
                Some(DesignConstraintKind::ProhibitedDependency) => {
                    related.kind != DesignElementKind::Component
                }
                Some(DesignConstraintKind::RequiredContract) => {
                    related.kind != DesignElementKind::Contract
                }
                None => true,
            } {
                return Err("Constraint references the wrong architectural element kind".into());
            }
        }
        if correction.kind == DesignCorrectionKind::ContractOwnership
            && design
                .elements
                .iter()
                .find(|element| element.id == correction.target_id)
                .is_none_or(|element| element.kind != DesignElementKind::Contract)
        {
            return Err("Contract ownership requires a contract element".into());
        }
        if correction.kind == DesignCorrectionKind::DataOwnership
            && design
                .elements
                .iter()
                .find(|element| element.id == correction.target_id)
                .is_none_or(|element| element.kind != DesignElementKind::Schema)
        {
            return Err("Data ownership requires a schema element".into());
        }
        if matches!(
            correction.kind,
            DesignCorrectionKind::ContractOwnership
                | DesignCorrectionKind::DataOwnership
                | DesignCorrectionKind::Correspondence
        ) && correction.related_ids.is_empty()
        {
            return Err("Ownership or correspondence assertion requires related elements".into());
        }
        if correction.kind == DesignCorrectionKind::Correspondence
            && !matches!(
                correction.value.as_str(),
                "rename" | "move" | "split" | "merge"
            )
        {
            return Err("Correspondence must name rename, move, split or merge".into());
        }
        if correction.kind == DesignCorrectionKind::Correspondence {
            let targets = if correction.correspondence_targets.is_empty() {
                vec![correction.target_id.clone()]
            } else {
                correction.correspondence_targets.clone()
            };
            if !targets.contains(&correction.target_id)
                || targets.iter().any(|id| !elements.contains(id.as_str()))
                || targets.iter().collect::<BTreeSet<_>>().len() != targets.len()
                || correction.related_ids.iter().collect::<BTreeSet<_>>().len()
                    != correction.related_ids.len()
            {
                return Err("Correspondence identities must be known and unique".into());
            }
            if match correction.value.as_str() {
                "rename" | "move" => correction.related_ids.len() != 1 || targets.len() != 1,
                "split" => correction.related_ids.len() != 1 || targets.len() < 2,
                "merge" => correction.related_ids.len() < 2 || targets.len() != 1,
                _ => true,
            } {
                return Err(
                    "Correspondence cardinality does not match rename, move, split or merge".into(),
                );
            }
            let source = correction
                .reference_design_id
                .as_ref()
                .and_then(|id| references.get(id))
                .unwrap_or(design);
            let kind = design
                .elements
                .iter()
                .find(|element| element.id == correction.target_id)
                .ok_or("Unknown correspondence target")?
                .kind
                .clone();
            if targets.iter().any(|id| {
                design
                    .elements
                    .iter()
                    .find(|element| element.id == *id)
                    .is_none_or(|element| element.kind != kind)
            }) || correction.related_ids.iter().any(|id| {
                source
                    .elements
                    .iter()
                    .find(|element| element.id == *id)
                    .is_none_or(|element| element.kind != kind)
            }) {
                return Err("Correspondence must preserve architectural element kinds".into());
            }
        }
    }
    Ok(())
}

/// A display/diff interpretation. The persisted observed version is untouched.
pub fn interpret_design_review(
    observed: &DesignVersion,
    review: &DesignReview,
) -> Result<DesignVersion, String> {
    if review.design_id != observed.id || review.input_fingerprint != fingerprint(review)? {
        return Err("Design interpretation does not match its observed source".into());
    }
    let mut design = observed.clone();
    design.id = digest(
        format!(
            "{DESIGN_REVIEW_VERSION}:{}:{}",
            observed.id, review.input_fingerprint
        )
        .as_bytes(),
    );
    design.view = match review.interpretation {
        DesignInterpretation::AsBuilt => DesignView::AcceptedAsBuilt,
        DesignInterpretation::Intended => DesignView::Intended,
    };
    design.review_state = review.state.clone();
    for correction in &review.corrections {
        if design.lld.is_some() && correction.kind == DesignCorrectionKind::Correspondence {
            design.correspondence.push(DesignCorrespondence {
                kind: correction.value.clone(),
                from_ids: correction.related_ids.clone(),
                to_ids: correction.correspondence_targets.clone(),
                resolution: DesignResolution::UserAssertion,
            });
            continue;
        }
        if correction.kind == DesignCorrectionKind::LldInvariant {
            let lld = design
                .lld
                .as_mut()
                .ok_or("LLD invariant requires a local detail")?;
            if correction.target_id != lld.entry_id {
                return Err("Unknown LLD invariant target".into());
            }
            lld.invariants.push(crate::lld::LldInvariant {
                id: correction.id.clone(),
                entry_id: lld.entry_id.clone(),
                body_fingerprint: lld.behavior_fingerprint.clone(),
                statement: correction.value.clone(),
                resolution: "user_assertion".into(),
                verification: "requires_reviewer_verification".into(),
            });
            continue;
        }
        if correction.kind == DesignCorrectionKind::MajorFlow {
            if correction.related_ids.len() != 2 {
                return Err("Invalid major flow interpretation".into());
            }
            design.relations.push(DesignRelation {
                id: digest(format!("user_major_flow:{}:{}", design.id, correction.id).as_bytes()),
                from: correction.related_ids[0].clone(),
                to: correction.related_ids[1].clone(),
                kind: "user_major_flow".into(),
                direction: "user_asserted".into(),
                protocol: None,
                synchrony: "unknown".into(),
                resolution: DesignResolution::UserAssertion,
                evidence_ids: vec![],
                unknowns: vec![
                    format!("Reviewer flow assertion: {}", correction.value),
                    "Source observations remain separate; this assertion is not runtime evidence"
                        .into(),
                ],
            });
            continue;
        }
        let element = design
            .elements
            .iter_mut()
            .find(|element| element.id == correction.target_id)
            .ok_or("Unknown design interpretation target")?;
        if correction.kind != DesignCorrectionKind::DocumentReference {
            element.user_assertions.push(format!(
                "Reviewer {:?} assertion: {}",
                correction.kind, correction.value
            ));
        }
        match correction.kind {
            DesignCorrectionKind::Responsibility => element.responsibility = Some(correction.value.clone()),
            DesignCorrectionKind::Boundary => element.roots = correction.paths.clone(),
            DesignCorrectionKind::ContractOwnership | DesignCorrectionKind::DataOwnership => element.component_id = correction.related_ids.first().cloned(),
            DesignCorrectionKind::Constraint => design.constraints.push(DesignConstraint {
                id:correction.id.clone(),kind:match correction.constraint_kind.ok_or("Missing typed constraint")? { DesignConstraintKind::ProhibitedDependency=>"prohibited_dependency",DesignConstraintKind::RequiredContract=>"required_contract" }.into(),
                from_component_id:correction.target_id.clone(),to_component_id:correction.related_ids.first().cloned(),
                value:correction.value.clone(),user_assertion:"Explicit reviewer constraint; static evidence does not establish runtime enforcement".into(),evidence_ids:vec![],
            }),
            DesignCorrectionKind::Correspondence => design.correspondence.push(DesignCorrespondence {
                kind:correction.value.clone(),from_ids:correction.related_ids.clone(),to_ids:if correction.correspondence_targets.is_empty() { vec![correction.target_id.clone()] } else { correction.correspondence_targets.clone() },resolution:DesignResolution::UserAssertion,
            }),
            DesignCorrectionKind::DocumentReference => {
                let evidence_id = correction.document_evidence_id.as_ref().ok_or("Missing document selection")?;
                let candidate = observed.document_candidates.iter().find(|candidate|candidate.evidence_id == *evidence_id).ok_or("Document source unavailable")?;
                element.evidence_ids.push(evidence_id.clone());
                element.evidence_ids.sort();
                element.evidence_ids.dedup();
                design.decision_records.push(DesignDecisionRecord {
                    id:correction.id.clone(),component_id:correction.target_id.clone(),document_candidate_id:candidate.id.clone(),evidence_ids:vec![evidence_id.clone()],
                    intent:correction.value.clone(),resolution:DesignResolution::UserAssertion,verification:"requires_reviewer_verification".into(),
                });
            },
            DesignCorrectionKind::MajorFlow | DesignCorrectionKind::LldInvariant => unreachable!("handled separately"),
        }
    }
    if let Some(lld) = &design.lld {
        design.mermaid = lld.detail.mermaid.clone();
        design.markdown = lld.detail.markdown.clone();
        for invariant in &lld.invariants {
            design.markdown.push_str(&format!(
                "\n\nReviewer invariant (requires verification; body {}): {}",
                invariant.body_fingerprint, invariant.statement
            ));
        }
    } else {
        design.mermaid = crate::design::render_hld_mermaid(&design);
        design.markdown = crate::design::render_hld_markdown(&design);
    }
    Ok(design)
}

impl Storage {
    #[allow(clippy::too_many_arguments)]
    fn validate_review_corrections(
        &self,
        repository: &RepositoryId,
        corrections: &[DesignCorrection],
        design: &DesignVersion,
        scope: &ScopeRecord,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<(), String> {
        if corrections.len() > MAX_CORRECTIONS {
            return Err("Design corrections exceed 200 entries".into());
        }
        let mut references = BTreeMap::new();
        for correction in corrections {
            if let Some(reference) = &correction.reference_design_id {
                if correction.kind != DesignCorrectionKind::Correspondence {
                    return Err(
                        "Only correspondence assertions may reference another design".into(),
                    );
                }
                label(reference)?;
                if !references.contains_key(reference) {
                    if references.len() >= 2 {
                        return Err(
                            "A design preview may reference at most two pinned versions".into()
                        );
                    }
                    let source =
                        self.load_design_version(repository, reference, principal, grants)?;
                    references.insert(reference.clone(), source);
                }
            }
        }
        validate_corrections(corrections, design, scope, &references)
    }
    fn design_repository_id(&self, repository: &RepositoryId) -> Result<i64, String> {
        self.connection.query_row("SELECT id FROM repositories WHERE security_domain=?1 AND provider=?2 AND owner=?3 AND name=?4",
            params![repository.security_domain().as_str(),repository.provider(),repository.owner(),repository.name()], |row| row.get(0))
            .map_err(|_| "Design unavailable".into())
    }
    fn design_context(
        &self,
        repository: &RepositoryId,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<Value, String> {
        self.load_onboarding_defaults(repository, principal, grants)
            .map_err(|_| {
                "Design requires a currently authorized accepted onboarding context".into()
            })
    }
    fn design_envelope(
        &self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
        action: Action,
    ) -> Result<(DesignVersion, ScopeRecord, Value), String> {
        label(id)?;
        let context = self.design_context(repository, principal, grants)?;
        let active_scope = current_scope(&context)?;
        // Denied readers cannot probe whether a candidate exists. This local
        // profile requires authority over the explicit active context before
        // looking up any record, then checks the immutable historical scope.
        authorize_scope(action, &active_scope, principal, grants)?;
        authorize_scope(Action::Source, &active_scope, principal, grants)?;
        let repository_id = self.design_repository_id(repository)?;
        let (bytes,stored_fingerprint): (Vec<u8>,String) = self
            .connection
            .query_row(
                "SELECT payload,input_fingerprint FROM design_versions WHERE repository_id=?1 AND design_id=?2",
                params![repository_id, id],
                |row| Ok((row.get(0)?,row.get(1)?)),
            )
            .map_err(|_| "Design unavailable")?;
        if digest(&bytes) != stored_fingerprint {
            return Err("Stored design integrity mismatch".into());
        }
        let envelope: DesignEnvelope = decode(&bytes)?;
        let scope = envelope.scope.model()?;
        if envelope.version != "csb-design-storage-v1"
            || envelope.design.id != id
            || envelope.design.repository != *repository
        {
            return Err("Invalid stored design identity".into());
        }
        validate_design_scope(&envelope.design, &scope)?;
        scope_visible(&scope, &active_scope)?;
        if action == Action::Export && !active_scope.export_allowed {
            return Err("Export is disabled by the current accepted context".into());
        }
        authorize_scope(action, &scope, principal, grants)?;
        // Evidence snippets are source-sensitive even when embedded in a diagram.
        authorize_scope(Action::Source, &scope, principal, grants)?;
        let selected: String = self
            .connection
            .query_row(
                "SELECT context_id FROM onboarding_defaults WHERE repository_id=?1",
                params![repository_id],
                |row| row.get(0),
            )
            .map_err(|_| "Design context unavailable")?;
        if selected != context_id(&context)? {
            return Err("Stale design context; reload the current accepted context".into());
        }
        Ok((envelope.design, scope, context))
    }
    fn check_design_current(
        design: &DesignVersion,
        scope: &ScopeRecord,
        context: &Value,
    ) -> Result<(), String> {
        if context["session_id"] != design.onboarding_session_id
            || context["session_revision"] != design.onboarding_revision
            || context["input_fingerprint"] != design.onboarding_input_fingerprint
            || context["proposal_fingerprint"] != design.onboarding_proposal_fingerprint
            || context["accepted_scope_fingerprint"] != accepted_scope_fingerprint(scope)
            || scope.canonical_bytes() != current_scope(context)?.canonical_bytes()
        {
            return Err("Stale design context; generate and preview a new candidate".into());
        }
        Ok(())
    }

    /// Candidates are immutable; generating one neither accepts nor activates it.
    pub fn save_design_version(
        &mut self,
        design: &DesignVersion,
        scope: &ScopeRecord,
        principal: &PrincipalId,
        grants: &[Grant],
        at_ms: i64,
    ) -> Result<bool, String> {
        if at_ms < 0
            || design.view != DesignView::Observed
            || matches!(
                design.review_state,
                DesignReviewState::Accepted
                    | DesignReviewState::Superseded
                    | DesignReviewState::Stale
            )
        {
            return Err("Only generated observed design candidates may be stored".into());
        }
        label(&design.id)?;
        validate_design_scope(design, scope)?;
        authorize_scope(Action::Index, scope, principal, grants)?;
        let context = self.design_context(&design.repository, principal, grants)?;
        Self::check_design_current(design, scope, &context)?;
        let bytes = payload(&DesignEnvelope {
            version: "csb-design-storage-v1".into(),
            scope: ScopeWire::new(scope),
            design: design.clone(),
        })?;
        let repository = &design.repository;
        let repository_id = self.register_repository(
            repository.security_domain().as_str(),
            repository.provider(),
            repository.owner(),
            repository.name(),
        )?;
        // The shared registry has one immutable row per canonical boundary.
        // Review-derived scope labels may differ from the original scan label;
        // that explicit label remains in the design identity/envelope.
        let existing_scope: Option<i64> = self
            .connection
            .query_row(
                "SELECT id FROM scopes WHERE repository_id=?1 AND canonical=?2",
                params![repository_id, scope.canonical_bytes()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let scope_id = if let Some(id) = existing_scope {
            id
        } else {
            self.register_scope(
                repository_id,
                scope.version.as_str(),
                &scope.canonical_bytes(),
            )?
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let selected: String = transaction
            .query_row(
                "SELECT context_id FROM onboarding_defaults WHERE repository_id=?1",
                params![repository_id],
                |row| row.get(0),
            )
            .map_err(|_| "Design context unavailable")?;
        if selected != context_id(&context)? {
            return Err(
                "Stale design candidate; accepted context changed during publication".into(),
            );
        }
        let old: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT payload FROM design_versions WHERE repository_id=?1 AND design_id=?2",
                params![repository_id, design.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if old.as_ref().is_some_and(|old| old != &bytes) {
            return Err("Design identity already has different immutable facts".into());
        }
        if old.is_none() {
            transaction.execute("INSERT INTO design_versions(repository_id,design_id,scope_id,onboarding_context_id,input_fingerprint,payload,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![repository_id,design.id,scope_id,context_id(&context)?,digest(&bytes),bytes,at_ms]).map_err(|error|error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(old.is_none())
    }

    pub fn load_design_version(
        &self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignVersion, String> {
        self.design_envelope(repository, id, principal, grants, Action::Graph)
            .map(|record| record.0)
    }
    pub fn export_design_version(
        &self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignVersion, String> {
        self.design_envelope(repository, id, principal, grants, Action::Export)
            .map(|record| record.0)
    }

    // Keep the current authority and immutable input pins explicit at this
    // command boundary rather than hiding them in an unvalidated client DTO.
    #[allow(clippy::too_many_arguments)]
    pub fn create_design_review(
        &mut self,
        repository: &RepositoryId,
        design_id: &str,
        id: &str,
        interpretation: DesignInterpretation,
        principal: &PrincipalId,
        grants: &[Grant],
        at_ms: i64,
    ) -> Result<DesignReview, String> {
        label(id)?;
        let (design, scope, context) =
            self.design_envelope(repository, design_id, principal, grants, Action::Graph)?;
        authorize_scope(Action::ConfirmArchitecture, &scope, principal, grants)?;
        Self::check_design_current(&design, &scope, &context)?;
        let mut review = DesignReview {
            version: DESIGN_REVIEW_VERSION.into(),
            id: id.into(),
            design_id: design_id.into(),
            principal: principal.as_str().into(),
            revision: 0,
            interpretation,
            corrections: vec![],
            state: DesignReviewState::Draft,
            input_fingerprint: String::new(),
            confirmation: None,
        };
        review.input_fingerprint = fingerprint(&review)?;
        self.write_design_review(repository, &review, None, at_ms, false)?;
        Ok(review)
    }
    pub fn load_design_review(
        &self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignReview, String> {
        label(id)?;
        // Access is tested before even exposing the existence of a private draft.
        let context = self.design_context(repository, principal, grants)?;
        let active_scope = current_scope(&context)?;
        authorize_scope(Action::Graph, &active_scope, principal, grants)?;
        authorize_scope(Action::Source, &active_scope, principal, grants)?;
        let repository_id = self.design_repository_id(repository)?;
        let (revision, bytes): (i64,Vec<u8>) = self.connection.query_row("SELECT revision,payload FROM design_reviews WHERE repository_id=?1 AND review_id=?2 AND principal=?3",params![repository_id,id,principal.as_str()],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|_|"Design review unavailable")?;
        let review: DesignReview = decode(&bytes)?;
        if review.version != DESIGN_REVIEW_VERSION
            || review.id != id
            || review.principal != principal.as_str()
            || review.revision != revision as u64
            || review.input_fingerprint != fingerprint(&review)?
        {
            return Err("Invalid stored design review".into());
        }
        let (design, scope, _) = self.design_envelope(
            repository,
            &review.design_id,
            principal,
            grants,
            Action::Graph,
        )?;
        self.validate_review_corrections(
            repository,
            &review.corrections,
            &design,
            &scope,
            principal,
            grants,
        )?;
        Ok(review)
    }
    pub fn preview_design_interpretation(
        &self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignVersion, String> {
        let review = self.load_design_review(repository, id, principal, grants)?;
        let (observed, scope, context) = self.design_envelope(
            repository,
            &review.design_id,
            principal,
            grants,
            Action::Graph,
        )?;
        Self::check_design_current(&observed, &scope, &context)?;
        let interpreted = interpret_design_review(&observed, &review)?;
        validate_design_scope(&interpreted, &scope)?;
        Ok(interpreted)
    }
    /// Every correction is previewed as user assertion while generated facts stay unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn preview_design_review(
        &mut self,
        repository: &RepositoryId,
        id: &str,
        expected_revision: u64,
        interpretation: DesignInterpretation,
        corrections: Vec<DesignCorrection>,
        principal: &PrincipalId,
        grants: &[Grant],
        at_ms: i64,
    ) -> Result<DesignReview, String> {
        let mut review = self.load_design_review(repository, id, principal, grants)?;
        if review.revision != expected_revision || review.confirmation.is_some() {
            return Err("Stale or already accepted design review; create a fresh review".into());
        }
        let (design, scope, context) = self.design_envelope(
            repository,
            &review.design_id,
            principal,
            grants,
            Action::Graph,
        )?;
        authorize_scope(Action::ConfirmArchitecture, &scope, principal, grants)?;
        Self::check_design_current(&design, &scope, &context)?;
        self.validate_review_corrections(
            repository,
            &corrections,
            &design,
            &scope,
            principal,
            grants,
        )?;
        review.revision = review
            .revision
            .checked_add(1)
            .ok_or("Design revision overflow")?;
        review.interpretation = interpretation;
        review.corrections = corrections;
        review.state = DesignReviewState::AwaitingConfirmation;
        review.input_fingerprint = fingerprint(&review)?;
        let interpreted = interpret_design_review(&design, &review)?;
        validate_design_scope(&interpreted, &scope)?;
        payload(&interpreted)?;
        self.write_design_review(repository, &review, Some(expected_revision), at_ms, false)?;
        Ok(review)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn confirm_design_review(
        &mut self,
        repository: &RepositoryId,
        id: &str,
        expected_revision: u64,
        expected_fingerprint: &str,
        known_gaps_accepted: bool,
        activate_default: bool,
        principal: &PrincipalId,
        grants: &[Grant],
        at_ms: i64,
    ) -> Result<DesignAccepted, String> {
        let mut review = self.load_design_review(repository, id, principal, grants)?;
        let (design, scope, context) = self.design_envelope(
            repository,
            &review.design_id,
            principal,
            grants,
            Action::Graph,
        )?;
        authorize_scope(Action::ConfirmArchitecture, &scope, principal, grants)?;
        Self::check_design_current(&design, &scope, &context)?;
        if at_ms < 0
            || review.revision != expected_revision
            || review.input_fingerprint != expected_fingerprint
            || review.state != DesignReviewState::AwaitingConfirmation
            || review.confirmation.is_some()
        {
            return Err(
                "Stale design confirmation; reload and preview the latest corrections".into(),
            );
        }
        if (!design.coverage.complete || !design.coverage.gaps.is_empty()) && !known_gaps_accepted {
            return Err(
                "Incomplete design requires explicit acknowledgement of coverage gaps".into(),
            );
        }
        review.confirmation = Some(DesignDecision {
            actor: principal.as_str().into(),
            accepted_at_ms: at_ms,
            preview_revision: review.revision,
            input_fingerprint: review.input_fingerprint.clone(),
            known_gaps_accepted,
        });
        review.revision = review
            .revision
            .checked_add(1)
            .ok_or("Design revision overflow")?;
        review.state = DesignReviewState::Accepted;
        let interpreted = interpret_design_review(&design, &review)?;
        validate_design_scope(&interpreted, &scope)?;
        payload(&interpreted)?;
        self.write_design_review(
            repository,
            &review,
            Some(expected_revision),
            at_ms,
            activate_default,
        )?;
        self.load_design_acceptance(repository, &accepted_id(&review), principal, grants)
    }

    fn write_design_review(
        &mut self,
        repository: &RepositoryId,
        review: &DesignReview,
        expected_revision: Option<u64>,
        at_ms: i64,
        activate: bool,
    ) -> Result<(), String> {
        if at_ms < 0
            || review.revision > i64::MAX as u64
            || review.input_fingerprint != fingerprint(review)?
        {
            return Err("Invalid design review revision, timestamp or fingerprint".into());
        }
        let bytes = payload(review)?;
        let repository_id = self.design_repository_id(repository)?;
        let stored: Vec<u8> = self
            .connection
            .query_row(
                "SELECT payload FROM design_versions WHERE repository_id=?1 AND design_id=?2",
                params![repository_id, review.design_id],
                |row| row.get(0),
            )
            .map_err(|_| "Design candidate unavailable")?;
        let envelope: DesignEnvelope = decode(&stored)?;
        let lld_key = envelope
            .design
            .lld
            .as_ref()
            .map(|data| data.logical_key.clone());
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let (bound,selected): (String,String) = transaction.query_row("SELECT candidate.onboarding_context_id,active.context_id FROM design_versions candidate JOIN onboarding_defaults active ON active.repository_id=candidate.repository_id WHERE candidate.repository_id=?1 AND candidate.design_id=?2",params![repository_id,review.design_id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|_|"Design context unavailable")?;
        if bound != selected {
            return Err("Stale design context; generate and preview a new candidate".into());
        }
        let current: Option<(i64,Vec<u8>)> = transaction.query_row("SELECT revision,payload FROM design_reviews WHERE repository_id=?1 AND review_id=?2",params![repository_id,review.id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|error|error.to_string())?;
        match current {
            None if expected_revision.is_none()
                && review.revision == 0
                && review.state == DesignReviewState::Draft =>
            {
                transaction.execute("INSERT INTO design_reviews(repository_id,review_id,design_id,principal,revision,payload) VALUES(?1,?2,?3,?4,0,?5)",params![repository_id,review.id,review.design_id,review.principal,bytes]).map_err(|error|error.to_string())?;
            }
            Some((revision, old))
                if expected_revision == Some(revision as u64)
                    && review.revision == revision as u64 + 1 =>
            {
                let old: DesignReview = decode(&old)?;
                if old.principal != review.principal
                    || old.design_id != review.design_id
                    || old.confirmation.is_some()
                {
                    return Err("Accepted design reviews are immutable".into());
                }
                transaction.execute("UPDATE design_reviews SET revision=?3,payload=?4 WHERE repository_id=?1 AND review_id=?2",params![repository_id,review.id,review.revision as i64,bytes]).map_err(|error|error.to_string())?;
            }
            _ => return Err("Stale design review save; reload the latest review".into()),
        }
        transaction.execute("INSERT INTO design_review_history(repository_id,review_id,revision,actor,recorded_at_ms,payload) VALUES(?1,?2,?3,?4,?5,?6)",params![repository_id,review.id,review.revision as i64,review.principal,at_ms,bytes]).map_err(|error|error.to_string())?;
        if let Some(receipt) = &review.confirmation {
            if receipt.actor != review.principal
                || receipt.accepted_at_ms != at_ms
                || receipt.preview_revision + 1 != review.revision
                || receipt.input_fingerprint != review.input_fingerprint
                || review.state != DesignReviewState::Accepted
            {
                return Err("Invalid design acceptance receipt".into());
            }
            let id = accepted_id(review);
            transaction.execute("INSERT INTO design_accepted(repository_id,accepted_id,design_id,review_id,review_revision,interpretation,actor,accepted_at_ms,payload) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![repository_id,id,review.design_id,review.id,review.revision as i64,review.interpretation.key(),review.principal,at_ms,bytes]).map_err(|error|error.to_string())?;
            if activate {
                if let Some(key) = &lld_key {
                    transaction.execute("INSERT INTO lld_design_defaults(repository_id,logical_key,interpretation,accepted_id,selected_by,selected_at_ms) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(repository_id,logical_key,interpretation) DO UPDATE SET accepted_id=excluded.accepted_id,selected_by=excluded.selected_by,selected_at_ms=excluded.selected_at_ms",params![repository_id,key,review.interpretation.key(),id,review.principal,at_ms]).map_err(|error|error.to_string())?;
                } else {
                    transaction.execute("INSERT INTO design_defaults(repository_id,interpretation,accepted_id,selected_by,selected_at_ms) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(repository_id,interpretation) DO UPDATE SET accepted_id=excluded.accepted_id,selected_by=excluded.selected_by,selected_at_ms=excluded.selected_at_ms",params![repository_id,review.interpretation.key(),id,review.principal,at_ms]).map_err(|error|error.to_string())?;
                }
            }
        } else if activate {
            return Err("An unconfirmed design cannot become a default".into());
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn load_design_acceptance(
        &self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignAccepted, String> {
        label(id)?;
        let context = self.design_context(repository, principal, grants)?;
        let active_scope = current_scope(&context)?;
        authorize_scope(Action::Graph, &active_scope, principal, grants)?;
        authorize_scope(Action::Source, &active_scope, principal, grants)?;
        let repository_id = self.design_repository_id(repository)?;
        let (bytes,design_id,interpretation,review_id,revision,actor,accepted_at): (Vec<u8>,String,String,String,i64,String,i64) = self.connection.query_row("SELECT payload,design_id,interpretation,review_id,review_revision,actor,accepted_at_ms FROM design_accepted WHERE repository_id=?1 AND accepted_id=?2",params![repository_id,id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?))).map_err(|_|"Accepted design unavailable")?;
        let review: DesignReview = decode(&bytes)?;
        let (design, scope, context) =
            self.design_envelope(repository, &design_id, principal, grants, Action::Graph)?;
        self.validate_review_corrections(
            repository,
            &review.corrections,
            &design,
            &scope,
            principal,
            grants,
        )?;
        let receipt = review
            .confirmation
            .as_ref()
            .ok_or("Invalid stored acceptance")?;
        if review.version != DESIGN_REVIEW_VERSION
            || review.state != DesignReviewState::Accepted
            || review.design_id != design_id
            || review.interpretation.key() != interpretation
            || accepted_id(&review) != id
            || review.input_fingerprint != fingerprint(&review)?
            || receipt.input_fingerprint != review.input_fingerprint
            || receipt.actor != review.principal
            || receipt.preview_revision.checked_add(1) != Some(review.revision)
            || review.id != review_id
            || review.revision != revision as u64
            || receipt.actor != actor
            || receipt.accepted_at_ms != accepted_at
            || accepted_at < 0
        {
            return Err("Invalid stored design acceptance".into());
        }
        let history: Vec<u8> = self.connection.query_row("SELECT payload FROM design_review_history WHERE repository_id=?1 AND review_id=?2 AND revision=?3",params![repository_id,review_id,revision],|row|row.get(0)).map_err(|_|"Accepted design history unavailable")?;
        if history != bytes {
            return Err("Accepted design history integrity mismatch".into());
        }
        let active: Option<String> = if let Some(lld) = &design.lld {
            self.connection.query_row("SELECT accepted_id FROM lld_design_defaults WHERE repository_id=?1 AND logical_key=?2 AND interpretation=?3",params![repository_id,lld.logical_key,interpretation],|row|row.get(0)).optional().map_err(|error|error.to_string())?
        } else {
            self.connection.query_row("SELECT accepted_id FROM design_defaults WHERE repository_id=?1 AND interpretation=?2",params![repository_id,interpretation],|row|row.get(0)).optional().map_err(|error|error.to_string())?
        };
        let active_default = active.as_deref() == Some(id);
        let lifecycle_state = if Self::check_design_current(&design, &scope, &context).is_err() {
            DesignReviewState::Stale
        } else if !active_default && active.is_some() {
            DesignReviewState::Superseded
        } else {
            DesignReviewState::Accepted
        };
        let interpreted = interpret_design_review(&design, &review)?;
        validate_design_scope(&interpreted, &scope)?;
        Ok(DesignAccepted {
            accepted_id: id.into(),
            design: interpreted,
            observed_design: design,
            review,
            lifecycle_state,
            active_default,
        })
    }
    pub fn load_design_default(
        &self,
        repository: &RepositoryId,
        interpretation: DesignInterpretation,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignAccepted, String> {
        let context = self.design_context(repository, principal, grants)?;
        let active_scope = current_scope(&context)?;
        authorize_scope(Action::Graph, &active_scope, principal, grants)?;
        authorize_scope(Action::Source, &active_scope, principal, grants)?;
        let repository_id = self.design_repository_id(repository)?;
        let id: String = self.connection.query_row("SELECT accepted_id FROM design_defaults WHERE repository_id=?1 AND interpretation=?2",params![repository_id,interpretation.key()],|row|row.get(0)).map_err(|_|"Accepted design default unavailable")?;
        self.load_design_acceptance(repository, &id, principal, grants)
    }
    /// Independent default per qualified local detail and interpretation.
    pub fn load_lld_design_default(
        &self,
        repository: &RepositoryId,
        logical_key: &str,
        interpretation: DesignInterpretation,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignAccepted, String> {
        if logical_key.is_empty()
            || logical_key.len() > 4096
            || logical_key.chars().any(char::is_control)
        {
            return Err("Invalid LLD default identity".into());
        }
        let context = self.design_context(repository, principal, grants)?;
        let active_scope = current_scope(&context)?;
        authorize_scope(Action::Graph, &active_scope, principal, grants)?;
        authorize_scope(Action::Source, &active_scope, principal, grants)?;
        let repository_id = self.design_repository_id(repository)?;
        let id: String = self.connection.query_row("SELECT accepted_id FROM lld_design_defaults WHERE repository_id=?1 AND logical_key=?2 AND interpretation=?3",params![repository_id,logical_key,interpretation.key()],|row|row.get(0)).map_err(|_|"Accepted LLD default unavailable")?;
        let accepted = self.load_design_acceptance(repository, &id, principal, grants)?;
        if accepted
            .observed_design
            .lld
            .as_ref()
            .is_none_or(|data| data.logical_key != logical_key)
        {
            return Err("Invalid LLD default binding".into());
        }
        Ok(accepted)
    }
    pub fn export_design_acceptance(
        &self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
    ) -> Result<DesignAccepted, String> {
        let accepted = self.load_design_acceptance(repository, id, principal, grants)?;
        self.design_envelope(
            repository,
            &accepted.observed_design.id,
            principal,
            grants,
            Action::Export,
        )?;
        Ok(accepted)
    }
    /// Selecting a previously accepted current interpretation is an explicit
    /// maintainer action. It does not alter the acceptance receipt or history.
    pub fn activate_design_acceptance(
        &mut self,
        repository: &RepositoryId,
        id: &str,
        principal: &PrincipalId,
        grants: &[Grant],
        at_ms: i64,
    ) -> Result<DesignAccepted, String> {
        if at_ms < 0 {
            return Err("Invalid design selection timestamp".into());
        }
        let accepted = self.load_design_acceptance(repository, id, principal, grants)?;
        let (design, scope, context) = self.design_envelope(
            repository,
            &accepted.observed_design.id,
            principal,
            grants,
            Action::Graph,
        )?;
        authorize_scope(Action::ConfirmArchitecture, &scope, principal, grants)?;
        Self::check_design_current(&design, &scope, &context)?;
        let repository_id = self.design_repository_id(repository)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let selected: String = transaction
            .query_row(
                "SELECT context_id FROM onboarding_defaults WHERE repository_id=?1",
                params![repository_id],
                |row| row.get(0),
            )
            .map_err(|_| "Design context unavailable")?;
        if selected != context_id(&context)? {
            return Err("Stale design selection; accepted context changed".into());
        }
        if let Some(lld) = &design.lld {
            transaction.execute("INSERT INTO lld_design_defaults(repository_id,logical_key,interpretation,accepted_id,selected_by,selected_at_ms) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(repository_id,logical_key,interpretation) DO UPDATE SET accepted_id=excluded.accepted_id,selected_by=excluded.selected_by,selected_at_ms=excluded.selected_at_ms",params![repository_id,lld.logical_key,accepted.review.interpretation.key(),id,principal.as_str(),at_ms]).map_err(|error|error.to_string())?;
        } else {
            transaction.execute("INSERT INTO design_defaults(repository_id,interpretation,accepted_id,selected_by,selected_at_ms) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(repository_id,interpretation) DO UPDATE SET accepted_id=excluded.accepted_id,selected_by=excluded.selected_by,selected_at_ms=excluded.selected_at_ms",params![repository_id,accepted.review.interpretation.key(),id,principal.as_str(),at_ms]).map_err(|error|error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        self.load_design_acceptance(repository, id, principal, grants)
    }
}

fn accepted_id(review: &DesignReview) -> String {
    digest(
        format!(
            "{DESIGN_REVIEW_VERSION}:{}:{}:{}:{}",
            review.design_id, review.id, review.revision, review.input_fingerprint
        )
        .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{
        DesignCoverage, DesignElement, DesignEvidence, DesignFact, DesignResolution, DesignSection,
        HLD_GENERATOR_VERSION, design_scope_fingerprint,
    };
    use crate::{
        ComponentDecision, ComponentProposal, EvidenceConfidence, OnboardingEvidence,
        OnboardingSession, OnboardingState,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: std::path::PathBuf,
        storage: Storage,
        repository: RepositoryId,
        principal: PrincipalId,
        grants: Vec<Grant>,
        scope: ScopeRecord,
        design: DesignVersion,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn confirm_context(
        storage: &mut Storage,
        principal: &PrincipalId,
        grants: &[Grant],
        scope: ScopeRecord,
        id: &str,
    ) -> (ScopeRecord, Value) {
        let repository = scope.primary_repository.clone();
        let root = scope.primary_paths.includes().next().unwrap().to_owned();
        let evidence_path = format!("{root}/lib.rs");
        let mut session = OnboardingSession::new(
            id.into(),
            principal.clone(),
            SnapshotId::new("a".repeat(40)).unwrap(),
            scope,
            "scan-1".into(),
            grants,
        )
        .unwrap();
        if let Some(version) = storage
            .onboarding_scope_version(&session, principal, grants)
            .unwrap()
        {
            let mut canonical = session.scope().clone();
            canonical.version = version;
            session = OnboardingSession::new(
                id.into(),
                principal.clone(),
                SnapshotId::new("a".repeat(40)).unwrap(),
                canonical,
                "scan-1".into(),
                grants,
            )
            .unwrap();
        }
        storage
            .save_onboarding_session(&session, None, 0, principal, grants)
            .unwrap();
        session
            .transition(0, OnboardingState::Scanning, principal, grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(0), 1, principal, grants)
            .unwrap();
        let mut component = ComponentProposal {
            id: "engine".into(),
            name: "Scoped engine".into(),
            role: "library".into(),
            repository_index: 0,
            roots: vec![root],
            stack_candidates: vec![],
            signals: vec![],
            evidence: vec![OnboardingEvidence {
                repository_index: 0,
                path: evidence_path,
                blob: "b".repeat(40),
                start_line: 1,
                end_line: 4,
                producer_version: "fixture-v1".into(),
            }],
            confidence: EvidenceConfidence::Candidate,
            contradictions: vec![],
            decision: ComponentDecision::Pending,
        };
        session
            .set_proposal(
                1,
                vec![component.clone()],
                vec![],
                vec![],
                principal,
                grants,
            )
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(1), 2, principal, grants)
            .unwrap();
        session
            .transition(2, OnboardingState::AwaitingConfirmation, principal, grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(2), 3, principal, grants)
            .unwrap();
        component.decision = ComponentDecision::Confirmed;
        session
            .decide_component(3, component, principal, grants)
            .unwrap();
        storage
            .save_onboarding_session(&session, Some(3), 4, principal, grants)
            .unwrap();
        session
            .confirm(
                4,
                &session.proposal_fingerprint(),
                true,
                5,
                principal,
                grants,
            )
            .unwrap();
        storage
            .save_onboarding_session_with_defaults(&session, Some(4), 5, principal, grants, true)
            .unwrap();
        (
            session.accepted_scope(principal, grants).unwrap(),
            storage
                .load_onboarding_defaults(&repository, principal, grants)
                .unwrap(),
        )
    }
    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "csb-hld-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut storage =
            Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
        let repository = RepositoryId::new(
            SecurityDomain::new("test-a").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let principal = PrincipalId::new("maintainer").unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("source-boundary").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::new(vec!["src".into()], vec!["src/private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let grants = vec![Grant {
            principal: principal.clone(),
            repository: repository.clone(),
            actions: BTreeSet::from([
                Action::Index,
                Action::Graph,
                Action::Source,
                Action::ConfirmArchitecture,
                Action::Export,
            ]),
            paths: scope.primary_paths.clone(),
        }];
        let (mut scope, context) =
            confirm_context(&mut storage, &principal, &grants, scope, "onboarding-hld");
        // Review interpretation scope IDs differ from original scan IDs. Only
        // normalized boundaries and exact receipts govern acceptance.
        scope.version = ScopeVersion::new("derived-review-boundary").unwrap();
        let evidence = DesignEvidence {
            id: "evidence-1".into(),
            repository: repository.clone(),
            commit: SnapshotId::new("a".repeat(40)).unwrap(),
            path: "src/lib.rs".into(),
            blob: "b".repeat(40),
            start_line: 1,
            end_line: 4,
            producer_version: "fixture-v1".into(),
            relationship_type: "declaration".into(),
            resolution: DesignResolution::ObservedStructure,
        };
        let design = DesignVersion {
            level: crate::design::DesignLevel::Hld,
            lld: None,
            id: "candidate-1".into(),
            generator_version: HLD_GENERATOR_VERSION.into(),
            repository: repository.clone(),
            snapshot: SnapshotId::new("a".repeat(40)).unwrap(),
            scope_version: scope.version.clone(),
            scope_fingerprint: design_scope_fingerprint(&scope),
            onboarding_session_id: context["session_id"].as_str().unwrap().into(),
            onboarding_revision: context["session_revision"].as_u64().unwrap(),
            onboarding_input_fingerprint: context["input_fingerprint"].as_str().unwrap().into(),
            onboarding_proposal_fingerprint: context["proposal_fingerprint"]
                .as_str()
                .unwrap()
                .into(),
            review_state: DesignReviewState::Partial,
            view: DesignView::Observed,
            elements: vec![DesignElement {
                id: "component-1".into(),
                kind: DesignElementKind::Component,
                name: "Scoped engine".into(),
                component_id: None,
                roots: vec!["src".into()],
                responsibility: None,
                user_assertions: vec![],
                facts: vec![DesignFact {
                    kind: "declaration".into(),
                    value: "pub struct Engine".into(),
                    resolution: DesignResolution::ObservedStructure,
                    evidence_ids: vec!["evidence-1".into()],
                }],
                evidence_ids: vec!["evidence-1".into()],
                unknowns: vec!["Runtime direction unknown".into()],
            }],
            relations: vec![],
            evidence: vec![evidence],
            sections: [
                "identity",
                "system_context",
                "components",
                "interactions",
                "data_ownership",
                "deployment",
                "constraints",
                "major_flows",
                "review_state",
            ]
            .into_iter()
            .map(|id| DesignSection {
                id: id.into(),
                title: id.into(),
                element_ids: vec![],
                relation_ids: vec![],
                unknowns: vec![],
            })
            .collect(),
            coverage: DesignCoverage {
                profile: "fixture".into(),
                enumeration_complete: true,
                indexed: 1,
                unsupported: 0,
                failed: 0,
                deferred: 0,
                complete: false,
                gaps: vec!["Runtime behavior unverified".into()],
            },
            constraints: vec![],
            correspondence: vec![],
            document_candidates: vec![],
            decision_records: vec![],
            mermaid: "flowchart LR".into(),
            markdown: "Scoped source facts".into(),
        };
        storage
            .save_design_version(&design, &scope, &principal, &grants, 6)
            .unwrap();
        Fixture {
            root,
            storage,
            repository,
            principal,
            grants,
            scope,
            design,
        }
    }
    fn responsibility() -> DesignCorrection {
        DesignCorrection {
            id: "responsibility-1".into(),
            kind: DesignCorrectionKind::Responsibility,
            target_id: "component-1".into(),
            value: "Owns review scheduling".into(),
            paths: vec![],
            related_ids: vec![],
            constraint_kind: None,
            reference_design_id: None,
            correspondence_targets: vec![],
            document_evidence_id: None,
        }
    }
    fn preview(
        fixture: &mut Fixture,
        id: &str,
        interpretation: DesignInterpretation,
    ) -> DesignReview {
        fixture
            .storage
            .create_design_review(
                &fixture.repository,
                &fixture.design.id,
                id,
                interpretation,
                &fixture.principal,
                &fixture.grants,
                7,
            )
            .unwrap();
        fixture
            .storage
            .preview_design_review(
                &fixture.repository,
                id,
                0,
                interpretation,
                vec![responsibility()],
                &fixture.principal,
                &fixture.grants,
                8,
            )
            .unwrap()
    }
    fn confirm(fixture: &mut Fixture, preview: &DesignReview, activate: bool) -> DesignAccepted {
        fixture
            .storage
            .confirm_design_review(
                &fixture.repository,
                &preview.id,
                preview.revision,
                &preview.input_fingerprint,
                true,
                activate,
                &fixture.principal,
                &fixture.grants,
                9,
            )
            .unwrap()
    }

    #[test]
    fn design_candidates_are_immutable_idempotent_and_never_autoaccepted() {
        let mut fixture = fixture();
        assert!(
            !fixture
                .storage
                .save_design_version(
                    &fixture.design,
                    &fixture.scope,
                    &fixture.principal,
                    &fixture.grants,
                    7
                )
                .unwrap()
        );
        let mut changed = fixture.design.clone();
        changed.elements[0].facts[0].value = "different immutable source fact".into();
        assert!(
            fixture
                .storage
                .save_design_version(
                    &changed,
                    &fixture.scope,
                    &fixture.principal,
                    &fixture.grants,
                    7
                )
                .is_err()
        );
        assert_eq!(
            fixture
                .storage
                .load_design_version(
                    &fixture.repository,
                    &fixture.design.id,
                    &fixture.principal,
                    &fixture.grants
                )
                .unwrap(),
            fixture.design
        );
        assert!(
            fixture
                .storage
                .load_design_default(
                    &fixture.repository,
                    DesignInterpretation::AsBuilt,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_err()
        );
    }

    #[test]
    fn preview_and_accept_keep_observations_separate_and_intended_defaults_distinct() {
        let mut fixture = fixture();
        let review = preview(
            &mut fixture,
            "intended-review",
            DesignInterpretation::Intended,
        );
        let display = fixture
            .storage
            .preview_design_interpretation(
                &fixture.repository,
                &review.id,
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap();
        assert_eq!(
            display.elements[0].responsibility.as_deref(),
            Some("Owns review scheduling")
        );
        assert_eq!(display.elements[0].facts, fixture.design.elements[0].facts);
        assert_eq!(display.evidence, fixture.design.evidence);
        assert_eq!(display.view, DesignView::Intended);
        assert!(
            display.elements[0].user_assertions[0]
                .starts_with("Reviewer Responsibility assertion:")
        );
        let intended = confirm(&mut fixture, &review, true);
        assert_eq!(intended.observed_design, fixture.design);
        assert!(
            fixture
                .storage
                .load_design_default(
                    &fixture.repository,
                    DesignInterpretation::AsBuilt,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_err()
        );
        assert_eq!(
            fixture
                .storage
                .load_design_default(
                    &fixture.repository,
                    DesignInterpretation::Intended,
                    &fixture.principal,
                    &fixture.grants
                )
                .unwrap()
                .accepted_id,
            intended.accepted_id
        );
        let review = preview(
            &mut fixture,
            "as-built-review",
            DesignInterpretation::AsBuilt,
        );
        let old = confirm(&mut fixture, &review, true);
        let review = preview(
            &mut fixture,
            "new-as-built-review",
            DesignInterpretation::AsBuilt,
        );
        let new = confirm(&mut fixture, &review, true);
        assert_ne!(old.accepted_id, new.accepted_id);
        let historical = fixture
            .storage
            .load_design_acceptance(
                &fixture.repository,
                &old.accepted_id,
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap();
        assert_eq!(historical.lifecycle_state, DesignReviewState::Superseded);
        assert_eq!(historical.review, old.review);
        assert_eq!(historical.observed_design, old.observed_design);
        assert_eq!(
            fixture
                .storage
                .load_design_default(
                    &fixture.repository,
                    DesignInterpretation::Intended,
                    &fixture.principal,
                    &fixture.grants
                )
                .unwrap()
                .accepted_id,
            intended.accepted_id
        );
    }

    #[test]
    fn acceptance_requires_latest_explicit_preview_gaps_and_atomic_publication() {
        let mut fixture = fixture();
        fixture
            .storage
            .create_design_review(
                &fixture.repository,
                &fixture.design.id,
                "review",
                DesignInterpretation::AsBuilt,
                &fixture.principal,
                &fixture.grants,
                7,
            )
            .unwrap();
        let draft = fixture
            .storage
            .load_design_review(
                &fixture.repository,
                "review",
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap();
        assert!(
            fixture
                .storage
                .confirm_design_review(
                    &fixture.repository,
                    "review",
                    0,
                    &draft.input_fingerprint,
                    true,
                    true,
                    &fixture.principal,
                    &fixture.grants,
                    8
                )
                .is_err()
        );
        let review = fixture
            .storage
            .preview_design_review(
                &fixture.repository,
                "review",
                0,
                DesignInterpretation::AsBuilt,
                vec![responsibility()],
                &fixture.principal,
                &fixture.grants,
                8,
            )
            .unwrap();
        for (revision, fingerprint, gaps) in [
            (0, review.input_fingerprint.as_str(), true),
            (1, "forged", true),
            (1, review.input_fingerprint.as_str(), false),
        ] {
            assert!(
                fixture
                    .storage
                    .confirm_design_review(
                        &fixture.repository,
                        "review",
                        revision,
                        fingerprint,
                        gaps,
                        true,
                        &fixture.principal,
                        &fixture.grants,
                        9
                    )
                    .is_err()
            );
        }
        fixture.storage.connection.execute_batch("CREATE TRIGGER reject_design_default BEFORE INSERT ON design_defaults BEGIN SELECT RAISE(ABORT,'injected default failure'); END;").unwrap();
        assert!(
            fixture
                .storage
                .confirm_design_review(
                    &fixture.repository,
                    "review",
                    1,
                    &review.input_fingerprint,
                    true,
                    true,
                    &fixture.principal,
                    &fixture.grants,
                    9
                )
                .is_err()
        );
        assert_eq!(
            fixture
                .storage
                .load_design_review(
                    &fixture.repository,
                    "review",
                    &fixture.principal,
                    &fixture.grants
                )
                .unwrap(),
            review
        );
        let accepted: i64 = fixture
            .storage
            .connection
            .query_row("SELECT count(*) FROM design_accepted", [], |row| row.get(0))
            .unwrap();
        let history: i64 = fixture
            .storage
            .connection
            .query_row("SELECT count(*) FROM design_review_history", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!((accepted, history), (0, 2));
        fixture
            .storage
            .connection
            .execute_batch("DROP TRIGGER reject_design_default;")
            .unwrap();
        let accepted = confirm(&mut fixture, &review, false);
        assert!(!accepted.active_default);
        assert!(
            fixture
                .storage
                .load_design_default(
                    &fixture.repository,
                    DesignInterpretation::AsBuilt,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_err()
        );
        assert!(
            fixture
                .storage
                .preview_design_review(
                    &fixture.repository,
                    "review",
                    2,
                    DesignInterpretation::AsBuilt,
                    vec![],
                    &fixture.principal,
                    &fixture.grants,
                    10
                )
                .is_err()
        );
    }

    #[test]
    fn every_read_and_confirmation_rechecks_current_scope_and_source_permissions() {
        let mut fixture = fixture();
        let review = preview(&mut fixture, "review", DesignInterpretation::AsBuilt);
        let accepted = confirm(&mut fixture, &review, true);
        assert!(
            fixture
                .storage
                .load_design_version(
                    &fixture.repository,
                    &fixture.design.id,
                    &fixture.principal,
                    &[]
                )
                .is_err()
        );
        let mut no_source = fixture.grants.clone();
        no_source[0].actions.remove(&Action::Source);
        assert!(
            fixture
                .storage
                .load_design_default(
                    &fixture.repository,
                    DesignInterpretation::AsBuilt,
                    &fixture.principal,
                    &no_source
                )
                .is_err()
        );
        let mut narrow = fixture.grants.clone();
        narrow[0].paths = PathScope::new(vec!["src/public".into()], vec![]).unwrap();
        assert!(
            fixture
                .storage
                .load_design_acceptance(
                    &fixture.repository,
                    &accepted.accepted_id,
                    &fixture.principal,
                    &narrow
                )
                .is_err()
        );
        assert!(
            fixture
                .storage
                .export_design_version(
                    &fixture.repository,
                    &fixture.design.id,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_err()
        );
        let viewer = PrincipalId::new("viewer").unwrap();
        let mut viewer_grants = fixture.grants.clone();
        viewer_grants[0].principal = viewer.clone();
        viewer_grants[0]
            .actions
            .remove(&Action::ConfirmArchitecture);
        assert!(
            fixture
                .storage
                .load_design_review(&fixture.repository, &review.id, &viewer, &viewer_grants)
                .is_err()
        );
        assert_eq!(
            fixture
                .storage
                .load_design_default(
                    &fixture.repository,
                    DesignInterpretation::AsBuilt,
                    &viewer,
                    &viewer_grants
                )
                .unwrap()
                .accepted_id,
            accepted.accepted_id
        );
        assert!(
            fixture
                .storage
                .create_design_review(
                    &fixture.repository,
                    &fixture.design.id,
                    "viewer-draft",
                    DesignInterpretation::AsBuilt,
                    &viewer,
                    &viewer_grants,
                    10
                )
                .is_err()
        );
        let mut other_domain = fixture.repository.clone();
        other_domain = RepositoryId::new(
            SecurityDomain::new("other-tenant").unwrap(),
            other_domain.provider(),
            other_domain.owner(),
            other_domain.name(),
        )
        .unwrap();
        assert!(
            fixture
                .storage
                .load_design_version(
                    &other_domain,
                    &fixture.design.id,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_err()
        );
    }

    #[test]
    fn changed_context_blocks_acceptance_and_narrowed_context_blocks_historical_labels() {
        let mut fixture = fixture();
        let first = preview(
            &mut fixture,
            "accepted-review",
            DesignInterpretation::AsBuilt,
        );
        let accepted = confirm(&mut fixture, &first, true);
        let pending = preview(
            &mut fixture,
            "pending-review",
            DesignInterpretation::AsBuilt,
        );
        let (_, _) = confirm_context(
            &mut fixture.storage,
            &fixture.principal,
            &fixture.grants,
            fixture.scope.clone(),
            "new-scan",
        );
        assert!(
            fixture
                .storage
                .confirm_design_review(
                    &fixture.repository,
                    &pending.id,
                    pending.revision,
                    &pending.input_fingerprint,
                    true,
                    true,
                    &fixture.principal,
                    &fixture.grants,
                    12
                )
                .is_err()
        );
        let old = fixture
            .storage
            .load_design_acceptance(
                &fixture.repository,
                &accepted.accepted_id,
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap();
        assert_eq!(old.lifecycle_state, DesignReviewState::Stale);
        assert_eq!(old.review, accepted.review);
        let mut scope = fixture.scope.clone();
        scope.version = ScopeVersion::new("narrow-scope").unwrap();
        scope.primary_paths =
            PathScope::new(vec!["src/public".into()], vec!["src/public/private".into()]).unwrap();
        confirm_context(
            &mut fixture.storage,
            &fixture.principal,
            &fixture.grants,
            scope,
            "narrow-context",
        );
        let error = fixture
            .storage
            .load_design_default(
                &fixture.repository,
                DesignInterpretation::AsBuilt,
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap_err();
        assert_eq!(error, "Design unavailable in the current accepted context");
        assert!(!error.contains("Scoped engine"));
        assert!(
            fixture
                .storage
                .load_design_version(
                    &fixture.repository,
                    &fixture.design.id,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_err()
        );
    }

    #[test]
    fn correction_preview_rejects_unknown_forged_excluded_and_unbounded_inputs() {
        let mut fixture = fixture();
        fixture
            .storage
            .create_design_review(
                &fixture.repository,
                &fixture.design.id,
                "review",
                DesignInterpretation::AsBuilt,
                &fixture.principal,
                &fixture.grants,
                7,
            )
            .unwrap();
        let mut bad = responsibility();
        bad.target_id = "unknown-private-element".into();
        let mut forbidden = responsibility();
        forbidden.kind = DesignCorrectionKind::Boundary;
        forbidden.paths = vec!["src/private/secrets.rs".into()];
        let mut invented = forbidden.clone();
        invented.paths = vec!["src/invented".into()];
        let mut too_large = responsibility();
        too_large.value = "x".repeat(2049);
        let mut wrong_fields = responsibility();
        wrong_fields.constraint_kind = Some(DesignConstraintKind::ProhibitedDependency);
        for correction in [bad, forbidden, invented, too_large, wrong_fields] {
            assert!(
                fixture
                    .storage
                    .preview_design_review(
                        &fixture.repository,
                        "review",
                        0,
                        DesignInterpretation::AsBuilt,
                        vec![correction],
                        &fixture.principal,
                        &fixture.grants,
                        8
                    )
                    .is_err()
            );
        }
        assert!(serde_json::from_value::<DesignCorrection>(serde_json::json!({"id":"forged","kind":"responsibility","target_id":"component-1","value":"intent","constraint_kind":null,"evidence":["forged"]})).is_err());
        let duplicate = vec![responsibility(), responsibility()];
        assert!(
            fixture
                .storage
                .preview_design_review(
                    &fixture.repository,
                    "review",
                    0,
                    DesignInterpretation::AsBuilt,
                    duplicate,
                    &fixture.principal,
                    &fixture.grants,
                    8
                )
                .is_err()
        );
        assert_eq!(
            fixture
                .storage
                .load_design_review(
                    &fixture.repository,
                    "review",
                    &fixture.principal,
                    &fixture.grants
                )
                .unwrap()
                .revision,
            0
        );
    }

    #[test]
    fn typed_flow_ownership_constraints_and_correspondence_preserve_observed_evidence() {
        let mut fixture = fixture();
        let mut rich = fixture.design.clone();
        rich.id = "candidate-rich".into();
        for (id, kind, name) in [
            ("component-2", DesignElementKind::Component, "Consumer"),
            ("contract-1", DesignElementKind::Contract, "Review API"),
            ("schema-1", DesignElementKind::Schema, "Review schema"),
        ] {
            let mut element = rich.elements[0].clone();
            element.id = id.into();
            element.kind = kind;
            element.name = name.into();
            rich.elements.push(element);
        }
        rich.relations.push(DesignRelation {
            id: "observed-relation".into(),
            from: "component-1".into(),
            to: "component-2".into(),
            kind: "local_package_dependency".into(),
            direction: "source_to_dependency".into(),
            protocol: None,
            synchrony: "unknown".into(),
            resolution: DesignResolution::ObservedStructure,
            evidence_ids: vec!["evidence-1".into()],
            unknowns: vec!["Runtime call direction unknown".into()],
        });
        fixture
            .storage
            .save_design_version(
                &rich,
                &fixture.scope,
                &fixture.principal,
                &fixture.grants,
                7,
            )
            .unwrap();
        fixture
            .storage
            .create_design_review(
                &fixture.repository,
                &rich.id,
                "rich-review",
                DesignInterpretation::Intended,
                &fixture.principal,
                &fixture.grants,
                8,
            )
            .unwrap();
        let mut corrections = vec![];
        let mut boundary = responsibility();
        boundary.id = "boundary".into();
        boundary.kind = DesignCorrectionKind::Boundary;
        boundary.paths = vec!["src/lib.rs".into()];
        corrections.push(boundary);
        for (id, kind, target) in [
            (
                "contract-owner",
                DesignCorrectionKind::ContractOwnership,
                "contract-1",
            ),
            (
                "data-owner",
                DesignCorrectionKind::DataOwnership,
                "schema-1",
            ),
        ] {
            let mut assertion = responsibility();
            assertion.id = id.into();
            assertion.kind = kind;
            assertion.target_id = target.into();
            assertion.related_ids = vec!["component-2".into()];
            corrections.push(assertion);
        }
        for (id, kind, target) in [
            (
                "no-dependency",
                DesignConstraintKind::ProhibitedDependency,
                "component-2",
            ),
            (
                "required-contract",
                DesignConstraintKind::RequiredContract,
                "contract-1",
            ),
        ] {
            let mut assertion = responsibility();
            assertion.id = id.into();
            assertion.kind = DesignCorrectionKind::Constraint;
            assertion.constraint_kind = Some(kind);
            assertion.related_ids = vec![target.into()];
            corrections.push(assertion);
        }
        let mut flow = responsibility();
        flow.id = "flow".into();
        flow.kind = DesignCorrectionKind::MajorFlow;
        flow.target_id = "observed-relation".into();
        flow.value = "Requests should originate from the consumer".into();
        flow.related_ids = vec!["component-2".into(), "component-1".into()];
        corrections.push(flow);
        let mut correspondence = responsibility();
        correspondence.id = "identity-assertion".into();
        correspondence.kind = DesignCorrectionKind::Correspondence;
        correspondence.value = "rename".into();
        correspondence.related_ids = vec!["component-2".into()];
        corrections.push(correspondence);
        let preview = fixture
            .storage
            .preview_design_review(
                &fixture.repository,
                "rich-review",
                0,
                DesignInterpretation::Intended,
                corrections,
                &fixture.principal,
                &fixture.grants,
                9,
            )
            .unwrap();
        let interpreted = fixture
            .storage
            .preview_design_interpretation(
                &fixture.repository,
                "rich-review",
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap();
        assert_eq!(interpreted.evidence, rich.evidence);
        assert_eq!(interpreted.relations[0], rich.relations[0]);
        assert_eq!(
            interpreted.relations[1].resolution,
            DesignResolution::UserAssertion
        );
        assert!(interpreted.relations[1].evidence_ids.is_empty());
        assert_eq!(interpreted.relations[1].from, "component-2");
        assert_eq!(interpreted.constraints.len(), 2);
        assert_eq!(
            interpreted.correspondence[0].resolution,
            DesignResolution::UserAssertion
        );
        assert_eq!(interpreted.elements[0].roots, vec!["src/lib.rs"]);
        assert_eq!(
            interpreted.elements[2].component_id.as_deref(),
            Some("component-2")
        );
        assert_eq!(
            interpreted.elements[3].component_id.as_deref(),
            Some("component-2")
        );
        let accepted = fixture
            .storage
            .confirm_design_review(
                &fixture.repository,
                "rich-review",
                preview.revision,
                &preview.input_fingerprint,
                true,
                false,
                &fixture.principal,
                &fixture.grants,
                10,
            )
            .unwrap();
        assert_eq!(accepted.observed_design, rich);
        assert_eq!(accepted.design.elements[0].facts, rich.elements[0].facts);
    }
    #[test]
    fn cross_snapshot_correspondence_requires_pinned_authorized_versions_and_typed_cardinality() {
        let mut fixture = fixture();
        let mut proposal = fixture.design.clone();
        proposal.id = "proposal-candidate".into();
        proposal.snapshot = SnapshotId::new("c".repeat(40)).unwrap();
        proposal.evidence[0].commit = proposal.snapshot.clone();
        proposal.elements[0].id = "component-renamed".into();
        proposal.elements[0].name = "Renamed engine".into();
        fixture
            .storage
            .save_design_version(
                &proposal,
                &fixture.scope,
                &fixture.principal,
                &fixture.grants,
                7,
            )
            .unwrap();
        fixture
            .storage
            .create_design_review(
                &fixture.repository,
                &proposal.id,
                "correspondence-review",
                DesignInterpretation::Intended,
                &fixture.principal,
                &fixture.grants,
                8,
            )
            .unwrap();
        let mut mapping = responsibility();
        mapping.kind = DesignCorrectionKind::Correspondence;
        mapping.value = "rename".into();
        mapping.target_id = "component-renamed".into();
        mapping.related_ids = vec!["component-1".into()];
        mapping.reference_design_id = Some(fixture.design.id.clone());
        mapping.correspondence_targets = vec!["component-renamed".into()];
        let mut unknown = mapping.clone();
        unknown.reference_design_id = Some("unavailable-private-design".into());
        let mut wrong_kind = mapping.clone();
        wrong_kind.value = "merge".into();
        let mut duplicated = mapping.clone();
        duplicated.related_ids = vec!["component-1".into(), "component-1".into()];
        for invalid in [unknown, wrong_kind, duplicated] {
            assert!(
                fixture
                    .storage
                    .preview_design_review(
                        &fixture.repository,
                        "correspondence-review",
                        0,
                        DesignInterpretation::Intended,
                        vec![invalid],
                        &fixture.principal,
                        &fixture.grants,
                        9
                    )
                    .is_err()
            );
        }
        let review = fixture
            .storage
            .preview_design_review(
                &fixture.repository,
                "correspondence-review",
                0,
                DesignInterpretation::Intended,
                vec![mapping],
                &fixture.principal,
                &fixture.grants,
                9,
            )
            .unwrap();
        let view = fixture
            .storage
            .preview_design_interpretation(
                &fixture.repository,
                &review.id,
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap();
        assert_eq!(view.correspondence[0].from_ids, vec!["component-1"]);
        assert_eq!(view.correspondence[0].to_ids, vec!["component-renamed"]);
        assert_eq!(
            view.correspondence[0].resolution,
            DesignResolution::UserAssertion
        );
        let delta = crate::design::compare_hld(&fixture.design, &view).unwrap();
        assert!(
            delta
                .questions
                .iter()
                .any(|question| question.contains("correspondence"))
        );
        assert!(
            !delta
                .changes
                .iter()
                .any(|change| change.reason.contains("Explicit reviewer rename"))
        );
        let accepted = fixture
            .storage
            .confirm_design_review(
                &fixture.repository,
                &review.id,
                review.revision,
                &review.input_fingerprint,
                true,
                false,
                &fixture.principal,
                &fixture.grants,
                10,
            )
            .unwrap();
        assert_eq!(accepted.observed_design, proposal);
        let original: Vec<u8> = fixture
            .storage
            .connection
            .query_row(
                "SELECT payload FROM design_versions WHERE design_id=?1",
                params![fixture.design.id],
                |row| row.get(0),
            )
            .unwrap();
        fixture
            .storage
            .connection
            .execute(
                "UPDATE design_versions SET input_fingerprint='invalid' WHERE design_id=?1",
                params![fixture.design.id],
            )
            .unwrap();
        assert!(
            fixture
                .storage
                .load_design_acceptance(
                    &fixture.repository,
                    &accepted.accepted_id,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_err()
        );
        fixture
            .storage
            .connection
            .execute(
                "UPDATE design_versions SET input_fingerprint=?2 WHERE design_id=?1",
                params![fixture.design.id, digest(&original)],
            )
            .unwrap();
        assert!(
            fixture
                .storage
                .load_design_acceptance(
                    &fixture.repository,
                    &accepted.accepted_id,
                    &fixture.principal,
                    &fixture.grants
                )
                .is_ok()
        );
    }
    #[test]
    fn selected_documents_are_pinned_user_intent_not_code_facts_and_stale_docs_require_review() {
        let mut fixture = fixture();
        let mut observed = fixture.design.clone();
        observed.id = "document-candidate-design".into();
        let mut evidence = observed.evidence[0].clone();
        evidence.id = "evidence-document-1".into();
        evidence.path = "src/architecture.md".into();
        evidence.relationship_type = "documentation_candidate".into();
        evidence.end_line = evidence.start_line;
        observed.evidence.push(evidence.clone());
        observed
            .document_candidates
            .push(crate::design::DesignDocumentCandidate {
                id: "document-1".into(),
                title: "architecture.md".into(),
                evidence_id: evidence.id.clone(),
            });
        fixture
            .storage
            .save_design_version(
                &observed,
                &fixture.scope,
                &fixture.principal,
                &fixture.grants,
                7,
            )
            .unwrap();
        fixture
            .storage
            .create_design_review(
                &fixture.repository,
                &observed.id,
                "document-review",
                DesignInterpretation::Intended,
                &fixture.principal,
                &fixture.grants,
                8,
            )
            .unwrap();
        let mut decision = responsibility();
        decision.id = "decision-1".into();
        decision.kind = DesignCorrectionKind::DocumentReference;
        decision.value =
            "<script>DOC_INTENT_SENTINEL</script> `instructions` remain reviewer intent".into();
        decision.document_evidence_id = Some(evidence.id.clone());
        let mut fake = decision.clone();
        fake.document_evidence_id = Some("evidence-1".into());
        let mut missing = decision.clone();
        missing.document_evidence_id = None;
        let mut wrong_field = responsibility();
        wrong_field.document_evidence_id = Some(evidence.id.clone());
        for invalid in [fake, missing, wrong_field] {
            assert!(
                fixture
                    .storage
                    .preview_design_review(
                        &fixture.repository,
                        "document-review",
                        0,
                        DesignInterpretation::Intended,
                        vec![invalid],
                        &fixture.principal,
                        &fixture.grants,
                        9
                    )
                    .is_err()
            );
        }
        let review = fixture
            .storage
            .preview_design_review(
                &fixture.repository,
                "document-review",
                0,
                DesignInterpretation::Intended,
                vec![decision],
                &fixture.principal,
                &fixture.grants,
                9,
            )
            .unwrap();
        let display = fixture
            .storage
            .preview_design_interpretation(
                &fixture.repository,
                &review.id,
                &fixture.principal,
                &fixture.grants,
            )
            .unwrap();
        assert_eq!(display.elements[0].facts, observed.elements[0].facts);
        assert_eq!(
            display.elements[0].user_assertions,
            observed.elements[0].user_assertions
        );
        assert_eq!(display.relations, observed.relations);
        assert!(
            crate::design::compare_hld(&observed, &display)
                .unwrap()
                .changes
                .is_empty(),
            "Selecting an intent reference alone must not manufacture a structural architecture change"
        );
        assert_eq!(display.decision_records.len(), 1);
        assert_eq!(
            display.decision_records[0].resolution,
            DesignResolution::UserAssertion
        );
        assert_eq!(
            display.decision_records[0].verification,
            "requires_reviewer_verification"
        );
        assert!(!display.markdown.contains("<script>"));
        assert!(
            display.markdown.contains("&lt;script&gt;DOC")
                && display.markdown.contains("Selected document decision")
        );
        let accepted = fixture
            .storage
            .confirm_design_review(
                &fixture.repository,
                &review.id,
                review.revision,
                &review.input_fingerprint,
                true,
                false,
                &fixture.principal,
                &fixture.grants,
                10,
            )
            .unwrap();
        assert_eq!(accepted.observed_design, observed);
        let findings = crate::design::evaluate_hld_decisions(&observed, &accepted.design);
        assert_eq!(findings[0].resolution, DesignResolution::Unknown);
        let mut changed = observed.clone();
        changed.id = "changed-document-design".into();
        changed.snapshot = SnapshotId::new("c".repeat(40)).unwrap();
        for evidence in &mut changed.evidence {
            evidence.commit = changed.snapshot.clone();
            if evidence.relationship_type == "documentation_candidate" {
                evidence.blob = "d".repeat(40);
            }
        }
        let findings = crate::design::evaluate_hld_decisions(&changed, &accepted.design);
        assert!(findings[0].message.contains("may be stale"));
        assert_eq!(findings[0].resolution, DesignResolution::Unknown);
        let delta = crate::design::compare_hld(&observed, &changed).unwrap();
        assert!(
            delta.changes.is_empty(),
            "An unselected document edit is not an observed architecture change"
        );
        let mut outside = observed.clone();
        outside.id = "excluded-doc-design".into();
        outside.evidence.last_mut().unwrap().path =
            "src/private/SECRET_DECISION_SENTINEL.md".into();
        assert!(
            fixture
                .storage
                .save_design_version(
                    &outside,
                    &fixture.scope,
                    &fixture.principal,
                    &fixture.grants,
                    11
                )
                .is_err()
        );
        let mut revoked = fixture.grants.clone();
        revoked[0].actions.remove(&Action::Source);
        assert!(
            fixture
                .storage
                .load_design_acceptance(
                    &fixture.repository,
                    &accepted.accepted_id,
                    &fixture.principal,
                    &revoked
                )
                .is_err()
        );
        assert!(
            !serde_json::to_value(responsibility())
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("document_evidence_id"),
            "Adding optional doc selection must preserve existing correction fingerprints"
        );
    }
}
