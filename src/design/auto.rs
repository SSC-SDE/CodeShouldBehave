//! Provisional design context derived from a committed Cargo workspace.
//!
//! This never replaces human confirmation. It exists so a local reviewer can
//! see an explicitly provisional HLD/LLD without a web onboarding session.
//! Every component is labeled provisional by the caller and by the report.

use super::safe_path;
use crate::{ComponentDecision, ComponentProposal, EvidenceConfidence, ScopeRecord, SnapshotId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_COMPONENTS: usize = 100;

/// Build the accepted-context JSON consumed by `ConfirmedDesignContext`.
pub fn provisional_context(
    repository: &Path,
    commit: &SnapshotId,
    scope: &ScopeRecord,
) -> Result<Value, String> {
    let manifest = crate::git::git(
        repository,
        &["show", &format!("{}:Cargo.toml", commit.as_str())],
        MAX_MANIFEST_BYTES,
    )
    .map_err(|_| "--design auto requires a Cargo.toml at the reviewed head commit".to_string())?;
    let manifest = String::from_utf8(manifest)
        .map_err(|_| "Cargo.toml at the reviewed head commit is not UTF-8".to_string())?;
    let tree = crate::git::git(
        repository,
        &["ls-tree", "-r", "--name-only", commit.as_str()],
        MAX_MANIFEST_BYTES,
    )
    .map_err(|_| "Cannot list committed paths for --design auto".to_string())?;
    let tree =
        String::from_utf8(tree).map_err(|_| "Committed path list is not UTF-8".to_string())?;
    let paths: Vec<String> = tree.lines().map(str::to_owned).collect();
    let mut names = std::collections::BTreeMap::new();
    for path in paths
        .iter()
        .filter(|path| path.ends_with("Cargo.toml"))
        .take(MAX_COMPONENTS)
    {
        let Ok(bytes) = crate::git::git(
            repository,
            &["show", &format!("{}:{path}", commit.as_str())],
            64 * 1024,
        ) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let Ok(value) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        if let Some(name) = value
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        {
            let root = path.strip_suffix("/Cargo.toml").unwrap_or(".");
            names.insert(
                if root.is_empty() {
                    ".".to_owned()
                } else {
                    root.to_owned()
                },
                name.to_owned(),
            );
        }
    }
    let (mut components, gaps) = components_from_cargo(&manifest, &paths, &names, scope)?;
    components.sort_by(|left, right| left.id.cmp(&right.id));
    let components: Vec<Value> = components
        .iter()
        .map(|component| serde_json::to_value(component).map_err(|_| "Cannot encode component"))
        .collect::<Result<_, _>>()?;
    let mut gaps = gaps;
    gaps.push(
        "Component boundaries are workspace-derived and provisional; runtime ownership and service boundaries are unconfirmed.".into(),
    );
    gaps.sort();
    gaps.dedup();
    let input =
        fingerprint(&serde_json::to_vec(&components).map_err(|_| "Cannot encode components")?);
    let mut proposal = Sha256::new();
    proposal.update(b"csb-auto-context-proposal-v1");
    proposal.update(commit.as_str().as_bytes());
    proposal.update(input.as_bytes());
    let proposal = hex(&proposal.finalize());
    let repository_id = &scope.primary_repository;
    Ok(json!({
        "version": "csb-accepted-context-v1",
        "usable": true,
        "sensitive_paths_excluded": true,
        "authority": "provisional",
        "repository": {
            "security_domain": repository_id.security_domain().as_str(),
            "provider": repository_id.provider(),
            "owner": repository_id.owner(),
            "name": repository_id.name(),
        },
        "commit": commit.as_str(),
        "session_id": "auto-workspace",
        "session_revision": 1,
        "input_fingerprint": input,
        "proposal_fingerprint": proposal,
        "confirmation": {
            "input_fingerprint": input,
            "proposal_fingerprint": proposal,
            "confirmed_at_ms": 0,
            "actor": "auto-workspace",
        },
        "components": components,
        "providers": [],
        "integrations": [],
        "contexts": [],
        "gaps": gaps,
    }))
}

fn components_from_cargo(
    manifest: &str,
    paths: &[String],
    names: &std::collections::BTreeMap<String, String>,
    scope: &ScopeRecord,
) -> Result<(Vec<ComponentProposal>, Vec<String>), String> {
    let manifest: toml::Value = toml::from_str(manifest)
        .map_err(|_| "Cargo.toml at the reviewed head is invalid".to_string())?;
    let cargo_files: BTreeSet<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|path| path.ends_with("Cargo.toml"))
        .collect();
    let mut gaps = Vec::new();
    let members: Vec<String> = match manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
    {
        Some(values) => {
            let mut roots = Vec::new();
            for value in values {
                let pattern = value.as_str().ok_or("Workspace member is not a string")?;
                if pattern.contains("**") {
                    gaps.push(format!(
                        "Workspace member pattern {pattern} is unsupported by --design auto and was not expanded"
                    ));
                } else if let Some(prefix) = pattern.strip_suffix("/*") {
                    let needle = format!("{prefix}/");
                    let mut matches: Vec<&str> = cargo_files
                        .iter()
                        .copied()
                        .filter_map(|path| path.strip_suffix("/Cargo.toml"))
                        .filter(|root| {
                            root.starts_with(&needle)
                                && !root[needle.len()..].contains('/')
                                && !root[needle.len()..].is_empty()
                        })
                        .collect();
                    matches.sort();
                    if matches.is_empty() {
                        gaps.push(format!(
                            "Workspace member pattern {pattern} matched no committed Cargo.toml"
                        ));
                    }
                    roots.extend(matches.into_iter().map(str::to_owned));
                } else {
                    let root = pattern.trim_end_matches('/').to_owned();
                    if !cargo_files.contains(format!("{root}/Cargo.toml").as_str())
                        && !root.is_empty()
                    {
                        gaps.push(format!(
                            "Workspace member {root} has no committed Cargo.toml"
                        ));
                    }
                    roots.push(if root.is_empty() {
                        ".".to_owned()
                    } else {
                        root
                    });
                }
            }
            roots
        }
        None => vec![".".to_owned()],
    };
    let mut roots = members;
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err("Cargo workspace declares no usable members".into());
    }
    if roots.len() > MAX_COMPONENTS {
        return Err(format!(
            "--design auto supports at most {MAX_COMPONENTS} workspace components"
        ));
    }
    let mut used = BTreeSet::new();
    let mut components = Vec::new();
    for root in roots {
        if !safe_path(&root, scope) {
            return Err(format!(
                "Workspace component {root} is outside the approved review scope"
            ));
        }
        let name = names.get(&root).cloned().unwrap_or_else(|| {
            if root == "." {
                "root".to_owned()
            } else {
                root.rsplit('/').next().unwrap_or("component").to_owned()
            }
        });
        let id = unique_id(&name, &mut used);
        components.push(ComponentProposal {
            id: id.clone(),
            name: name.clone(),
            role: format!("Provisional Rust component at {root}"),
            repository_index: 0,
            roots: vec![root.clone()],
            stack_candidates: vec!["rust".into()],
            signals: vec![],
            evidence: vec![],
            confidence: EvidenceConfidence::Candidate,
            contradictions: vec!["Workspace-derived boundary has no human confirmation".into()],
            decision: ComponentDecision::Confirmed,
        });
    }
    Ok((components, gaps))
}

