//! Deterministic local review site. The site is self-contained so it renders
//! from `file://` as well as through `csb serve`; every emitted artifact is
//! listed in a content-addressed manifest that `csb verify` rechecks.

use crate::analysis::{FileReview, Review};
use crate::core::{AnalysisResult, Coverage};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SITE_SCHEMA: &str = "csb-local-review-v1";
pub const MANIFEST_SCHEMA: &str = "csb-local-manifest-v1";

#[derive(Clone, Debug)]
pub struct DesignArtifacts {
    pub markdown: String,
    pub json: Value,
}

#[derive(Clone, Debug)]
pub struct ManifestEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
}

#[derive(Clone, Debug)]
pub struct SiteManifest {
    pub content_sha256: String,
    pub files: Vec<ManifestEntry>,
    pub incomplete: bool,
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn emit(
    directory: &Path,
    result: &AnalysisResult,
    design: Option<&DesignArtifacts>,
) -> Result<SiteManifest, String> {
    if directory.exists() {
        if !directory.is_dir() {
            return Err(format!(
                "{} exists and is not a directory. Choose a new --out-dir.",
                directory.display()
            ));
        }
        let empty = fs::read_dir(directory)
            .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?
            .next()
            .is_none();
        if !empty {
            return Err(format!(
                "{} already exists and is not empty. Choose a new --out-dir; existing output is never overwritten.",
                directory.display()
            ));
        }
    }
    fs::create_dir_all(directory.join("sources"))
        .map_err(|error| format!("Cannot create {}: {error}", directory.display()))?;

    let review = &result.review;
    let source_paths = source_paths(review);
    let mut files: Vec<(PathBuf, Vec<u8>)> = vec![
        (
            PathBuf::from("index.html"),
            crate::render_html(review).into_bytes(),
        ),
        (
            PathBuf::from("review.md"),
            crate::render_markdown(review).into_bytes(),
        ),
        (
            PathBuf::from("summary.md"),
            summary_markdown(result).into_bytes(),
        ),
        (
            PathBuf::from("review.json"),
            encode(&review_json(result, &source_paths))?,
        ),
    ];
    for (file, name) in review.files.iter().zip(&source_paths) {
        files.push((
            PathBuf::from("sources").join(name),
            encode(&source_json(file))?,
        ));
    }
    if let Some(design) = design {
        files.push((
            PathBuf::from("design.md"),
            design.markdown.clone().into_bytes(),
        ));
        files.push((PathBuf::from("artifact.json"), encode(&design.json)?));
    }

    let index = index_json(result, design.is_some());
    let index_bytes = encode(&index)?;
    let content_sha256 = sha256(&index_bytes);
    files.push((PathBuf::from("index.json"), index_bytes));

    let mut entries = files
        .iter()
        .map(|(path, bytes)| ManifestEntry {
            path: path.to_string_lossy().replace('\\', "/"),
            sha256: sha256(bytes),
            bytes: bytes.len(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    for (path, bytes) in &files {
        let target = directory.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
        }
        let mut handle = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| format!("Cannot create {}: {error}", target.display()))?;
        handle
            .write_all(bytes)
            .map_err(|error| format!("Cannot write {}: {error}", target.display()))?;
    }

    let manifest = json!({
        "schema": MANIFEST_SCHEMA,
        "generated_by": {"name": "csb", "version": env!("CARGO_PKG_VERSION")},
        "content_sha256": content_sha256,
        "baseline": result.baseline.as_str(),
        "selected_baseline": result.selected_baseline.as_str(),
        "proposal": result.proposal.as_str(),
        "incomplete": incomplete(result),
        "files": entries.iter().map(|entry| json!({
            "path": entry.path,
            "sha256": entry.sha256,
            "bytes": entry.bytes,
        })).collect::<Vec<_>>(),
    });
    let manifest_path = directory.join("manifest.json");
    let mut handle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_path)
        .map_err(|error| format!("Cannot create {}: {error}", manifest_path.display()))?;
    handle
        .write_all(&encode(&manifest)?)
        .map_err(|error| format!("Cannot write {}: {error}", manifest_path.display()))?;

    Ok(SiteManifest {
        content_sha256,
        files: entries,
        incomplete: incomplete(result),
    })
}

pub fn verify(directory: &Path) -> Result<SiteManifest, String> {
    let manifest_path = directory.join("manifest.json");
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path)
            .map_err(|error| format!("Cannot read {}: {error}", manifest_path.display()))?,
    )
    .map_err(|_| "manifest.json is not valid JSON".to_string())?;
    let object = manifest
        .as_object()
        .ok_or("manifest.json must be a JSON object")?;
    if object.get("schema").and_then(Value::as_str) != Some(MANIFEST_SCHEMA) {
        return Err(format!("manifest.json schema must be {MANIFEST_SCHEMA}"));
    }
    let content_sha256 = object
        .get("content_sha256")
        .and_then(Value::as_str)
        .ok_or("manifest.json is missing content_sha256")?
        .to_string();
    let incomplete = object
        .get("incomplete")
        .and_then(Value::as_bool)
        .ok_or("manifest.json is missing incomplete")?;
    let entries = object
        .get("files")
        .and_then(Value::as_array)
        .ok_or("manifest.json is missing files")?;
    let index_bytes = fs::read(directory.join("index.json"))
        .map_err(|error| format!("Missing index.json: {error}"))?;
    if sha256(&index_bytes) != content_sha256 {
        return Err("index.json does not match manifest content_sha256".into());
    }
    let index: Value =
        serde_json::from_slice(&index_bytes).map_err(|_| "index.json is not valid JSON")?;
    if index.get("schema").and_then(Value::as_str) != Some(SITE_SCHEMA) {
        return Err(format!("index.json schema must be {SITE_SCHEMA}"));
    }
    let mut checked = Vec::with_capacity(entries.len());
    for entry in entries {
        let entry = entry
            .as_object()
            .ok_or("manifest file entry must be an object")?;
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .ok_or("manifest file entry is missing path")?;
        let expected = entry
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or("manifest file entry is missing sha256")?;
        let size = entry
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or("manifest file entry is missing bytes")? as usize;
        if path.starts_with('/') || path.split('/').any(|part| part == ".." || part == ".git") {
            return Err(format!("Unsafe manifest path: {path}"));
        }
        let bytes = fs::read(directory.join(path))
            .map_err(|error| format!("Missing artifact {path}: {error}"))?;
        if bytes.len() != size {
            return Err(format!("Size mismatch for {path}"));
        }
        let actual = sha256(&bytes);
        if actual != expected {
            return Err(format!("Content hash mismatch for {path}"));
        }
        checked.push(ManifestEntry {
            path: path.to_string(),
            sha256: actual,
            bytes: size,
        });
    }
    checked.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(SiteManifest {
        content_sha256,
        files: checked,
        incomplete,
    })
}

