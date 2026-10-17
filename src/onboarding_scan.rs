//! Scoped Git-object inventory: metadata and coverage only, never source/config values.
use crate::{Cancellation, OnboardingSession, PathScope, RepositoryId, SnapshotId};
use crate::{
    git::{BatchBlob, git},
    snapshot::{ReadLimits, read_chunk},
};
use std::path::Path;

pub const INVENTORY_VERSION: &str = "csb-inventory-v2";
pub const ENV_NAME_SCAN_VERSION: &str = "csb-inventory-v2+sample-env-names";
fn inventory_version(session: &OnboardingSession) -> &'static str {
    if session.scan_version().starts_with("csb-inventory-v1") {
        "csb-inventory-v1"
    } else {
        INVENTORY_VERSION
    }
}
fn sample_names_enabled(session: &OnboardingSession) -> bool {
    matches!(
        session.scan_version(),
        "csb-inventory-v1+sample-env-names" | ENV_NAME_SCAN_VERSION
    )
}
const CHUNK: usize = 64;
const MAX_ITEMS: usize = 2000;
const MAX_METADATA_BYTES: usize = 256 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(deny_unknown_fields))]
pub struct InventoryItem {
    pub repository_index: usize,
    pub path: String,
    pub blob: String,
    pub kind: String,
    pub status: String,
    #[cfg_attr(feature = "server", serde(default))]
    pub variable_names: Vec<String>,
}
impl InventoryItem {
    // Conservatively budget JSON escaping and field overhead before durable publication.
    fn metadata_bytes(&self) -> usize {
        256 + 2 * (self.path.len() + self.blob.len() + self.kind.len() + 16)
            + self
                .variable_names
                .iter()
                .map(|name| 2 * name.len() + 4)
                .sum::<usize>()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "server", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "server", serde(deny_unknown_fields))]
