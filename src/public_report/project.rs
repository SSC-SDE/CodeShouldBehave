use super::*;
use crate::design::{DesignElementKind, DesignEvidence, DesignResolution, DesignVersion};
use crate::lld::{LldCatalog, LldEntry, LldFunctionDetail, LldReview, LldSnapshot};
use crate::{Cancellation, PathScope, ScopeRecord, ScopeVersion};
use serde_json::json;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("Missing public input {key}"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("Missing public input {key}"))
}
fn select(value: &Value, fields: &[&str]) -> Value {
    let mut map = serde_json::Map::new();
    for field in fields {
        if let Some(value) = value.get(*field) {
            map.insert((*field).into(), value.clone());
        }
    }
    Value::Object(map)
}
fn evidence(value: &DesignEvidence) -> Value {
    json!({"id":value.id,"commit":value.commit.as_str(),"path":value.path,"blob":value.blob,
        "start_line":value.start_line,"end_line":value.end_line,"producer_version":value.producer_version,
        "relationship_type":value.relationship_type,"resolution":value.resolution})
}
fn gap_list(gaps: &[String]) -> Vec<String> {
    if gaps.is_empty() {
        vec![]
    } else {
        vec![format!(
            "{} bounded native analysis limitations; static coverage is incomplete",
            gaps.len()
        )]
    }
}
fn public_scope(version: &ScopeVersion) -> String {
    format!("public-scope-{}", sha256(version.as_str().as_bytes()))
}
fn lld_coverage(value: &crate::lld::LldCoverage) -> Value {
    json!({"inventory_complete":value.inventory_complete,"structure_complete":value.structure_complete,
        "local_flow":value.local_flow,"call_target_resolution":"unresolved_syntax_only","runtime_validation":"unavailable",
        "indexed":value.indexed,"unsupported":value.unsupported,"failed":value.failed,"deferred":value.deferred,"gaps":gap_list(&value.gaps)})
}
fn public_delta(delta: &crate::lld::LldDelta) -> Result<Value, String> {
    let mut value = serde_json::to_value(delta).map_err(|_| "Cannot project LLD delta")?;
    value["limitations"] = json!(gap_list(&delta.limitations));
    for field in ["changes", "possible_callers"] {
        if let Some(changes) = value[field].as_array_mut() {
            for change in changes {
                if let Some(limits) = change["limitations"].as_array() {
                    let count = limits.len();
                    change["limitations"] = if count == 0 {
                        json!([])
                    } else {
                        json!([format!("{count} bounded native change limitations")])
                    };
                }
            }
        }
    }
    Ok(value)
}
fn public_hld(version: &DesignVersion) -> Result<(Value, String, DesignVersion), String> {
    let mut clean = version.clone();
    clean.scope_version = ScopeVersion::new(public_scope(&version.scope_version))?;
    clean.onboarding_session_id.clear();
    clean.constraints.clear();
    clean.correspondence.clear();
    clean.document_candidates.clear();
    clean.decision_records.clear();
    for element in &mut clean.elements {
        element.user_assertions.clear();
        element.responsibility = None;
        if element.kind == DesignElementKind::Component {
            element.name = if element.roots.is_empty() {
                "Unmapped source component".into()
            } else if element.roots == ["."] {
                "Repository source".into()
            } else {
                element.roots.join(", ")
            };
        }
        element
            .facts
            .retain(|fact| fact.resolution != DesignResolution::UserAssertion);
        element.unknowns = gap_list(&element.unknowns);
    }
    clean
        .relations
        .retain(|relation| relation.resolution != DesignResolution::UserAssertion);
    let relations: BTreeSet<_> = clean.relations.iter().map(|r| r.id.clone()).collect();
    for relation in &mut clean.relations {
        relation.unknowns = gap_list(&relation.unknowns);
    }
    for section in &mut clean.sections {
        section.title = section.id.replace('_', " ");
        section.relation_ids.retain(|id| relations.contains(id));
        section.unknowns = gap_list(&section.unknowns);
    }
    clean.coverage.gaps = gap_list(&clean.coverage.gaps);
    clean.coverage.profile = crate::design::HLD_PROFILE.into();
    clean.mermaid = crate::design::render_hld_mermaid(&clean);
    clean.markdown = crate::design::render_hld_markdown(&clean);
    let value = serde_json::to_value(&clean).map_err(|_| "Cannot project HLD")?;
    let mut projected = select(
        &value,
        &[
            "id",
            "generator_version",
            "snapshot",
            "scope_version",
            "scope_fingerprint",
            "review_state",
            "view",
            "elements",
            "relations",
            "sections",
            "coverage",
            "mermaid",
            "markdown",
        ],
    );
    projected["evidence"] = Value::Array(clean.evidence.iter().map(evidence).collect());
    projected["constraints"] = json!([]);
    projected["correspondence"] = json!([]);
    projected["document_candidates"] = json!([]);
    projected["decision_records"] = json!([]);
    Ok((projected, clean.markdown.clone(), clean))
}
fn public_catalog(catalog: &LldCatalog, published: &BTreeSet<String>) -> Result<Value, String> {
    let value = serde_json::to_value(catalog).map_err(|_| "Cannot project catalog")?;
    let mut result = select(
        &value,
        &[
            "id",
            "generator_version",
            "parser_version",
            "profile",
            "snapshot",
            "scope_version",
            "scope_fingerprint",
            "hld_design_id",
            "coverage",
        ],
    );
    result["evidence"] = Value::Array(catalog.evidence.iter().map(evidence).collect());
    result["scope_version"] = json!(public_scope(&catalog.scope_version));
    result["coverage"] = lld_coverage(&catalog.coverage);
    result["coverage"]["local_flow"] = json!("static_precomputed_selection");
    result["entries"] = Value::Array(
        catalog
            .entries
            .iter()
            .map(|entry| public_entry(entry, published.contains(&entry.id)))
            .collect(),
    );
    Ok(result)
}
fn public_entry(entry: &LldEntry, published: bool) -> Value {
    let mut value = serde_json::to_value(entry).expect("typed LLD entry");
    value["detail_state"] = json!(if published {
        "published"
    } else {
        "unavailable_static_detail"
    });
    value["limitations"] = json!(gap_list(&entry.limitations));
    value
}
fn public_detail(detail: &LldFunctionDetail) -> Result<Value, String> {
    let value = serde_json::to_value(detail).map_err(|_| "Cannot project detail")?;
    let mut result = select(
        &value,
        &[
            "id",
            "catalog_id",
            "entry",
            "profile",
            "label",
            "declarations",
            "behavior_fingerprint",
            "coverage",
        ],
    );
    result["entry"] = public_entry(&detail.entry, true);
    result["declarations"] = Value::Array(
        detail
            .declarations
            .iter()
            .map(|e| public_entry(e, false))
            .collect(),
    );
    result["coverage"] = lld_coverage(&detail.coverage);
    // Canonical AST tokens are private IR, not an additional source export.
    let nodes=detail.graph.nodes.iter().map(|node|json!({"id":node.id,"region_id":node.region_id,
        "ast_kind":node.ast_kind,"operation":node.operation,"label":node.label,"start_byte":node.start_byte,
        "end_byte":node.end_byte,"start_line":node.start_line,"end_line":node.end_line,
        "evidence_id":node.evidence_id,"resolution":node.resolution})).collect::<Vec<_>>();
    result["graph"] = json!({"nodes":nodes,"edges":detail.graph.edges,"regions":detail.graph.regions,
        "complete":detail.graph.complete,"limitations":gap_list(&detail.graph.limitations),
        "omitted_nodes":detail.graph.omitted_nodes,"omitted_edges":detail.graph.omitted_edges,
        "fingerprint":detail.graph.fingerprint});
    result["evidence"] = Value::Array(detail.evidence.iter().map(evidence).collect());
    let mermaid = crate::lld::render_lld_mermaid(detail);
    result["mermaid"] = json!(mermaid);
    result["markdown"] = json!(format!(
        "# Static LLD detail\n\nEntry: {}\nCommit-qualified source: {}:{}-{}\n\nRuntime and call effects are not verified.\n\n```mermaid\n{}```\n",
        detail.entry.id,
        markdown_escape(&detail.entry.path),
        detail.entry.start_line,
        detail.entry.end_line,
        mermaid
    ));
    Ok(result)
}
fn markdown_escape(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '`' | '[' | ']' | '(' | ')' | '*' | '_' | '#' | '|' | '\\' => {
                format!("&#{};", c as u32)
            }
            c if c.is_control() => " ".into(),
            _ => c.to_string(),
        })
        .collect()
}

