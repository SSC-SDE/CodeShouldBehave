//! Immutable, source-backed low-level design. Local syntax flow is advisory
//! static evidence, never an executed trace or a compiler/type-checking claim.
mod blob;
mod catalog;
mod diff;
mod flow;
mod render;
#[cfg(test)]
mod tests;

use crate::design::{DesignEvidence, DesignVersion, wire};
use crate::{RepositoryId, ScopeVersion, SnapshotId};
use serde::{Deserialize, Serialize};

pub(crate) use catalog::semantic_tokens;
pub use catalog::{
    build_lld_detail, generate_lld, lld_detail_design, lld_source, validate_lld_snapshot,
};
pub use diff::{compare_lld, compare_lld_detail, compare_lld_with_correspondence};
pub use render::{render_lld_markdown, render_lld_mermaid};

pub const LLD_GENERATOR_VERSION: &str = "csb-lld-rust-static-v1";
pub const LLD_PROFILE: &str = "rust-local-syntax-flow-v1";
pub const LLD_LABEL: &str = "Static local control flow";
pub const LLD_IR_VERSION: &str = "tree-sitter-rust-0.24.2-core-0.25.10-lld-ir-v1";
pub const MAX_LLD_ENTRIES: usize = 2000;
pub const MAX_LLD_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LLD_CATALOG_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldCoverage {
    pub inventory_complete: bool,
    pub structure_complete: bool,
    pub local_flow: String,
    pub call_target_resolution: String,
    pub runtime_validation: String,
    pub indexed: usize,
    pub unsupported: usize,
    pub failed: usize,
    pub deferred: usize,
    pub gaps: Vec<String>,
}
impl Default for LldCoverage {
    fn default() -> Self {
        Self {
            inventory_complete: true,
            structure_complete: true,
            local_flow: "lazy_detail".into(),
            call_target_resolution: "unresolved_syntax_only".into(),
            runtime_validation: "unavailable".into(),
            indexed: 0,
            unsupported: 0,
            failed: 0,
            deferred: 0,
            gaps: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldParentBinding {
    pub hld_design_id: String,
    pub component_id: Option<String>,
    pub candidate_component_ids: Vec<String>,
    pub state: String,
    pub mapping_basis: String,
    pub mapping_fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldProjection {
    pub kind: String,
    pub fingerprint: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldEntry {
    pub id: String,
    pub logical_key: String,
    pub name: String,
    pub kind: String,
    pub module: String,
    pub container: String,
    pub parent_entry_id: Option<String>,
    pub child_entry_ids: Vec<String>,
    pub path: String,
    pub blob: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub visibility: String,
    pub signature: String,
    pub signature_fingerprint: String,
    pub body_fingerprint: String,
    pub projections: Vec<LldProjection>,
    pub call_observations: Vec<String>,
    pub detail_state: String,
    pub evidence_ids: Vec<String>,
    pub parent: LldParentBinding,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldCatalog {
    pub id: String,
    pub generator_version: String,
    pub parser_version: String,
    pub profile: String,
    #[serde(with = "wire::repository")]
    pub repository: RepositoryId,
    #[serde(with = "wire::snapshot")]
    pub snapshot: SnapshotId,
    #[serde(with = "wire::scope_version")]
    pub scope_version: ScopeVersion,
    pub scope_fingerprint: String,
    pub onboarding_session_id: String,
    pub onboarding_revision: u64,
    pub onboarding_input_fingerprint: String,
    pub onboarding_proposal_fingerprint: String,
    pub hld_design_id: String,
    pub entries: Vec<LldEntry>,
    pub evidence: Vec<DesignEvidence>,
    pub coverage: LldCoverage,
}

/// Private immutable input, retained before a temporary repository lease ends.
/// Never included in public catalog or detail JSON; exact source is delivered
/// only through the separately authorized, bounded evidence endpoint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldSyntaxInput {
    pub id: String,
    pub version: String,
    pub path: String,
    pub blob: String,
    pub source: String,
    pub source_fingerprint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldSnapshot {
    pub catalog: LldCatalog,
    pub inputs: Vec<LldSyntaxInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldBehaviorNode {
    pub id: String,
    pub region_id: String,
    pub ast_kind: String,
    pub operation: String,
    pub tokens: String,
    pub label: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub evidence_id: String,
    pub resolution: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldBehaviorEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub label: String,
    pub evidence_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldFlowRegion {
    pub id: String,
    pub parent_node: Option<String>,
    pub kind: String,
    pub entry: String,
    pub exit: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldFlow {
    pub nodes: Vec<LldBehaviorNode>,
    pub edges: Vec<LldBehaviorEdge>,
    pub regions: Vec<LldFlowRegion>,
    pub complete: bool,
    pub limitations: Vec<String>,
    pub omitted_nodes: usize,
    pub omitted_edges: usize,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldFunctionDetail {
    pub id: String,
    pub catalog_id: String,
    pub entry: LldEntry,
    pub profile: String,
    pub label: String,
    pub graph: LldFlow,
    pub declarations: Vec<LldEntry>,
    pub evidence: Vec<DesignEvidence>,
    pub behavior_fingerprint: String,
    pub coverage: LldCoverage,
    pub mermaid: String,
    pub markdown: String,
}

/// The shared design stores typed local detail rather than fabricated HLD
/// sections. Assertions remain in the existing separate interpretation store.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldDesignData {
    pub catalog_id: String,
    pub entry_id: String,
    pub logical_key: String,
    pub behavior_fingerprint: String,
    pub parent: LldParentBinding,
    pub detail: LldFunctionDetail,
    #[serde(default)]
    pub invariants: Vec<LldInvariant>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldInvariant {
    pub id: String,
    pub entry_id: String,
    pub body_fingerprint: String,
    pub statement: String,
    pub resolution: String,
    pub verification: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldChange {
    pub id: String,
    pub kind: String,
    pub classification: String,
    pub logical_key: String,
    pub baseline_entry_id: Option<String>,
    pub proposal_entry_id: Option<String>,
    pub before_evidence_ids: Vec<String>,
    pub after_evidence_ids: Vec<String>,
    pub before_parent: Option<LldParentBinding>,
    pub after_parent: Option<LldParentBinding>,
    pub resolution: String,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldDelta {
    pub id: String,
    pub baseline_catalog_id: String,
    pub proposal_catalog_id: String,
    pub outcome: String,
    pub changes: Vec<LldChange>,
    pub limitations: Vec<String>,
    pub ambiguous_groups: usize,
    pub possible_callers: Vec<LldChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub correspondence: Vec<crate::design::DesignCorrespondence>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LldReview {
    pub baseline: LldSnapshot,
    pub proposal: LldSnapshot,
    pub delta: LldDelta,
}

pub fn validate_lld_design(
    version: &DesignVersion,
    scope: &crate::ScopeRecord,
) -> Result<(), String> {
    let data = version
        .lld
        .as_ref()
        .ok_or("LLD design requires typed detail")?;
    let detail = &data.detail;
    if !version.sections.is_empty()
        || !version.elements.is_empty()
        || !version.relations.is_empty()
        || data.entry_id != detail.entry.id
        || data.catalog_id != detail.catalog_id
        || data.behavior_fingerprint != detail.behavior_fingerprint
        || detail.graph.nodes.len() > 200
        || detail.graph.edges.len() > 400
        || detail.evidence != version.evidence
        || detail.profile != LLD_PROFILE
        || detail.entry.path.is_empty()
        || !crate::design::safe_path(&detail.entry.path, scope)
        || version.evidence.iter().any(|item| {
            item.repository != version.repository
                || item.commit != version.snapshot
                || !crate::design::safe_path(&item.path, scope)
                || item.start_line == 0
                || item.end_line < item.start_line
                || SnapshotId::new(item.blob.clone()).is_err()
        })
    {
        return Err("LLD design is outside its typed immutable detail scope".into());
    }
    if data.invariants.len() > 100
        || data.invariants.iter().any(|item| {
            item.entry_id != data.entry_id
                || item.body_fingerprint != data.behavior_fingerprint
                || item.statement.is_empty()
                || item.statement.len() > 2048
                || item.statement.chars().any(char::is_control)
                || item.resolution != "user_assertion"
                || item.verification != "requires_reviewer_verification"
        })
    {
        return Err("LLD invariant is not bound to the selected immutable behavior".into());
    }
    if version.correspondence.iter().any(|item| {
        item.resolution != crate::design::DesignResolution::UserAssertion
            || !matches!(item.kind.as_str(), "rename" | "move" | "split" | "merge")
            || item.from_ids.is_empty()
            || item.to_ids.is_empty()
            || item.from_ids.len() > 100
            || item.to_ids.len() > 100
            || item
                .from_ids
                .iter()
                .chain(&item.to_ids)
                .any(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
            || !item.to_ids.contains(&data.entry_id)
    }) {
        return Err("LLD correspondence must remain a bounded explicit assertion targeting this immutable detail".into());
    }
    let ids = detail
        .graph
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if ids.len() != detail.graph.nodes.len()
        || detail
            .graph
            .edges
            .iter()
            .any(|edge| !ids.contains(edge.from.as_str()) || !ids.contains(edge.to.as_str()))
    {
        return Err("LLD graph endpoints are invalid".into());
    }
    Ok(())
}