fn unique_id(name: &str, used: &mut BTreeSet<String>) -> String {
    let mut base: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    base = base.trim_matches('-').to_owned();
    if base.is_empty() || base.len() > 64 {
        base = "component".to_owned();
    }
    let mut id = base.clone();
    let mut suffix = 2;
    while !used.insert(id.clone()) {
        id = format!("{}-{suffix}", &base[..base.len().min(56)]);
        suffix += 1;
    }
    id
}

fn fingerprint(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"csb-auto-context-input-v1");
    digest.update(bytes);
    hex(&digest.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PathScope, RepositoryId, ScopeVersion, SecurityDomain};

    fn scope() -> ScopeRecord {
        ScopeRecord {
            version: ScopeVersion::local_default(),
            primary_repository: RepositoryId::new(
                SecurityDomain::new("local").unwrap(),
                "local-git",
                "local-user",
                "fixture",
            )
            .unwrap(),
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: true,
            model_destinations: BTreeSet::new(),
        }
    }

    #[test]
    fn workspace_members_become_sorted_provisional_components() {
        let manifest = r#"
[workspace]
members = ["crates/core", "crates/app"]
"#;
        let paths = vec![
            "Cargo.toml".to_string(),
            "crates/app/Cargo.toml".to_string(),
            "crates/app/src/lib.rs".to_string(),
            "crates/core/Cargo.toml".to_string(),
            "crates/core/src/lib.rs".to_string(),
        ];
        let (components, gaps) = components_from_cargo(
            manifest,
            &paths,
            &std::collections::BTreeMap::new(),
            &scope(),
        )
        .unwrap();
        assert_eq!(components.len(), 2);
        assert_eq!(components[0].id, "app");
        assert_eq!(components[1].id, "core");
        assert!(components.iter().all(|component| {
            component
                .role
                .starts_with("Provisional Rust component at crates/")
        }));
        assert!(gaps.is_empty());
    }

    #[test]
    fn single_package_uses_root_component() {
        let manifest = "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n";
        let paths = vec!["Cargo.toml".to_string(), "src/lib.rs".to_string()];
        let (components, _) = components_from_cargo(
            manifest,
            &paths,
            &std::collections::BTreeMap::new(),
            &scope(),
        )
        .unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].roots, vec![".".to_string()]);
    }

    #[test]
    fn unsupported_glob_is_disclosed_not_fatal() {
        let manifest = "[workspace]\nmembers = [\"crates/**\", \"crates/one\"]\n";
        let paths = vec![
            "Cargo.toml".to_string(),
            "crates/one/Cargo.toml".to_string(),
        ];
        let (components, gaps) = components_from_cargo(
            manifest,
            &paths,
            &std::collections::BTreeMap::new(),
            &scope(),
        )
        .unwrap();
        assert_eq!(components.len(), 1);
        assert!(gaps.iter().any(|gap| gap.contains("crates/**")));
    }

    #[test]
    fn duplicate_names_receive_stable_suffixes() {
        let manifest = "[workspace]\nmembers = [\"a/tool\", \"b/tool\"]\n";
        let paths = vec![
            "Cargo.toml".to_string(),
            "a/tool/Cargo.toml".to_string(),
            "b/tool/Cargo.toml".to_string(),
        ];
        let (components, _) = components_from_cargo(
            manifest,
            &paths,
            &std::collections::BTreeMap::new(),
            &scope(),
        )
        .unwrap();
        assert_eq!(components[0].id, "tool");
        assert_eq!(components[1].id, "tool-2");
    }
}