struct Builder {
    files: BTreeMap<String, Vec<u8>>,
    source_bytes: usize,
    routes: PublicRoutes,
    gaps: BTreeSet<String>,
    deferred: usize,
    verified_sources: BTreeSet<(String, String, String)>,
}
impl Builder {
    fn add(&mut self, path: &str, value: &Value) -> Result<(), String> {
        self.raw(path, canonical_json(value)?)
    }
    fn raw(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        policy::safe_path(path)?;
        if self.files.contains_key(path) {
            return Err("Duplicate generated public chunk".into());
        }
        if self.files.len() >= MAX_PUBLIC_FILES
            || self
                .files
                .values()
                .map(Vec::len)
                .sum::<usize>()
                .saturating_add(bytes.len())
                > 7 * 1024 * 1024
        {
            return Err("Public projection file/byte limit".into());
        }
        self.files.insert(path.into(), bytes);
        Ok(())
    }
    fn source(
        &mut self,
        snapshot: &LldSnapshot,
        span: &DesignEvidence,
    ) -> Result<Option<String>, String> {
        if !self.verified_sources.contains(&(
            snapshot.catalog.snapshot.as_str().into(),
            span.path.clone(),
            span.blob.clone(),
        )) {
            self.gaps.insert(
                "Some source inputs have no verified retained declaration and remain unavailable"
                    .into(),
            );
            return Ok(None);
        }
        let input = snapshot
            .inputs
            .iter()
            .find(|i| i.path == span.path && i.blob == span.blob)
            .ok_or("Public source has no verified immutable input")?;
        if span.commit != snapshot.catalog.snapshot
            || span.repository != snapshot.catalog.repository
            || span.start_line == 0
            || span.end_line < span.start_line
            || span.end_line > input.source.lines().count().max(1)
        {
            return Err("Public source span identity invalid".into());
        }
        let mut lines = vec![];
        let mut size = 0;
        for line in input
            .source
            .lines()
            .skip(span.start_line - 1)
            .take((span.end_line - span.start_line + 1).min(200))
        {
            if size + line.len() + usize::from(!lines.is_empty()) > 65536 {
                break;
            }
            size += line.len() + usize::from(!lines.is_empty());
            lines.push(line);
        }
        if lines.is_empty()
            || self.source_bytes + size > MAX_PUBLIC_SOURCE_BYTES
            || self.files.len() >= 900
        {
            self.gaps.insert(
                "Some exact source excerpts were deferred by the static source/file budget".into(),
            );
            return Ok(None);
        }
        let end = span.start_line + lines.len() - 1;
        let value = json!({"evidence_id":span.id,"commit":span.commit.as_str(),"path":span.path,"blob_id":span.blob,
            "start_line":span.start_line,"end_line":end,"text":lines.join("\n"),"truncated":end<span.end_line});
        let bytes = canonical_json(&value)?;
        let path = format!("evidence/{}.json", sha256(&bytes));
        if !self.files.contains_key(&path) {
            self.raw(&path, bytes)?;
            self.source_bytes += size;
        }
        if end < span.end_line {
            self.gaps
                .insert("Some published source previews are bounded excerpts".into());
        }
        Ok(Some(path))
    }
}

