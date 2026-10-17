//! Deterministic, bounded proposals. No dependency declaration proves active runtime use.
use crate::git::BatchBlob;
use crate::snapshot::{ReadLimits, read_chunk};
use crate::{
    Cancellation, ComponentDecision, ComponentProposal, ComponentSignal, EvidenceConfidence, Grant,
    OnboardingEvidence, OnboardingSession, PrincipalId,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const DETECTOR_VERSION: &str = "csb-component-detectors-v2";

// Deliberately finite rules. Imports imply candidates, not a running service,
// data ownership, protocol compatibility or resolved cross-repository edges.
fn import_rule(root: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match root {
        "axum" => (
            "backend",
            "Axum",
            "An HTTP framework is imported; listening endpoints and active deployment remain unverified",
        ),
        "actix_web" => (
            "backend",
            "Actix Web",
            "An HTTP framework is imported; listening endpoints and active deployment remain unverified",
        ),
        "warp" => (
            "backend",
            "Warp",
            "An HTTP framework is imported; listening endpoints and active deployment remain unverified",
        ),
        "apalis" => (
            "worker",
            "Apalis",
            "A task-processing framework is imported; task registration and active worker processes remain unverified",
        ),
        "tokio_cron_scheduler" => (
            "scheduled job",
            "Tokio Cron Scheduler",
            "A scheduler is imported; job registration, schedule and active execution remain unverified",
        ),
        "sqlx" => (
            "database",
            "SQLx",
            "A database API is imported; engine, schema ownership and connections remain unknown",
        ),
        "diesel" => (
            "database",
            "Diesel",
            "A database API is imported; engine, schema ownership and connections remain unknown",
        ),
        "rusqlite" => (
            "database",
            "Rusqlite",
            "A SQLite API is imported; database creation, ownership and active access remain unverified",
        ),
        "postgres" | "tokio_postgres" => (
            "database",
            "PostgreSQL client",
            "A database client is imported; schema ownership and live connections remain unknown",
        ),
        "mongodb" => (
            "database",
            "MongoDB client",
            "A database client is imported; collection ownership and live connections remain unknown",
        ),
        "redis" => (
            "data client",
            "Redis",
            "A Redis API is imported; cache, storage or queue usage requires confirmation",
        ),
        "lapin" => (
            "queue",
            "Lapin",
            "A messaging API is imported; producer/consumer direction and broker deployment remain unknown",
        ),
        "rdkafka" => (
            "queue",
            "Kafka client",
            "A messaging API is imported; producer/consumer direction and broker deployment remain unknown",
        ),
        "async_nats" => (
            "queue",
            "NATS client",
            "A messaging API is imported; producer/consumer direction and broker deployment remain unknown",
        ),
        "aws_sdk_s3" => (
            "storage",
            "S3 client",
            "An object-storage API is imported; bucket ownership, permissions and provider deployment remain unknown",
        ),
        "object_store" => (
            "storage",
            "Object Store",
            "An object-storage abstraction is imported; concrete provider and data ownership remain unknown",
        ),
        "reqwest" => (
            "external contract",
            "HTTP client",
            "An HTTP client is imported; destination, protocol contract and remote behavior are unresolved; no URL is contacted",
        ),
        "tonic" => (
            "external contract",
            "Tonic",
            "An RPC framework is imported; service/client direction and contract correspondence require confirmation",
        ),
        "crate" | "self" | "super" => (
            "internal contract",
            "Internal module import",
            "An internal module import is observed; component ownership and target resolution remain unresolved; scope is not expanded",
        ),
        _ => return None,
    })
}
pub(crate) fn detect(
    roots: &BTreeMap<crate::RepositoryId, PathBuf>,
    session: &OnboardingSession,
    principal: &PrincipalId,
    grants: &[Grant],
) -> Result<(Vec<ComponentProposal>, Vec<String>), String> {
    session.check_access(principal, grants)?;
    if !matches!(
        session.state(),
        crate::OnboardingState::Scanning | crate::OnboardingState::Partial
    ) {
        return Err("Component detection requires a completed scan awaiting proposals".into());
    }
    let inventory = session
        .inventory()
        .filter(|item| item.complete)
        .ok_or("Complete inventory required")?;
    let mut gaps = inventory.gaps.clone();
    for (count, label) in [
        (inventory.unsupported, "Unsupported inventory"),
        (inventory.deferred, "Deferred inventory"),
        (inventory.failed, "Failed inventory"),
    ] {
        if count > 0 {
            gaps.push(format!(
                "{label}: {count} paths; component coverage incomplete"
            ));
        }
    }
    let boundaries = inventory.candidate_roots();
    let mut proposals = Vec::new();
    let started = std::time::Instant::now();
    let cancel = Cancellation::default();
    for (repository_index, root) in boundaries.iter().take(64) {
        if started.elapsed() > std::time::Duration::from_secs(10) {
            gaps.push(
                "Component detection reached its ten-second budget; remaining roots unclassified"
                    .into(),
            );
            break;
        }
        let (repository, _, paths) = crate::onboarding_scan::pin(session, *repository_index)?;
        let local = roots.get(repository).ok_or("Repository unavailable")?;
        let under = |path: &str| {
            root == "."
                || path
                    .strip_prefix(root)
                    .is_some_and(|rest| rest.starts_with('/'))
        };
        // A nested manifest creates its own boundary, rather than being absorbed by a parent.
        let items = inventory
            .items
            .iter()
            .filter(|item| {
                item.repository_index == *repository_index
                    && item.status == "indexed"
                    && under(&item.path)
                    && !boundaries.iter().any(|(index, nested)| {
                        index == repository_index
                            && nested != root
                            && nested != "."
                            && item
                                .path
                                .strip_prefix(nested)
                                .is_some_and(|rest| rest.starts_with('/'))
                    })
            })
            .collect::<Vec<_>>();
        let manifests = items
            .iter()
            .filter(|item| item.kind == "manifest")
            .copied()
            .take(8)
            .collect::<Vec<_>>();
        let sources = items
            .iter()
            .filter(|item| item.kind == "source")
            .copied()
            .take(8)
            .collect::<Vec<_>>();
        let schemas = items
            .iter()
            .filter(|item| matches!(item.kind.as_str(), "api_schema" | "database_schema"))
            .copied()
            .take(4)
            .collect::<Vec<_>>();
        let selected = manifests
            .iter()
            .chain(&sources)
            .chain(&schemas)
            .copied()
            .collect::<Vec<_>>();
        let ids = selected
            .iter()
            .map(|item| item.blob.clone())
            .collect::<Vec<_>>();
        let contents = read_chunk(
            local,
            &ids,
            ReadLimits {
                objects_per_chunk: 20,
                bytes_per_object: 256 * 1024,
                bytes_per_chunk: 5 * 1024 * 1024,
            },
            &cancel,
        )?;
        let mut stack = BTreeSet::new();
        let mut evidence = Vec::new();
        let mut signals = Vec::new();
        let mut roles = BTreeSet::new();
        let mut contradictions = Vec::new();
        let mut role = "unclassified".to_owned();
        let mut language_sources = 0;
        let has_rust_manifest = manifests
            .iter()
            .any(|item| item.path.rsplit('/').next() == Some("Cargo.toml"));
        if items.iter().filter(|item| item.kind == "source").count() > sources.len() {
            contradictions.push("Source sampling is limited to eight files per manifest root; roles and integrations outside the sample remain unknown".into());
        }
        if items
            .iter()
            .filter(|item| matches!(item.kind.as_str(), "api_schema" | "database_schema"))
            .count()
            > schemas.len()
        {
            contradictions.push("Schema sampling is limited to four files per component; additional contracts remain unknown".into());
        }
        for (item, content) in selected.into_iter().zip(contents) {
            if started.elapsed() > std::time::Duration::from_secs(10) {
                gaps.push("Component detection reached its ten-second budget; remaining files unclassified".into());
                break;
            }
            let BatchBlob::Data(bytes) = content else {
                contradictions.push(format!("{} content unavailable", item.path));
                continue;
            };
            let Ok(source) = std::str::from_utf8(&bytes) else {
                contradictions.push("Non-UTF-8 candidate evidence was not analyzed".into());
                continue;
            };
            let mut observed = false;
            if matches!(item.kind.as_str(), "api_schema" | "database_schema") {
                observed = true;
                if signals.len() < 64 {
                    signals.push(ComponentSignal {
                        kind: "schema artifact".into(),
                        name: if item.kind == "api_schema" { "API schema candidate" } else { "Database schema candidate" }.into(),
                        reason: "A schema-format artifact is present; syntax, generated status, active usage and component ownership are unresolved; no migration or generator is executed".into(),
                        evidence_index: evidence.len(), test_only: test_path(&item.path),
                    });
                }
            }
            match item.path.rsplit('/').next().unwrap_or(&item.path) {
                "Cargo.toml" => {
                    stack.insert("Rust · manifest candidate".into());
                    observed = true;
                }
                "pyproject.toml" => {
                    stack.insert("Python · manifest candidate".into());
                    observed = true;
                    contradictions.push(
                        "Python package configuration and active framework require confirmation"
                            .into(),
                    );
                }
                "go.mod" => {
                    stack.insert("Go · manifest candidate".into());
                    observed = true;
                }
                "pom.xml" => {
                    stack.insert("Java · manifest candidate".into());
                    observed = true;
                    contradictions.push("XML build semantics are not evaluated".into());
                }
                "package.json" => {
                    observed = true;
                    match serde_json::from_str::<serde_json::Value>(source) {
                        Ok(data) if data.is_object() => {
                            stack.insert("JavaScript/TypeScript · package manifest".into());
                            for (section, classification) in [
                                ("dependencies", "runtime declaration"),
                                ("devDependencies", "build/test declaration"),
                            ] {
                                if let Some(deps) =
                                    data.get(section).and_then(serde_json::Value::as_object)
                                {
                                    for (name, display) in [
                                        ("next", "Next.js"),
                                        ("react", "React"),
                                        ("vue", "Vue"),
                                        ("svelte", "Svelte"),
                                        ("express", "Express"),
                                        ("fastify", "Fastify"),
                                        ("vite", "Vite"),
                                    ] {
                                        if deps.contains_key(name) {
                                            stack.insert(format!("{display} · {classification}"));
                                        }
                                    }
                                }
                            }
                            contradictions.push("Declared dependencies do not prove active use; JavaScript/TypeScript imports are not resolved by this detector".into());
                        }
                        _ => contradictions.push(
                            "Package JSON is invalid or unsupported; no framework inferred".into(),
                        ),
                    }
                }
                _ if item.path.ends_with(".rs") => {
                    if let Ok(parsed) = crate::parser::parse_rust(&item.path, source, &cancel) {
                        if parsed.error_nodes == 0 {
                            language_sources += 1;
                            observed = true;
                            stack.insert("Rust · parsed source".into());
                            for import in &parsed.import_evidence {
                                let Some(import_root) = import.root.as_deref() else {
                                    contradictions.push("Grouped import roots are unresolved; no integration is inferred from them".into());
                                    continue;
                                };
                                if let Some((kind, name, reason)) = import_rule(import_root) {
                                    if signals.len() == 64 {
                                        contradictions.push("Integration signals are limited to 64 per component; additional observations remain unknown".into());
                                        break;
                                    }
                                    let test_only = import.test_only || test_path(&item.path);
                                    let evidence_index = evidence.len();
                                    evidence.push(OnboardingEvidence {
                                        repository_index: *repository_index,
                                        path: item.path.clone(),
                                        blob: item.blob.clone(),
                                        start_line: import.span.start_line as u32,
                                        end_line: import.span.end_line as u32,
                                        producer_version: DETECTOR_VERSION.into(),
                                    });
                                    signals.push(ComponentSignal {
                                        kind: kind.into(),
                                        name: name.into(),
                                        reason: reason.into(),
                                        evidence_index,
                                        test_only,
                                    });
                                    if test_only {
                                        stack.insert(format!("{name} · test-scoped import"));
                                    } else {
                                        stack.insert(format!("{name} · static import"));
                                        if matches!(kind, "backend" | "worker" | "scheduled job") {
                                            roles.insert(kind);
                                        }
                                    }
                                }
                            }
                            for symbol in &parsed.trait_evidence {
                                if signals.len() == 64 {
                                    break;
                                }
                                let evidence_index = evidence.len();
                                evidence.push(OnboardingEvidence {
                                    repository_index: *repository_index,
                                    path: item.path.clone(),
                                    blob: item.blob.clone(),
                                    start_line: symbol.span.start_line as u32,
                                    end_line: symbol.span.end_line as u32,
                                    producer_version: DETECTOR_VERSION.into(),
                                });
                                signals.push(ComponentSignal {
                                    kind: "interface contract".into(), name: symbol.name.chars().take(64).collect(),
                                    reason: "A Rust trait is declared; public exposure, implementation correspondence and cross-component ownership remain unresolved".into(),
                                    evidence_index, test_only: symbol.test_only || test_path(&item.path),
                                });
                            }
                            if role == "unclassified"
                                && item.path.ends_with("/main.rs")
                                && !test_path(&item.path)
                            {
                                role = "application candidate".into();
                            }
                            if role == "unclassified"
                                && item.path.ends_with("/lib.rs")
                                && !test_path(&item.path)
                            {
                                role = "library candidate".into();
                            }
                        } else {
                            contradictions.push("Rust source contains parser gaps".into());
                        }
                    } else {
                        contradictions.push("Rust parsing failed or reached its budget; affected source semantics remain unknown".into());
                    }
                }
                _ => {}
            }
            if observed {
                evidence.push(OnboardingEvidence {
                    repository_index: *repository_index,
                    path: item.path.clone(),
                    blob: item.blob.clone(),
                    start_line: 1,
                    end_line: source.lines().count().clamp(1, 10000) as u32,
                    producer_version: DETECTOR_VERSION.into(),
                });
            }
        }
        if roles.len() == 1 {
            role = format!("{} candidate", roles.first().unwrap());
        } else if roles.len() > 1 {
            role = "mixed-role candidate".into();
            contradictions.push("Multiple role signals coexist; confirm whether this is one process or separate components".into());
        }
        if !signals.is_empty() {
            contradictions.push("Static signals do not prove runtime use; conditional compilation, dependency aliases, grouped imports and generated code require confirmation".into());
        }
        if stack.len() > 20 {
            contradictions.push("Stack display is limited to 20 entries; inspect detailed integration signals for additional evidence".into());
        }
        contradictions.sort();
        contradictions.dedup();
        if contradictions.len() > 20 {
            contradictions.truncate(19);
            contradictions.push("Additional detection limits were omitted from this bounded proposal; coverage remains incomplete".into());
        }
        if evidence.is_empty() {
            gaps.push(
                "A manifest boundary had no readable evidence; no component assertion was made"
                    .into(),
            );
            continue;
        }
        let digest = Sha256::digest(format!("{repository_index}:{root}").as_bytes());
        let id = format!(
            "component-{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let approved_roots = if paths.allows(root) {
            vec![root.clone()]
        } else {
            paths
                .includes()
                .filter(|path| under(path))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        if approved_roots.is_empty() {
            gaps.push(
                "Manifest root has no approved component boundary; expand scope separately".into(),
            );
            continue;
        }
        proposals.push(ComponentProposal {
            id,
            name: format!(
                "Repository {} · {}",
                repository_index + 1,
                root.chars().take(220).collect::<String>()
            ),
            role,
            repository_index: *repository_index,
            roots: approved_roots,
            stack_candidates: stack.into_iter().take(20).collect(),
            signals,
            evidence,
            confidence: if has_rust_manifest && language_sources > 0 {
                EvidenceConfidence::Corroborated
            } else {
                EvidenceConfidence::Candidate
            },
            contradictions,
            decision: ComponentDecision::Pending,
        });
    }
    if boundaries.len() > 64 {
        gaps.push("Only the first 64 manifest boundaries were classified".into());
    }
    if proposals.is_empty() {
        gaps.push(
            "No component candidate could be established within the approved evidence".into(),
        );
    }
    gaps.sort();
    gaps.dedup();
    if gaps.len() > 100 {
        gaps.truncate(99);
        gaps.push(
            "Additional gaps exceed the proposal display budget; analysis remains incomplete"
                .into(),
        );
    }
    Ok((proposals, gaps))
}

fn test_path(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "tests" | "benches" | "examples"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Action, PathScope, RepositoryId, ScopeRecord, ScopeVersion, SecurityDomain, SnapshotId,
    };
    use std::process::Command;

    #[test]
    fn mixed_manifest_candidates_preserve_weak_signals_and_pinned_provenance() {
        let root = std::env::temp_dir().join(format!("csb-detector-corpus-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let run = |args: &[&str]| {
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
        run(&["init", "-q"]);
        for (path, source) in [
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"services/api\", \"libs/shared\"]\n",
            ),
            (
                "services/api/Cargo.toml",
                "[package]\nname=\"api\"\nversion=\"0.1.0\"\n",
            ),
            (
                "services/api/src/main.rs",
                "use axum::Router;\nfn main() {}\n",
            ),
            (
                "libs/shared/Cargo.toml",
                "[package]\nname=\"shared\"\nversion=\"0.1.0\"\n",
            ),
            (
                "libs/shared/src/lib.rs",
                "// use warp::Filter;\nuse unrelated::{axum, sqlx};\nuse /* reqwest */ harmless as tonic;\npub fn helper() {}\n",
            ),
            (
                "services/worker/Cargo.toml",
                "[package]\nname='worker'\nversion='0.1.0'\n",
            ),
            (
                "services/worker/src/main.rs",
                "use apalis::prelude::*;\nuse lapin::Channel;\nuse sqlx::Pool;\nuse object_store::ObjectStore;\nuse reqwest::Client;\nuse crate::contracts::Job;\npub trait Handler { fn handle(&self); }\n#[cfg(test)]\nmod tests { use axum::Router; }\nfn main() {}\n",
            ),
            (
                "services/worker/api.proto",
                "syntax = \"proto3\"; // SCHEMA_CONTENT_SENTINEL\n",
            ),
            (
                "services/worker/migrations/001.sql",
                "-- MIGRATION_CONTENT_SENTINEL\nCREATE TABLE jobs (id INT);\n",
            ),
            (
                "services/cron/Cargo.toml",
                "[package]\nname='cron'\nversion='0.1.0'\n",
            ),
            (
                "services/cron/src/main.rs",
                "use tokio_cron_scheduler::JobScheduler;\nuse redis::Client;\nfn main() {}\n",
            ),
            (
                "services/mixed/Cargo.toml",
                "[package]\nname='mixed'\nversion='0.1.0'\n",
            ),
            (
                "services/mixed/src/main.rs",
                "use axum::Router;\nuse apalis::prelude::*;\nfn main() {}\n",
            ),
            (
                "services/harness/Cargo.toml",
                "[package]\nname='tests'\nversion='0.1.0'\n",
            ),
            (
                "services/harness/src/lib.rs",
                "#[cfg(test)]\nmod tests {\nuse axum::Router;\nuse sqlx::Pool;\n}\n",
            ),
            (
                "apps/web/package.json",
                r#"{"dependencies":{"react":"1","unused-sdk":"1"},"devDependencies":{"express":"1"},"scripts":{"prepare":"touch EXECUTION_SENTINEL"},"secret":"CONFIG_SENTINEL"}"#,
            ),
            (
                "apps/web/src/main.ts",
                "// This file does not import react\nexport const value = 1;\n",
            ),
            ("apps/python/pyproject.toml", "[project]\nname='job'\n"),
            ("apps/broken/package.json", "invalid json"),
            (
                "README.md",
                "The entire system runs on Warp, Redis and Railway. MISLEADING_DOC_SENTINEL",
            ),
            ("private/package.json", r#"{"dependencies":{"next":"1"}}"#),
        ] {
            let destination = root.join(path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, source).unwrap();
        }
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "detector corpus",
        ]);
        let principal = PrincipalId::new("owner").unwrap();
        let repository =
            RepositoryId::new(SecurityDomain::new("test").unwrap(), "fixture", "o", "r").unwrap();
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
        let commit = SnapshotId::new(crate::git::resolve(&root, "HEAD").unwrap()).unwrap();
        let mut session = OnboardingSession::new(
            "detect".into(),
            principal.clone(),
            commit,
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
        std::fs::write(root.join("services/api/src/main.rs"), "WORKTREE_SENTINEL").unwrap();
        let (components, gaps) = detect(
            &BTreeMap::from([(repository, root.clone())]),
            &session,
            &principal,
            &grants,
        )
        .unwrap();
        assert!(gaps.iter().any(|gap| gap.contains("Unsupported")));
        let find = |path: &str| {
            components
                .iter()
                .find(|component| component.roots == vec![path.to_owned()])
                .unwrap()
        };
        let api = find("services/api");
        assert_eq!(api.role, "backend candidate");
        assert_eq!(api.confidence, EvidenceConfidence::Corroborated);
        assert!(
            api.stack_candidates
                .contains(&"Axum · static import".into())
        );
        let shared = find("libs/shared");
        assert_eq!(shared.role, "library candidate");
        assert!(
            !shared
                .stack_candidates
                .iter()
                .any(|stack| stack.contains("Warp"))
        );
        assert!(
            shared.signals.is_empty(),
            "Nested names, aliases and comments cannot establish integration roots"
        );
        let worker = find("services/worker");
        assert_eq!(worker.role, "worker candidate");
        for kind in [
            "worker",
            "queue",
            "database",
            "storage",
            "external contract",
            "internal contract",
            "interface contract",
            "schema artifact",
        ] {
            assert!(
                worker
                    .signals
                    .iter()
                    .any(|signal| signal.kind == kind && !signal.test_only),
                "{kind}"
            );
        }
        let worker_backend = worker
            .signals
            .iter()
            .find(|signal| signal.kind == "backend")
            .unwrap();
        assert!(worker_backend.test_only);
        let database = worker
            .signals
            .iter()
            .find(|signal| signal.name == "SQLx")
            .unwrap();
        assert_eq!(worker.evidence[database.evidence_index].start_line, 3);
        assert_eq!(worker.evidence[database.evidence_index].end_line, 3);
        assert_eq!(find("services/cron").role, "scheduled job candidate");
        assert!(
            find("services/cron")
                .signals
                .iter()
                .any(|signal| signal.kind == "data client")
        );
        assert_eq!(find("services/mixed").role, "mixed-role candidate");
        assert_eq!(find("services/harness").role, "library candidate");
        assert!(
            find("services/harness")
                .signals
                .iter()
                .all(|signal| signal.test_only)
        );
        let web = find("apps/web");
        assert_eq!(web.confidence, EvidenceConfidence::Candidate);
        assert!(
            web.stack_candidates
                .contains(&"Express · build/test declaration".into())
        );
        assert!(
            web.stack_candidates
                .contains(&"React · runtime declaration".into())
        );
        assert_eq!(web.role, "unclassified");
        assert!(
            find("apps/broken")
                .contradictions
                .iter()
                .any(|item| item.contains("invalid"))
        );
        assert!(
            find("apps/python")
                .contradictions
                .iter()
                .any(|item| item.contains("require confirmation"))
        );
        let wire = serde_json::to_string(&components).unwrap();
        for sentinel in [
            "CONFIG_SENTINEL",
            "MISLEADING_DOC_SENTINEL",
            "WORKTREE_SENTINEL",
            "private/package.json",
            "Next.js",
            "unused-sdk",
            "SCHEMA_CONTENT_SENTINEL",
            "MIGRATION_CONTENT_SENTINEL",
        ] {
            assert!(!wire.contains(sentinel), "{sentinel}");
        }
        let repeated = detect(
            &BTreeMap::from([(session.scope().primary_repository.clone(), root.clone())]),
            &session,
            &principal,
            &grants,
        )
        .unwrap();
        assert_eq!((&components, &gaps), (&repeated.0, &repeated.1));
        assert!(!root.join("EXECUTION_SENTINEL").exists());
        for component in &components {
            assert_eq!(component.decision, ComponentDecision::Pending);
            assert!(!component.evidence.is_empty());
            for evidence in &component.evidence {
                assert_eq!(evidence.producer_version, DETECTOR_VERSION);
                assert!(evidence.start_line > 0);
                assert_eq!(evidence.blob.len(), 40);
            }
        }
        session
            .set_proposal(
                session.revision(),
                components,
                vec![],
                gaps,
                &principal,
                &grants,
            )
            .unwrap();
        assert!(detect(&BTreeMap::new(), &session, &principal, &grants).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
