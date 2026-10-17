//! Private operator bridge. Browser input selects immutable IDs, never files.
use flowdiff::design::{DesignVersion, validate_design_scope};
use flowdiff::design_storage::{DesignCorrection, DesignInterpretation};
use flowdiff::lld::{
    LldEntry, LldFunctionDetail, LldReview, LldSnapshot, build_lld_detail, compare_lld,
    compare_lld_detail, lld_detail_design, lld_source,
};
use flowdiff::{Cancellation, Grant, PrincipalId, RepositoryId, Storage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

fn field<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request[key]
        .as_str()
        .ok_or_else(|| format!("Missing LLD {key}"))
}
fn fingerprint(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn revision(request: &Value) -> Result<u64, String> {
    request["expected_revision"]
        .as_u64()
        .ok_or("Missing LLD preview revision".into())
}
fn matches_component(entry: &LldEntry, component: &str) -> bool {
    component.is_empty()
        || entry.parent.component_id.as_deref() == Some(component)
        || entry
            .parent
            .candidate_component_ids
            .iter()
            .any(|id| id == component)
}
fn selected_pair<'a>(
    pair: &'a LldReview,
    entry: &str,
    baseline_override: Option<&str>,
) -> Result<(Option<&'a LldEntry>, Option<&'a LldEntry>), String> {
    let before = pair
        .baseline
        .catalog
        .entries
        .iter()
        .find(|item| item.id == entry);
    let after = pair
        .proposal
        .catalog
        .entries
        .iter()
        .find(|item| item.id == entry);
    let selected = before.or(after).ok_or("Unknown immutable LLD entry")?;
    if let Some(id) = baseline_override.filter(|id| !id.is_empty()) {
        let selected_proposal =
            after.ok_or("Declared counterpart requires a proposal occurrence")?;
        let baseline = pair
            .baseline
            .catalog
            .entries
            .iter()
            .find(|item| item.id == id)
            .ok_or("Declared baseline entry is not in this comparison")?;
        if baseline.kind != selected_proposal.kind {
            return Err("Declared correspondence must preserve declaration kinds".into());
        }
        return Ok((Some(baseline), Some(selected_proposal)));
    }
    if pair
        .baseline
        .catalog
        .entries
        .iter()
        .filter(|item| item.logical_key == selected.logical_key)
        .count()
        > 1
        || pair
            .proposal
            .catalog
            .entries
            .iter()
            .filter(|item| item.logical_key == selected.logical_key)
            .count()
            > 1
    {
        // The exact selected occurrence remains inspectable; no invented
        // cross-revision correspondence is attached to an ambiguous name group.
        return Ok((before, after));
    }
    if let Some(moved) = pair.delta.changes.iter().find(|change| {
        change.kind == "declaration_moved"
            && (change.baseline_entry_id.as_deref() == Some(entry)
                || change.proposal_entry_id.as_deref() == Some(entry))
    }) {
        return Ok((
            moved.baseline_entry_id.as_ref().and_then(|id| {
                pair.baseline
                    .catalog
                    .entries
                    .iter()
                    .find(|entry| entry.id == *id)
            }),
            moved.proposal_entry_id.as_ref().and_then(|id| {
                pair.proposal
                    .catalog
                    .entries
                    .iter()
                    .find(|entry| entry.id == *id)
            }),
        ));
    }
    let unique = |snapshot: &'a LldSnapshot| {
        let mut matches = snapshot
            .catalog
            .entries
            .iter()
            .filter(|item| item.logical_key == selected.logical_key);
        let first = matches.next();
        if matches.next().is_some() {
            None
        } else {
            first
        }
    };
    Ok((
        before.or_else(|| unique(&pair.baseline)),
        after.or_else(|| unique(&pair.proposal)),
    ))
}
fn details(
    pair: &LldReview,
    entry: &str,
    component: &str,
    baseline_override: Option<&str>,
) -> Result<(Option<LldFunctionDetail>, Option<LldFunctionDetail>), String> {
    let (before, after) = selected_pair(pair, entry, baseline_override)?;
    if before
        .into_iter()
        .chain(after)
        .any(|item| !matches_component(item, component))
    {
        return Err("LLD selection is outside its HLD component".into());
    }
    let cancellation = Cancellation::default();
    let mut result = (
        before
            .map(|item| build_lld_detail(&pair.baseline, &item.id, &cancellation))
            .transpose()?,
        after
            .map(|item| build_lld_detail(&pair.proposal, &item.id, &cancellation))
            .transpose()?,
    );
    let duplicate = before.or(after).is_some_and(|selected| {
        pair.baseline
            .catalog
            .entries
            .iter()
            .filter(|item| item.logical_key == selected.logical_key)
            .count()
            > 1
            || pair
                .proposal
                .catalog
                .entries
                .iter()
                .filter(|item| item.logical_key == selected.logical_key)
                .count()
                > 1
    });
    let missing_is_uncertain = (before.is_none()
        && (!pair.baseline.catalog.coverage.inventory_complete
            || !pair.baseline.catalog.coverage.structure_complete))
        || (after.is_none()
            && (!pair.proposal.catalog.coverage.inventory_complete
                || !pair.proposal.catalog.coverage.structure_complete));
    if (duplicate && baseline_override.filter(|id| !id.is_empty()).is_none())
        || missing_is_uncertain
    {
        for detail in result.0.iter_mut().chain(result.1.iter_mut()) {
            detail.coverage.structure_complete = false;
            detail.coverage.gaps.push("Cross-revision correspondence is ambiguous or counterpart inventory is incomplete; absence does not establish addition/removal".into());
        }
    }
    Ok(result)
}

