//! Static configuration adapters. No commands, URLs, dashboard or secret APIs.
use crate::git::BatchBlob;
use crate::snapshot::{ReadLimits, read_chunk};
use crate::{
    Cancellation, ComponentDecision, ComponentProposal, ComponentProviderMapping, Grant,
    OnboardingEvidence, OnboardingSession, PrincipalId, ProviderVerification, RepositoryId,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const VERSION: &str = "csb-static-hosting-v1-2026-10-07";

fn adapter(path: &str) -> Option<(&'static str, bool)> {
    match path.rsplit('/').next()? {
        "netlify.toml" => Some(("Netlify", false)),
        "vercel.json" | "vercel.toml" => Some(("Vercel", false)),
        "vercel.ts" => Some(("Vercel", true)),
        "railway.json" | "railway.toml" => Some(("Railway", false)),
        "railway.ts" if path.ends_with(".railway/railway.ts") => Some(("Railway", true)),
        _ => None,
    }
}
fn directory(path: &str) -> &str {
    path.rsplit_once('/').map_or(".", |(parent, _)| parent)
}
fn under(path: &str, root: &str) -> bool {
    root == "."
        || root == path
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}
fn static_root(value: &str) -> Option<String> {
    if value.len() > 256
        || value.chars().any(char::is_control)
        || value.contains(['$', '{', '}', '\\', ':', '*', '?'])
    {
        return None;
    }
    let value = value.trim_matches('/').trim_end_matches('/');
    let value = value.strip_prefix("./").unwrap_or(value);
    let value = if value.is_empty() { "." } else { value };
    let paths = crate::PathScope::new(vec![value.into()], vec![]).ok()?;
    if crate::sensitive_path(value) {
        return None;
    }
    Some(paths.includes().next()?.into())
}

pub(crate) fn detect(
    roots: &BTreeMap<RepositoryId, PathBuf>,
    session: &OnboardingSession,
    components: &[ComponentProposal],
    principal: &PrincipalId,
    grants: &[Grant],
) -> Result<(Vec<ComponentProviderMapping>, Vec<String>), String> {
    session.check_access(principal, grants)?;
    let inventory = session
        .inventory()
        .filter(|item| item.complete)
        .ok_or("Complete inventory required")?;
    let configs = inventory
        .items
        .iter()
        .filter(|item| item.status == "indexed" && adapter(&item.path).is_some())
        .collect::<Vec<_>>();
    let mut mappings = Vec::new();
    let mut gaps = Vec::new();
    let started = std::time::Instant::now();
    if configs.len() > 64 {
        gaps.push("Hosting detection is limited to 64 configuration files; remaining mappings are unknown".into());
    }
    for item in configs.into_iter().take(64) {
        if started.elapsed() > std::time::Duration::from_secs(10) {
            gaps.push("Static hosting detection reached its ten-second budget; remaining mappings are unknown".into());
            break;
        }
        let (repository, commit, paths) =
            crate::onboarding_scan::pin(session, item.repository_index)?;
        let local = roots.get(repository).ok_or("Repository unavailable")?;
        let (provider, executable) = adapter(&item.path).unwrap();
        if executable {
            gaps.push(format!("{provider} executable configuration is present; it was not evaluated or executed. Confirm its component/provider mapping separately"));
            continue;
        }
        let blobs = read_chunk(
            local,
            std::slice::from_ref(&item.blob),
            ReadLimits {
                objects_per_chunk: 1,
                bytes_per_object: 256 * 1024,
                bytes_per_chunk: 256 * 1024,
            },
            &Cancellation::default(),
        )?;
        let Some(BatchBlob::Data(bytes)) = blobs.into_iter().next() else {
            gaps.push(format!(
                "{provider} configuration content is unavailable; deployment remains unknown"
            ));
            continue;
        };
        let Ok(source) = std::str::from_utf8(&bytes) else {
            gaps.push(format!("{provider} configuration encoding is unsupported"));
            continue;
        };
        let data = if item.path.ends_with(".json") {
            serde_json::from_str::<Value>(source).ok()
        } else {
            toml::from_str::<Value>(source).ok()
        };
        let Some(data) = data.filter(Value::is_object) else {
            gaps.push(format!(
                "{provider} configuration is invalid or unsupported; no mapping was asserted"
            ));
            continue;
        };
        let mut notes = vec!["Repository configuration is a candidate, not evidence of an active deployment; confirm project, source root and environment".into()];
        let config_root = directory(&item.path);
        let mut candidates = vec![("unknown", config_root.to_owned())];
        match provider {
            "Netlify" => {
                if let Some(base) = data.get("build").and_then(|build| build.get("base")) {
                    let Some(base) = base.as_str().and_then(static_root) else {
                        gaps.push("Netlify build base is dynamic or unsupported; no root mapping was asserted".into());
                        continue;
                    };
                    candidates[0].1 = base;
                } else {
                    notes.push("No explicit base directory; dashboard/package-directory overrides and inherited roots are unknown".into());
                }
                if data.get("functions").is_some()
                    || data.get("build").and_then(|v| v.get("functions")).is_some()
                {
                    notes.push("Function configuration is present; source ownership and deployment require confirmation".into());
                }
                if data.get("build").and_then(|v| v.get("publish")).is_some() {
                    notes.push("A publish directory is declared; it is build output, not proof of a separate component".into());
                }
                if let Some(contexts) = data.get("context").and_then(Value::as_object) {
                    for (key, environment) in [
                        ("production", "production"),
                        ("deploy-preview", "preview"),
                        ("branch-deploy", "branch deployment"),
                        ("dev", "development"),
                    ] {
                        if let Some(context) = contexts.get(key) {
                            let base = match context.get("base") {
                                Some(value) => value.as_str().and_then(static_root),
                                None => Some(candidates[0].1.clone()),
                            };
                            if let Some(base) = base {
                                candidates.push((environment, base));
                            } else {
                                gaps.push("A Netlify context has an unsupported base; that mapping remains unknown".into());
                            }
                        }
                    }
                    if contexts.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "production" | "deploy-preview" | "branch-deploy" | "dev"
                        )
                    }) {
                        notes.push("Branch-specific contexts are present; their environment identity requires confirmation".into());
                    }
                }
                if data.get("dev").is_some()
                    && !candidates.iter().any(|(env, _)| *env == "development")
                {
                    candidates.push(("development", candidates[0].1.clone()));
                }
            }
            "Vercel" => {
                notes.push("Project/team linkage, root-directory dashboard settings and deployment environment are unknown".into());
                if data.get("functions").is_some() {
                    notes.push("Function configuration is present; routes, ownership and deployed functions remain unverified".into());
                }
                if ["routes", "rewrites", "redirects"]
                    .iter()
                    .any(|key| data.get(key).is_some())
                {
                    notes.push("Routing configuration is present; destinations are not contacted and contract resolution remains unknown".into());
                }
                if data.get("builds").is_some() || data.get("version").is_some() {
                    notes.push("Legacy Vercel configuration fields are present; current compatibility and active use require confirmation".into());
                }
                if data.get("functions").is_some() && data.get("builds").is_some() {
                    notes.push("Both functions and legacy builds are declared; the conflicting configuration requires correction".into());
                }
            }
            "Railway" => {
                notes.push("Legacy Railway Config as Code; documentation checked 2026-10-07. Existing-file compatibility does not prove an active service".into());
                notes.push("Current Railway infrastructure uses executable .railway/railway.ts; this detector does not execute it or read dashboard settings".into());
                if data.get("build").is_some() {
                    notes.push("Build configuration is present; build commands and secret values are not retained or executed".into());
                }
                if data.get("deploy").is_some() {
                    notes.push("Deployment configuration is present; start/pre-deploy commands are not retained or executed".into());
                }
            }
            _ => unreachable!(),
        }
        for (environment, source_root) in candidates {
            let Some(source_root) = static_root(&source_root).filter(|root| paths.allows(root))
            else {
                gaps.push(format!("{provider} suggested root is outside approved scope or unsupported; no scope expansion occurred"));
                continue;
            };
            if !inventory.items.iter().any(|entry| {
                entry.repository_index == item.repository_index
                    && entry.status == "indexed"
                    && under(&entry.path, &source_root)
            }) {
                gaps.push(format!("{provider} source root has no indexed evidence; configuration may be stale or require a separate scope addition"));
                continue;
            }
            let owners = components
                .iter()
                .filter(|component| {
                    component.repository_index == item.repository_index
                        && component.roots.iter().any(|root| under(&source_root, root))
                })
                .map(|component| {
                    (
                        component,
                        component
                            .roots
                            .iter()
                            .filter(|root| under(&source_root, root))
                            .map(String::len)
                            .max()
                            .unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>();
            let deepest = owners.iter().map(|(_, depth)| *depth).max();
            let owners = owners
                .iter()
                .filter(|(_, depth)| Some(*depth) == deepest)
                .collect::<Vec<_>>();
            if owners.len() > 1 {
                gaps.push(format!(
                    "{provider} root has ambiguous component ownership; no mapping was asserted"
                ));
                continue;
            }
            let Some((component, _)) = owners.first() else {
                gaps.push(format!("{provider} configuration has no corresponding authorized component; confirm ownership before use"));
                continue;
            };
            if mappings.len() == 200 {
                gaps.push("Static provider mappings reached the 200-mapping bound; additional coverage is unknown".into());
                break;
            }
            mappings.push(ComponentProviderMapping {
                component_id: component.id.clone(),
                provider: provider.into(),
                project_id: None,
                service_id: None,
                environment: environment.into(),
                repository_index: item.repository_index,
                source_root,
                observed_commit: Some(commit.as_str().into()),
                verified_at_ms: None,
                verification: ProviderVerification::NotRequested,
                evidence: vec![OnboardingEvidence {
                    repository_index: item.repository_index,
                    path: item.path.clone(),
                    blob: item.blob.clone(),
                    start_line: 1,
                    end_line: source.lines().count().clamp(1, 10000) as u32,
                    producer_version: VERSION.into(),
                }],
                notes: notes.clone(),
                decision: ComponentDecision::Pending,
            });
        }
    }
    for component in components {
        let providers = mappings
            .iter()
            .filter(|mapping| mapping.component_id == component.id)
            .map(|mapping| &mapping.provider)
            .collect::<BTreeSet<_>>()
            .len();
        if providers > 1 {
            for mapping in mappings
                .iter_mut()
                .filter(|mapping| mapping.component_id == component.id)
            {
                mapping.notes.push("Multiple providers are suggested for this component; active mappings and environments must be explicitly confirmed".into());
            }
        }
        if providers == 0 {
            gaps.push("A component has no supported hosting evidence; provider may be unknown, self-hosted, different, or not deployed".into());
        }
    }
    let workflows = inventory
        .items
        .iter()
        .filter(|item| item.kind == "ci_config" && item.status == "indexed")
        .collect::<Vec<_>>();
    if workflows.len() > 8 {
        gaps.push("CI clue inspection is limited to eight workflows; remaining deployment evidence is unknown".into());
    }
    for item in workflows.into_iter().take(8) {
        if started.elapsed() > std::time::Duration::from_secs(10) {
            gaps.push(
                "Hosting budget exhausted before CI clues; workflow evidence remains unknown"
                    .into(),
            );
            break;
        }
        let (repository, _, _) = crate::onboarding_scan::pin(session, item.repository_index)?;
        let local = roots.get(repository).ok_or("Repository unavailable")?;
        let blobs = read_chunk(
            local,
            std::slice::from_ref(&item.blob),
            ReadLimits {
                objects_per_chunk: 1,
                bytes_per_object: 256 * 1024,
                bytes_per_chunk: 256 * 1024,
            },
            &Cancellation::default(),
        )?;
        let Some(BatchBlob::Data(bytes)) = blobs.into_iter().next() else {
            gaps.push("CI workflow content is unavailable; provider remains unknown".into());
            continue;
        };
        let Ok(source) = std::str::from_utf8(&bytes) else {
            gaps.push("CI workflow encoding is unsupported".into());
            continue;
        };
        for provider in ["Railway", "Netlify", "Vercel"] {
            if let Some((line, _)) = source.lines().enumerate().find(|(_, text)| {
                !text.trim_start().starts_with('#')
                    && text
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .any(|word| word.eq_ignore_ascii_case(provider))
            }) {
                let clue = format!(
                    "{provider} lexical CI clue: repo {} {}:{} · blob {}. YAML semantics, component and environment are unresolved; no mapping or execution.",
                    item.repository_index + 1,
                    item.path,
                    line + 1,
                    &item.blob[..12]
                );
                if clue.len() <= 256 {
                    gaps.push(clue);
                } else {
                    gaps.push("A CI provider clue exceeds the display bound; its component and environment remain unresolved".into());
                }
            }
        }
    }
    if inventory.items.iter().any(|item| item.kind == "ci_config") {
        gaps.push("CI workflows are inspected only for lexical provider clues; YAML and executable semantics remain unsupported, never provider proof".into());
    }
    gaps.sort();
    gaps.dedup();
    if gaps.len() > 100 {
        gaps.truncate(99);
        gaps.push(
            "Additional hosting gaps exceed the display budget; coverage remains incomplete".into(),
        );
    }
    Ok((mappings, gaps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, PathScope, ScopeRecord, ScopeVersion, SecurityDomain, SnapshotId};
    use std::process::Command;

    #[test]
    fn mixed_provider_corpus_keeps_static_roots_environments_conflicts_and_unknowns_separate() {
        let root = std::env::temp_dir().join(format!("csb-hosting-corpus-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "-q"]);
        for (path, source) in [
            ("Cargo.toml", "[workspace]\nmembers=[]\n"),
            (
                "apps/web/package.json",
                "{\"dependencies\":{\"react\":\"1\"}}",
            ),
            (
                "apps/web/vercel.toml",
                "[functions]\n\"api/*.js\" = { maxDuration = 30 }\n",
            ),
            (
                "services/api/Cargo.toml",
                "[package]\nname='api'\nversion='0.1.0'\n",
            ),
            ("services/api/src/main.rs", "use axum::Router; fn main() {}"),
            (
                "services/api/railway.json",
                r#"{"build":{"builder":"RAILPACK"},"deploy":{"startCommand":"touch EXECUTION_SENTINEL","secret":"CONFIG_SECRET_SENTINEL"}}"#,
            ),
            (
                "services/api/vercel.json",
                r#"{"version":2,"functions":{"api/*.js":{}},"builds":[],"rewrites":[{"source":"/api","destination":"https://NEVER_CONTACT_SENTINEL.invalid"}],"env":{"PASSWORD":"VERCEL_SECRET_SENTINEL"}}"#,
            ),
            ("services/job/package.json", "{}"),
            (
                "services/job/railway.toml",
                "[deploy]\nstartCommand = 'touch EXECUTION_SENTINEL'\n",
            ),
            ("lib/Cargo.toml", "[package]\nname='lib'\nversion='0.1.0'\n"),
            ("lib/src/lib.rs", "pub fn helper() {}"),
            (
                "netlify.toml",
                "[build]\nbase='apps/web'\npublish='dist'\ncommand='touch EXECUTION_SENTINEL'\n[build.environment]\nPASSWORD='NETLIFY_SECRET_SENTINEL'\n[functions]\ndirectory='functions'\n[context.production]\n[context.deploy-preview]\n[context.dev]\nbase='services/api'\n",
            ),
            ("broken/vercel.json", "{INVALID_SECRET_SENTINEL"),
            ("stale/netlify.toml", "[build]\nbase='removed-app'\n"),
            ("outside/netlify.toml", "[build]\nbase='private'\n"),
            (
                "dynamic/vercel.ts",
                "require('fs').writeFileSync('EXECUTION_SENTINEL','DYNAMIC_SECRET_SENTINEL');",
            ),
            (
                ".railway/railway.ts",
                "throw new Error('EXECUTION_SENTINEL');",
            ),
            ("private/package.json", "{}"),
            (
                ".github/workflows/deploy.yml",
                "name: Deploy\n# railway comment is not a clue\njobs:\n  deploy:\n    steps:\n      - run: npx vercel --token CI_SECRET_SENTINEL\n",
            ),
        ] {
            let destination = root.join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, source).unwrap();
        }
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "hosting corpus",
        ]);
        let repository =
            RepositoryId::new(SecurityDomain::new("test").unwrap(), "fixture", "o", "r").unwrap();
        let principal = PrincipalId::new("owner").unwrap();
        let grants = vec![Grant {
            principal: principal.clone(),
            repository: repository.clone(),
            actions: BTreeSet::from([Action::Index]),
            paths: PathScope::all(),
        }];
        let scope = ScopeRecord {
            version: ScopeVersion::new("scope").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::new(vec![".".into()], vec!["private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let mut session = OnboardingSession::new(
            "hosting".into(),
            principal.clone(),
            SnapshotId::new(crate::git::resolve(&root, "HEAD").unwrap()).unwrap(),
            scope,
            crate::INVENTORY_VERSION.into(),
            &grants,
        )
        .unwrap();
        let inventory = crate::scan_onboarding_chunk(
            &root,
            &session,
            None,
            &Cancellation::default(),
            &principal,
            &grants,
        )
        .unwrap();
        assert!(inventory.complete);
        session
            .record_inventory(0, inventory, &principal, &grants)
            .unwrap();
        let roots = BTreeMap::from([(repository, root.clone())]);
        let (components, _) =
            crate::onboarding_detection::detect(&roots, &session, &principal, &grants).unwrap();
        std::fs::write(root.join("netlify.toml"), "WORKTREE_SECRET_SENTINEL").unwrap();
        let (mappings, gaps) = detect(&roots, &session, &components, &principal, &grants).unwrap();
        assert_eq!(
            (mappings.clone(), gaps.clone()),
            detect(&roots, &session, &components, &principal, &grants).unwrap()
        );
        let web = components
            .iter()
            .find(|component| component.roots == vec!["apps/web"])
            .unwrap();
        let api = components
            .iter()
            .find(|component| component.roots == vec!["services/api"])
            .unwrap();
        let job = components
            .iter()
            .find(|component| component.roots == vec!["services/job"])
            .unwrap();
        assert!(mappings.iter().any(|mapping| mapping.component_id == web.id
            && mapping.provider == "Netlify"
            && mapping.environment == "preview"));
        assert!(mappings.iter().any(|mapping| mapping.component_id == web.id
            && mapping.provider == "Netlify"
            && mapping.environment == "production"));
        assert!(mappings.iter().any(|mapping| mapping.component_id == api.id
            && mapping.provider == "Netlify"
            && mapping.environment == "development"));
        assert!(
            mappings
                .iter()
                .any(|mapping| mapping.component_id == api.id && mapping.provider == "Railway")
        );
        assert!(
            mappings
                .iter()
                .any(|mapping| mapping.component_id == job.id && mapping.provider == "Railway")
        );
        assert!(
            mappings
                .iter()
                .any(|mapping| mapping.component_id == web.id && mapping.provider == "Vercel")
        );
        assert!(
            mappings
                .iter()
                .filter(|mapping| mapping.component_id == api.id)
                .all(|mapping| mapping
                    .notes
                    .iter()
                    .any(|note| note.contains("Multiple providers")))
        );
        assert!(
            mappings
                .iter()
                .filter(|mapping| mapping.provider == "Railway")
                .all(|mapping| mapping.notes.iter().any(|note| note.contains("Legacy")))
        );
        for fragment in [
            "invalid",
            "executable",
            "no supported hosting evidence",
            "outside approved",
            "no indexed",
            "CI workflows",
            "Vercel lexical CI clue",
        ] {
            assert!(gaps.iter().any(|gap| gap.contains(fragment)), "{fragment}");
        }
        assert!(!root.join("EXECUTION_SENTINEL").exists());
        let wire = serde_json::to_string(&(&mappings, &gaps)).unwrap();
        for sentinel in [
            "EXECUTION_SENTINEL",
            "CONFIG_SECRET_SENTINEL",
            "VERCEL_SECRET_SENTINEL",
            "NETLIFY_SECRET_SENTINEL",
            "DYNAMIC_SECRET_SENTINEL",
            "NEVER_CONTACT_SENTINEL",
            "WORKTREE_SECRET_SENTINEL",
            "private",
            "removed-app",
            "CI_SECRET_SENTINEL",
        ] {
            assert!(!wire.contains(sentinel), "{sentinel}");
        }
        for mapping in &mappings {
            assert_eq!(mapping.verification, ProviderVerification::NotRequested);
            assert_eq!(mapping.decision, ComponentDecision::Pending);
            assert_eq!(mapping.verified_at_ms, None);
            assert_eq!(mapping.project_id, None);
            assert_eq!(
                mapping.observed_commit.as_deref(),
                Some(session.commit().as_str())
            );
            assert_eq!(mapping.evidence[0].producer_version, VERSION);
        }
        assert!(detect(&BTreeMap::new(), &session, &components, &principal, &[]).is_err());
        let mut ambiguous = components.clone();
        let mut duplicate = web.clone();
        duplicate.id = "duplicate".into();
        ambiguous.push(duplicate);
        let (ambiguous_mappings, ambiguous_gaps) =
            detect(&roots, &session, &ambiguous, &principal, &grants).unwrap();
        assert!(
            !ambiguous_mappings
                .iter()
                .any(|mapping| mapping.component_id == web.id)
        );
        assert!(
            ambiguous_gaps
                .iter()
                .any(|gap| gap.contains("ambiguous component ownership"))
        );
        session
            .set_proposal(
                session.revision(),
                components,
                mappings,
                gaps,
                &principal,
                &grants,
            )
            .unwrap();
        session
            .transition(
                session.revision(),
                crate::OnboardingState::AwaitingConfirmation,
                &principal,
                &grants,
            )
            .unwrap();
        for mut component in session.proposals().to_vec() {
            component.decision = ComponentDecision::Confirmed;
            session
                .decide_component(session.revision(), component, &principal, &grants)
                .unwrap();
        }
        let fingerprint = session.proposal_fingerprint();
        assert_eq!(
            session
                .confirm(
                    session.revision(),
                    &fingerprint,
                    true,
                    10,
                    &principal,
                    &grants
                )
                .unwrap_err(),
            "Architecture confirmation access denied"
        );
        let mut maintainer = grants.clone();
        maintainer[0].actions.insert(Action::ConfirmArchitecture);
        assert!(
            session
                .confirm(
                    session.revision(),
                    &fingerprint,
                    true,
                    10,
                    &principal,
                    &maintainer
                )
                .unwrap_err()
                .contains("Every provider mapping")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