pub fn incomplete(result: &AnalysisResult) -> bool {
    let coverage = &result.coverage;
    !coverage.enumeration_complete
        || coverage.unsupported > 0
        || coverage.failed > 0
        || coverage.deferred > 0
}

pub fn verdict(result: &AnalysisResult) -> (&'static str, &'static str) {
    let review = &result.review;
    let has_notes = review.files.iter().any(|file| !file.notes.is_empty());
    if incomplete(result) || has_notes {
        ("incomplete", "Analysis incomplete · Review required")
    } else if review.flow_edits() == 0 && review.impacts.is_empty() {
        (
            "no_change_detected",
            "No behavior change detected within scope",
        )
    } else {
        ("change_detected", "Behavior change · Review required")
    }
}

pub fn coverage_line(result: &AnalysisResult) -> String {
    let coverage = &result.coverage;
    let mut parts = vec![format!("{} files analyzed", coverage.indexed)];
    for (count, label) in [
        (coverage.unsupported, "unsupported"),
        (coverage.excluded, "excluded"),
        (coverage.failed, "failed"),
        (coverage.deferred, "deferred"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {label}"));
        }
    }
    if !coverage.enumeration_complete {
        parts.push("enumeration incomplete".into());
    }
    format!("csb: {}", parts.join(" · "))
}

