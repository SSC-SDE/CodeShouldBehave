use super::*;
use crate::design::digest;
use std::collections::{BTreeMap, BTreeSet};

fn change(
    before: Option<&LldEntry>,
    after: Option<&LldEntry>,
    kind: &str,
    classification: &str,
    resolution: &str,
) -> LldChange {
    let entry = after.or(before).expect("one change side");
    let before_id = before.map(|entry| entry.id.clone());
    let after_id = after.map(|entry| entry.id.clone());
    LldChange {
        id: format!(
            "lld-change-{}",
            digest(&[
                before_id.as_deref().unwrap_or(""),
                after_id.as_deref().unwrap_or(""),
                kind,
                classification
            ])
        ),
        kind: kind.into(),
        classification: classification.into(),
        logical_key: entry.logical_key.clone(),
        baseline_entry_id: before_id,
        proposal_entry_id: after_id,
        before_evidence_ids: before
            .map(|entry| entry.evidence_ids.clone())
            .unwrap_or_default(),
        after_evidence_ids: after
            .map(|entry| entry.evidence_ids.clone())
            .unwrap_or_default(),
        before_parent: before.map(|entry| entry.parent.clone()),
        after_parent: after.map(|entry| entry.parent.clone()),
        resolution: resolution.into(),
        limitations: vec![],
    }
}
fn compatible(before: &LldEntry, after: &LldEntry) -> bool {
    before.kind == after.kind
        && before.parent.state == "matched"
        && after.parent.state == "matched"
        && before.parent.component_id == after.parent.component_id
}
fn compare_entry(before: &LldEntry, after: &LldEntry, changes: &mut Vec<LldChange>) {
    if matches!(before.kind.as_str(), "schema" | "configuration")
        && before.body_fingerprint != after.body_fingerprint
    {
        changes.push(change(
            Some(before),
            Some(after),
            if before.kind == "schema" {
                "schema_artifact_changed"
            } else {
                "configuration_artifact_changed"
            },
            "modified",
            "observed_structure",
        ));
        return;
    }
    if before.signature_fingerprint != after.signature_fingerprint {
        changes.push(change(
            Some(before),
            Some(after),
            "signature_changed",
            "modified",
            "observed_structure",
        ));
    }
    if before.body_fingerprint != after.body_fingerprint {
        let mut classified = false;
        let kinds = before
            .projections
            .iter()
            .chain(after.projections.iter())
            .map(|item| item.kind.as_str())
            .collect::<BTreeSet<_>>();
        for kind in kinds {
            let a = before
                .projections
                .iter()
                .find(|projection| projection.kind == kind);
            let b = after
                .projections
                .iter()
                .find(|projection| projection.kind == kind);
            if a != b {
                let kind = match kind {
                    "condition" => "condition_changed",
                    "pattern" => "pattern_or_guard_changed",
                    "exit" => "exit_or_residual_changed",
                    "call" => "call_observation_changed",
                    "await" => "await_boundary_changed",
                    "assignment" => "assignment_or_operation_changed",
                    "loop" => "loop_changed",
                    _ => "unsupported_region_changed",
                };
                changes.push(change(
                    Some(before),
                    Some(after),
                    kind,
                    "modified",
                    "observed_structure",
                ));
                classified = true;
            }
        }
        if !classified {
            changes.push(change(
                Some(before),
                Some(after),
                if before.kind == "schema" {
                    "schema_artifact_changed"
                } else {
                    "body_or_declaration_changed"
                },
                "modified",
                "observed_structure",
            ));
        }
    }
    if before.visibility != after.visibility {
        changes.push(change(
            Some(before),
            Some(after),
            "visibility_changed",
            "modified",
            "observed_structure",
        ));
    }
    // Parent design versions differ for every commit; compare containment
    // identities/state, not the immutable parent design ID itself.
    if before.parent.state != after.parent.state
        || before.parent.component_id != after.parent.component_id
        || before.parent.candidate_component_ids != after.parent.candidate_component_ids
    {
        changes.push(change(
            Some(before),
            Some(after),
            "parent_containment_changed",
            "modified",
            "observed_structure",
        ));
    }
}
pub fn compare_lld(before: &LldCatalog, after: &LldCatalog) -> LldDelta {
    let mut changes = vec![];
    let mut limitations = vec![];
    let mut ambiguous_groups = 0;
    let same_identity = before.repository == after.repository
        && before.scope_version == after.scope_version
        && before.scope_fingerprint == after.scope_fingerprint
        && before.parser_version == after.parser_version
        && before.profile == after.profile;
    if !same_identity {
        limitations.push("Catalog repository/scope/profile differ; no cross-boundary declaration matching attempted".into());
    }
    let mut before_groups = BTreeMap::<&str, Vec<&LldEntry>>::new();
    let mut after_groups = BTreeMap::<&str, Vec<&LldEntry>>::new();
    for entry in &before.entries {
        before_groups
            .entry(&entry.logical_key)
            .or_default()
            .push(entry);
    }
    for entry in &after.entries {
        after_groups
            .entry(&entry.logical_key)
            .or_default()
            .push(entry);
    }
    let keys = before_groups
        .keys()
        .chain(after_groups.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut unmatched_before = vec![];
    let mut unmatched_after = vec![];
    if same_identity {
        for key in keys {
            let a = before_groups.get(key).map(Vec::as_slice).unwrap_or(&[]);
            let b = after_groups.get(key).map(Vec::as_slice).unwrap_or(&[]);
            if a.len() > 1 || b.len() > 1 {
                ambiguous_groups += 1;
                limitations.push(format!(
                    "Duplicate qualified declaration remains ambiguous: {}",
                    a.first()
                        .or(b.first())
                        .map(|entry| entry.name.as_str())
                        .unwrap_or("unknown")
                ));
                continue;
            }
            match (a.first(), b.first()) {
                (Some(a), Some(b)) if compatible(a, b) => compare_entry(a, b, &mut changes),
                (Some(a), Some(b)) => {
                    let mut item = change(
                        Some(a),
                        Some(b),
                        "correspondence_ambiguous",
                        "unknown",
                        "unknown",
                    );
                    item.limitations.push(
                        "Equal logical spelling has incompatible or unresolved parent containment"
                            .into(),
                    );
                    changes.push(item);
                    ambiguous_groups += 1;
                }
                (Some(a), None) => unmatched_before.push(*a),
                (None, Some(b)) => unmatched_after.push(*b),
                _ => {}
            }
        }
    }
    let mut used_after = BTreeSet::new();
    let inventory_complete = before.coverage.inventory_complete
        && after.coverage.inventory_complete
        && before.coverage.structure_complete
        && after.coverage.structure_complete;
    for a in &unmatched_before {
        let candidates = unmatched_after
            .iter()
            .filter(|b| {
                !used_after.contains(&b.id)
                    && a.kind == b.kind
                    && a.blob == b.blob
                    && a.signature_fingerprint == b.signature_fingerprint
                    && a.body_fingerprint == b.body_fingerprint
                    && (a.name == b.name || a.kind == "file")
            })
            .collect::<Vec<_>>();
        let unique_before = unmatched_before
            .iter()
            .filter(|entry| {
                entry.kind == a.kind
                    && entry.blob == a.blob
                    && entry.signature_fingerprint == a.signature_fingerprint
                    && entry.body_fingerprint == a.body_fingerprint
                    && (entry.name == a.name || a.kind == "file")
            })
            .count()
            == 1;
        if candidates.len() == 1 && unique_before {
            let b = *candidates[0];
            used_after.insert(b.id.clone());
            let mut item = change(
                Some(a),
                Some(b),
                "declaration_moved",
                "moved",
                "observed_structure",
            );
            item.limitations.push("Unique identical pinned blob supports a source move observation; compiler module resolution and behavioral equivalence remain unverified".into());
            changes.push(item);
        } else {
            changes.push(change(
                Some(a),
                None,
                "declaration_removed",
                if inventory_complete {
                    "removed"
                } else {
                    "unknown"
                },
                if inventory_complete {
                    "observed_structure"
                } else {
                    "unknown"
                },
            ));
        }
    }
    for b in unmatched_after {
        if !used_after.contains(&b.id) {
            changes.push(change(
                None,
                Some(b),
                "declaration_added",
                if inventory_complete {
                    "added"
                } else {
                    "unknown"
                },
                if inventory_complete {
                    "observed_structure"
                } else {
                    "unknown"
                },
            ));
        }
    }
    if !inventory_complete {
        limitations.push("Inventory/structure is incomplete; missing declarations cannot establish an addition/removal and matching diagrams do not prove no impact".into());
    }
    limitations.extend(before.coverage.gaps.iter().cloned());
    limitations.extend(after.coverage.gaps.iter().cloned());
    limitations.sort();
    limitations.dedup();
    limitations.truncate(64);
    let mut possible_callers = vec![];
    let changed_names = changes
        .iter()
        .filter_map(|change| {
            change
                .proposal_entry_id
                .as_ref()
                .and_then(|id| after.entries.iter().find(|entry| entry.id == *id))
        })
        .filter(|entry| entry.kind == "function")
        .map(|entry| entry.name.as_str())
        .collect::<BTreeSet<_>>();
    let changed_ids = changes
        .iter()
        .filter_map(|change| change.proposal_entry_id.as_ref())
        .collect::<BTreeSet<_>>();
    for entry in after
        .entries
        .iter()
        .filter(|entry| entry.kind == "function" && !changed_ids.contains(&entry.id))
    {
        for name in &changed_names {
            if entry.call_observations.iter().any(|call| {
                call.rsplit("::")
                    .next()
                    .unwrap_or(call)
                    .rsplit('.')
                    .next()
                    .unwrap_or(call)
                    == *name
            }) {
                let mut item = change(
                    None,
                    Some(entry),
                    "possible_unchanged_caller",
                    "possible",
                    "name_only_candidate",
                );
                item.limitations.push("Terminal-name syntax candidate only; receiver/trait/alias/generic resolution and runtime effect are unknown".into());
                possible_callers.push(item);
                break;
            }
        }
        if possible_callers.len() >= 200 {
            limitations.push("Possible caller frontier reached the 200-entry bound; remaining impact is deferred".into());
            break;
        }
    }
    let detected = changes
        .iter()
        .any(|change| change.resolution == "observed_structure");
    let detail_pending =
        before.coverage.local_flow == "lazy_detail" || after.coverage.local_flow == "lazy_detail";
    if detail_pending && !detected {
        limitations.push("No normalized declaration/body delta observed; local function graphs remain lazy and cannot establish a complete behavior no-impact result".into());
    }
    let outcome = if detected {
        "change_detected"
    } else if !same_identity
        || !inventory_complete
        || detail_pending
        || ambiguous_groups > 0
        || changes.iter().any(|change| change.resolution == "unknown")
    {
        "incomplete"
    } else {
        "no_change_detected_within_scope"
    };
    LldDelta {
        id: format!(
            "lld-delta-{}",
            digest(&[
                &before.id,
                &after.id,
                outcome,
                &serde_json::to_string(&changes).unwrap_or_default()
            ])
        ),
        baseline_catalog_id: before.id.clone(),
        proposal_catalog_id: after.id.clone(),
        outcome: outcome.into(),
        changes,
        limitations,
        ambiguous_groups,
        possible_callers,
        correspondence: vec![],
    }
}

/// Authorized UI/native callers validate fresh scope and referenced designs
/// before supplying reviewer correspondence. This function validates exact
/// catalog occurrence IDs and cardinality; the assertion never establishes
/// equivalent behavior or transfers another body's acceptance receipt.
pub fn compare_lld_with_correspondence(
    before: &LldCatalog,
    after: &LldCatalog,
    correspondence: &[crate::design::DesignCorrespondence],
) -> Result<LldDelta, String> {
    if correspondence.len() > 100
        || before.repository != after.repository
        || before.scope_version != after.scope_version
        || before.scope_fingerprint != after.scope_fingerprint
    {
        return Err("LLD correspondence exceeds its bounded same-scope comparison".into());
    }
    let mut from_used = BTreeSet::new();
    let mut to_used = BTreeSet::new();
    let mut delta = compare_lld(before, after);
    for item in correspondence {
        if item.resolution != crate::design::DesignResolution::UserAssertion
            || !matches!(item.kind.as_str(), "rename" | "move" | "split" | "merge")
            || item.from_ids.is_empty()
            || item.to_ids.is_empty()
            || item.from_ids.len() > 100
            || item.to_ids.len() > 100
        {
            return Err(
                "LLD correspondence requires explicit bounded reviewer identity assertions".into(),
            );
        }
        let cardinality = match item.kind.as_str() {
            "rename" | "move" => item.from_ids.len() == 1 && item.to_ids.len() == 1,
            "split" => item.from_ids.len() == 1 && item.to_ids.len() >= 2,
            "merge" => item.from_ids.len() >= 2 && item.to_ids.len() == 1,
            _ => false,
        };
        if !cardinality {
            return Err("LLD correspondence cardinality does not match its typed kind".into());
        }
        let from = item
            .from_ids
            .iter()
            .map(|id| {
                before
                    .entries
                    .iter()
                    .find(|entry| entry.id == *id)
                    .ok_or("LLD correspondence baseline entry unavailable")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let to = item
            .to_ids
            .iter()
            .map(|id| {
                after
                    .entries
                    .iter()
                    .find(|entry| entry.id == *id)
                    .ok_or("LLD correspondence proposal entry unavailable")
            })
            .collect::<Result<Vec<_>, _>>()?;
        if from
            .iter()
            .chain(&to)
            .any(|entry| entry.kind != from[0].kind)
            || item.from_ids.iter().any(|id| !from_used.insert(id.clone()))
            || item.to_ids.iter().any(|id| !to_used.insert(id.clone()))
        {
            return Err("LLD correspondence has incompatible kinds or conflicting duplicate identity assertions".into());
        }
        for a in &from {
            for b in &to {
                let mut assertion = change(
                    Some(a),
                    Some(b),
                    &format!("correspondence_{}", item.kind),
                    "user_assertion",
                    "user_assertion",
                );
                assertion.limitations.push("Reviewer-selected identity correspondence; behavioral equivalence, runtime compatibility and acceptance transfer remain unverified".into());
                delta.changes.push(assertion);
            }
        }
        if from.len() == 1 && to.len() == 1 {
            compare_entry(from[0], to[0], &mut delta.changes);
        }
    }
    delta.correspondence = correspondence.to_vec();
    delta.id = format!(
        "lld-correspondence-delta-{}",
        digest(&[
            &delta.id,
            &serde_json::to_string(correspondence)
                .map_err(|_| "Cannot encode LLD correspondence")?
        ])
    );
    Ok(delta)
}

/// Detail comparison stays independent from HLD and includes explicit partial
/// local-flow coverage. A known supported change can coexist with unknowns.
pub fn compare_lld_detail(
    before: Option<&LldFunctionDetail>,
    after: Option<&LldFunctionDetail>,
) -> LldDelta {
    let mut changes = vec![];
    let mut limitations = vec![];
    match (before, after) {
        (Some(a), Some(b)) => {
            compare_entry(&a.entry, &b.entry, &mut changes);
            if a.graph.fingerprint != b.graph.fingerprint && changes.is_empty() {
                changes.push(change(
                    Some(&a.entry),
                    Some(&b.entry),
                    "local_flow_changed",
                    "modified",
                    "observed_structure",
                ));
            }
            limitations.extend(a.graph.limitations.iter().cloned());
            limitations.extend(b.graph.limitations.iter().cloned());
        }
        (Some(a), None) => {
            let known = a.coverage.inventory_complete && a.coverage.structure_complete;
            changes.push(change(
                Some(&a.entry),
                None,
                "declaration_removed",
                if known { "removed" } else { "unknown" },
                if known {
                    "observed_structure"
                } else {
                    "unknown"
                },
            ));
            if !known {
                limitations.push(
                    "Missing counterpart cannot establish removal from incomplete catalog coverage"
                        .into(),
                );
            }
        }
        (None, Some(b)) => {
            let known = b.coverage.inventory_complete && b.coverage.structure_complete;
            changes.push(change(
                None,
                Some(&b.entry),
                "declaration_added",
                if known { "added" } else { "unknown" },
                if known {
                    "observed_structure"
                } else {
                    "unknown"
                },
            ));
            if !known {
                limitations.push("Missing counterpart cannot establish addition from incomplete catalog coverage".into());
            }
        }
        _ => {
            limitations.push("Selected local detail is unavailable on both exact revisions".into())
        }
    }
    let complete = before
        .into_iter()
        .chain(after)
        .all(|detail| detail.graph.complete && detail.coverage.structure_complete);
    let outcome = if changes
        .iter()
        .any(|change| change.resolution == "observed_structure")
    {
        "change_detected"
    } else if before.is_none() && after.is_none() {
        "unavailable"
    } else if !complete || !changes.is_empty() {
        "incomplete"
    } else {
        "no_change_detected_within_scope"
    };
    limitations.sort();
    limitations.dedup();
    LldDelta {
        id: format!(
            "lld-detail-delta-{}",
            digest(&[
                before.map(|detail| detail.id.as_str()).unwrap_or(""),
                after.map(|detail| detail.id.as_str()).unwrap_or(""),
                outcome
            ])
        ),
        baseline_catalog_id: before
            .map(|detail| detail.catalog_id.clone())
            .unwrap_or_default(),
        proposal_catalog_id: after
            .map(|detail| detail.catalog_id.clone())
            .unwrap_or_default(),
        outcome: outcome.into(),
        changes,
        limitations,
        ambiguous_groups: 0,
        possible_callers: vec![],
        correspondence: vec![],
    }
}
