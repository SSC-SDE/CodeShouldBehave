//! Primary-repository CLI handoff from a trusted, confirmed onboarding record.
//! The hosted caller must fetch this record under current grants; a local file
//! is operator input, never proof of remote authorization.
use crate::{PathScope, RepositoryId, ScopeRecord, ScopeVersion, SecurityDomain, SnapshotId};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

#[derive(Deserialize)]
struct Context {
    version: String,
    session_id: String,
    session_revision: u64,
    commit: String,
    input_fingerprint: String,
    proposal_fingerprint: String,
    accepted_scope_fingerprint: String,
    scope_version: String,
    export_allowed: bool,
    sensitive_paths_excluded: bool,
    repository: Repository,
    include: Vec<String>,
    exclude: Vec<String>,
    #[serde(default)]
    contexts: Vec<Value>,
    #[serde(default)]
    contracts: Vec<Value>,
    usable: bool,
    confirmation: Value,
    #[serde(default)]
    drift: Option<Value>,
    #[serde(default)]
    components: Vec<crate::ComponentProposal>,
    #[serde(default)]
    providers: Vec<crate::ComponentProviderMapping>,
    #[serde(default)]
    integrations: Vec<crate::IntegrationCandidate>,
    #[serde(default)]
    gaps: Vec<String>,
}
#[derive(Deserialize)]
struct Repository {
    security_domain: String,
    provider: String,
    owner: String,
    name: String,
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn load_onboarding_review_context(
    path: &Path,
    repository: &RepositoryId,
) -> Result<(PathScope, ScopeVersion, Value), String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|_| "Accepted context file unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1048576 {
        return Err("Accepted context must be a regular file bounded to 1 MiB".into());
    }
    let mut bytes = vec![];
    File::open(path)
        .map_err(|_| "Accepted context file unavailable")?
        .take(1048577)
        .read_to_end(&mut bytes)
        .map_err(|_| "Accepted context file unavailable")?;
    if bytes.len() > 1048576 {
        return Err("Accepted context exceeds 1 MiB".into());
    }
    let context: Context =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid accepted context")?;
    if context.version != "csb-accepted-context-v1"
        || !context.usable
        || !context.sensitive_paths_excluded
        || context.repository.provider != "github"
        || repository.provider() != "github"
        || context.repository.owner != repository.owner()
        || context.repository.name != repository.name()
        || context.repository.security_domain.is_empty()
        || context.session_id.is_empty()
        || context.session_id.len() > 128
        || context.session_id.chars().any(char::is_control)
        || context.session_revision == 0
        || !hash(&context.input_fingerprint)
        || !hash(&context.proposal_fingerprint)
        || !hash(&context.accepted_scope_fingerprint)
        || context.include.len() > 100
        || context.exclude.len() > 100
        || context
            .include
            .iter()
            .chain(&context.exclude)
            .any(|value| value.chars().any(char::is_control))
        || context.confirmation["input_fingerprint"].as_str() != Some(&context.input_fingerprint)
        || context.confirmation["proposal_fingerprint"].as_str()
            != Some(&context.proposal_fingerprint)
        || context.confirmation["confirmed_at_ms"]
            .as_i64()
            .is_none_or(|value| value < 0)
        || context.confirmation["actor"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        return Err("Accepted context identity or confirmation is invalid".into());
    }
    if !context.contexts.is_empty() || !context.contracts.is_empty() {
        return Err("Related repository review requires a separately authorized mirror adapter; primary-only MVP context required".into());
    }
    SnapshotId::new(context.commit.clone())?;
    let paths = PathScope::new(context.include.clone(), context.exclude.clone())?;
    let source_scope = ScopeRecord {
        version: ScopeVersion::new(context.scope_version.clone())?,
        primary_repository: RepositoryId::new(
            SecurityDomain::new(context.repository.security_domain.clone())?,
            "github",
            repository.owner(),
            repository.name(),
        )?,
        primary_paths: paths.clone(),
        contexts: vec![],
        contracts: Default::default(),
        export_allowed: context.export_allowed,
        model_destinations: Default::default(),
    };
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"csb-accepted-context-v1:sensitive-path-policy-v1");
    fingerprint.update(source_scope.canonical_bytes());
    if fingerprint
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        != context.accepted_scope_fingerprint
    {
        return Err("Accepted context paths differ from its scope fingerprint".into());
    }
    if let Some(drift) = &context.drift {
        validate_drift(drift, &context, &paths)?;
    }
    // An incomplete/disabled drift assessment cannot replace a complete replay
    // under the same immutable artifact ID, even with identical accepted paths.
    let interpretation = if let Some(drift) = &context.drift {
        let mut digest = Sha256::new();
        digest.update(b"csb-onboarding-review-drift-v1");
        digest.update(context.proposal_fingerprint.as_bytes());
        digest.update(serde_json::to_vec(drift).map_err(|_| "Invalid drift provenance")?);
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    } else {
        context.proposal_fingerprint.clone()
    };
    let scope_version = ScopeVersion::new(format!("onboarding-{interpretation}"))?;
    let mut provenance = json!({"version":context.version,"session_id":context.session_id,"session_revision":context.session_revision,
        "accepted_commit":context.commit,"input_fingerprint":context.input_fingerprint,
        "proposal_fingerprint":context.proposal_fingerprint,"accepted_scope_fingerprint":context.accepted_scope_fingerprint,
        "source_security_domain":context.repository.security_domain,"export_allowed":context.export_allowed,"confirmation":context.confirmation});
    if !context.components.is_empty() {
        // Only the scoped, typed accepted projection travels into the design
        // generator. Unknown file fields and private onboarding observations
        // cannot become diagram labels or artifact metadata.
        let confirmed = json!({"version":context.version,"session_id":context.session_id,"session_revision":context.session_revision,
            "commit":context.commit,"repository":{"security_domain":context.repository.security_domain,"provider":context.repository.provider,"owner":context.repository.owner,"name":context.repository.name},
            "input_fingerprint":context.input_fingerprint,"proposal_fingerprint":context.proposal_fingerprint,
            "accepted_scope_fingerprint":context.accepted_scope_fingerprint,"scope_version":context.scope_version,
            "export_allowed":context.export_allowed,"sensitive_paths_excluded":context.sensitive_paths_excluded,
            "include":context.include,"exclude":context.exclude,"contexts":[],"contracts":[],"usable":context.usable,
            "confirmation":context.confirmation,"components":context.components,"providers":context.providers,
            "integrations":context.integrations,"gaps":context.gaps});
        crate::onboarding_service::validate_accepted_context_scope(&confirmed)?;
        crate::design::ConfirmedDesignContext::from_accepted(&confirmed, &source_scope)?;
        provenance["confirmed_context"] = confirmed;
    }
    if let Some(drift) = context.drift {
        provenance["drift"] = drift;
    }
    Ok((paths, scope_version, provenance))
}

