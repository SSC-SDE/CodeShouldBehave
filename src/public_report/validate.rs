//! Validate semantic route pins as well as byte hashes. Recomputing a manifest
//! identity cannot authorize a cross-parent/component/evidence rebind.
use super::*;
use crate::PathScope;
use serde_json::Value;
use std::collections::BTreeMap;
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .ok_or_else(|| format!("Missing public {key}"))
}
fn list<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    v[key]
        .as_array()
        .ok_or_else(|| format!("Missing public {key}"))
}
fn chunk(bundle: &PublicBundle, path: &str) -> Result<Value, String> {
    super::write::parse_json(bundle.files.get(path).ok_or("Missing public chunk")?)
}
fn allowed(value: &Value, keys: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("Public record must be an object")?;
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("Unknown field in public projection".into());
    }
    Ok(())
}
fn public_evidence(value: &Value) -> Result<(), String> {
    allowed(
        value,
        &[
            "id",
            "commit",
            "path",
            "blob",
            "start_line",
            "end_line",
            "producer_version",
            "relationship_type",
            "resolution",
        ],
    )
}
fn array_paths(value: &Value, key: &str) -> Result<Vec<String>, String> {
    list(value, key)?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or("Invalid public path policy".into())
        })
        .collect()
}
fn scoped(scope: &PathScope, path: &str) -> Result<(), String> {
    policy::safe_path(path)?;
    if !scope.allows(path) || crate::sensitive_path(path) {
        Err("Unapproved public source path".into())
    } else {
        Ok(())
    }
}
fn source_span(source: &Value, evidence: &Value, scope: &PathScope) -> Result<(), String> {
    allowed(
        source,
        &[
            "evidence_id",
            "commit",
            "path",
            "blob_id",
            "start_line",
            "end_line",
            "text",
            "truncated",
        ],
    )?;
    let start = source["start_line"]
        .as_u64()
        .ok_or("Invalid source start")?;
    let end = source["end_line"].as_u64().ok_or("Invalid source end")?;
    let low = evidence["start_line"]
        .as_u64()
        .ok_or("Invalid evidence start")?;
    let high = evidence["end_line"]
        .as_u64()
        .ok_or("Invalid evidence end")?;
    let content = text(source, "text")?;
    scoped(scope, text(source, "path")?)?;
    if source["evidence_id"] != evidence["id"]
        || source["commit"] != evidence["commit"]
        || source["path"] != evidence["path"]
        || source["blob_id"] != evidence["blob"]
        || start == 0
        || start < low
        || end > high
        || end < start
        || end - start >= 200
        || content.len() > 65536
        || content.split('\n').count() as u64 != end - start + 1
        || source["truncated"].as_bool().is_none()
        || !policy::oid(text(source, "blob_id")?)
    {
        return Err("Public source/evidence pin mismatch".into());
    }
    Ok(())
}
type Key = (Option<String>, String, Option<String>);
fn key(binding: &DetailBinding) -> Key {
    (
        binding.entry.clone(),
        binding.parent_hld.clone(),
        binding.component.clone(),
    )
}
pub(super) fn routes(bundle: &PublicBundle) -> Result<(), String> {
    let m = &bundle.manifest;
    let artifact = chunk(bundle, &m.routes.artifact)?;
    allowed(
        &m.engine,
        &[
            "package",
            "version",
            "hld_generator",
            "lld_generator",
            "lld_parser",
            "public_projection",
        ],
    )?;
    allowed(
        &m.capabilities,
        &["hld", "lld", "source", "export", "confirmation"],
    )?;
    allowed(
        &artifact,
        &[
            "artifact_version",
            "analysis_review_id",
            "selected_baseline",
            "baseline",
            "proposal",
            "scope_version",
            "scope_manifest",
            "files",
            "architecture",
            "warnings",
            "coverage",
            "hld_status",
            "lld_status",
        ],
    )?;
    if m.coverage.complete
        && (!m.coverage.gaps.is_empty()
            || m.coverage.deferred_details > 0
            || !["hld_status", "lld_status"].iter().all(|key| {
                matches!(
                    artifact[key].as_str(),
                    Some("change_detected" | "no_change_detected_within_scope")
                )
            }))
    {
        return Err("Public completeness contradicts coverage/outcomes".into());
    }
    if artifact["artifact_version"] != "csb-public-artifact-v1"
        || artifact["analysis_review_id"] != m.analysis_review_id
        || artifact["scope_version"] != m.scope_version
        || artifact["baseline"] != m.baseline
        || artifact["proposal"] != m.proposal
        || artifact["selected_baseline"] != m.selected_baseline
    {
        return Err("Public artifact identity mismatch".into());
    }
    let policy = &artifact["scope_manifest"];
    allowed(
        policy,
        &[
            "primary_repository",
            "include",
            "exclude",
            "contexts",
            "export_allowed",
        ],
    )?;
    allowed(
        &policy["primary_repository"],
        &["provider", "owner", "name"],
    )?;
    if policy["primary_repository"]["owner"] != m.repository.owner
        || policy["primary_repository"]["name"] != m.repository.name
        || policy["primary_repository"]["provider"] != "github"
        || policy["export_allowed"] != true
        || !list(policy, "contexts")?.is_empty()
    {
        return Err("Public artifact scope mismatch".into());
    }
    let scope = PathScope::new(
        array_paths(policy, "include")?,
        array_paths(policy, "exclude")?,
    )?;
    let files = list(&artifact, "files")?;
    for file in files {
        scoped(&scope, text(file, "path")?)?;
    }
    let Some(hld_path) = &m.routes.hld else {
        if m.routes.lld_catalog.is_some()
            || !m.routes.sources.is_empty()
            || !m.routes.file_sources.is_empty()
            || !m.routes.lld_details.is_empty()
        {
            return Err("Public detail lacks HLD parent".into());
        }
        return Ok(());
    };
    let hld = chunk(bundle, hld_path)?;
    let mut parents = BTreeMap::new();
    for (side, commit) in [("baseline", &m.baseline), ("proposal", &m.proposal)] {
        let design = &hld[side];
        allowed(
            design,
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
                "evidence",
                "constraints",
                "correspondence",
                "document_candidates",
                "decision_records",
            ],
        )?;
        if design["snapshot"] != *commit || design["scope_version"] != m.scope_version {
            return Err("Public HLD pin mismatch".into());
        }
        for e in list(design, "evidence")? {
            public_evidence(e)?;
            scoped(&scope, text(e, "path")?)?;
            if e["commit"] != *commit {
                return Err("Public HLD evidence commit mismatch".into());
            }
        }
        parents.insert(text(design, "id")?.to_owned(), design.clone());
    }
    let catalog = m
        .routes
        .lld_catalog
        .as_ref()
        .map(|path| chunk(bundle, path))
        .transpose()?;
    let mut entries = BTreeMap::new();
    if let Some(catalog) = &catalog {
        for side in ["baseline", "proposal"] {
            let value = &catalog[side];
            allowed(
                value,
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
                    "evidence",
                    "entries",
                ],
            )?;
            let parent = &hld[side];
            if value["hld_design_id"] != parent["id"]
                || value["snapshot"] != parent["snapshot"]
                || value["scope_version"] != m.scope_version
            {
                return Err("Public catalog parent mismatch".into());
            }
            for entry in list(value, "entries")? {
                serde_json::from_value::<crate::lld::LldEntry>(entry.clone())
                    .map_err(|_| "Invalid strict public entry")?;
                scoped(&scope, text(entry, "path")?)?;
                if entry["parent"]["hld_design_id"] != parent["id"] {
                    return Err("Public entry parent mismatch".into());
                }
                let id = text(entry, "id")?.to_owned();
                if let Some(old) = entries.insert(id, entry.clone()) {
                    if old != *entry {
                        return Err("Conflicting public entry identity".into());
                    }
                }
            }
        }
    }
    let mut details: BTreeMap<Key, Value> = BTreeMap::new();
    for binding in &m.routes.lld_details {
        if binding.baseline_entry.is_some() || binding.correspondence_kind.is_some() {
            return Err("Undeclared static correspondence route".into());
        }
        let entry_id = binding
            .entry
            .as_ref()
            .ok_or("Missing public detail entry")?;
        if !parents.contains_key(&binding.parent_hld) {
            return Err("Unknown public detail parent".into());
        }
        let value = chunk(bundle, &binding.path)?;
        allowed(&value, &["entry_id", "baseline", "proposal", "delta"])?;
        if !["baseline", "proposal"]
            .iter()
            .any(|s| value[s]["entry"]["id"] == *entry_id)
        {
            return Err("Public detail alias mismatch".into());
        }
        for side in ["baseline", "proposal"] {
            if value[side].is_null() {
                continue;
            }
            let entry = &value[side]["entry"];
            allowed(
                &value[side],
                &[
                    "id",
                    "catalog_id",
                    "entry",
                    "profile",
                    "label",
                    "declarations",
                    "behavior_fingerprint",
                    "coverage",
                    "graph",
                    "evidence",
                    "mermaid",
                    "markdown",
                ],
            )?;
            let known = entries
                .get(text(entry, "id")?)
                .ok_or("Unknown public detail declaration")?;
            if entry != known {
                return Err("Public detail differs from catalog declaration".into());
            }
            if let Some(component) = &binding.component {
                if entry["parent"]["component_id"] != *component
                    && !list(&entry["parent"], "candidate_component_ids")?
                        .iter()
                        .any(|v| v == component)
                {
                    return Err("Public detail component mismatch".into());
                }
            }
            if value[side]["catalog_id"]
                != catalog.as_ref().ok_or("Detail catalog unavailable")?[side]["id"]
            {
                return Err("Public detail catalog mismatch".into());
            }
            let proof = list(&value[side], "evidence")?;
            for item in proof {
                public_evidence(item)?;
                if item["path"] != entry["path"]
                    || item["blob"] != entry["blob"]
                    || item["commit"] != hld[side]["snapshot"]
                {
                    return Err("Public detail evidence identity mismatch".into());
                }
            }
            let graph = &value[side]["graph"];
            allowed(
                graph,
                &[
                    "nodes",
                    "edges",
                    "regions",
                    "complete",
                    "limitations",
                    "omitted_nodes",
                    "omitted_edges",
                    "fingerprint",
                ],
            )?;
            let nodes = list(graph, "nodes")?;
            let edges = list(graph, "edges")?;
            if nodes.len() > 200 || edges.len() > 400 {
                return Err("Public local graph exceeds limits".into());
            }
            let mut node_ids = std::collections::BTreeSet::new();
            for node in nodes {
                allowed(
                    node,
                    &[
                        "id",
                        "region_id",
                        "ast_kind",
                        "operation",
                        "label",
                        "start_byte",
                        "end_byte",
                        "start_line",
                        "end_line",
                        "evidence_id",
                        "resolution",
                    ],
                )?;
                if !node_ids.insert(text(node, "id")?) {
                    return Err("Duplicate public graph node".into());
                }
                let span = proof
                    .iter()
                    .find(|e| e["id"] == node["evidence_id"])
                    .ok_or("Public node lacks selected evidence")?;
                if node["start_line"] != span["start_line"] || node["end_line"] != span["end_line"]
                {
                    return Err("Public graph/source span mismatch".into());
                }
            }
            for edge in edges {
                if !node_ids.contains(text(edge, "from")?) || !node_ids.contains(text(edge, "to")?)
                {
                    return Err("Public edge endpoint missing".into());
                }
            }
        }
        if let Some(old) = details.insert(key(binding), value.clone()) {
            if old != value {
                return Err("Conflicting public detail binding".into());
            }
        }
    }
    for binding in &m.routes.sources {
        if binding.baseline_entry.is_some() || binding.correspondence_kind.is_some() {
            return Err("Undeclared public source correspondence".into());
        }
        let id = binding
            .evidence
            .as_ref()
            .ok_or("Missing public evidence binding")?;
        let value = chunk(bundle, &binding.path)?;
        if binding.entry.is_some() {
            let pair = details
                .get(&key(binding))
                .ok_or("Source outside selected public detail")?;
            let evidence = ["baseline", "proposal"]
                .into_iter()
                .filter_map(|side| pair[side]["evidence"].as_array())
                .flatten()
                .find(|e| e["id"] == *id)
                .ok_or("Source evidence absent from detail")?;
            source_span(&value, evidence, &scope)?;
        } else {
            if binding.component.is_some() {
                return Err("HLD source component route unsupported".into());
            }
            let parent = parents
                .get(&binding.parent_hld)
                .ok_or("Unknown public source parent")?;
            let evidence = list(parent, "evidence")?
                .iter()
                .find(|e| e["id"] == *id)
                .ok_or("Source evidence absent from HLD parent")?;
            source_span(&value, evidence, &scope)?;
        }
    }
    for binding in &m.routes.file_sources {
        let side = binding.side.as_str();
        if !matches!(side, "baseline" | "proposal") {
            return Err("Invalid public file source side".into());
        }
        let value = chunk(bundle, &binding.path)?;
        if !files.iter().any(|f| f["path"] == binding.file_path)
            || value["path"] != binding.file_path
            || value["commit"] != hld[side]["snapshot"]
            || binding.parent_hld != hld[side]["id"]
        {
            return Err("Public file source identity mismatch".into());
        }
        let entry = entries
            .values()
            .find(|e| {
                e["path"] == binding.file_path
                    && e["blob"] == value["blob_id"]
                    && e["parent"]["hld_design_id"] == binding.parent_hld
                    && matches!(
                        e["kind"].as_str(),
                        Some("file" | "configuration" | "schema")
                    )
            })
            .ok_or("Public file source has no declaration")?;
        let evidence = serde_json::json!({"id":value["evidence_id"],"commit":value["commit"],"path":entry["path"],"blob":entry["blob"],"start_line":1,"end_line":entry["end_line"]});
        source_span(&value, &evidence, &scope)?;
    }
    for export in &m.routes.exports {
        let path = text(export, "path")?;
        if !bundle.files.contains_key(path) || !path.ends_with(".md") {
            return Err("Public export not allowlisted".into());
        }
    }
    Ok(())
}
