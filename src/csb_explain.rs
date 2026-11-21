//! Explicit, bounded AI explanation over a verified local review directory.
//!
//! Deterministic review artifacts stay untouched. The explanation is advisory,
//! cites only supplied evidence indexes and never writes back into the review.

use crate::ai::{ModelAdapter, ModelRequest, redact_sensitive};
use crate::{OllamaAdapter, OpenAiAdapter, csb_site};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const RULE_VERSION: &str = "csb-explain-v2";
pub const MAX_EVIDENCE: usize = 16;
pub const MAX_EVIDENCE_BYTES: usize = 16 * 1024;
const MAX_KEY_BYTES: u64 = 4096;
const MAX_COMPONENT_DECLARATIONS: usize = 8;
const MAX_COMPONENT_RELATIONS: usize = 8;
const MAX_COMPONENT_BASE: usize = 12;
const MAX_CALLERS: usize = 8;
const MAX_LISTED_BOUNDARIES: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Ollama,
    OpenAi,
}

impl Provider {
    pub fn parse(value: &str) -> Result<Self, ExplainError> {
        match value {
            "ollama" => Ok(Self::Ollama),
            "openai" => Ok(Self::OpenAi),
            other => Err(ExplainError::Usage(format!(
                "--provider must be ollama or openai (got {other})"
            ))),
        }
    }

    fn default_endpoint(self) -> &'static str {
        match self {
            Self::Ollama => crate::ai::ollama::DEFAULT_ENDPOINT,
            Self::OpenAi => crate::ai::openai::DEFAULT_ENDPOINT,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAi => "openai",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExplainOptions {
    pub dir: PathBuf,
    pub boundary: String,
    pub provider: Provider,
    pub model: String,
    pub endpoint: Option<String>,
    pub key_file: Option<PathBuf>,
    pub send: bool,
    pub out: Option<PathBuf>,
    pub list: bool,
}

#[derive(Debug)]
pub enum ExplainError {
    Usage(String),
    Provider(String),
}

impl ExplainError {
    pub fn code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Provider(_) => 5,
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Self::Usage(message) | Self::Provider(message) => message,
        }
    }
}

pub struct ExplainOutcome {
    pub preview: Option<PathBuf>,
    pub explanation: Option<(PathBuf, PathBuf)>,
}

#[derive(Clone)]
struct Evidence {
    index: usize,
    side: &'static str,
    source_kind: &'static str,
    lld_entry: Option<String>,
    path: String,
    commit: String,
    blob: String,
    start_line: u64,
    end_line: u64,
    text: String,
}