pub fn invoke(
    storage: &mut Storage,
    repository: &RepositoryId,
    principal: &PrincipalId,
    grants: &[Grant],
    action: &str,
    request: Value,
) -> Result<Value, String> {
    let value = super::hld::operator_artifact(Path::new(field(&request, "artifact_path")?))?;
    let comparison = field(&request, "comparison_id")?;
    let (_, digest) = comparison
        .strip_prefix("pr-")
        .and_then(|id| id.split_once('-'))
        .ok_or("Invalid LLD comparison")?;
    if value["artifact_version"] != "csb-review-artifact-v1" || value["review_id"] != digest {
        return Err("LLD comparison mismatch".into());
    }
    let scope = super::hld::candidate_scope(&value, repository)?;
    // The existing shared design boundary intersects current grants, accepted
    // context and immutable scope before any retained source/detail is returned.
    let baseline_hld: DesignVersion = serde_json::from_value(value["hld"]["baseline"].clone())
        .map_err(|_| "LLD HLD parent unavailable")?;
    let proposal_hld: DesignVersion = serde_json::from_value(value["hld"]["proposal"].clone())
        .map_err(|_| "LLD HLD parent unavailable")?;
    if storage.load_design_version(repository, &baseline_hld.id, principal, grants)? != baseline_hld
        || storage.load_design_version(repository, &proposal_hld.id, principal, grants)?
            != proposal_hld
    {
        return Err("LLD parent differs from its immutable authorized design".into());
    }
    let parent = field(&request, "parent_hld")?;
    if parent != baseline_hld.id && parent != proposal_hld.id {
        return Err("LLD parent version mismatch".into());
    }
    let component = request["component"].as_str().unwrap_or("");
    if !component.is_empty()
        && !baseline_hld
            .elements
            .iter()
            .chain(&proposal_hld.elements)
            .any(|item| {
                item.id == component && item.kind == flowdiff::design::DesignElementKind::Component
            })
    {
        return Err("Unknown HLD component".into());
    }
    let pair: LldReview = serde_json::from_value(value["lld"].clone())
        .map_err(|_| "LLD catalog unavailable; regenerate review")?;
    for (snapshot, hld, commit) in [
        (&pair.baseline, &baseline_hld, "baseline"),
        (&pair.proposal, &proposal_hld, "proposal"),
    ] {
        flowdiff::lld::validate_lld_snapshot(snapshot, hld, &scope)?;
        if snapshot.catalog.hld_design_id != hld.id
            || snapshot.catalog.snapshot.as_str() != value[commit].as_str().unwrap_or("")
            || snapshot.catalog.onboarding_input_fingerprint != hld.onboarding_input_fingerprint
            || snapshot.catalog.onboarding_proposal_fingerprint
                != hld.onboarding_proposal_fingerprint
        {
            return Err("LLD immutable parent/context mismatch".into());
        }
    }
    if pair.delta != compare_lld(&pair.baseline.catalog, &pair.proposal.catalog) {
        return Err("LLD immutable catalog delta identity mismatch".into());
    }
    let identity = json!({"scope_version":scope.version.as_str(),"parent_hld":parent,"component":component,"baseline_entry":request["baseline_entry"],"correspondence_kind":request["correspondence_kind"]});
    if action == "lld-catalog" {
        let mut delta = compare_lld(&pair.baseline.catalog, &pair.proposal.catalog);
        if !component.is_empty() {
            delta.changes.retain(|change| {
                change.before_parent.as_ref().is_some_and(|parent| {
                    parent.component_id.as_deref() == Some(component)
                        || parent
                            .candidate_component_ids
                            .iter()
                            .any(|id| id == component)
                }) || change.after_parent.as_ref().is_some_and(|parent| {
                    parent.component_id.as_deref() == Some(component)
                        || parent
                            .candidate_component_ids
                            .iter()
                            .any(|id| id == component)
                })
            });
            delta.possible_callers.retain(|change| {
                change
                    .after_parent
                    .as_ref()
                    .is_some_and(|parent| parent.component_id.as_deref() == Some(component))
            });
            if delta.changes.is_empty() && delta.outcome == "change_detected" {
                delta.outcome = "incomplete".into();
                delta.limitations.push("No structural changes in this selected component; local details and unresolved coverage still require review".into());
            }
        }
        if !component.is_empty() {
            delta.id = format!(
                "lld-component-delta-{}",
                fingerprint(&format!(
                    "{}:{component}:{}",
                    delta.id,
                    serde_json::to_string(&delta.changes)
                        .map_err(|_| "Cannot encode component delta")?
                ))
            );
        }
        let mut rows = Vec::new();
        let mut consumed = std::collections::BTreeSet::new();
        for entry in &pair.proposal.catalog.entries {
            if !matches_component(entry, component) {
                continue;
            }
            let (before, after) = selected_pair(&pair, &entry.id, None)?;
            if let Some(before) = before {
                consumed.insert(before.id.clone());
            }
            let status = if before.is_none() {
                "added_or_unmatched"
            } else if before.is_some_and(|b| b.path != entry.path) {
                "moved"
            } else if before.is_some_and(|b| {
                b.signature_fingerprint != entry.signature_fingerprint
                    || b.body_fingerprint != entry.body_fingerprint
            }) {
                "modified"
            } else {
                "unchanged"
            };
            rows.push(json!({"baseline":before,"proposal":after,"status":status}));
        }
        for entry in &pair.baseline.catalog.entries {
            if !consumed.contains(&entry.id) && matches_component(entry, component) {
                rows.push(
                    json!({"baseline":entry,"proposal":null,"status":"removed_or_unmatched"}),
                );
            }
        }
        rows.sort_by_key(|row| {
            let entry = if row["proposal"].is_object() {
                &row["proposal"]
            } else {
                &row["baseline"]
            };
            (
                if row["status"] == "unchanged" { 1 } else { 0 },
                entry["path"].as_str().unwrap_or("").to_owned(),
                entry["start_byte"].as_u64().unwrap_or(0),
                entry["kind"].as_str().unwrap_or("").to_owned(),
                entry["id"].as_str().unwrap_or("").to_owned(),
            )
        });
        let page_size = request["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
        let binding = fingerprint(&format!(
            "{}:{}:{}:{}:{}",
            comparison, pair.baseline.catalog.id, pair.proposal.catalog.id, parent, component
        ));
        let start = match request["cursor"]
            .as_str()
            .filter(|cursor| !cursor.is_empty())
        {
            None => 0,
            Some(cursor) => {
                let (key, offset) = cursor.split_once(':').ok_or("Invalid LLD cursor")?;
                if key != binding {
                    return Err("Stale LLD catalog cursor".into());
                }
                offset.parse::<usize>().map_err(|_| "Invalid LLD cursor")?
            }
        };
        if start > rows.len() {
            return Err("Invalid LLD page offset".into());
        }
        let end = start.saturating_add(page_size).min(rows.len());
        let mut before =
            serde_json::to_value(&pair.baseline.catalog).map_err(|_| "Invalid LLD catalog")?;
        let mut after =
            serde_json::to_value(&pair.proposal.catalog).map_err(|_| "Invalid LLD catalog")?;
        before["entries"] = json!([]);
        before["evidence"] = json!([]);
        after["entries"] = json!([]);
        after["evidence"] = json!([]);
        return Ok(
            json!({"baseline":before,"proposal":after,"delta":delta,"entries":rows[start..end],"next_cursor":(end<rows.len()).then(||format!("{binding}:{end}")),"total":rows.len(),"identity":identity}),
        );
    }
    let entry = field(&request, "entry")?;
    if action == "lld-history" {
        let (before, after) = selected_pair(&pair, entry, request["baseline_entry"].as_str())?;
        let selected = after.or(before).ok_or("Unknown LLD history entry")?;
        if !matches_component(selected, component) {
            return Err("LLD history component mismatch".into());
        }
        let accepted = storage
            .load_lld_design_default(
                repository,
                &selected.logical_key,
                DesignInterpretation::AsBuilt,
                principal,
                grants,
            )
            .ok();
        let intended = storage
            .load_lld_design_default(
                repository,
                &selected.logical_key,
                DesignInterpretation::Intended,
                principal,
                grants,
            )
            .ok();
        return Ok(
            json!({"entry_id":entry,"accepted":accepted,"accepted_intended":intended,"current_behavior":"unavailable_without_new_detail_generation","identity":identity}),
        );
    }
    let (before, after) = details(&pair, entry, component, request["baseline_entry"].as_str())?;
    let mut delta = compare_lld_detail(before.as_ref(), after.as_ref());
    delta.limitations.extend(
        before
            .iter()
            .chain(after.iter())
            .flat_map(|detail| detail.coverage.gaps.iter().cloned()),
    );
    let selected_ids = before
        .iter()
        .chain(after.iter())
        .map(|detail| detail.entry.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let changed_names = delta
        .changes
        .iter()
        .filter_map(|change| change.proposal_entry_id.as_ref())
        .filter_map(|id| {
            pair.proposal
                .catalog
                .entries
                .iter()
                .find(|entry| entry.id == *id)
        })
        .map(|entry| entry.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    delta.possible_callers = compare_lld(&pair.baseline.catalog, &pair.proposal.catalog)
        .possible_callers
        .into_iter()
        .filter(|caller| {
            caller
                .proposal_entry_id
                .as_ref()
                .and_then(|id| {
                    pair.proposal
                        .catalog
                        .entries
                        .iter()
                        .find(|entry| entry.id == *id)
                })
                .is_some_and(|entry| {
                    !selected_ids.contains(entry.id.as_str())
                        && entry.call_observations.iter().any(|call| {
                            changed_names.iter().any(|name| {
                                call.rsplit("::")
                                    .next()
                                    .unwrap_or(call)
                                    .rsplit('.')
                                    .next()
                                    .unwrap_or(call)
                                    == *name
                            })
                        })
                })
        })
        .collect();
    if request["baseline_entry"]
        .as_str()
        .is_some_and(|id| !id.is_empty())
    {
        let before = before
            .as_ref()
            .ok_or("Declared baseline detail unavailable")?;
        let after = after
            .as_ref()
            .ok_or("Declared proposal detail unavailable")?;
        let correspondence = flowdiff::design::DesignCorrespondence {
            kind: request["correspondence_kind"]
                .as_str()
                .unwrap_or("rename")
                .into(),
            from_ids: vec![before.entry.id.clone()],
            to_ids: vec![after.entry.id.clone()],
            resolution: flowdiff::design::DesignResolution::UserAssertion,
        };
        let declared = flowdiff::lld::compare_lld_with_correspondence(
            &pair.baseline.catalog,
            &pair.proposal.catalog,
            std::slice::from_ref(&correspondence),
        )?;
        delta.correspondence = declared.correspondence;
        delta.changes.extend(
            declared
                .changes
                .into_iter()
                .filter(|change| change.resolution == "user_assertion"),
        );
        delta.limitations.push("Reviewer-declared baseline correspondence; paired syntax differences do not establish behavioral equivalence".into());
    }
    delta.limitations.sort();
    delta.limitations.dedup();
    if action == "lld-source" {
        let evidence_id = field(&request, "evidence")?;
        for (detail, snapshot) in [(&before, &pair.baseline), (&after, &pair.proposal)] {
            let Some(detail) = detail else {
                continue;
            };
            let Some(evidence) = detail
                .evidence
                .iter()
                .find(|evidence| evidence.id == evidence_id)
            else {
                continue;
            };
            let start = request["start_line"]
                .as_u64()
                .unwrap_or(evidence.start_line as u64) as usize;
            let end = request["end_line"]
                .as_u64()
                .unwrap_or(evidence.end_line.min(start.saturating_add(199)) as u64)
                as usize;
            if start < evidence.start_line
                || end > evidence.end_line
                || start > end
                || end - start >= 200
            {
                return Err("LLD source span exceeds evidence bounds".into());
            }
            let mut requested = evidence.clone();
            requested.start_line = start;
            requested.end_line = end;
            let text = lld_source(snapshot, &requested)?;
            if text.split('\n').count() != end - start + 1 {
                return Err("Exact LLD source span missing".into());
            }
            if text.len() > 65536 {
                return Err("LLD source span exceeds byte budget".into());
            }
            return Ok(
                json!({"evidence_id":evidence.id,"commit":evidence.commit.as_str(),"path":evidence.path,"blob_id":evidence.blob,"start_line":start,"end_line":end,"text":text,"truncated":end<evidence.end_line,"identity":identity}),
            );
        }
        return Err("Exact LLD evidence unavailable".into());
    }
    let selected = after
        .as_ref()
        .or(before.as_ref())
        .ok_or("LLD selected detail unavailable")?;
    let key = &selected.entry.logical_key;
    let accepted = storage
        .load_lld_design_default(
            repository,
            key,
            DesignInterpretation::AsBuilt,
            principal,
            grants,
        )
        .ok();
    let intended = storage
        .load_lld_design_default(
            repository,
            key,
            DesignInterpretation::Intended,
            principal,
            grants,
        )
        .ok();
    if action == "lld-detail" {
        let drift = accepted.as_ref().and_then(|record|record.observed_design.lld.as_ref()).map(|data| json!({"body_changed": before.as_ref().is_none_or(|detail|data.behavior_fingerprint != detail.behavior_fingerprint),"invariants":accepted.as_ref().and_then(|record|record.design.lld.as_ref()).map(|data|&data.invariants),"verification":"requires_reviewer_verification"}));
        return Ok(
            json!({"entry_id":entry,"baseline":before,"proposal":after,"delta":delta,"accepted":accepted,"accepted_intended":intended,"accepted_drift":drift,"identity":identity}),
        );
    }
    let at = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Invalid clock")?
            .as_millis(),
    )
    .map_err(|_| "Clock overflow")?;
    let preview_prefix = format!("lld-review-{}-", &fingerprint(comparison)[..32]);
    if matches!(action, "lld-preview" | "lld-accept") {
        let suffix = field(&request, "preview_id")?
            .strip_prefix(&preview_prefix)
            .ok_or("LLD preview comparison mismatch")?;
        if suffix.len() != 32 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid LLD preview identity".into());
        }
        let candidates = before
            .as_ref()
            .map(|detail| lld_detail_design(&pair.baseline, detail))
            .transpose()?
            .into_iter()
            .chain(
                after
                    .as_ref()
                    .map(|detail| lld_detail_design(&pair.proposal, detail))
                    .transpose()?,
            )
            .collect::<Vec<_>>();
        for design in &candidates {
            validate_design_scope(design, &scope)?;
            if storage
                .load_design_version(repository, &design.id, principal, grants)
                .is_err()
            {
                storage.save_design_version(design, &scope, principal, grants, at)?;
            }
        }
        let id = field(&request, "preview_id")?;
        if action == "lld-preview" {
            let design_id = field(&request, "design_id")?;
            if !candidates.iter().any(|design| design.id == design_id) {
                return Err("LLD candidate is outside selected detail".into());
            }
            let interpretation: DesignInterpretation =
                serde_json::from_value(request["interpretation"].clone())
                    .map_err(|_| "Invalid LLD interpretation")?;
            let corrections: Vec<DesignCorrection> =
                serde_json::from_value(request["corrections"].clone())
                    .map_err(|_| "Invalid LLD invariant")?;
            if corrections.iter().any(|correction| {
                correction
                    .reference_design_id
                    .as_ref()
                    .is_some_and(|id| !candidates.iter().any(|design| design.id == *id))
            }) {
                return Err(
                    "LLD correspondence reference is outside this selected comparison".into(),
                );
            }
            for correction in corrections.iter().filter(|correction| {
                correction.kind == flowdiff::design_storage::DesignCorrectionKind::Correspondence
            }) {
                if before.as_ref().is_none_or(|detail| {
                    correction.reference_design_id.as_deref() != Some(detail.id.as_str())
                }) || after.as_ref().is_none_or(|detail| design_id != detail.id)
                {
                    return Err(
                        "LLD correspondence requires exact baseline-to-proposal direction".into(),
                    );
                }
                let declared = flowdiff::design::DesignCorrespondence {
                    kind: correction.value.clone(),
                    from_ids: correction.related_ids.clone(),
                    to_ids: correction.correspondence_targets.clone(),
                    resolution: flowdiff::design::DesignResolution::UserAssertion,
                };
                flowdiff::lld::compare_lld_with_correspondence(
                    &pair.baseline.catalog,
                    &pair.proposal.catalog,
                    &[declared],
                )?;
            }
            if request["create_preview"] == true {
                if revision(&request)? != 0 {
                    return Err("Stale initial LLD preview".into());
                }
                storage.create_design_review(
                    repository,
                    design_id,
                    id,
                    interpretation,
                    principal,
                    grants,
                    at,
                )?;
            } else if storage
                .load_design_review(repository, id, principal, grants)?
                .design_id
                != design_id
            {
                return Err("LLD preview candidate changed".into());
            }
            let review = storage.preview_design_review(
                repository,
                id,
                revision(&request)?,
                interpretation,
                corrections,
                principal,
                grants,
                at,
            )?;
            let design =
                storage.preview_design_interpretation(repository, id, principal, grants)?;
            return Ok(
                json!({"preview_id":review.id,"expected_revision":review.revision,"input_fingerprint":review.input_fingerprint,"review":review,"design":design,"identity":identity}),
            );
        }
        let review = storage.load_design_review(repository, id, principal, grants)?;
        if !candidates
            .iter()
            .any(|design| design.id == review.design_id)
        {
            return Err("LLD preview belongs to another selected body".into());
        }
        if review.corrections.iter().any(|correction| {
            correction
                .reference_design_id
                .as_ref()
                .is_some_and(|id| !candidates.iter().any(|design| design.id == *id))
        }) {
            return Err("LLD preview correspondence changed".into());
        }
        for correction in review.corrections.iter().filter(|correction| {
            correction.kind == flowdiff::design_storage::DesignCorrectionKind::Correspondence
        }) {
            if before.as_ref().is_none_or(|detail| {
                correction.reference_design_id.as_deref() != Some(detail.id.as_str())
            }) || after
                .as_ref()
                .is_none_or(|detail| review.design_id != detail.id)
            {
                return Err("LLD stored correspondence direction mismatch".into());
            }
        }
        let accepted = storage.confirm_design_review(
            repository,
            id,
            revision(&request)?,
            field(&request, "input_fingerprint")?,
            request["known_gaps_accepted"]
                .as_bool()
                .ok_or("Missing LLD gap decision")?,
            request["activate_default"]
                .as_bool()
                .ok_or("Missing LLD default decision")?,
            principal,
            grants,
            at,
        )?;
        return Ok(json!({"accepted":accepted,"identity":identity}));
    }
    if action == "lld-export" {
        // Both old and current consent are checked by the shared design boundary.
        for design in [&baseline_hld, &proposal_hld] {
            storage.export_design_version(repository, &design.id, principal, grants)?;
        }
        let view = request["view"].as_str().unwrap_or("diff");
        let markdown = match view {
            "baseline" => before
                .as_ref()
                .ok_or("LLD baseline unavailable")?
                .markdown
                .clone(),
            "proposal" => after
                .as_ref()
                .ok_or("LLD proposal unavailable")?
                .markdown
                .clone(),
            "diff" => format!(
                "# HLD / LLD impact\n\nHLD outcome: {}\nLLD outcome: {}\nBaseline: {}\nProposal: {}\n\n{}\n\n{}\n\n{}\n\n{}\n",
                value["hld_status"].as_str().unwrap_or("unavailable"),
                delta.outcome,
                pair.baseline.catalog.snapshot.as_str(),
                pair.proposal.catalog.snapshot.as_str(),
                baseline_hld.markdown,
                proposal_hld.markdown,
                before
                    .as_ref()
                    .map(|detail| detail.markdown.as_str())
                    .unwrap_or("Baseline detail unavailable"),
                after
                    .as_ref()
                    .map(|detail| detail.markdown.as_str())
                    .unwrap_or("Proposal detail unavailable")
            ),
            _ => return Err("Unknown LLD export view".into()),
        };
        if markdown.len() > 2 * 1024 * 1024 {
            return Err("LLD export byte limit".into());
        }
        return Ok(
            json!({"markdown":markdown,"filename":format!("codeshouldbehave-lld-{view}.md"),"identity":identity}),
        );
    }
    Err("Unknown LLD operation".into())
}