fn pair_entries(review: &LldReview) -> Vec<(Option<&LldEntry>, Option<&LldEntry>)> {
    let mut pairs = vec![];
    let mut used = BTreeSet::new();
    for before in &review.baseline.catalog.entries {
        let unique = review
            .baseline
            .catalog
            .entries
            .iter()
            .filter(|e| e.logical_key == before.logical_key)
            .count()
            == 1;
        let matches = review
            .proposal
            .catalog
            .entries
            .iter()
            .filter(|e| e.logical_key == before.logical_key)
            .collect::<Vec<_>>();
        let after = if unique && matches.len() == 1 {
            Some(matches[0])
        } else {
            None
        };
        let after = after.or_else(|| {
            review
                .delta
                .changes
                .iter()
                .find(|c| {
                    c.kind == "declaration_moved"
                        && c.baseline_entry_id.as_deref() == Some(&before.id)
                })
                .and_then(|c| c.proposal_entry_id.as_ref())
                .and_then(|id| review.proposal.catalog.entries.iter().find(|e| e.id == *id))
        });
        let after = after.filter(|e| used.insert(e.id.clone()));
        pairs.push((Some(before), after));
    }
    for after in &review.proposal.catalog.entries {
        if !used.contains(&after.id) {
            pairs.push((None, Some(after)));
        }
    }
    pairs.sort_by_key(|(b, a)| a.or(*b).map(|e| (&e.path, e.start_byte, &e.id)));
    pairs
}
fn bindings(
    before: Option<&LldEntry>,
    after: Option<&LldEntry>,
    review: &LldReview,
    path: &str,
) -> Vec<DetailBinding> {
    let mut output = vec![];
    for entry in before.into_iter().chain(after) {
        let mut components = BTreeSet::from([None]);
        for item in [before, after].into_iter().flatten() {
            if let Some(id) = &item.parent.component_id {
                components.insert(Some(id.clone()));
            }
            components.extend(
                item.parent
                    .candidate_component_ids
                    .iter()
                    .cloned()
                    .map(Some),
            );
        }
        for parent in [
            &review.baseline.catalog.hld_design_id,
            &review.proposal.catalog.hld_design_id,
        ] {
            for component in &components {
                if component.as_ref().is_some_and(|id| {
                    [before, after].into_iter().flatten().any(|e| {
                        e.parent.component_id.as_ref() != Some(id)
                            && !e.parent.candidate_component_ids.contains(id)
                    })
                }) {
                    continue;
                }
                output.push(DetailBinding {
                    entry: Some(entry.id.clone()),
                    parent_hld: parent.clone(),
                    component: component.clone(),
                    baseline_entry: None,
                    correspondence_kind: None,
                    path: path.into(),
                    evidence: None,
                });
            }
        }
    }
    output
}

