//! Trusted local host bridge. Operator artifact paths never come from HTTP JSON.
use flowdiff::design::{
    DesignVersion, compare_hld, evaluate_hld_constraints, evaluate_hld_decisions,
    validate_design_scope,
};
use flowdiff::design_storage::{DesignCorrection, DesignInterpretation};
use flowdiff::{Grant, PathScope, PrincipalId, RepositoryId, ScopeRecord, ScopeVersion, Storage};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs::File, io::Read, path::Path};

#[derive(Deserialize)]
struct Candidates {
    baseline: DesignVersion,
    proposal: DesignVersion,
}

pub(super) fn operator_artifact(path: &Path) -> Result<Value, String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|_| "HLD artifact unavailable")?;
    if !path.is_absolute()
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 16 * 1024 * 1024
    {
        return Err("HLD requires a bounded private operator artifact".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err("HLD artifact is not private".into());
        }
    }
    // This is trusted operator input. Metadata identity plus a bounded read
    // prevents a mutable browser request from choosing another source record.
    let mut file = File::open(path).map_err(|_| "HLD artifact unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata().map_err(|_| "HLD artifact unavailable")?;
        if !opened.is_file()
            || opened.dev() != metadata.dev()
            || opened.ino() != metadata.ino()
            || opened.uid() != metadata.uid()
            || opened.mode() & 0o077 != 0
            || opened.nlink() != 1
        {
            return Err("HLD artifact changed".into());
        }
    }
    let mut bytes = vec![];
    file.by_ref()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "HLD artifact unavailable")?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("HLD artifact exceeds limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid HLD artifact".into())
}
fn paths(value: &Value, key: &str) -> Result<Vec<String>, String> {
    let items = value[key].as_array().ok_or("Invalid HLD paths")?;
    if items.len() > 100 {
        return Err("HLD path limit".into());
    }
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or("Invalid HLD path".into())
        })
        .collect()
}
pub(super) fn candidate_scope(
    value: &Value,
    repository: &RepositoryId,
) -> Result<ScopeRecord, String> {
    let manifest = &value["scope_manifest"];
    let identity = &manifest["primary_repository"];
    if identity["security_domain"] != repository.security_domain().as_str()
        || identity["provider"] != repository.provider()
        || identity["owner"] != repository.owner()
        || identity["name"] != repository.name()
        || manifest["contexts"]
            .as_array()
            .is_none_or(|items| !items.is_empty())
    {
        return Err("HLD artifact repository or primary scope is invalid".into());
    }
    Ok(ScopeRecord {
        version: ScopeVersion::new(value["scope_version"].as_str().ok_or("Invalid HLD scope")?)?,
        primary_repository: repository.clone(),
        primary_paths: PathScope::new(paths(manifest, "include")?, paths(manifest, "exclude")?)?,
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: manifest["export_allowed"].as_bool().unwrap_or(false),
        model_destinations: BTreeSet::new(),
    })
}
fn field<'a>(request: &'a Value, name: &str) -> Result<&'a str, String> {
    request[name]
        .as_str()
        .ok_or_else(|| format!("Missing HLD {name}"))
}
fn revision(request: &Value) -> Result<u64, String> {
    request["expected_revision"]
        .as_u64()
        .ok_or("Missing HLD expected revision".into())
}