pub struct OnboardingInventory {
    pub version: String,
    pub manifest_fingerprint: String,
    pub repository_index: usize,
    pub offset: usize,
    pub complete: bool,
    pub enumeration_complete: bool,
    pub items: Vec<InventoryItem>,
    pub excluded: usize,
    pub sensitive_excluded: usize,
    pub unsupported: usize,
    pub deferred: usize,
    pub failed: usize,
    pub gaps: Vec<String>,
}
impl OnboardingInventory {
    /// Candidate boundaries from selected manifest metadata, never active deployable units.
    pub fn candidate_roots(&self) -> Vec<(usize, String)> {
        self.items
            .iter()
            .filter(|item| item.kind == "manifest" && item.status == "indexed")
            .map(|item| {
                (
                    item.repository_index,
                    item.path
                        .rsplit_once('/')
                        .map_or(".", |(parent, _)| parent)
                        .to_owned(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn empty(session: &OnboardingSession) -> Self {
        Self {
            version: inventory_version(session).into(),
            manifest_fingerprint: session.input_fingerprint(),
            repository_index: 0,
            offset: 0,
            complete: false,
            enumeration_complete: true,
            items: vec![],
            excluded: 0,
            sensitive_excluded: 0,
            unsupported: 0,
            deferred: 0,
            failed: 0,
            gaps: vec![],
        }
    }
    pub(crate) fn validate(&self, session: &OnboardingSession) -> Result<(), String> {
        if self.version != inventory_version(session)
            || self.manifest_fingerprint != session.input_fingerprint()
            || self.repository_index > session.scope().contexts.len() + 1
            || self.items.len() > MAX_ITEMS
            || self
                .items
                .iter()
                .map(InventoryItem::metadata_bytes)
                .sum::<usize>()
                > MAX_METADATA_BYTES
            || self.gaps.len() > 32
        {
            return Err("Invalid inventory boundary or limit".into());
        }
        for item in &self.items {
            if item.variable_names.len() > 128
                || item.variable_names.iter().any(|name| !variable_name(name))
            {
                return Err("Invalid environment variable names".into());
            }
            let (_, _, paths) = pin(session, item.repository_index)?;
            if item.path.len() > 256
                || item.path.chars().any(char::is_control)
                || !paths.allows(&item.path)
                || (sensitive_path(&item.path)
                    && !(sample_names_enabled(session)
                        && sample_env_path(&item.path)
                        && item.kind == "environment_variable_names"))
                || !matches!(item.blob.len(), 40 | 64)
                || !item.blob.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid inventory evidence outside boundary".into());
            }
        }
        Ok(())
    }
}
pub(crate) fn pin(
    session: &OnboardingSession,
    index: usize,
) -> Result<(&RepositoryId, &SnapshotId, &PathScope), String> {
    if index == 0 {
        Ok((
            &session.scope().primary_repository,
            session.commit(),
            &session.scope().primary_paths,
        ))
    } else {
        let pin = session
            .scope()
            .contexts
            .get(index - 1)
            .ok_or("Invalid repository inventory index")?;
        Ok((&pin.repository, &pin.commit, &pin.paths))
    }
}
/// Includes examples: samples are not evidence that environment values are safe.
pub fn sensitive_path(path: &str) -> bool {
    path.split('/').any(|part| {
        let part = part.to_ascii_lowercase();
        part == ".env"
            || part.starts_with(".env.")
            || part.ends_with(".env")
            || part.starts_with("credentials")
            || part.contains("secret")
            || matches!(
                part.as_str(),
                ".aws"
                    | ".ssh"
                    | ".gnupg"
                    | ".npmrc"
                    | ".pypirc"
                    | "id_rsa"
                    | "id_ed25519"
                    | ".netrc"
                    | ".git-credentials"
            )
            || [
                ".pem",
                ".key",
                ".p12",
                ".pfx",
                ".keystore",
                ".jks",
                ".tfstate",
                ".tfstate.backup",
            ]
            .iter()
            .any(|ending| part.ends_with(ending))
    })
}
fn sample_env_path(path: &str) -> bool {
    !path
        .rsplit_once('/')
        .is_some_and(|(parent, _)| sensitive_path(parent))
        && matches!(
            path.rsplit('/').next().unwrap_or(path),
            ".env.example" | ".env.sample" | ".env.template"
        )
}
fn variable_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().enumerate().all(|(index, ch)| {
            ch.is_ascii_alphabetic() || ch == b'_' || (index > 0 && ch.is_ascii_digit())
        })
}
fn environment_names(source: &str) -> (Vec<String>, bool) {
    let names = source
        .lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
            let (name, _) = line.split_once('=')?;
            let name = name.trim();
            variable_name(name).then(|| name.to_owned())
        })
        .collect::<std::collections::BTreeSet<_>>();
    let truncated = names.len() > 128;
    (names.into_iter().take(128).collect(), truncated)
}
fn kind(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod" | "pom.xml" => "manifest",
        "Cargo.lock" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "uv.lock" => {
            "lockfile"
        }
        "vercel.json" | "vercel.toml" | "netlify.toml" | "railway.json" | "railway.toml" => {
            "deployment_config"
        }
        "vercel.ts" => "executable_deployment_config",
        "railway.ts" if path.ends_with(".railway/railway.ts") => "executable_deployment_config",
        "Dockerfile" | "Makefile" => "build_metadata",
        _ if path.ends_with(".sql") || path.ends_with(".prisma") => "database_schema",
        _ if path.starts_with(".github/workflows/") => "ci_config",
        _ if [".graphql", ".proto"].iter().any(|ext| path.ends_with(ext))
            || name.starts_with("openapi.") =>
        {
            "api_schema"
        }
        _ if [".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java"]
            .iter()
            .any(|ext| path.ends_with(ext)) =>
        {
            "source"
        }
        _ => "unsupported",
    }
}
/// Each request enumerates bounded metadata and reads at most 64 blobs using the
/// existing Phase 1 batch reader. Resume never changes commits or reads worktree files.
pub fn scan_chunk(
    root: &Path,
    session: &OnboardingSession,
    previous: Option<&OnboardingInventory>,
    cancel: &Cancellation,
    principal: &crate::PrincipalId,
    grants: &[crate::Grant],
) -> Result<OnboardingInventory, String> {
    session.check_access(principal, grants)?;
    if !matches!(
        session.state(),
        crate::OnboardingState::Draft
            | crate::OnboardingState::Scanning
            | crate::OnboardingState::Partial
    ) {
        return Err("Inventory scan is not available in this session state".into());
    }
    cancel.checkpoint()?;
    let mut result = previous
        .cloned()
        .unwrap_or_else(|| OnboardingInventory::empty(session));
    result.validate(session)?;
    if result.complete {
        return Ok(result);
    }
    let (_, commit, paths) = pin(session, result.repository_index)?;
    let bytes = match git(
        root,
        &["ls-tree", "-r", "-z", commit.as_str(), "--"],
        8 * 1024 * 1024,
    ) {
        Ok(bytes) => bytes,
        Err(_) => {
            result.enumeration_complete = false;
            result.failed += 1;
            if result.gaps.len() < 32 {
                result.gaps.push(format!("Repository {} tree enumeration unavailable or exceeds 8 MiB; no complete denominator",result.repository_index));
            }
            advance(session, &mut result);
            return Ok(result);
        }
    };
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    if result.offset > entries.len() {
        return Err("Stale inventory cursor".into());
    }
    let end = (result.offset + CHUNK).min(entries.len());
    let mut metadata_bytes = result
        .items
        .iter()
        .map(InventoryItem::metadata_bytes)
        .sum::<usize>();
    let mut pending = Vec::new();
    for entry in &entries[result.offset..end] {
        cancel.checkpoint()?;
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            return Err("Invalid Git tree metadata".into());
        };
        let path = match std::str::from_utf8(&entry[tab + 1..]) {
            Ok(path) if path.len() <= 256 && !path.chars().any(char::is_control) => path,
            _ => {
                result.failed += 1;
                continue;
            }
        };
        if !paths.allows(path) {
            result.excluded += 1;
            continue;
        }
        let sample_names = sample_names_enabled(session) && sample_env_path(path);
        if sensitive_path(path) && !sample_names {
            result.sensitive_excluded += 1;
            continue;
        }
        let header = std::str::from_utf8(&entry[..tab])
            .map_err(|_| "Invalid tree metadata")?
            .split_ascii_whitespace()
            .collect::<Vec<_>>();
        if header.len() != 3 {
            return Err("Invalid tree metadata".into());
        }
        if result.items.len() >= MAX_ITEMS {
            result.deferred += 1;
            continue;
        }
        let kind = if sample_names {
            "environment_variable_names"
        } else {
            let detected = kind(path);
            if result.version == "csb-inventory-v1"
                && matches!(detected, "database_schema" | "executable_deployment_config")
            {
                if path.ends_with(".ts") {
                    "source"
                } else {
                    "unsupported"
                }
            } else if result.version == "csb-inventory-v1" && path.ends_with("vercel.toml") {
                "unsupported"
            } else {
                detected
            }
        };
        let mut status = "pending";
        if !matches!(header[0], "100644" | "100755") || header[1] != "blob" || kind == "unsupported"
        {
            status = "unsupported";
            result.unsupported += 1;
        }
        let item = InventoryItem {
            repository_index: result.repository_index,
            path: path.into(),
            blob: header[2].into(),
            kind: kind.into(),
            status: status.into(),
            variable_names: vec![],
        };
        if metadata_bytes + item.metadata_bytes() > MAX_METADATA_BYTES {
            if status == "unsupported" {
                result.unsupported -= 1;
            }
            result.deferred += 1;
            continue;
        }
        metadata_bytes += item.metadata_bytes();
        result.items.push(item);
        if status == "pending" {
            pending.push((result.items.len() - 1, header[2].to_string()));
        }
    }
    let ids = pending.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>();
    let blobs = read_chunk(
        root,
        &ids,
        ReadLimits {
            objects_per_chunk: CHUNK,
            bytes_per_object: 256 * 1024,
            bytes_per_chunk: 16 * 1024 * 1024,
        },
        cancel,
    )?;
    for ((index, _), blob) in pending.into_iter().zip(blobs) {
        let status = match blob {
            BatchBlob::Data(bytes)
                if !bytes.contains(&0) && std::str::from_utf8(&bytes).is_ok() =>
            {
                if result.items[index].kind == "environment_variable_names" {
                    let (names, truncated) =
                        environment_names(std::str::from_utf8(&bytes).unwrap());
                    let added = names.iter().map(|name| 2 * name.len() + 4).sum::<usize>();
                    if metadata_bytes + added > MAX_METADATA_BYTES {
                        result.deferred += 1;
                        result.items[index].status = "deferred".into();
                        continue;
                    }
                    metadata_bytes += added;
                    result.items[index].variable_names = names;
                    if truncated {
                        result.deferred += 1;
                        result.items[index].status = "deferred".into();
                        continue;
                    }
                }
                "indexed"
            }
            BatchBlob::Data(_) => {
                result.unsupported += 1;
                "unsupported"
            }
            BatchBlob::Missing => {
                result.failed += 1;
                "failed"
            }
            BatchBlob::Oversized(_) => {
                result.deferred += 1;
                "deferred"
            }
        };
        result.items[index].status = status.into();
    }
    result.offset = end;
    if end == entries.len() {
        advance(session, &mut result);
    }
    result.validate(session)?;
    Ok(result)
}
fn advance(session: &OnboardingSession, result: &mut OnboardingInventory) {
    result.repository_index += 1;
    result.offset = 0;
    result.complete = result.repository_index > session.scope().contexts.len();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Grant, PrincipalId, ScopeRecord, ScopeVersion, SecurityDomain};
    use std::{
        collections::BTreeSet,
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn git_fixture(root: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn pinned_scans_resume_without_executing_or_persisting_sensitive_content() {
        let root = std::env::temp_dir().join(format!(
            "csb-safe-scan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(root.join("src/private")).unwrap();
        git_fixture(&root, &["init", "-q"]);
        for index in 0..90 {
            std::fs::write(
                root.join(format!("src/f{index:03}.rs")),
                "pub fn hello() {}\n",
            )
            .unwrap();
        }
        std::fs::write(root.join("src/private/key.rs"), "SCOPE_LEAK_SENTINEL").unwrap();
        std::fs::write(root.join(".env.example"), "API_KEY=ENV_SECRET_SENTINEL").unwrap();
        std::fs::write(root.join("secrets.json"), "PASSWORD_SENTINEL").unwrap();
        std::fs::write(root.join("package.json"),r#"{"scripts":{"prepare":"touch EXECUTION_SENTINEL"},"password":"CONFIG_SECRET_SENTINEL"}"#).unwrap();
        std::fs::write(root.join("src/large.rs"), vec![b'x'; 300 * 1024]).unwrap();
        std::fs::write(root.join("src/binary.rs"), [0, 1, 2]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("../../outside", root.join("src/link.rs")).unwrap();
        git_fixture(&root, &["add", "."]);
        git_fixture(
            &root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        let principal = PrincipalId::new("owner").unwrap();
        let repository =
            RepositoryId::new(SecurityDomain::new("test").unwrap(), "fixture", "o", "r").unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("scope").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::new(vec![".".into()], vec!["src/private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let grants = vec![Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index]),
            paths: PathScope::all(),
        }];
        let commit = SnapshotId::new(crate::git::resolve(&root, "HEAD").unwrap()).unwrap();
        let mut session = OnboardingSession::new(
            "scan".into(),
            principal.clone(),
            commit,
            scope,
            INVENTORY_VERSION.into(),
            &grants,
        )
        .unwrap();
        // Worktree edits after pinning must not affect the scanned object IDs or contents.
        std::fs::write(root.join("src/f000.rs"), "WORKTREE_SENTINEL").unwrap();
        let first = scan_chunk(
            &root,
            &session,
            None,
            &Cancellation::default(),
            &principal,
            &grants,
        )
        .unwrap();
        assert!(!first.complete);
        assert_eq!(first.offset, 64);
        session
            .record_inventory(0, first, &principal, &grants)
            .unwrap();
        let done = scan_chunk(
            &root,
            &session,
            session.inventory(),
            &Cancellation::default(),
            &principal,
            &grants,
        )
        .unwrap();
        assert!(done.complete && done.enumeration_complete);
        assert_eq!(done.sensitive_excluded, 2);
        assert_eq!(done.excluded, 1);
        assert_eq!(done.deferred, 1);
        assert!(done.items.iter().any(|item| item.path == "package.json"
            && item.kind == "manifest"
            && item.status == "indexed"));
        assert!(
            done.items
                .iter()
                .any(|item| item.path == "src/binary.rs" && item.status == "unsupported")
        );
        assert!(!root.join("EXECUTION_SENTINEL").exists());
        #[cfg(feature = "server")]
        {
            let output = serde_json::to_string(&done).unwrap();
            for value in [
                "ENV_SECRET_SENTINEL",
                "CONFIG_SECRET_SENTINEL",
                "SCOPE_LEAK_SENTINEL",
                "PASSWORD_SENTINEL",
                "WORKTREE_SENTINEL",
            ] {
                assert!(!output.contains(value));
            }
        }
        let names_session = OnboardingSession::new(
            "names".into(),
            principal.clone(),
            session.commit().clone(),
            session.scope().clone(),
            "csb-inventory-v1+sample-env-names".into(),
            &grants,
        )
        .unwrap();
        let names = scan_chunk(
            &root,
            &names_session,
            None,
            &Cancellation::default(),
            &principal,
            &grants,
        )
        .unwrap();
        let item = names
            .items
            .iter()
            .find(|item| item.path == ".env.example")
            .unwrap();
        assert_eq!(item.variable_names, vec!["API_KEY"]);
        #[cfg(feature = "server")]
        assert!(
            !serde_json::to_string(&names)
                .unwrap()
                .contains("ENV_SECRET_SENTINEL")
        );
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(scan_chunk(&root, &session, None, &cancel, &principal, &grants).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn environment_name_inventory_defers_metadata_without_losing_resume_progress() {
        let root = std::env::temp_dir().join(format!("csb-env-budget-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        git_fixture(&root, &["init", "-q"]);
        let source = (0..128)
            .map(|index| format!("KEY_{index:03}_{}=VALUE_SENTINEL\n", "X".repeat(50)))
            .collect::<String>();
        for index in 0..70 {
            std::fs::create_dir_all(root.join(format!("app-{index:03}"))).unwrap();
            std::fs::write(root.join(format!("app-{index:03}/.env.example")), &source).unwrap();
        }
        git_fixture(&root, &["add", "."]);
        git_fixture(
            &root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "environment budget",
            ],
        );
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
            primary_repository: repository,
            primary_paths: PathScope::all(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let commit = SnapshotId::new(crate::git::resolve(&root, "HEAD").unwrap()).unwrap();
        let mut session = OnboardingSession::new(
            "environment-budget".into(),
            principal.clone(),
            commit,
            scope,
            "csb-inventory-v1+sample-env-names".into(),
            &grants,
        )
        .unwrap();
        loop {
            let inventory = scan_chunk(
                &root,
                &session,
                session.inventory(),
                &Cancellation::default(),
                &principal,
                &grants,
            )
            .unwrap();
            let done = inventory.complete;
            session
                .record_inventory(session.revision(), inventory, &principal, &grants)
                .unwrap();
            if done {
                break;
            }
        }
        let inventory = session.inventory().unwrap();
        assert!(inventory.enumeration_complete);
        assert!(inventory.deferred > 0);
        assert!(
            inventory
                .items
                .iter()
                .map(InventoryItem::metadata_bytes)
                .sum::<usize>()
                <= MAX_METADATA_BYTES
        );
        assert_eq!(
            inventory
                .items
                .iter()
                .filter(|item| item.status == "indexed")
                .count()
                + inventory.deferred,
            70
        );
        #[cfg(feature = "server")]
        {
            let wire = serde_json::to_string(inventory).unwrap();
            assert!(wire.len() < MAX_METADATA_BYTES);
            assert!(!wire.contains("VALUE_SENTINEL"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sensitive_paths_cover_samples_keys_and_state() {
        assert!(!sample_env_path("secrets/.env.example"));
        assert!(!sample_env_path(".aws/.env.sample"));
        let (names, truncated) = environment_names(
            &(0..129)
                .map(|index| format!("KEY_{index}=SENTINEL\n"))
                .collect::<String>(),
        );
        assert_eq!(names.len(), 128);
        assert!(truncated);
        for path in [
            ".env.example",
            "service/.env.production",
            "keys/id_rsa",
            ".aws/config",
            "config/secrets.yml",
            "tls/key.pem",
            "terraform.tfstate",
            ".npmrc",
        ] {
            assert!(sensitive_path(path), "{path}");
        }
        assert!(!sensitive_path("src/service.rs"));
    }
}