pub fn summary_markdown(result: &AnalysisResult) -> String {
    let review = &result.review;
    let (_, label) = verdict(result);
    let mut out = String::from("## CodeShouldBehave Review\n\n");
    out.push_str(&format!("**{label}**\n\n"));
    out.push_str(&format!(
        "**Baseline:** `{}` → **Proposal:** `{}` (exact)\n\n",
        result.selected_baseline.as_str(),
        result.proposal.as_str()
    ));
    out.push_str(&format!(
        "**Coverage:** {} · **Flow landmarks edited:** {} · **Cross-file candidates:** {}\n\n",
        coverage_line(result).trim_start_matches("csb: "),
        review.flow_edits(),
        review.impacts.len()
    ));

    out.push_str("### Changed files\n\n");
    if review.files.is_empty() {
        out.push_str("No changed files between these revisions.\n\n");
    } else {
        out.push_str("| File | Change | + | − |\n|---|---|---:|---:|\n");
        for file in review.files.iter().take(50) {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                cell(&file.path),
                cell(&file.status),
                file.additions,
                file.deletions
            ));
        }
        if review.files.len() > 50 {
            out.push_str(&format!("\n…and {} more files.\n", review.files.len() - 50));
        }
        out.push('\n');
    }

    let changed = review
        .files
        .iter()
        .filter(|file| !file.added.is_empty() || !file.removed.is_empty())
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        out.push_str("### Behavioral changes\n\n");
        let mut budget = 20;
        for file in changed {
            if budget == 0 {
                out.push_str("- …more files changed their flow; open the full review.\n");
                break;
            }
            out.push_str(&format!(
                "- `{}`: {} added · {} removed landmark(s)\n",
                inline(&file.path),
                file.added.len(),
                file.removed.len()
            ));
            budget -= 1;
            for index in file.removed.iter().take(3) {
                if let Some(node) = file.before.nodes.get(*index) {
                    out.push_str(&format!(
                        "  - removed L{}: {}\n",
                        node.line,
                        inline(&node.label)
                    ));
                }
            }
            for index in file.added.iter().take(3) {
                if let Some(node) = file.after.nodes.get(*index) {
                    out.push_str(&format!(
                        "  - added L{}: {}\n",
                        node.line,
                        inline(&node.label)
                    ));
                }
            }
        }
        out.push('\n');
    }

    let added_edges = review
        .architecture
        .edges
        .iter()
        .filter(|(_, _, _, status)| *status == "added")
        .collect::<Vec<_>>();
    if !added_edges.is_empty() {
        out.push_str("### New cross-file candidates\n\n");
        for (from, to, name, _) in added_edges.iter().take(10) {
            out.push_str(&format!(
                "- `{}` → `{}` via `{}` (name match, unverified)\n",
                inline(from),
                inline(to),
                inline(name)
            ));
        }
        if added_edges.len() > 10 {
            out.push_str(&format!(
                "- …and {} more; inspect the architecture map.\n",
                added_edges.len() - 10
            ));
        }
        out.push('\n');
    }

    if !review.warnings.is_empty() || !review.architecture.notes.is_empty() {
        out.push_str("### Warnings\n\n");
        for warning in review
            .warnings
            .iter()
            .chain(&review.architecture.notes)
            .take(20)
        {
            out.push_str(&format!("- {}\n", inline(warning)));
        }
        out.push('\n');
    }

    let notes = review
        .files
        .iter()
        .flat_map(|file| file.notes.iter().map(move |note| (&file.path, note)))
        .collect::<Vec<_>>();
    if !notes.is_empty() {
        out.push_str("### Coverage gaps\n\n");
        for (path, note) in notes.iter().take(20) {
            out.push_str(&format!("- `{}`: {}\n", inline(path), inline(note)));
        }
        if notes.len() > 20 {
            out.push_str(&format!("- …and {} more.\n", notes.len() - 20));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "_Static, heuristic review. No reviewed code was executed. {}_\n",
        if incomplete(result) {
            "Coverage is incomplete; treat missing evidence as unknown."
        } else {
            "Coverage is complete within the selected scope."
        }
    ));
    out
}

fn incomplete_for_files(review: &Review) -> bool {
    review.files.iter().any(|file| !file.notes.is_empty())
}

fn index_json(result: &AnalysisResult, design: bool) -> Value {
    let review = &result.review;
    let (kind, label) = verdict(result);
    let mut routes = Map::new();
    routes.insert("report".into(), json!("index.html"));
    routes.insert("markdown".into(), json!("review.md"));
    routes.insert("summary".into(), json!("summary.md"));
    routes.insert("review".into(), json!("review.json"));
    routes.insert("manifest".into(), json!("manifest.json"));
    if design {
        routes.insert("design".into(), json!("design.md"));
        routes.insert("artifact".into(), json!("artifact.json"));
    }
    json!({
        "schema": SITE_SCHEMA,
        "generated_by": {"name": "csb", "version": env!("CARGO_PKG_VERSION")},
        "title": review.title,
        "analysis_review_id": result.review_id.as_str(),
        "baseline": result.baseline.as_str(),
        "selected_baseline": result.selected_baseline.as_str(),
        "proposal": result.proposal.as_str(),
        "scope_version": result.scope_version.as_str(),
        "coverage": coverage_json(&result.coverage),
        "verdict": {"kind": kind, "label": label},
        "incomplete": incomplete(result) || incomplete_for_files(review),
        "counts": {
            "files": review.files.len(),
            "flow_edits": review.flow_edits(),
            "impacts": review.impacts.len(),
            "warnings": review.warnings.len(),
        },
        "files": review.files.iter().map(|file| json!({
            "path": file.path,
            "status": file.status,
            "additions": file.additions,
            "deletions": file.deletions,
        })).collect::<Vec<_>>(),
        "routes": routes,
    })
}