pub fn run(options: &ExplainOptions) -> Result<ExplainOutcome, ExplainError> {
    let manifest = csb_site::verify(&options.dir).map_err(ExplainError::Usage)?;
    let index = read_json(&options.dir.join("index.json")).map_err(ExplainError::Usage)?;
    let artifact = read_json(&options.dir.join("artifact.json")).map_err(|_| {
        ExplainError::Usage(
            "artifact.json has no design; run csb review --design auto first".into(),
        )
    })?;
    let hld = artifact.get("hld").ok_or_else(|| {
        ExplainError::Usage(
            "artifact.json has no design; run csb review --design auto first".into(),
        )
    })?;
    let review_id = index
        .get("analysis_review_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ExplainError::Usage("index.json has no analysis_review_id".into()))?;
    let baseline = index
        .get("baseline")
        .and_then(Value::as_str)
        .ok_or_else(|| ExplainError::Usage("index.json has no baseline".into()))?;
    let proposal = index
        .get("proposal")
        .and_then(Value::as_str)
        .ok_or_else(|| ExplainError::Usage("index.json has no proposal".into()))?;
    if options.list {
        list_boundaries(hld, artifact.get("lld"), baseline, proposal)?;
        return Ok(ExplainOutcome {
            preview: None,
            explanation: None,
        });
    }
    let boundary = resolve_boundary(hld, &options.boundary)?;
    let change = &boundary.change;
    let lld = artifact.get("lld");
    let (mut evidence, declaration_names) = match boundary.component.as_deref() {
        Some(component) => {
            collect_component_evidence(hld, lld, change, component, baseline, proposal)?
        }
        None => {
            let evidence = collect_evidence(hld, lld, change, baseline, proposal)?;
            let names = selected_names(change);
            (evidence, names)
        }
    };
    let mut total = evidence.iter().map(|item| item.text.len()).sum::<usize>();
    append_callers(
        &mut evidence,
        &mut total,
        lld,
        &declaration_names,
        baseline,
        proposal,
    );
    if evidence.is_empty() {
        return Err(ExplainError::Usage(
            "selected boundary has no source evidence; choose another HLD change".into(),
        ));
    }
    let prompt = prompt(change, &evidence);
    let endpoint = options
        .endpoint
        .clone()
        .unwrap_or_else(|| options.provider.default_endpoint().to_owned());
    let bundle = json!({
        "schema": "csb-ai-bundle-v1",
        "review_id": review_id,
        "manifest_sha256": manifest.content_sha256,
        "boundary": options.boundary,
        "provider": options.provider.name(),
        "model": options.model,
        "rule_version": RULE_VERSION,
        "evidence": evidence.iter().map(|item| json!({
            "index": item.index,
            "side": item.side,
            "source_kind": item.source_kind,
            "lld_entry": item.lld_entry,
            "path": item.path,
            "commit": item.commit,
            "blob": item.blob,
            "start_line": item.start_line,
            "end_line": item.end_line,
            "sha256": csb_site::sha256(item.text.as_bytes()),
            "bytes": item.text.len(),
        })).collect::<Vec<_>>(),
    });
    let bundle_id = csb_site::sha256(
        &serde_json::to_vec(&bundle)
            .map_err(|_| ExplainError::Usage("Cannot encode AI bundle".into()))?,
    );
    let base = options
        .out
        .clone()
        .unwrap_or_else(|| options.dir.join("ai"));
    let preview_path = if options.out.is_some() {
        base.with_extension("preview.json")
    } else {
        options.dir.join("ai-preview.json")
    };
    let preview = json!({
        "schema": "csb-ai-preview-v1",
        "bundle_id": bundle_id,
        "review_id": review_id,
        "manifest_sha256": manifest.content_sha256,
        "boundary": options.boundary,
        "provider": options.provider.name(),
        "model": options.model,
        "destination": endpoint,
        "evidence": bundle["evidence"],
        "evidence_bytes": evidence.iter().map(|item| item.text.len()).sum::<usize>(),
        "prompt_bytes": prompt.len(),
    });
    write_json(&preview_path, &preview).map_err(ExplainError::Usage)?;
    if !options.send {
        return Ok(ExplainOutcome {
            preview: Some(preview_path),
            explanation: None,
        });
    }
    let key = match options.provider {
        Provider::Ollama => None,
        Provider::OpenAi => Some(read_key(options.key_file.as_deref())?),
    };
    let request = ModelRequest {
        contract_version: crate::AI_CONTRACT_VERSION,
        bundle_id: bundle_id.clone(),
        rule_version: RULE_VERSION.into(),
        prompt,
    };
    let result = match options.provider {
        Provider::Ollama => OllamaAdapter::new(endpoint, options.model.clone())
            .map_err(ExplainError::Usage)?
            .dispatch(&request),
        Provider::OpenAi => {
            OpenAiAdapter::new(endpoint, options.model.clone(), key.unwrap_or_default())
                .map_err(ExplainError::Usage)?
                .dispatch(&request)
        }
    }
    .map_err(ExplainError::Provider)?;
    if result.bundle_id != bundle_id || result.rule_version != RULE_VERSION {
        return Err(ExplainError::Provider(
            "provider_result_identity_mismatch".into(),
        ));
    }
    let value: Value = serde_json::from_str(&result.explanation)
        .map_err(|_| ExplainError::Provider("model_invalid_json".into()))?;
    let uncertain = change.get("kind").and_then(Value::as_str) == Some("uncertain");
    let mut value = validate_response(&value, evidence.len(), !uncertain)?;
    if uncertain {
        value["headline"] = json!("Unresolved design difference — verification required");
    }
    let explanation = json!({
        "schema": "csb-ai-explanation-v1",
        "bundle_id": bundle_id,
        "review_id": review_id,
        "manifest_sha256": manifest.content_sha256,
        "boundary": options.boundary,
        "provider": result.provider,
        "model": result.model,
        "rule_version": result.rule_version,
        "evidence": bundle["evidence"],
        "result": value,
    });
    let markdown_path = base.with_extension("md");
    let json_path = base.with_extension("json");
    write_json(&json_path, &explanation).map_err(ExplainError::Usage)?;
    std::fs::write(&markdown_path, render_markdown(&explanation)).map_err(|error| {
        ExplainError::Usage(format!("Cannot write {}: {error}", markdown_path.display()))
    })?;
    Ok(ExplainOutcome {
        preview: Some(preview_path),
        explanation: Some((markdown_path, json_path)),
    })
}

fn collect_evidence(
    hld: &Value,
    lld: Option<&Value>,
    change: &Value,
    baseline: &str,
    proposal: &str,
) -> Result<Vec<Evidence>, ExplainError> {
    let sources = hld
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| ExplainError::Usage("artifact.json has no HLD sources".into()))?;
    let by_id: BTreeMap<&str, &Value> = sources
        .iter()
        .filter_map(|source| {
            source
                .get("evidence_id")
                .and_then(Value::as_str)
                .map(|id| (id, source))
        })
        .collect();
    let ids = change
        .get("evidence_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| ExplainError::Usage("boundary has no evidence ids".into()))?;
    let mut evidence = Vec::new();
    let mut total = 0usize;
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        let Some(id) = id.as_str() else { continue };
        if !seen.insert(id) {
            continue;
        }
        let Some(source) = by_id.get(id) else {
            continue;
        };
        let text = source.get("text").and_then(Value::as_str).unwrap_or("");
        let text = redact_sensitive(text);
        if text.is_empty() || total + text.len() > MAX_EVIDENCE_BYTES {
            continue;
        }
        total += text.len();
        let commit = source.get("commit").and_then(Value::as_str).unwrap_or("");
        evidence.push(Evidence {
            index: evidence.len(),
            side: if commit == baseline {
                "baseline"
            } else if commit == proposal {
                "proposal"
            } else {
                "both"
            },
            source_kind: "hld",
            lld_entry: None,
            path: source
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            commit: commit.to_owned(),
            blob: source
                .get("blob_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            start_line: source
                .get("start_line")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            end_line: source.get("end_line").and_then(Value::as_u64).unwrap_or(0),
            text,
        });
        if evidence.len() >= MAX_EVIDENCE {
            break;
        }
    }
    append_lld_bodies(&mut evidence, &mut total, lld, change, baseline, proposal);
    Ok(evidence)
}

struct Boundary {
    change: Value,
    component: Option<String>,
}

fn resolve_boundary(hld: &Value, boundary: &str) -> Result<Boundary, ExplainError> {
    if let Some(component) = boundary.strip_prefix("component:") {
        let element = ["baseline", "proposal"]
            .iter()
            .filter_map(|side| hld.get(*side))
            .filter_map(|version| version.get("elements").and_then(Value::as_array))
            .flatten()
            .find(|element| {
                element.get("kind").and_then(Value::as_str) == Some("component")
                    && element.get("id").and_then(Value::as_str) == Some(component)
            })
            .ok_or_else(|| {
                ExplainError::Usage(format!("unknown component boundary: {component}"))
            })?;
        let name = element
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(component);
        return Ok(Boundary {
            change: json!({
                "id": boundary,
                "kind": "component",
                "element_id": component,
                "before_name": Value::Null,
                "after_name": name,
                "reason": "Explicit component boundary; aggregates static contracts, relations, possible impacts and retained declarations only"
            }),
            component: Some(component.to_owned()),
        });
    }
    let change = hld
        .get("delta")
        .and_then(|delta| delta.get("changes"))
        .and_then(Value::as_array)
        .and_then(|changes| {
            changes
                .iter()
                .find(|change| change.get("id").and_then(Value::as_str) == Some(boundary))
        })
        .cloned()
        .ok_or_else(|| ExplainError::Usage(format!("unknown HLD boundary: {boundary}")))?;
    Ok(Boundary {
        change,
        component: None,
    })
}

fn selected_names(change: &Value) -> BTreeSet<String> {
    ["before_name", "after_name"]
        .iter()
        .filter_map(|key| change.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn is_declaration_kind(kind: &str) -> bool {
    !matches!(kind, "file" | "parameter" | "configuration" | "schema")
}

#[allow(clippy::too_many_arguments)]
fn push_hld_span(
    evidence: &mut Vec<Evidence>,
    total: &mut usize,
    seen: &mut BTreeSet<String>,
    by_id: &BTreeMap<&str, &Value>,
    id: &str,
    baseline: &str,
    proposal: &str,
    source_kind: &'static str,
) {
    if evidence.len() >= MAX_COMPONENT_BASE || !seen.insert(id.to_owned()) {
        return;
    }
    let Some(source) = by_id.get(id) else { return };
    let text = source.get("text").and_then(Value::as_str).unwrap_or("");
    let text = redact_sensitive(text);
    if text.is_empty() || *total + text.len() > MAX_EVIDENCE_BYTES {
        return;
    }
    *total += text.len();
    let commit = source.get("commit").and_then(Value::as_str).unwrap_or("");
    evidence.push(Evidence {
        index: evidence.len(),
        side: if commit == baseline {
            "baseline"
        } else if commit == proposal {
            "proposal"
        } else {
            "both"
        },
        source_kind,
        lld_entry: None,
        path: source
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        commit: commit.to_owned(),
        blob: source
            .get("blob_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        start_line: source
            .get("start_line")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        end_line: source.get("end_line").and_then(Value::as_u64).unwrap_or(0),
        text,
    });
}

fn collect_component_evidence(
    hld: &Value,
    lld: Option<&Value>,
    change: &Value,
    component: &str,
    baseline: &str,
    proposal: &str,
) -> Result<(Vec<Evidence>, BTreeSet<String>), ExplainError> {
    let sources = hld
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| ExplainError::Usage("artifact.json has no HLD sources".into()))?;
    let by_id: BTreeMap<&str, &Value> = sources
        .iter()
        .filter_map(|source| {
            source
                .get("evidence_id")
                .and_then(Value::as_str)
                .map(|id| (id, source))
        })
        .collect();
    let mut member_ids = BTreeSet::new();
    for side in ["baseline", "proposal"] {
        let Some(elements) = hld
            .get(side)
            .and_then(|version| version.get("elements"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for element in elements {
            let owned = element.get("component_id").and_then(Value::as_str) == Some(component);
            let is_component = element.get("id").and_then(Value::as_str) == Some(component);
            if (owned || is_component)
                && let Some(id) = element.get("id").and_then(Value::as_str)
            {
                member_ids.insert(id.to_owned());
            }
        }
    }
    let mut relations: Vec<&Value> = Vec::new();
    let mut relation_ids = BTreeSet::new();
    for side in ["baseline", "proposal"] {
        let Some(items) = hld
            .get(side)
            .and_then(|version| version.get("relations"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for relation in items {
            let from = relation.get("from").and_then(Value::as_str).unwrap_or("");
            let to = relation.get("to").and_then(Value::as_str).unwrap_or("");
            let Some(id) = relation.get("id").and_then(Value::as_str) else {
                continue;
            };
            if (member_ids.contains(from) || member_ids.contains(to))
                && relation_ids.insert(id.to_owned())
                && relations.len() < MAX_COMPONENT_RELATIONS
            {
                relations.push(relation);
            }
        }
    }
    let mut evidence = Vec::new();
    let mut total = 0usize;
    let mut seen = BTreeSet::new();
    if let Some(changes) = hld
        .get("delta")
        .and_then(|delta| delta.get("changes"))
        .and_then(Value::as_array)
    {
        for item in changes {
            let element = item.get("element_id").and_then(Value::as_str);
            let relation = item.get("relation_id").and_then(Value::as_str);
            let belongs = element.is_some_and(|id| member_ids.contains(id))
                || relation.is_some_and(|id| relation_ids.contains(id));
            if !belongs {
                continue;
            }
            if let Some(ids) = item.get("evidence_ids").and_then(Value::as_array) {
                for id in ids.iter().filter_map(Value::as_str) {
                    push_hld_span(
                        &mut evidence,
                        &mut total,
                        &mut seen,
                        &by_id,
                        id,
                        baseline,
                        proposal,
                        "component",
                    );
                }
            }
            if evidence.len() >= MAX_COMPONENT_BASE {
                break;
            }
        }
    }
    for relation in &relations {
        if let Some(ids) = relation.get("evidence_ids").and_then(Value::as_array) {
            for id in ids.iter().filter_map(Value::as_str) {
                push_hld_span(
                    &mut evidence,
                    &mut total,
                    &mut seen,
                    &by_id,
                    id,
                    baseline,
                    proposal,
                    "component",
                );
            }
        }
        if evidence.len() >= MAX_COMPONENT_BASE {
            break;
        }
    }
    if let Some(impacts) = hld
        .get("delta")
        .and_then(|delta| delta.get("possible_impacts"))
        .and_then(Value::as_array)
    {
        for impact in impacts {
            if impact.get("component_id").and_then(Value::as_str) != Some(component) {
                continue;
            }
            let Some(ids) = impact.get("via_relation_ids").and_then(Value::as_array) else {
                continue;
            };
            for relation_id in ids.iter().filter_map(Value::as_str) {
                for side in ["baseline", "proposal"] {
                    let Some(relation) = hld
                        .get(side)
                        .and_then(|version| version.get("relations"))
                        .and_then(Value::as_array)
                        .and_then(|items| {
                            items.iter().find(|item| {
                                item.get("id").and_then(Value::as_str) == Some(relation_id)
                            })
                        })
                    else {
                        continue;
                    };
                    if let Some(evidence_ids) =
                        relation.get("evidence_ids").and_then(Value::as_array)
                    {
                        for id in evidence_ids.iter().filter_map(Value::as_str) {
                            push_hld_span(
                                &mut evidence,
                                &mut total,
                                &mut seen,
                                &by_id,
                                id,
                                baseline,
                                proposal,
                                "component",
                            );
                        }
                    }
                }
            }
        }
    }
    let mut declarations = Vec::new();
    let mut declaration_names = BTreeSet::new();
    let mut declaration_ids = BTreeSet::new();
    for side in ["baseline", "proposal"] {
        let Some(entries) = lld
            .and_then(|lld| lld.get(side))
            .and_then(|value| value.get("catalog"))
            .and_then(|catalog| catalog.get("entries"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            if entry
                .get("parent")
                .and_then(|parent| parent.get("component_id"))
                .and_then(Value::as_str)
                != Some(component)
            {
                continue;
            }
            let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
            if !is_declaration_kind(kind) {
                continue;
            }
            let Some(id) = entry.get("id").and_then(Value::as_str) else {
                continue;
            };
            if !declaration_ids.insert(id.to_owned())
                || declarations.len() >= MAX_COMPONENT_DECLARATIONS
            {
                continue;
            }
            if let Some(name) = entry.get("name").and_then(Value::as_str) {
                declaration_names.insert(name.to_owned());
            }
            declarations.push((side, entry));
        }
    }
    let mut seen_entries = BTreeSet::new();
    for (side, entry) in declarations {
        let commit = if side == "baseline" {
            baseline
        } else {
            proposal
        };
        append_entry_evidence(
            &mut evidence,
            &mut total,
            lld,
            side,
            commit,
            entry,
            &mut seen_entries,
            "",
        );
        if evidence.len() >= MAX_EVIDENCE {
            break;
        }
    }
    if evidence.is_empty() {
        return Err(ExplainError::Usage(format!(
            "component {} has no retained evidence; choose another boundary",
            change
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(component)
        )));
    }
    Ok((evidence, declaration_names))
}

fn list_boundaries(
    hld: &Value,
    lld: Option<&Value>,
    baseline: &str,
    proposal: &str,
) -> Result<String, ExplainError> {
    let changes = hld
        .get("delta")
        .and_then(|delta| delta.get("changes"))
        .and_then(Value::as_array)
        .ok_or_else(|| ExplainError::Usage("artifact.json has no HLD delta changes".into()))?;
    let mut sorted = changes.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|change| {
        change
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    });
    let mut out = String::from("id\tkind\tpath\tdeclaration\tevidence_bytes\tuncertain\n");
    for change in sorted.into_iter().take(MAX_LISTED_BOUNDARIES) {
        let id = change.get("id").and_then(Value::as_str).unwrap_or("");
        let kind = change.get("kind").and_then(Value::as_str).unwrap_or("");
        let declaration = ["after_name", "before_name"]
            .iter()
            .find_map(|key| change.get(*key).and_then(Value::as_str))
            .unwrap_or("");
        let evidence = collect_evidence(hld, lld, change, baseline, proposal).unwrap_or_default();
        let bytes = evidence.iter().map(|item| item.text.len()).sum::<usize>();
        let path = evidence
            .first()
            .map(|item| item.path.as_str())
            .unwrap_or("");
        out.push_str(&format!(
            "{id}\t{kind}\t{path}\t{declaration}\t{bytes}\t{}\n",
            kind == "uncertain"
        ));
    }
    print!("{out}");
    Ok(out)
}

fn append_callers(
    evidence: &mut Vec<Evidence>,
    total: &mut usize,
    lld: Option<&Value>,
    declarations: &BTreeSet<String>,
    baseline: &str,
    proposal: &str,
) {
    let Some(lld) = lld else { return };
    let Some(callers) = lld
        .get("delta")
        .and_then(|delta| delta.get("possible_callers"))
        .and_then(Value::as_array)
    else {
        return;
    };
    let mut seen = BTreeSet::new();
    let mut added = 0usize;
    for caller in callers {
        if added >= MAX_CALLERS || evidence.len() >= MAX_EVIDENCE {
            break;
        }
        let logical_key = caller
            .get("logical_key")
            .and_then(Value::as_str)
            .unwrap_or("");
        let proposal_id = caller.get("proposal_entry_id").and_then(Value::as_str);
        let baseline_id = caller.get("baseline_entry_id").and_then(Value::as_str);
        let (side, commit, entry_id) = if let Some(id) = proposal_id {
            ("proposal", proposal, id)
        } else if let Some(id) = baseline_id {
            ("baseline", baseline, id)
        } else {
            continue;
        };
        if !seen.insert(entry_id.to_owned()) {
            continue;
        }
        let entry = lld
            .get(side)
            .and_then(|value| value.get("catalog"))
            .and_then(|catalog| catalog.get("entries"))
            .and_then(Value::as_array)
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
            });
        let matched = match entry {
            Some(entry) if !declarations.is_empty() => entry
                .get("call_observations")
                .and_then(Value::as_array)
                .and_then(|calls| {
                    calls.iter().filter_map(Value::as_str).find_map(|call| {
                        let terminal = call
                            .rsplit("::")
                            .next()
                            .unwrap_or(call)
                            .rsplit('.')
                            .next()
                            .unwrap_or(call);
                        declarations.get(terminal).cloned()
                    })
                }),
            Some(_) => None,
            None => None,
        };
        if !declarations.is_empty() && matched.is_none() {
            continue;
        }
        let name = matched
            .as_deref()
            .or_else(|| entry.and_then(|entry| entry.get("name").and_then(Value::as_str)))
            .or_else(|| logical_key.rsplit('|').next())
            .unwrap_or("");
        let (path, blob, start_line, end_line, text) = match entry {
            Some(entry) => {
                let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
                let blob = entry.get("blob").and_then(Value::as_str).unwrap_or("");
                let start_line = entry.get("start_line").and_then(Value::as_u64).unwrap_or(0);
                let end_line = entry.get("end_line").and_then(Value::as_u64).unwrap_or(0);
                let signature = entry.get("signature").and_then(Value::as_str).unwrap_or("");
                let mut text = format!(
                    "Lexical caller candidate for declaration `{name}` (possible_callers; not a resolved runtime call):\n\
                     {signature}\n\
                     path={path} lines={start_line}-{end_line}\n\
                     call-site line: not retained in the bounded catalog\n\
                     logical_key={logical_key}"
                );
                if let Some(calls) = entry.get("call_observations").and_then(Value::as_array) {
                    let observed = calls
                        .iter()
                        .filter_map(Value::as_str)
                        .take(8)
                        .collect::<Vec<_>>();
                    if !observed.is_empty() {
                        text.push_str(&format!(
                            "\nobserved lexical calls: {}",
                            observed.join(", ")
                        ));
                    }
                }
                (path.to_owned(), blob.to_owned(), start_line, end_line, text)
            }
            None => (
                String::new(),
                String::new(),
                0,
                0,
                format!(
                    "Lexical caller candidate (possible_callers; not a resolved runtime call):\n\
                     caller entry is not retained on the {side} side\n\
                     logical_key={logical_key}\n\
                     call-site line: unknown"
                ),
            ),
        };
        let text = redact_sensitive(&text);
        if text.is_empty() || *total + text.len() > MAX_EVIDENCE_BYTES {
            continue;
        }
        *total += text.len();
        evidence.push(Evidence {
            index: evidence.len(),
            side: if side == "baseline" {
                "baseline"
            } else {
                "proposal"
            },
            source_kind: "call_candidate",
            lld_entry: Some(entry_id.to_owned()),
            path,
            commit: commit.to_owned(),
            blob,
            start_line,
            end_line,
            text,
        });
        added += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn append_entry_evidence(
    evidence: &mut Vec<Evidence>,
    total: &mut usize,
    lld: Option<&Value>,
    side: &str,
    commit: &str,
    entry: &Value,
    seen_entries: &mut BTreeSet<String>,
    duplicate_body: &str,
) {
    if evidence.len() >= MAX_EVIDENCE {
        return;
    }
    let Some(lld) = lld else { return };
    let Some(side_value) = lld.get(side) else {
        return;
    };
    let Some(entry_id) = entry.get("id").and_then(Value::as_str) else {
        return;
    };
    if !seen_entries.insert(entry_id.to_owned()) {
        return;
    }
    let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
    let start_line = entry.get("start_line").and_then(Value::as_u64).unwrap_or(0);
    let end_line = entry.get("end_line").and_then(Value::as_u64).unwrap_or(0);
    if let Some(inputs) = side_value.get("inputs").and_then(Value::as_array) {
        if let Some((source, blob)) = inputs
            .iter()
            .find_map(|input| {
                (input.get("path").and_then(Value::as_str) == Some(path)).then(|| {
                    (
                        input.get("source").and_then(Value::as_str),
                        input.get("blob").and_then(Value::as_str),
                    )
                })
            })
            .and_then(|(source, blob)| Some((source?, blob?)))
        {
            let start = entry.get("start_byte").and_then(Value::as_u64).unwrap_or(0) as usize;
            let end = entry.get("end_byte").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(text) = source.get(start..end) {
                let text = redact_sensitive(text);
                if !text.is_empty()
                    && text.trim() != duplicate_body.trim()
                    && *total + text.len() <= MAX_EVIDENCE_BYTES
                {
                    *total += text.len();
                    evidence.push(Evidence {
                        index: evidence.len(),
                        side: if side == "baseline" {
                            "baseline"
                        } else {
                            "proposal"
                        },
                        source_kind: "lld_body",
                        lld_entry: Some(entry_id.to_owned()),
                        path: path.to_owned(),
                        commit: commit.to_owned(),
                        blob: blob.to_owned(),
                        start_line,
                        end_line,
                        text,
                    });
                }
            }
        }
    }
    if evidence.len() >= MAX_EVIDENCE {
        return;
    }
    if let Some(calls) = entry.get("call_observations").and_then(Value::as_array) {
        let mut observed: Vec<String> = calls
            .iter()
            .filter_map(Value::as_str)
            .take(20)
            .map(|call| call.chars().take(120).collect())
            .collect();
        observed.dedup();
        if !observed.is_empty() {
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("");
            let text = format!(
                "Deterministic lexical call observations for {name}:\n- {}",
                observed.join("\n- ")
            );
            if *total + text.len() <= MAX_EVIDENCE_BYTES {
                *total += text.len();
                evidence.push(Evidence {
                    index: evidence.len(),
                    side: if side == "baseline" {
                        "baseline"
                    } else {
                        "proposal"
                    },
                    source_kind: "lld_calls",
                    lld_entry: Some(entry_id.to_owned()),
                    path: path.to_owned(),
                    commit: commit.to_owned(),
                    blob: String::new(),
                    start_line,
                    end_line,
                    text,
                });
            }
        }
    }
}

fn append_lld_bodies(
    evidence: &mut Vec<Evidence>,
    total: &mut usize,
    lld: Option<&Value>,
    change: &Value,
    baseline: &str,
    proposal: &str,
) {
    let Some(lld) = lld else { return };
    let spans = evidence.clone();
    let mut seen_entries = std::collections::BTreeSet::new();
    for span in spans {
        for (side, commit) in [("baseline", baseline), ("proposal", proposal)] {
            if span.side != side && span.side != "both" {
                continue;
            }
            let Some(side_value) = lld.get(side) else {
                continue;
            };
            let Some(entries) = side_value
                .get("catalog")
                .and_then(|catalog| catalog.get("entries"))
                .and_then(Value::as_array)
            else {
                continue;
            };
            let Some(entry) = matching_entry(entries, &span, change) else {
                continue;
            };
            append_entry_evidence(
                evidence,
                total,
                Some(lld),
                side,
                commit,
                entry,
                &mut seen_entries,
                &span.text,
            );
            if evidence.len() >= MAX_EVIDENCE {
                return;
            }
        }
    }
}

fn matching_entry<'a>(entries: &'a [Value], span: &Evidence, change: &Value) -> Option<&'a Value> {
    let names: Vec<&str> = ["before_name", "after_name"]
        .iter()
        .filter_map(|key| change.get(*key).and_then(Value::as_str))
        .collect();
    entries
        .iter()
        .filter(|entry| entry.get("kind").and_then(Value::as_str) != Some("file"))
        .filter(|entry| entry.get("path").and_then(Value::as_str) == Some(span.path.as_str()))
        .filter(|entry| {
            let start = entry.get("start_line").and_then(Value::as_u64).unwrap_or(0);
            let end = entry.get("end_line").and_then(Value::as_u64).unwrap_or(0);
            (start <= span.start_line && end >= span.end_line)
                || names
                    .iter()
                    .any(|name| entry.get("name").and_then(Value::as_str) == Some(*name))
        })
        .min_by_key(|entry| {
            entry
                .get("end_line")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .saturating_sub(entry.get("start_line").and_then(Value::as_u64).unwrap_or(0))
        })
}

fn prompt(change: &Value, evidence: &[Evidence]) -> String {
    let mut prompt = String::from(
        "Explain the selected high-level-design change for a Rust pull request.\n\
         Write for a senior Rust reviewer. State the design consequence of the change, not the syntax that changed.\n\
         Use only the evidence below. Do not invent runtime behavior, ownership or intent.\n\
         Cite evidence by its bracketed index. Return JSON with keys headline, changes, questions, unknowns.\n\
         At most 5 claims, 5 questions and 5 unknowns. Do not restate the evidence.\n\
         Questions must be specific to this change and answerable from code; omit generic compatibility, risk or timeline questions.\n\
         If the evidence is only a declaration line, say that behavior and callers are not established instead of guessing.\n\n",
    );
    if change.get("kind").and_then(Value::as_str) == Some("uncertain") {
        prompt.push_str(
            "This boundary is marked uncertain/unresolved. Do not present evidence items as added, removed or modified. \
             Return an empty changes array. Explain only that the difference is unresolved, why the deterministic coverage is incomplete, and what a reviewer must verify.\n\n",
        );
    }
    if change.get("kind").and_then(Value::as_str) == Some("component") {
        prompt.push_str(
            "This is an explicitly selected component boundary. The evidence aggregates static contracts, relations and retained declarations; it is not a complete component inventory. \
             Cite at least two independent items and reference at least one relation when present. Do not claim runtime order, reachability, ownership or intent.\n\n",
        );
    }
    if evidence
        .iter()
        .any(|item| item.source_kind == "call_candidate")
    {
        prompt.push_str(
            "Evidence of kind call_candidate is a lexical name-only caller candidate, not a resolved runtime call or a proven edge. \
             Name at least one caller and cite its index, but do not claim runtime order or reachability and do not infer caller behavior. \
             If no call_candidate evidence is present, state explicitly that callers are unknown.\n\n",
        );
    }
    prompt.push_str(&format!(
        "Boundary: {}\nBefore: {}\nAfter: {}\nReason: {}\n\nEvidence:\n",
        change.get("kind").and_then(Value::as_str).unwrap_or(""),
        change
            .get("before_name")
            .and_then(Value::as_str)
            .unwrap_or("—"),
        change
            .get("after_name")
            .and_then(Value::as_str)
            .unwrap_or("—"),
        change.get("reason").and_then(Value::as_str).unwrap_or(""),
    ));
    for item in evidence {
        prompt.push_str(&format!(
            "\n[{}] side={} kind={} path={} lines={}-{}\n{}\n",
            item.index,
            item.side,
            item.source_kind,
            item.path,
            item.start_line,
            item.end_line,
            item.text
        ));
    }
    prompt
}

fn validate_response(
    value: &Value,
    evidence_len: usize,
    allow_changes: bool,
) -> Result<Value, ExplainError> {
    let object = value
        .as_object()
        .ok_or_else(|| ExplainError::Provider("model_invalid_schema".into()))?;
    let allowed = ["headline", "changes", "questions", "unknowns"];
    if object.len() != allowed.len() || !allowed.iter().all(|key| object.contains_key(*key)) {
        return Err(ExplainError::Provider("model_invalid_schema".into()));
    }
    let text = |value: &Value, max: usize| -> Result<String, ExplainError> {
        let value = value
            .as_str()
            .ok_or_else(|| ExplainError::Provider("model_invalid_schema".into()))?;
        if value.len() > max
            || value.chars().any(char::is_control)
            || value.to_ascii_lowercase().contains("<script")
            || value.to_ascii_lowercase().contains("<iframe")
        {
            return Err(ExplainError::Provider("model_invalid_text".into()));
        }
        Ok(value.to_owned())
    };
    let headline = text(&object["headline"], 200)?;
    if !allow_changes
        && object["changes"]
            .as_array()
            .is_none_or(|changes| !changes.is_empty())
    {
        return Err(ExplainError::Provider("model_invalid_certainty".into()));
    }
    let mut changes = Vec::new();
    for change in object["changes"]
        .as_array()
        .ok_or_else(|| ExplainError::Provider("model_invalid_schema".into()))?
    {
        if changes.len() >= 10 {
            return Err(ExplainError::Provider("model_invalid_schema".into()));
        }
        let side = change
            .get("side")
            .and_then(Value::as_str)
            .filter(|side| matches!(*side, "baseline" | "proposal" | "both"))
            .ok_or_else(|| ExplainError::Provider("model_invalid_schema".into()))?;
        let claim = text(&change["claim"], 500)?;
        let ids = change
            .get("evidence_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| ExplainError::Provider("model_invalid_schema".into()))?;
        if ids.len() > 16 {
            return Err(ExplainError::Provider("model_invalid_schema".into()));
        }
        let mut citations = Vec::new();
        for id in ids {
            let id = id
                .as_u64()
                .and_then(|id| usize::try_from(id).ok())
                .filter(|id| *id < evidence_len)
                .ok_or_else(|| ExplainError::Provider("model_invalid_citation".into()))?;
            citations.push(id);
        }
        changes.push(json!({"claim": claim, "side": side, "evidence_ids": citations}));
    }
    let list = |value: &Value| -> Result<Vec<String>, ExplainError> {
        let items = value
            .as_array()
            .ok_or_else(|| ExplainError::Provider("model_invalid_schema".into()))?;
        if items.len() > 10 {
            return Err(ExplainError::Provider("model_invalid_schema".into()));
        }
        items.iter().map(|item| text(item, 300)).collect()
    };
    Ok(json!({
        "headline": headline,
        "changes": changes,
        "questions": list(&object["questions"])?,
        "unknowns": list(&object["unknowns"])?,
    }))
}

fn render_markdown(explanation: &Value) -> String {
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let result = &explanation["result"];
    let mut out = String::from(
        "# AI explanation (advisory)\n\n> Advisory AI interpretation — not observed evidence. The deterministic review remains the source of truth.\n\n",
    );
    out.push_str(&format!(
        "**Headline:** {}\n\n",
        escape(result["headline"].as_str().unwrap_or(""))
    ));
    out.push_str("## Change claims\n\n");
    if let Some(changes) = result["changes"].as_array() {
        for change in changes {
            let ids = change["evidence_ids"]
                .as_array()
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_u64)
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            out.push_str(&format!(
                "- **{}** — {} _(evidence: {})_\n",
                escape(change["side"].as_str().unwrap_or("")),
                escape(change["claim"].as_str().unwrap_or("")),
                ids
            ));
        }
    }
    for (title, key) in [("Review questions", "questions"), ("Unknowns", "unknowns")] {
        out.push_str(&format!("\n## {title}\n\n"));
        if let Some(items) = result[key].as_array() {
            for item in items {
                out.push_str(&format!("- {}\n", escape(item.as_str().unwrap_or(""))));
            }
        }
    }
    out.push_str(&format!(
        "\n---\nProvider: `{}` · model: `{}` · bundle: `{}`\n",
        escape(explanation["provider"].as_str().unwrap_or("")),
        escape(explanation["model"].as_str().unwrap_or("")),
        escape(explanation["bundle_id"].as_str().unwrap_or(""))
    ));
    out
}

fn read_key(path: Option<&Path>) -> Result<String, ExplainError> {
    if let Some(path) = path {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| {
            ExplainError::Usage(format!("{} is not a readable key file", path.display()))
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_KEY_BYTES
        {
            return Err(ExplainError::Usage(format!(
                "{} must be a regular key file of at most 4 KiB",
                path.display()
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o077 != 0 {
                return Err(ExplainError::Usage(
                    "key file permissions must not grant group or other access".into(),
                ));
            }
        }
        let key = std::fs::read_to_string(path)
            .map_err(|_| ExplainError::Usage("Cannot read key file".into()))?;
        return valid_key(key.trim());
    }
    match std::env::var("CSB_AI_API_KEY") {
        Ok(key) => valid_key(key.trim()),
        Err(_) => Err(ExplainError::Usage(
            "--provider openai requires CSB_AI_API_KEY or --key-file".into(),
        )),
    }
}

fn valid_key(key: &str) -> Result<String, ExplainError> {
    if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_control) {
        return Err(ExplainError::Usage("AI key is invalid".into()));
    }
    Ok(key.to_owned())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|_| format!("{} is not valid JSON", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "Cannot encode JSON")?;
    std::fs::write(path, bytes).map_err(|error| format!("Cannot write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hld() -> Value {
        json!({
            "sources": [{
                "evidence_id": "e1",
                "commit": "a".repeat(40),
                "path": "src/lib.rs",
                "blob_id": "b".repeat(40),
                "start_line": 1,
                "end_line": 2,
                "text": "pub fn add(api_key: &str) -> i32 { 1 }\n",
                "truncated": false
            }]
        })
    }

    #[test]
    fn evidence_text_is_redacted_before_the_prompt() {
        let change = json!({"evidence_ids": ["e1"]});
        let evidence =
            collect_evidence(&hld(), None, &change, &"a".repeat(40), &"c".repeat(40)).unwrap();
        assert_eq!(evidence.len(), 1);
        assert!(!evidence[0].text.contains("api_key"));
        assert!(evidence[0].text.contains("[REDACTED"));
    }

    #[test]
    fn lld_function_body_is_appended_to_the_signature_evidence() {
        let commit = "c".repeat(40);
        let source_text = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let hld = json!({
            "sources": [{
                "evidence_id": "e1",
                "commit": commit,
                "path": "src/lib.rs",
                "blob_id": "b".repeat(40),
                "start_line": 1,
                "end_line": 1,
                "text": "pub fn add(a: i32, b: i32) -> i32\n"
            }]
        });
        let change = json!({"evidence_ids": ["e1"], "after_name": "add"});
        let lld = json!({
            "baseline": {"catalog": {"entries": []}, "inputs": []},
            "proposal": {
                "catalog": {
                    "snapshot": commit,
                    "entries": [{
                        "id": "lld-entry-1",
                        "kind": "function",
                        "name": "add",
                        "path": "src/lib.rs",
                        "start_line": 1,
                        "end_line": 3,
                        "start_byte": 0,
                        "end_byte": source_text.len()
                    }]
                },
                "inputs": [{
                    "path": "src/lib.rs",
                    "source": source_text,
                    "blob": "d".repeat(40)
                }]
            }
        });
        let evidence =
            collect_evidence(&hld, Some(&lld), &change, &"a".repeat(40), &commit).unwrap();
        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[0].source_kind, "hld");
        assert_eq!(evidence[1].source_kind, "lld_body");
        assert!(evidence[1].text.contains("a + b"));
    }

    #[test]
    fn caller_candidates_are_lexical_only_and_include_signatures() {
        let lld = json!({
            "proposal": {
                "catalog": {"entries": [{
                    "id": "caller-1",
                    "name": "handle",
                    "path": "src/api.rs",
                    "blob": "b".repeat(40),
                    "start_line": 10,
                    "end_line": 12,
                    "signature": "pub fn handle(a: i32) -> i32",
                    "call_observations": ["core::add"]
                }]},
                "inputs": []
            },
            "delta": {"possible_callers": [{
                "proposal_entry_id": "caller-1",
                "logical_key": "api||function|handle"
            }]}
        });
        let mut evidence = Vec::new();
        let mut total = 0;
        append_callers(
            &mut evidence,
            &mut total,
            Some(&lld),
            &BTreeSet::from(["add".to_owned()]),
            &"a".repeat(40),
            &"c".repeat(40),
        );
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].source_kind, "call_candidate");
        assert!(evidence[0].text.contains("handle"));
        assert!(evidence[0].text.contains("pub fn handle(a: i32) -> i32"));
        assert!(evidence[0].text.contains("not a resolved runtime call"));
        assert!(evidence[0].text.contains("call-site line: not retained"));
    }

    #[test]
    fn response_rejects_foreign_citation_and_markup() {
        let foreign = json!({
            "headline": "x",
            "changes": [{"claim": "y", "side": "proposal", "evidence_ids": [1]}],
            "questions": [],
            "unknowns": []
        });
        assert!(matches!(
            validate_response(&foreign, 1, true),
            Err(ExplainError::Provider(message)) if message == "model_invalid_citation"
        ));
        assert!(matches!(
            validate_response(&foreign, 1, false),
            Err(ExplainError::Provider(message)) if message == "model_invalid_certainty"
        ));
        let script = json!({
            "headline": "<script>alert(1)</script>",
            "changes": [],
            "questions": [],
            "unknowns": []
        });
        assert!(matches!(
            validate_response(&script, 1, true),
            Err(ExplainError::Provider(message)) if message == "model_invalid_text"
        ));
    }
}