pub fn invoke(
    storage: &mut Storage,
    repository: &RepositoryId,
    principal: &PrincipalId,
    grants: &[Grant],
    action: &str,
    request: Value,
) -> Result<Value, String> {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Invalid clock")?
        .as_millis();
    let at = i64::try_from(at).map_err(|_| "Clock overflow")?;
    let value = operator_artifact(Path::new(field(&request, "artifact_path")?))?;
    if value["artifact_version"] != "csb-review-artifact-v1" {
        return Err("Invalid HLD artifact version".into());
    }
    let comparison = field(&request, "comparison_id")?;
    let pair = comparison
        .strip_prefix("pr-")
        .and_then(|value| value.split_once('-'))
        .ok_or("Invalid HLD comparison")?;
    if pair.0.parse::<u64>().ok().is_none_or(|number| number == 0) || value["review_id"] != pair.1 {
        return Err("HLD artifact comparison mismatch".into());
    }
    if matches!(action, "hld-preview" | "hld-accept") {
        let key = Sha256::digest(comparison.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let prefix = format!("hld-review-{}-", &key[..32]);
        let suffix = field(&request, "preview_id")?
            .strip_prefix(&prefix)
            .ok_or("HLD preview comparison mismatch")?;
        if suffix.len() != 32 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid HLD preview identity".into());
        }
    }
    let candidates: Candidates = serde_json::from_value(value["hld"].clone())
        .map_err(|_| "HLD generation unavailable; regenerate this review")?;
    let scope = candidate_scope(&value, repository)?;
    if candidates.baseline.snapshot.as_str()
        != value["baseline"].as_str().ok_or("Invalid HLD baseline")?
        || candidates.proposal.snapshot.as_str()
            != value["proposal"].as_str().ok_or("Invalid HLD proposal")?
    {
        return Err("HLD snapshot mismatch".into());
    }
    if action == "hld-accept" {
        let review = storage.load_design_review(
            repository,
            field(&request, "preview_id")?,
            principal,
            grants,
        )?;
        if review.design_id != candidates.baseline.id && review.design_id != candidates.proposal.id
        {
            return Err("HLD preview belongs to another comparison".into());
        }
        if review.corrections.iter().any(|correction| {
            correction
                .reference_design_id
                .as_ref()
                .is_some_and(|id| id != &candidates.baseline.id && id != &candidates.proposal.id)
        }) {
            return Err("HLD correspondence is outside this comparison".into());
        }
        let accepted = storage.confirm_design_review(
            repository,
            &review.id,
            revision(&request)?,
            field(&request, "input_fingerprint")?,
            request["known_gaps_accepted"]
                .as_bool()
                .ok_or("Missing HLD gap acknowledgement")?,
            request["activate_default"]
                .as_bool()
                .ok_or("Missing HLD activation decision")?,
            principal,
            grants,
            at,
        )?;
        return Ok(json!({"accepted":accepted}));
    }
    for design in [&candidates.baseline, &candidates.proposal] {
        validate_design_scope(design, &scope)?;
        if action != "hld-read"
            && storage
                .load_design_version(repository, &design.id, principal, grants)
                .is_err()
        {
            storage.save_design_version(design, &scope, principal, grants, at)?;
        }
    }
    if action == "hld-preview" {
        let design_id = field(&request, "design_id")?;
        if design_id != candidates.baseline.id && design_id != candidates.proposal.id {
            return Err("Unknown HLD candidate".into());
        }
        let interpretation: DesignInterpretation =
            serde_json::from_value(request["interpretation"].clone())
                .map_err(|_| "Invalid HLD interpretation")?;
        let corrections: Vec<DesignCorrection> =
            serde_json::from_value(request["corrections"].clone())
                .map_err(|_| "Invalid HLD corrections")?;
        if corrections.iter().any(|correction| {
            correction
                .reference_design_id
                .as_ref()
                .is_some_and(|id| id != &candidates.baseline.id && id != &candidates.proposal.id)
        }) {
            return Err("HLD correspondence is outside this comparison".into());
        }
        let id = field(&request, "preview_id")?;
        if request["create_preview"] == true {
            if revision(&request)? != 0 {
                return Err("Stale HLD initial preview".into());
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
            return Err("HLD preview candidate changed".into());
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
        let design = storage.preview_design_interpretation(repository, id, principal, grants)?;
        return Ok(
            json!({"preview_id":review.id,"expected_revision":review.revision,"input_fingerprint":review.input_fingerprint,"review":review,"design":design}),
        );
    }
    if action != "hld-view" && action != "hld-read" {
        return Err("Invalid HLD action".into());
    }
    let baseline =
        storage.load_design_version(repository, &candidates.baseline.id, principal, grants)?;
    let proposal =
        storage.load_design_version(repository, &candidates.proposal.id, principal, grants)?;
    // No default and a default unavailable under current paths share the same
    // nondisclosing response. Never substitute intended design for as-built.
    let accepted = storage
        .load_design_default(repository, DesignInterpretation::AsBuilt, principal, grants)
        .ok();
    let intended = storage
        .load_design_default(
            repository,
            DesignInterpretation::Intended,
            principal,
            grants,
        )
        .ok();
    let export_views = json!({"baseline":storage.export_design_version(repository,&baseline.id,principal,grants).is_ok(),
        "proposal":storage.export_design_version(repository,&proposal.id,principal,grants).is_ok(),
        "accepted":accepted.as_ref().is_some_and(|accepted|storage.export_design_acceptance(repository,&accepted.accepted_id,principal,grants).is_ok())});
    let mut delta = compare_hld(&baseline, &proposal)?;
    let drift = accepted
        .as_ref()
        .and_then(|accepted| compare_hld(&accepted.design, &baseline).ok());
    if let Some(accepted) = &accepted {
        delta.findings = evaluate_hld_constraints(&proposal, &accepted.design.constraints);
        delta
            .findings
            .extend(evaluate_hld_decisions(&proposal, &accepted.design));
    }
    Ok(
        json!({"baseline":baseline,"proposal":proposal,"delta":delta,"accepted":accepted.as_ref().map(|accepted|&accepted.design),
        "accepted_record":accepted,"accepted_intended":intended.as_ref().map(|accepted|&accepted.design),"accepted_intended_record":intended,"accepted_drift":drift,"export_views":export_views}),
    )
}