fn review_json(result: &AnalysisResult, source_paths: &[String]) -> Value {
    let review = &result.review;
    let architecture = &review.architecture;
    json!({
        "schema": SITE_SCHEMA,
        "artifact_version": "csb-local-artifact-v1",
        "analysis_review_id": result.review_id.as_str(),
        "title": review.title,
        "baseline": result.baseline.as_str(),
        "selected_baseline": result.selected_baseline.as_str(),
        "proposal": result.proposal.as_str(),
        "scope_version": result.scope_version.as_str(),
        "coverage": coverage_json(&result.coverage),
        "files": review.files.iter().zip(source_paths).map(|(file, source)| json!({
            "path": file.path,
            "status": file.status,
            "additions": file.additions,
            "deletions": file.deletions,
            "removed_landmarks": file.removed,
            "added_landmarks": file.added,
            "notes": file.notes,
            "evidence": format!("sources/{source}"),
        })).collect::<Vec<_>>(),
        "architecture": {
            "before": architecture.before.iter().map(file_node_json).collect::<Vec<_>>(),
            "after": architecture.after.iter().map(file_node_json).collect::<Vec<_>>(),
            "edges": architecture.edges.iter().map(|(from, to, name, status)| json!({
                "from": from, "to": to, "name": name, "status": status,
            })).collect::<Vec<_>>(),
            "notes": architecture.notes,
            "excluded": architecture.excluded,
            "unsupported": architecture.unsupported,
            "failed": architecture.failed,
            "deferred": architecture.deferred,
            "enumeration_complete": architecture.enumeration_complete,
        },
        "impacts": review.impacts.iter().map(|impact| json!({
            "from": impact.from, "to": impact.to, "reason": impact.reason, "change": impact.change,
        })).collect::<Vec<_>>(),
        "warnings": review.warnings,
    })
}

fn file_node_json(node: &crate::architecture::FileNode) -> Value {
    json!({
        "path": node.path,
        "oid": node.oid,
        "definitions": node.definitions,
        "calls": node.calls,
    })
}

fn source_json(file: &FileReview) -> Value {
    json!({
        "path": file.path,
        "status": file.status,
        "patch": file.patch,
        "additions": file.additions,
        "deletions": file.deletions,
        "notes": file.notes,
        "removed_landmarks": file.removed,
        "added_landmarks": file.added,
        "before": flow_json(&file.before),
        "after": flow_json(&file.after),
    })
}

fn flow_json(flow: &crate::analysis::Flow) -> Value {
    json!({
        "source": flow.source,
        "nodes": flow.nodes.iter().map(|node| json!({
            "kind": node.kind, "label": node.label, "line": node.line,
        })).collect::<Vec<_>>(),
        "definitions": flow.definitions,
        "calls": flow.calls,
        "imports": flow.imports,
        "supported": flow.supported,
        "truncated": flow.truncated,
    })
}

fn coverage_json(coverage: &Coverage) -> Value {
    json!({
        "indexed": coverage.indexed,
        "unsupported": coverage.unsupported,
        "excluded": coverage.excluded,
        "failed": coverage.failed,
        "deferred": coverage.deferred,
        "enumeration_complete": coverage.enumeration_complete,
    })
}

fn source_paths(review: &Review) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    review
        .files
        .iter()
        .map(|file| {
            let mut digest = sha256(file.path.as_bytes());
            if let Some(count) = seen.get_mut(&digest) {
                *count += 1;
                digest.push_str(&format!("-{count}"));
            } else {
                seen.insert(digest.clone(), 0);
            }
            format!("{digest}.json")
        })
        .collect()
}

fn encode(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|_| "Cannot encode review value".into())
}

fn cell(value: &str) -> String {
    let value = clean(value);
    if value.contains('`') {
        value.replace('|', "\\|")
    } else {
        format!("`{}`", value.replace('|', "\\|"))
    }
}

fn inline(value: &str) -> String {
    clean(value).replace('`', "'")
}

fn clean(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}
