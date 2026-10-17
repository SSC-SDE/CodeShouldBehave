use super::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn semantic_element(element: &DesignElement) -> serde_json::Value {
    // Provenance blobs/spans and observed source presence are not architecture
    // semantics. A private body change creates a new immutable snapshot but
    // does not itself change this public-contract/dependency profile.
    serde_json::json!({"kind":element.kind,"name":element.name,"component_id":element.component_id,"roots":element.roots,
        "responsibility":element.responsibility,"user_assertions":element.user_assertions,
        "facts":element.facts.iter().filter(|fact|fact.kind!="source_presence" && fact.resolution!=DesignResolution::Unknown).map(|fact|serde_json::json!({"kind":fact.kind,"value":fact.value})).collect::<Vec<_>>()})
}
fn semantic_relation(relation: &DesignRelation) -> serde_json::Value {
    serde_json::json!({"from":relation.from,"to":relation.to,"kind":relation.kind,"direction":relation.direction,"protocol":relation.protocol,"synchrony":relation.synchrony})
}
fn fact_semantics(element: &DesignElement) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut facts: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for fact in &element.facts {
        if fact.kind != "source_presence"
            && matches!(
                fact.resolution,
                DesignResolution::ObservedStructure
                    | DesignResolution::ResolvedReference
                    | DesignResolution::UserAssertion
            )
        {
            facts.entry(&fact.kind).or_default().insert(&fact.value);
        }
    }
    facts
}
fn proven_modification(
    before: &DesignElement,
    after: &DesignElement,
    baseline_complete: bool,
    proposal_complete: bool,
) -> bool {
    if (
        &before.kind,
        &before.name,
        &before.component_id,
        &before.roots,
        &before.responsibility,
        &before.user_assertions,
    ) != (
        &after.kind,
        &after.name,
        &after.component_id,
        &after.roots,
        &after.responsibility,
        &after.user_assertions,
    ) {
        return true;
    }
    let old = fact_semantics(before);
    let new = fact_semantics(after);
    for kind in old
        .keys()
        .chain(new.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let left = old.get(kind);
        let right = new.get(kind);
        if left == right {
            continue;
        }
        // A positive observed value on each side establishes a replacement
        // only for a unique field. Missing collection members under partial
        // coverage never establish a removal or exhaustive replacement.
        if left
            .zip(right)
            .is_some_and(|(left, right)| left.len() == 1 && right.len() == 1)
        {
            return true;
        }
        if baseline_complete
            && right.is_some_and(|values| {
                values
                    .iter()
                    .any(|value| left.is_none_or(|old| !old.contains(value)))
            })
        {
            return true;
        }
        if proposal_complete
            && left.is_some_and(|values| {
                values
                    .iter()
                    .any(|value| right.is_none_or(|new| !new.contains(value)))
            })
        {
            return true;
        }
    }
    false
}
pub fn compare_hld(
    baseline: &DesignVersion,
    proposal: &DesignVersion,
) -> Result<DesignDelta, String> {
    if baseline.repository != proposal.repository
        || baseline.scope_fingerprint != proposal.scope_fingerprint
        || baseline.generator_version != proposal.generator_version
    {
        return Err(
            "HLD comparison requires the same repository, evidence scope and generator profile"
                .into(),
        );
    }
    let old = baseline
        .elements
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let new = proposal
        .elements
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let mut changes = Vec::new();
    let mut questions = BTreeSet::new();
    let mut changed = BTreeSet::new();
    for id in old
        .keys()
        .chain(new.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let before = old.get(id).copied();
        let after = new.get(id).copied();
        let kind = match (before, after) {
            (None, Some(_)) => {
                if baseline.coverage.complete {
                    "added"
                } else {
                    "uncertain"
                }
            }
            (Some(_), None) => {
                if proposal.coverage.complete {
                    "removed"
                } else {
                    "uncertain"
                }
            }
            (Some(before), Some(after)) if semantic_element(before) != semantic_element(after) => {
                if !proven_modification(
                    before,
                    after,
                    baseline.coverage.complete,
                    proposal.coverage.complete,
                ) {
                    "uncertain"
                } else if before.roots != after.roots && before.kind == DesignElementKind::Component
                {
                    "moved"
                } else {
                    "modified"
                }
            }
            _ => continue,
        };
        if kind != "uncertain" {
            changed.insert(id.to_string());
            for element in before.into_iter().chain(after) {
                if element.kind == DesignElementKind::Contract {
                    if let Some(owner) = &element.component_id {
                        changed.insert(owner.clone());
                    }
                }
            }
        } else {
            questions.insert("An element is absent or differs under incomplete coverage; addition, removal and replacement cannot be established from missing evidence".into());
        }
        let mut evidence = before
            .into_iter()
            .chain(after)
            .flat_map(|item| item.evidence_ids.clone())
            .collect::<Vec<_>>();
        evidence.sort();
        evidence.dedup();
        changes.push(DesignChange{id:format!("change-{}",digest(&[id,kind])),kind:kind.into(),element_id:Some(id.into()),relation_id:None,
            before_name:before.map(|item|item.name.clone()),after_name:after.map(|item|item.name.clone()),evidence_ids:evidence,
            reason:if kind=="uncertain"{"Incomplete input leaves this element difference unresolved; it is not an observed architectural addition or removal"}else{match after.or(before).map(|item|&item.kind){Some(DesignElementKind::Component)=>"Confirmed component interpretation, boundary or observable structural declaration changed",
                Some(DesignElementKind::Contract)=>"Rust public declaration signature or selected contract inventory changed; compatibility and runtime effects require review",
                Some(DesignElementKind::Dependency)=>"Selected package dependency declaration changed; this does not establish a new running service",
                Some(DesignElementKind::Deployment)=>"Static deployment configuration changed; active hosting state remains unverified",
                Some(DesignElementKind::Schema)=>"Selected schema artifact fingerprint changed; data ownership and compatibility remain unverified",None=>"Selected design element changed"}}.into()});
    }
    let mut corresponded = BTreeSet::new();
    for correspondence in &proposal.correspondence {
        let cardinality = match correspondence.kind.as_str() {
            "rename" | "move" => {
                correspondence.from_ids.len() == 1 && correspondence.to_ids.len() == 1
            }
            "split" => correspondence.from_ids.len() == 1 && correspondence.to_ids.len() > 1,
            "merge" => correspondence.from_ids.len() > 1 && correspondence.to_ids.len() == 1,
            _ => false,
        };
        let usable = cardinality
            && baseline.coverage.complete
            && proposal.coverage.complete
            && correspondence.resolution == DesignResolution::UserAssertion
            && correspondence.from_ids.iter().all(|id| {
                old.contains_key(id.as_str())
                    && !new.contains_key(id.as_str())
                    && !corresponded.contains(id)
            })
            && correspondence.to_ids.iter().all(|id| {
                new.contains_key(id.as_str())
                    && !old.contains_key(id.as_str())
                    && !corresponded.contains(id)
            });
        if !usable {
            questions.insert("A proposed correspondence is ambiguous, overlaps another mapping or does not match actual removed/added identities; source facts remain unchanged".into());
            continue;
        }
        let mut evidence = correspondence
            .from_ids
            .iter()
            .flat_map(|id| old[id.as_str()].evidence_ids.iter().cloned())
            .chain(
                correspondence
                    .to_ids
                    .iter()
                    .flat_map(|id| new[id.as_str()].evidence_ids.iter().cloned()),
            )
            .collect::<Vec<_>>();
        evidence.sort();
        evidence.dedup();
        changes.retain(|change| {
            change.element_id.as_ref().is_none_or(|id| {
                !correspondence.from_ids.contains(id) && !correspondence.to_ids.contains(id)
            })
        });
        for id in &correspondence.to_ids {
            changes.push(DesignChange{id:format!("change-{}",digest(&[id,&correspondence.kind])),kind:match correspondence.kind.as_str(){"rename"=>"modified","move"=>"moved","split"=>"split",_=>"merged"}.into(),element_id:Some(id.clone()),relation_id:None,
                before_name:Some(correspondence.from_ids.iter().map(|id|old[id.as_str()].name.clone()).collect::<Vec<_>>().join(", ")),after_name:Some(new[id.as_str()].name.clone()),evidence_ids:evidence.clone(),
                reason:format!("Explicit reviewer {} correspondence connects these authorized source identities; source observations are preserved and behavioral equivalence is not inferred",correspondence.kind)});
        }
        corresponded.extend(
            correspondence
                .from_ids
                .iter()
                .chain(&correspondence.to_ids)
                .cloned(),
        );
    }
    let old_rel = baseline
        .relations
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let new_rel = proposal
        .relations
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    for id in old_rel
        .keys()
        .chain(new_rel.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let before = old_rel.get(id).copied();
        let after = new_rel.get(id).copied();
        if before.zip(after).is_some_and(|(left, right)| {
            left.resolution != right.resolution
                && semantic_relation(left) == semantic_relation(right)
        }) {
            questions.insert("Relationship evidence resolution changed without a structural direction change; certainty is not an architectural modification".into());
        }
        let kind = match (before, after) {
            (None, Some(_)) => {
                if baseline.coverage.complete {
                    "added"
                } else {
                    "uncertain"
                }
            }
            (Some(_), None) => {
                if proposal.coverage.complete {
                    "removed"
                } else {
                    "uncertain"
                }
            }
            (Some(before), Some(after))
                if semantic_relation(before) != semantic_relation(after) =>
            {
                "modified"
            }
            _ => continue,
        };
        let mut evidence = before
            .into_iter()
            .chain(after)
            .flat_map(|item| item.evidence_ids.clone())
            .collect::<Vec<_>>();
        evidence.sort();
        evidence.dedup();
        for relation in before.into_iter().chain(after) {
            if kind != "uncertain" {
                changed.insert(relation.to.clone());
            }
        }
        changes.push(DesignChange{id:format!("change-{}",digest(&[id,kind])),kind:kind.into(),element_id:None,relation_id:Some(id.into()),before_name:before.map(|item|item.kind.clone()),after_name:after.map(|item|item.kind.clone()),evidence_ids:evidence,
            reason:if kind=="uncertain"{"Incomplete input leaves this relationship difference unresolved; absence is not evidence of removal"}else{"Evidence-backed declaration or static package dependency direction changed; no runtime call graph is claimed"}.into()});
    }
    let removed = changes
        .iter()
        .filter(|item| item.kind == "removed")
        .filter_map(|item| item.element_id.as_ref())
        .filter_map(|id| old.get(id.as_str()))
        .collect::<Vec<_>>();
    let added = changes
        .iter()
        .filter(|item| item.kind == "added")
        .filter_map(|item| item.element_id.as_ref())
        .filter_map(|id| new.get(id.as_str()))
        .collect::<Vec<_>>();
    for before in &removed {
        if added
            .iter()
            .any(|after| before.name == after.name && before.kind == after.kind)
        {
            questions.insert("Same-named elements were removed and added at different identities; rename/move/split correspondence requires explicit reviewer confirmation".to_string());
        }
    }
    let mut reverse: BTreeMap<&str, Vec<&DesignRelation>> = BTreeMap::new();
    for relation in proposal.relations.iter().chain(&baseline.relations) {
        if matches!(
            relation.resolution,
            DesignResolution::ResolvedReference | DesignResolution::ObservedStructure
        ) {
            reverse.entry(&relation.to).or_default().push(relation);
        }
    }
    let components = baseline
        .elements
        .iter()
        .chain(&proposal.elements)
        .filter(|item| item.kind == DesignElementKind::Component)
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut impacts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut queue = VecDeque::new();
    let mut seen = BTreeSet::new();
    for item in &changed {
        queue.push_back((item.clone(), Vec::<String>::new(), 0));
    }
    let mut bounded = false;
    while let Some((item, path, depth)) = queue.pop_front() {
        if !seen.insert(item.clone()) {
            continue;
        }
        if seen.len() > 1000 || depth >= 4 {
            bounded = true;
            continue;
        }
        if let Some(relations) = reverse.get(item.as_str()) {
            for relation in relations {
                let mut path = path.clone();
                path.push(relation.id.clone());
                if components.contains(relation.from.as_str()) && !changed.contains(&relation.from)
                {
                    impacts
                        .entry(relation.from.clone())
                        .or_default()
                        .extend(path.clone());
                }
                queue.push_back((relation.from.clone(), path, depth + 1));
            }
        }
    }
    if bounded {
        questions.insert("Reverse dependency impact traversal reached four levels or 1000 nodes; additional downstream impact remains unknown".into());
    }
    let mut coverage = proposal.coverage.clone();
    coverage.complete &= baseline.coverage.complete && !bounded;
    if baseline
        .relations
        .iter()
        .chain(&proposal.relations)
        .any(|relation| {
            matches!(
                relation.resolution,
                DesignResolution::NameOnlyCandidate | DesignResolution::Unknown
            )
        })
    {
        coverage.complete = false;
        questions.insert(
            "A candidate or unknown relationship prevents a complete architectural comparison"
                .into(),
        );
    }
    coverage.enumeration_complete &= baseline.coverage.enumeration_complete;
    coverage.gaps.extend(baseline.coverage.gaps.clone());
    coverage.gaps.sort();
    coverage.gaps.dedup();
    for gap in &coverage.gaps {
        questions.insert(gap.clone());
    }
    let findings = evaluate_hld_constraints(proposal, &baseline.constraints);
    let missing_side = (baseline.evidence.is_empty() && !baseline.coverage.enumeration_complete)
        || (proposal.evidence.is_empty() && !proposal.coverage.enumeration_complete);
    let outcome = if !missing_side && changes.iter().any(|change| change.kind != "uncertain") {
        DesignOutcome::ChangeDetected
    } else if coverage.complete {
        DesignOutcome::NoChangeDetectedWithinScope
    } else {
        DesignOutcome::Incomplete
    };
    let id = format!(
        "delta-{}",
        digest(&[&baseline.id, &proposal.id, HLD_GENERATOR_VERSION])
    );
    let mermaid = super::render::render_delta_mermaid(baseline, proposal, &changes, &impacts);
    Ok(DesignDelta{id,baseline_design_id:baseline.id.clone(),proposal_design_id:proposal.id.clone(),baseline_commit:baseline.snapshot.as_str().into(),proposal_commit:proposal.snapshot.as_str().into(),outcome,changes,
        possible_impacts:impacts.into_iter().map(|(component_id,path)|DesignPossibleImpact{component_id,via_relation_ids:path.into_iter().collect(),reason:"An unchanged selected component depends on a changed declaration/component along these static relationships; runtime consequences require verification".into()}).collect(),
        questions:questions.into_iter().collect(),findings,coverage,mermaid})
}
pub fn evaluate_hld_constraints(
    proposal: &DesignVersion,
    constraints: &[DesignConstraint],
) -> Vec<DesignFinding> {
    let mut findings = Vec::new();
    for constraint in constraints.iter().take(100) {
        match constraint.kind.as_str(){
            "prohibited_dependency"=>{
                let matched=proposal.relations.iter().filter(|relation|relation.from==constraint.from_component_id && constraint.to_component_id.as_ref()==Some(&relation.to) && relation.kind=="local_package_dependency").collect::<Vec<_>>();
                if !matched.is_empty(){findings.push(DesignFinding{constraint_id:constraint.id.clone(),resolution:DesignResolution::ObservedStructure,message:"A selected manifest declares a component dependency prohibited by this user-confirmed rule; this proves a static dependency declaration, not an outage or runtime call".into(),evidence_ids:matched.into_iter().flat_map(|relation|relation.evidence_ids.clone()).collect()});}
                else if !proposal.coverage.complete{findings.push(DesignFinding{constraint_id:constraint.id.clone(),resolution:DesignResolution::Unknown,message:"The prohibited-dependency rule could not be evaluated completely under current coverage".into(),evidence_ids:vec![]});}
            },
            "required_contract"=>{
                let mut expected=constraint.to_component_id.clone();
                if expected.is_none(){findings.push(DesignFinding{constraint_id:constraint.id.clone(),resolution:DesignResolution::Unknown,message:"Required-contract rule has no selected logical contract identity; rationale text is not an identity selector".into(),evidence_ids:constraint.evidence_ids.clone()});continue;}
                if let Some(id)=expected.as_ref(){let matches=proposal.correspondence.iter().filter(|item|item.resolution==DesignResolution::UserAssertion&&matches!(item.kind.as_str(),"rename"|"move")&&item.from_ids==vec![id.clone()]&&item.to_ids.len()==1).collect::<Vec<_>>();if matches.len()==1{expected=matches[0].to_ids.first().cloned();}}
                let present=proposal.elements.iter().any(|element|element.kind==DesignElementKind::Contract && element.component_id.as_deref()==Some(&constraint.from_component_id) && expected.as_ref()==Some(&element.id));
                if !present{findings.push(DesignFinding{constraint_id:constraint.id.clone(),resolution:if proposal.coverage.complete{DesignResolution::ObservedStructure}else{DesignResolution::Unknown},message:if proposal.coverage.complete{"The required public contract was not observed in the selected complete static profile"}else{"The required public contract is not observed, but incomplete coverage prevents a definitive violation"}.into(),evidence_ids:constraint.evidence_ids.clone()});}
            },_=>findings.push(DesignFinding{constraint_id:constraint.id.clone(),resolution:DesignResolution::Unknown,message:"This user-confirmed constraint is outside the supported deterministic rule profile".into(),evidence_ids:constraint.evidence_ids.clone()}),
        }
    }
    findings
}
