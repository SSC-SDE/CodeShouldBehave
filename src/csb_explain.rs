//! Explicit, bounded AI explanation over a verified local review directory.
//!
//! Deterministic review artifacts stay untouched. The explanation is advisory,
//! cites only supplied evidence indexes and never writes back into the review.

use crate::ai::{ModelAdapter, ModelRequest, redact_sensitive};
use crate::{OllamaAdapter, OpenAiAdapter, csb_site};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const RULE_VERSION: &str = "csb-explain-v1";
pub const MAX_EVIDENCE: usize = 16;
pub const MAX_EVIDENCE_BYTES: usize = 16 * 1024;
const MAX_KEY_BYTES: u64 = 4096;

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
    pub preview: PathBuf,
    pub explanation: Option<(PathBuf, PathBuf)>,
}

#[derive(Clone)]
struct Evidence {
    index: usize,
    side: &'static str,
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
    let change = hld
        .get("delta")
        .and_then(|delta| delta.get("changes"))
        .and_then(Value::as_array)
        .and_then(|changes| {
            changes.iter().find(|change| {
                change.get("id").and_then(Value::as_str) == Some(options.boundary.as_str())
            })
        })
        .ok_or_else(|| {
            ExplainError::Usage(format!("unknown HLD boundary: {}", options.boundary))
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
    let evidence = collect_evidence(hld, change, baseline, proposal)?;
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
            preview: preview_path,
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
        preview: preview_path,
        explanation: Some((markdown_path, json_path)),
    })
}

fn collect_evidence(
    hld: &Value,
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
    Ok(evidence)
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
            "\n[{}] side={} path={} lines={}-{}\n{}\n",
            item.index, item.side, item.path, item.start_line, item.end_line, item.text
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
        let evidence = collect_evidence(&hld(), &change, &"a".repeat(40), &"c".repeat(40)).unwrap();
        assert_eq!(evidence.len(), 1);
        assert!(!evidence[0].text.contains("api_key"));
        assert!(evidence[0].text.contains("[REDACTED"));
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