fn validate_drift(drift: &Value, context: &Context, paths: &PathScope) -> Result<(), String> {
    let invalid = || "Accepted context drift provenance is invalid".to_string();
    let status = drift["status"].as_str().ok_or_else(invalid)?;
    let unchanged = status == "unchanged";
    if drift["version"] != "csb-onboarding-drift-v1"
        || drift["session_id"] != context.session_id
        || drift["session_revision"] != context.session_revision
        || drift["source_commit"] != context.commit
        || drift["input_fingerprint"] != context.input_fingerprint
        || drift["proposal_fingerprint"] != context.proposal_fingerprint
        || drift["accepted_scope_fingerprint"] != context.accepted_scope_fingerprint
        || !matches!(status, "unchanged" | "changed" | "incomplete")
        || drift["reuse_allowed"] != unchanged
        || drift["reconfirmation_required"] != !unchanged
        || drift["defaults_changed"] != false
        || drift["scope_expanded"] != false
    {
        return Err(invalid());
    }
    let target = drift["target_commit"].as_str().ok_or_else(invalid)?;
    SnapshotId::new(target.to_owned())?;
    let affected = drift["affected_components"]
        .as_array()
        .ok_or_else(invalid)?;
    let gaps = drift["gaps"].as_array().ok_or_else(invalid)?;
    if affected.len() > 100 || gaps.len() > 20 || (unchanged && !affected.is_empty()) {
        return Err(invalid());
    }
    for component in affected {
        let evidence = component["evidence"].as_array().ok_or_else(invalid)?;
        for item in evidence {
            let path = item["path"].as_str().ok_or_else(invalid)?;
            SnapshotId::new(item["blob"].as_str().ok_or_else(invalid)?.to_owned())?;
            let commit = match item["side"].as_str() {
                Some("baseline") => context.commit.as_str(),
                Some("target") => target,
                _ => return Err(invalid()),
            };
            if item["commit"] != commit
                || item["repository_index"] != 0
                || item["producer_version"] != "csb-onboarding-drift-v1"
                || item["start_line"] != 1
                || item["end_line"] != 1
                || !paths.allows(path)
                || crate::sensitive_path(path)
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