pub(super) fn export(
    artifact: &Value,
    policy: &PublicPolicy,
    expected: &ExpectedComparison,
) -> Result<PublicBundle, String> {
    let started = Instant::now();
    let paths = policy.validate()?;
    if expected.repository != policy.repository
        || expected.pr_number == 0
        || expected.pr_number > 1_000_000_000
        || ![
            &expected.selected_baseline,
            &expected.baseline,
            &expected.proposal,
        ]
        .iter()
        .all(|s| policy::oid(s))
        || (policy.baseline_mode == "exact" && expected.selected_baseline != expected.baseline)
        || artifact["artifact_version"] != "csb-review-artifact-v1"
        || string(artifact, "selected_baseline")? != expected.selected_baseline
        || string(artifact, "baseline")? != expected.baseline
        || string(artifact, "proposal")? != expected.proposal
    {
        return Err("Public artifact does not match the pinned comparison/policy".into());
    }
    let review_id = string(artifact, "review_id")?;
    if review_id.len() != 64 || !review_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid analysis review ID".into());
    }
    let manifest = &artifact["scope_manifest"];
    let repo = crate::design::wire::repository_from_value(&manifest["primary_repository"])?;
    if repo.provider() != "github"
        || repo.owner() != policy.repository.owner
        || repo.name() != policy.repository.name
        || !array(manifest, "contexts")?.is_empty()
        || manifest["export_allowed"] != true
    {
        return Err(
            "Public export denies private/related context or missing export consent".into(),
        );
    }
    let scope_paths = |key| -> Result<Vec<String>, String> {
        array(manifest, key)?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or("Invalid scope path".into())
            })
            .collect()
    };
    let native_paths = PathScope::new(scope_paths("include")?, scope_paths("exclude")?)?;
    if native_paths != paths {
        return Err("Artifact and base-owned public scope differ".into());
    }
    let scope = ScopeRecord {
        version: ScopeVersion::new(string(artifact, "scope_version")?)?,
        primary_repository: repo,
        primary_paths: paths.clone(),
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: true,
        model_destinations: BTreeSet::new(),
    };
    let mut builder = Builder {
        files: BTreeMap::new(),
        source_bytes: 0,
        routes: PublicRoutes::default(),
        gaps: BTreeSet::new(),
        deferred: 0,
        verified_sources: BTreeSet::new(),
    };
    let mut file_values = vec![];
    for file in array(artifact, "files")? {
        policy.allows(&paths, string(file, "path")?)?;
        if let Some(old) = file["old_path"].as_str() {
            policy.allows(&paths, old)?;
        }
        if file_values.len() >= 1000
            || !matches!(
                string(file, "kind")?,
                "added" | "deleted" | "removed" | "modified" | "renamed"
            )
            || file["additions"].as_u64().is_none()
            || file["deletions"].as_u64().is_none()
        {
            return Err("Invalid bounded public file inventory".into());
        }
        file_values.push(select(
            file,
            &["path", "old_path", "kind", "additions", "deletions"],
        ));
    }
    let mut nodes = vec![];
    let mut edges = vec![];
    for node in array(&artifact["architecture"], "nodes")? {
        policy.allows(&paths, string(node, "path")?)?;
        let mut value = select(
            node,
            &[
                "path",
                "status",
                "blobBefore",
                "blobAfter",
                "definitionsBefore",
                "definitionsAfter",
                "callsBefore",
                "callsAfter",
            ],
        );
        value["sourceBefore"] = Value::Null;
        value["sourceAfter"] = Value::Null;
        value["sourceBeforeTruncated"] = json!(true);
        value["sourceAfterTruncated"] = json!(true);
        nodes.push(value);
    }
    for edge in array(&artifact["architecture"], "edges")? {
        policy.allows(&paths, string(edge, "from")?)?;
        policy.allows(&paths, string(edge, "to")?)?;
        edges.push(select(
            edge,
            &["from", "to", "name", "status", "resolution"],
        ));
    }
    if file_values.iter().any(|f| {
        f["path"].as_str().is_some_and(|path| {
            !path.ends_with(".rs")
                && !path.ends_with("Cargo.toml")
                && !path.ends_with("Cargo.lock")
                && !path.ends_with("vercel.json")
                && ![".sql", ".graphql", ".proto", ".prisma"]
                    .iter()
                    .any(|ext| path.ends_with(ext))
        })
    }) {
        builder.gaps.insert(
            "Some changed files are outside the supported Rust/static-artifact semantic profile"
                .into(),
        );
    }
    let mut hld_status = "unavailable".to_string();
    let mut lld_status = "unavailable".to_string();
    let mut markdown = String::from(
        "# Public architecture review\n\nStatic source evidence, not runtime verification or merge approval.\n\n",
    );
    markdown.push_str(&format!(
        "Baseline: {}\nProposal: {}\n\n",
        expected.baseline, expected.proposal
    ));
    if artifact.get("hld").is_some_and(|v| !v.is_null()) {
        let before: DesignVersion = serde_json::from_value(artifact["hld"]["baseline"].clone())
            .map_err(|_| "Invalid baseline HLD")?;
        let after: DesignVersion = serde_json::from_value(artifact["hld"]["proposal"].clone())
            .map_err(|_| "Invalid proposal HLD")?;
        for (design, commit) in [(&before, &expected.baseline), (&after, &expected.proposal)] {
            crate::design::validate_design_scope(design, &scope)?;
            if design.snapshot.as_str() != commit
                || design.level != crate::design::DesignLevel::Hld
                || design.view != crate::design::DesignView::Observed
                || design.generator_version != crate::design::HLD_GENERATOR_VERSION
            {
                return Err("Public HLD snapshot/view mismatch".into());
            }
            for item in &design.evidence {
                policy.allows(&paths, &item.path)?;
            }
            for element in &design.elements {
                for root in &element.roots {
                    if root != "." {
                        policy.allows(&paths, root)?;
                    }
                }
            }
            if !design.coverage.complete {
                builder
                    .gaps
                    .insert("HLD contains bounded/unsupported structural coverage".into());
            }
        }
        let (b, bmd, clean_before) = public_hld(&before)?;
        let (a, amd, clean_after) = public_hld(&after)?;
        // Recompute observed delta; do not trust persisted Markdown, findings or user assertions.
        let mut delta =
            serde_json::to_value(crate::design::compare_hld(&clean_before, &clean_after)?)
                .map_err(|_| "Cannot compare HLD")?;
        delta["findings"] = json!([]);
        delta["questions"] = json!([]);
        hld_status = delta["outcome"].as_str().unwrap_or("incomplete").into();
        builder.add(
            "hld.json",
            &json!({"baseline":b,"proposal":a,"delta":delta,
            "export_views":{"baseline":bmd,"proposal":amd,"diff":format!("{bmd}\n\n{amd}")}}),
        )?;
        builder.routes.hld = Some("hld.json".into());
        markdown.push_str(&bmd);
        markdown.push_str("\n\n");
        markdown.push_str(&amd);
        if artifact.get("lld").is_some_and(|v| !v.is_null()) {
            let mut lld: LldReview = serde_json::from_value(artifact["lld"].clone())
                .map_err(|_| "Invalid immutable LLD input")?;
            for (snapshot, parent) in [(&lld.baseline, &before), (&lld.proposal, &after)] {
                crate::lld::validate_lld_snapshot(snapshot, parent, &scope)?;
                for input in &snapshot.inputs {
                    policy.allows(&paths, &input.path)?;
                    // Verify every retained full source once using maintained Git hashing.
                    let Some(entry) = snapshot.catalog.entries.iter().find(|e| {
                        e.path == input.path
                            && matches!(e.kind.as_str(), "file" | "configuration" | "schema")
                    }) else {
                        builder.gaps.insert("Some retained inputs lack bounded catalog declarations; their source remains unpublished".into());
                        continue;
                    };
                    crate::lld::build_lld_detail(snapshot, &entry.id, &Cancellation::default())?;
                    builder.verified_sources.insert((
                        snapshot.catalog.snapshot.as_str().into(),
                        input.path.clone(),
                        input.blob.clone(),
                    ));
                }
            }
            lld.delta = crate::lld::compare_lld(&lld.baseline.catalog, &lld.proposal.catalog);
            let mut published = BTreeSet::new();
            let mut catalog_pairs = vec![];
            let pairs = pair_entries(&lld);
            let mut selected = pairs.iter().collect::<Vec<_>>();
            selected.sort_by_key(|(b, a)| {
                let e = a.or(*b).expect("nonempty pair");
                let changed = lld.delta.changes.iter().any(|c| {
                    c.baseline_entry_id.as_deref() == Some(&e.id)
                        || c.proposal_entry_id.as_deref() == Some(&e.id)
                });
                (!changed, e.kind != "function", &e.path, e.start_byte, &e.id)
            });
            let mut count = 0;
            for (b, a) in selected {
                if count >= policy.max_details || started.elapsed() > Duration::from_secs(105) {
                    builder.deferred += 1;
                    continue;
                }
                let mut bd = b
                    .map(|e| {
                        crate::lld::build_lld_detail(&lld.baseline, &e.id, &Cancellation::default())
                    })
                    .transpose()?;
                let mut ad = a
                    .map(|e| {
                        crate::lld::build_lld_detail(&lld.proposal, &e.id, &Cancellation::default())
                    })
                    .transpose()?;
                let ambiguous = b.or(*a).is_some_and(|e| {
                    lld.baseline
                        .catalog
                        .entries
                        .iter()
                        .filter(|x| x.logical_key == e.logical_key)
                        .count()
                        > 1
                        || lld
                            .proposal
                            .catalog
                            .entries
                            .iter()
                            .filter(|x| x.logical_key == e.logical_key)
                            .count()
                            > 1
                });
                let missing_uncertain = (b.is_none()
                    && (!lld.baseline.catalog.coverage.inventory_complete
                        || !lld.baseline.catalog.coverage.structure_complete))
                    || (a.is_none()
                        && (!lld.proposal.catalog.coverage.inventory_complete
                            || !lld.proposal.catalog.coverage.structure_complete));
                if ambiguous || missing_uncertain {
                    for detail in bd.iter_mut().chain(ad.iter_mut()) {
                        detail.coverage.structure_complete = false;
                        detail.coverage.gaps.push("Counterpart correspondence/inventory is ambiguous; absence is not established addition/removal".into());
                    }
                }
                let delta = crate::lld::compare_lld_detail(bd.as_ref(), ad.as_ref());
                let selected_entry = a.or(*b).ok_or("Empty public detail pair")?;
                let value = json!({"entry_id":selected_entry.id,"baseline":bd.as_ref().map(public_detail).transpose()?,"proposal":ad.as_ref().map(public_detail).transpose()?,"delta":public_delta(&delta)?});
                let path = format!("lld/detail/{}.json", sha256(&canonical_json(&value)?));
                if builder.files.values().map(Vec::len).sum::<usize>()
                    + canonical_json(&value)?.len()
                    > 5 * 1024 * 1024
                    || builder.files.len() > 850
                {
                    builder.deferred += 1;
                    continue;
                }
                builder.add(&path, &value)?;
                for side in ["baseline", "proposal"] {
                    if let Some(detail_markdown) = value[side]["markdown"].as_str() {
                        markdown.push_str("\n\n");
                        markdown.push_str(detail_markdown);
                    }
                }
                let aliases = bindings(*b, *a, &lld, &path);
                builder.routes.lld_details.extend(aliases.clone());
                for (detail, snapshot) in [(&bd, &lld.baseline), (&ad, &lld.proposal)] {
                    let Some(detail) = detail else {
                        continue;
                    };
                    published.insert(detail.entry.id.clone());
                    if !detail.coverage.structure_complete
                        || !detail.coverage.inventory_complete
                        || (detail.entry.kind == "function"
                            && (!detail.graph.complete || !detail.coverage.gaps.is_empty()))
                    {
                        builder.gaps.insert(
                            "Some published LLD details have incomplete local static coverage"
                                .into(),
                        );
                    }
                    if detail
                        .graph
                        .nodes
                        .iter()
                        .any(|n| n.resolution == "target_and_effect_unresolved")
                    {
                        builder
                            .gaps
                            .insert("Call targets and runtime effects remain unresolved".into());
                    }
                    for span in &detail.evidence {
                        if let Some(source_path) = builder.source(snapshot, span)? {
                            for mut alias in aliases.clone() {
                                alias.path = source_path.clone();
                                alias.evidence = Some(span.id.clone());
                                builder.routes.sources.push(alias);
                            }
                        }
                    }
                }
                count += 1;
            }
            for (b, a) in pairs {
                let status = match (b, a) {
                    (Some(b), Some(a))
                        if b.body_fingerprint == a.body_fingerprint
                            && b.signature_fingerprint == a.signature_fingerprint =>
                    {
                        "unchanged"
                    }
                    (Some(_), Some(_)) => "modified",
                    (Some(_), None) => "removed",
                    _ => "added",
                };
                let uncertain = (b.is_none()
                    && (!lld.baseline.catalog.coverage.inventory_complete
                        || !lld.baseline.catalog.coverage.structure_complete))
                    || (a.is_none()
                        && (!lld.proposal.catalog.coverage.inventory_complete
                            || !lld.proposal.catalog.coverage.structure_complete))
                    || b.or(a).is_some_and(|e| {
                        lld.baseline
                            .catalog
                            .entries
                            .iter()
                            .filter(|x| x.logical_key == e.logical_key)
                            .count()
                            > 1
                            || lld
                                .proposal
                                .catalog
                                .entries
                                .iter()
                                .filter(|x| x.logical_key == e.logical_key)
                                .count()
                                > 1
                    });
                catalog_pairs.push(json!({"baseline":b.map(|e|public_entry(e,published.contains(&e.id))),"proposal":a.map(|e|public_entry(e,published.contains(&e.id))),"status":if uncertain {"unknown"} else {status}}));
            }
            for (snapshot, parent, side) in [
                (&lld.baseline, &before, "baseline"),
                (&lld.proposal, &after, "proposal"),
            ] {
                for span in &parent.evidence {
                    if snapshot
                        .inputs
                        .iter()
                        .any(|input| input.path == span.path && input.blob == span.blob)
                    {
                        if let Some(path) = builder.source(snapshot, span)? {
                            builder.routes.sources.push(DetailBinding {
                                entry: None,
                                parent_hld: parent.id.clone(),
                                component: None,
                                baseline_entry: None,
                                correspondence_kind: None,
                                path,
                                evidence: Some(span.id.clone()),
                            });
                        }
                    }
                }
                for file in &file_values {
                    if let Some(entry) = snapshot.catalog.entries.iter().find(|e| {
                        e.path == file["path"]
                            && matches!(e.kind.as_str(), "file" | "configuration" | "schema")
                    }) {
                        let span = DesignEvidence {
                            id: format!("public-file-{}", entry.id),
                            repository: snapshot.catalog.repository.clone(),
                            commit: snapshot.catalog.snapshot.clone(),
                            path: entry.path.clone(),
                            blob: entry.blob.clone(),
                            start_line: 1,
                            end_line: entry.end_line,
                            producer_version: crate::lld::LLD_GENERATOR_VERSION.into(),
                            relationship_type: "public_file_excerpt".into(),
                            resolution: DesignResolution::ObservedStructure,
                        };
                        if let Some(path) = builder.source(snapshot, &span)? {
                            builder.routes.file_sources.push(FileSourceBinding {
                                file_path: entry.path.clone(),
                                side: side.into(),
                                parent_hld: parent.id.clone(),
                                path,
                            });
                        }
                    }
                }
                if !snapshot.catalog.coverage.inventory_complete
                    || !snapshot.catalog.coverage.structure_complete
                {
                    builder.gaps.insert(
                        "LLD inventory/structure has unsupported or deferred regions".into(),
                    );
                }
            }
            if builder.deferred > 0 {
                builder.gaps.insert("Some catalog entries have no precomputed static detail; request a new bounded analysis to inspect them".into());
            }
            if lld.delta.ambiguous_groups > 0
                || lld.delta.changes.iter().any(|c| c.resolution == "unknown")
            {
                builder
                    .gaps
                    .insert("Some catalog correspondences remain ambiguous or unresolved".into());
            }
            lld_status = if builder.gaps.is_empty() {
                if lld
                    .delta
                    .changes
                    .iter()
                    .any(|change| change.resolution == "observed_structure")
                {
                    "change_detected".into()
                } else {
                    "no_change_detected_within_scope".into()
                }
            } else {
                "incomplete".into()
            };
            let mut aggregate_delta = public_delta(&lld.delta)?;
            aggregate_delta["native_delta_id"] = json!(lld.delta.id);
            aggregate_delta["outcome"] = json!(lld_status);
            if builder.gaps.is_empty() {
                aggregate_delta["limitations"] = json!([]);
            }
            aggregate_delta
                .as_object_mut()
                .ok_or("Invalid public aggregate delta")?
                .remove("id");
            aggregate_delta["id"] = json!(format!(
                "public-lld-delta-{}",
                sha256(&canonical_json(&aggregate_delta)?)
            ));
            builder.add("lld/catalog.json",&json!({"baseline":public_catalog(&lld.baseline.catalog,&published)? ,"proposal":public_catalog(&lld.proposal.catalog,&published)? ,"entries":catalog_pairs,"delta":aggregate_delta}))?;
            builder.routes.lld_catalog = Some("lld/catalog.json".into());
        } else {
            builder.gaps.insert(
                "LLD immutable inputs unavailable; no static details or source can be generated"
                    .into(),
            );
        }
    } else {
        builder
            .gaps
            .insert("Source-backed HLD is unavailable; structural sketches are not HLD/LLD".into());
    }
    if !builder.gaps.is_empty() && lld_status != "unavailable" {
        lld_status = "incomplete".into();
    }
    let coverage = select(
        &artifact["coverage"],
        &[
            "indexed",
            "unsupported",
            "excluded",
            "failed",
            "deferred",
            "enumeration_complete",
        ],
    );
    if coverage["enumeration_complete"] != true
        || ["unsupported", "failed", "deferred"]
            .iter()
            .any(|key| coverage[key].as_u64().unwrap_or(1) > 0)
    {
        builder
            .gaps
            .insert("The ordinary scoped source inventory is incomplete".into());
    }
    builder.add("artifact.json",&json!({"artifact_version":"csb-public-artifact-v1","analysis_review_id":review_id,
        "selected_baseline":expected.selected_baseline,"baseline":expected.baseline,"proposal":expected.proposal,"scope_version":public_scope(&scope.version),
        "scope_manifest":{"primary_repository":{"provider":"github","owner":policy.repository.owner,"name":policy.repository.name},"include":policy.include,"exclude":policy.exclude,"contexts":[],"export_allowed":true},
        "files":file_values,"architecture":{"nodes":nodes,"edges":edges},"warnings":builder.gaps.iter().collect::<Vec<_>>(),"coverage":coverage,"hld_status":hld_status,"lld_status":lld_status}))?;
    builder.routes.artifact = "artifact.json".into();
    markdown.push_str(&format!(
        "\n\nHLD outcome: {hld_status}\nLLD outcome: {lld_status}\n\n"
    ));
    for gap in &builder.gaps {
        markdown.push_str(&format!("- {}\n", markdown_escape(gap)));
    }
    builder.raw("exports/review.md", markdown.into_bytes())?;
    builder
        .routes
        .exports
        .push(json!({"view":"diff","path":"exports/review.md"}));
    builder
        .routes
        .lld_details
        .sort_by_key(|r| serde_json::to_string(r).expect("typed route"));
    builder.routes.lld_details.dedup();
    builder
        .routes
        .sources
        .sort_by_key(|r| serde_json::to_string(r).expect("typed route"));
    builder.routes.sources.dedup();
    let files = builder
        .files
        .iter()
        .map(|(path, bytes)| PublicFile {
            path: path.clone(),
            sha256: sha256(bytes),
            bytes: bytes.len(),
            kind: if path.ends_with(".md") {
                "markdown"
            } else {
                "json"
            }
            .into(),
        })
        .collect();
    let mut manifest = PublicManifest {
        schema_version: PUBLIC_SCHEMA.into(),
        review_id: String::new(),
        public_report_id: String::new(),
        analysis_review_id: review_id.into(),
        repository: policy.repository.clone(),
        pr_number: expected.pr_number,
        selected_baseline: expected.selected_baseline.clone(),
        baseline: expected.baseline.clone(),
        proposal: expected.proposal.clone(),
        scope_version: public_scope(&scope.version),
        policy_digest: policy.digest()?,
        engine: json!({"package":"flowdiff","version":env!("CARGO_PKG_VERSION"),"hld_generator":crate::design::HLD_GENERATOR_VERSION,"lld_generator":crate::lld::LLD_GENERATOR_VERSION,"lld_parser":crate::lld::LLD_IR_VERSION,"public_projection":PUBLIC_SCHEMA}),
        files,
        routes: builder.routes,
        coverage: PublicCoverage {
            complete: builder.gaps.is_empty()
                && matches!(
                    hld_status.as_str(),
                    "change_detected" | "no_change_detected_within_scope"
                )
                && matches!(
                    lld_status.as_str(),
                    "change_detected" | "no_change_detected_within_scope"
                ),
            gaps: builder.gaps.into_iter().collect(),
            deferred_details: builder.deferred,
        },
        capabilities: json!({"hld":hld_status!="unavailable","lld":lld_status!="unavailable","source":builder.source_bytes>0,"export":true,"confirmation":false}),
    };
    manifest.public_report_id = manifest.identity()?;
    manifest.review_id = manifest.public_report_id.clone();
    let bundle = PublicBundle {
        manifest,
        files: builder.files,
    };
    bundle.validate()?;
    Ok(bundle)
}
