//! Credential-free preview protocol. Production callers must use the isolated runner.
use super::*;
use crate::{
    Action, ComponentProposal, ComponentProviderMapping, Grant, OnboardingInventory,
    OnboardingSession, PathScope, PrincipalId, RepositoryId, ScopeRecord, ScopeVersion,
    SecurityDomain, SnapshotId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub const SCAN_VERSION: &str = "csb-app-preview-v1";
pub const MAX_SOURCE_FILES: usize = 32;
pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewFile {
    pub path: String,
    pub oid: String,
    pub content: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub actor_id: i64,
    pub repository_owner: String,
    pub repository_name: String,
    pub document: ScopeDocument,
    pub files: Vec<PreviewFile>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanResult {
    pub version: String,
    pub input_fingerprint: String,
    pub components: Vec<ComponentProposal>,
    pub providers: Vec<ComponentProviderMapping>,
    pub gaps: Vec<String>,
}
pub trait AppScanWorker: Send + Sync + 'static {
    /// Input contains only authorized objects, never credentials. Implementations
    /// must fail closed when isolation, pinned engine or limits are unavailable.
    fn preview(&self, request: &ScanRequest) -> Result<ScanResult>;
}
pub fn supported_source(path: &str) -> bool {
    path.ends_with(".rs")
        || matches!(
            path.rsplit('/').next(),
            Some(
                "Cargo.toml"
                    | "vercel.json"
                    | "vercel.toml"
                    | "netlify.toml"
                    | "railway.json"
                    | "railway.toml"
            )
        )
}
pub(super) fn session(
    document: &ScopeDocument,
    actor_id: i64,
    owner: &str,
    name: &str,
) -> Result<(OnboardingSession, PrincipalId, Vec<Grant>)> {
    let domain = SecurityDomain::new(format!("github-app-{}", document.installation_id))
        .map_err(|_| SetupError::Invalid)?;
    if actor_id <= 0 || !slug(owner) || !slug(name) {
        return Err(SetupError::Invalid);
    }
    let repo = RepositoryId::new(domain, "github", owner, name).map_err(|_| SetupError::Invalid)?;
    let principal =
        PrincipalId::new(format!("github-user-{actor_id}")).map_err(|_| SetupError::Invalid)?;
    let paths = PathScope::new(document.include.clone(), document.exclude.clone())
        .map_err(|_| SetupError::Invalid)?;
    let grants = vec![Grant {
        principal: principal.clone(),
        repository: repo.clone(),
        paths: paths.clone(),
        actions: BTreeSet::from([
            Action::Index,
            Action::Graph,
            Action::Source,
            Action::ConfirmArchitecture,
        ]),
    }];
    let scope = ScopeRecord {
        version: ScopeVersion::new(document.inventory_fingerprint.clone())
            .map_err(|_| SetupError::Invalid)?,
        primary_repository: repo,
        primary_paths: paths,
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: false,
        model_destinations: BTreeSet::new(),
    };
    let session = OnboardingSession::new(
        document.inventory_fingerprint.clone(),
        principal.clone(),
        SnapshotId::new(document.commit.clone()).map_err(|_| SetupError::Invalid)?,
        scope,
        crate::INVENTORY_VERSION.into(),
        &grants,
    )
    .map_err(|_| SetupError::Invalid)?;
    Ok((session, principal, grants))
}
pub(super) fn inventory(
    document: &ScopeDocument,
    files: &[PreviewFile],
    session: &OnboardingSession,
) -> Result<OnboardingInventory> {
    if files.len() != document.files.len() || files.len() > 2000 {
        return Err(SetupError::Invalid);
    }
    let paths = PathScope::new(document.include.clone(), document.exclude.clone())
        .map_err(|_| SetupError::Invalid)?;
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    let mut supported = 0;
    for f in files {
        if !document.files.contains(&f.path)
            || !seen.insert(&f.path)
            || !paths.allows(&f.path)
            || crate::sensitive_path(&f.path)
            || !scope::oid(&f.oid)
        {
            return Err(SetupError::Invalid);
        }
        let allowed = supported_source(&f.path);
        if allowed {
            supported += 1;
        }
        if allowed != f.content.is_some()
            || f.content
                .as_ref()
                .is_some_and(|s| s.len() > MAX_SOURCE_BYTES || s.contains('\0'))
        {
            return Err(SetupError::Invalid);
        }
        items.push(crate::InventoryItem {
            repository_index: 0,
            path: f.path.clone(),
            blob: f.oid.clone(),
            kind: if f.path.ends_with(".rs") {
                "source"
            } else if f.path.ends_with("Cargo.toml") {
                "manifest"
            } else if allowed {
                "deployment_config"
            } else {
                "unsupported"
            }
            .into(),
            status: if allowed { "indexed" } else { "unsupported" }.into(),
            variable_names: vec![],
        });
    }
    if supported == 0 || supported > MAX_SOURCE_FILES {
        return Err(SetupError::Capacity);
    }
    let result = OnboardingInventory {
        version: crate::INVENTORY_VERSION.into(),
        manifest_fingerprint: session.input_fingerprint(),
        repository_index: 1,
        offset: 0,
        complete: true,
        enumeration_complete: true,
        unsupported: files.len() - supported,
        items,
        excluded: 0,
        sensitive_excluded: document.automatically_excluded,
        deferred: 0,
        failed: 0,
        gaps: vec![],
    };
    result.validate(session).map_err(|_| SetupError::Capacity)?;
    Ok(result)
}
/// Native protocol implementation. Invoke only inside the networkless worker;
/// `root` is a new private temporary directory, never a user checkout.
pub fn analyze_preview(request: &ScanRequest, root: &Path) -> Result<ScanResult> {
    let (mut session, principal, grants) = session(
        &request.document,
        request.actor_id,
        &request.repository_owner,
        &request.repository_name,
    )?;
    let inventory = inventory(&request.document, &request.files, &session)?;
    if root.exists() {
        return Err(SetupError::Invalid);
    }
    let mut directory = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory
        .create(root)
        .map_err(|_| SetupError::Unavailable)?;
    let run = |args: &[&str], input: Option<&[u8]>| -> Result<Vec<u8>> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "core.hooksPath=/dev/null", "-c", "init.templateDir="])
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| SetupError::Unavailable)?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .ok_or(SetupError::Unavailable)?
                .write_all(input)
                .map_err(|_| SetupError::Unavailable)?;
        }
        let output = child
            .wait_with_output()
            .map_err(|_| SetupError::Unavailable)?;
        if !output.status.success() || output.stdout.len() > 1024 {
            return Err(SetupError::Unavailable);
        }
        Ok(output.stdout)
    };
    let result = (|| {
        run(&["init", "--bare", "--quiet"], None)?;
        for file in &request.files {
            if let Some(source) = &file.content {
                let actual = run(&["hash-object", "-w", "--stdin"], Some(source.as_bytes()))?;
                if std::str::from_utf8(&actual).ok().map(str::trim) != Some(file.oid.as_str()) {
                    return Err(SetupError::Invalid);
                }
            }
        }
        session
            .record_inventory(0, inventory, &principal, &grants)
            .map_err(|_| SetupError::Invalid)?;
        let roots = BTreeMap::from([(
            session.scope().primary_repository.clone(),
            root.to_path_buf(),
        )]);
        let (components, mut gaps) =
            crate::onboarding_detection::detect(&roots, &session, &principal, &grants)
                .map_err(|_| SetupError::Unavailable)?;
        let (providers, provider_gaps) = crate::onboarding::providers::detect(
            &roots,
            &session,
            &components,
            &principal,
            &grants,
        )
        .map_err(|_| SetupError::Unavailable)?;
        gaps.extend(provider_gaps);
        gaps.push("Runtime behavior and hosting are unverified; integrations remain deferred. No publication consent or activation.".into());
        if components.is_empty() {
            gaps.push("No evidenced component boundary. Include an eligible Cargo.toml in a new scope to detect Rust components.".into());
        }
        gaps.truncate(100);
        Ok(ScanResult {
            version: SCAN_VERSION.into(),
            input_fingerprint: request.document.inventory_fingerprint.clone(),
            components,
            providers,
            gaps,
        })
    })();
    let cleanup = std::fs::remove_dir_all(root).map_err(|_| SetupError::Unavailable);
    cleanup?;
    result
}

/// Operator-owned executable paths and verified engine digest. This adapter has
/// no fallback on macOS, on a missing sandbox or when the engine digest differs.
pub struct SandboxPreviewWorker {
    python: std::path::PathBuf,
    launcher: std::path::PathBuf,
    engine: std::path::PathBuf,
    digest: String,
}
impl SandboxPreviewWorker {
    pub fn new(
        python: std::path::PathBuf,
        launcher: std::path::PathBuf,
        engine: std::path::PathBuf,
        digest: String,
    ) -> Result<Self> {
        if ![&python, &launcher, &engine]
            .iter()
            .all(|p| p.is_absolute() && p.is_file())
            || !valid_secret(&digest)
        {
            return Err(SetupError::Invalid);
        }
        if !cfg!(target_os = "linux")
            || engine.is_symlink()
            || engine
                .metadata()
                .map_err(|_| SetupError::Unavailable)?
                .len()
                > 64 * 1024 * 1024
        {
            return Err(SetupError::Unavailable);
        }
        let bytes = std::fs::read(&engine).map_err(|_| SetupError::Unavailable)?;
        let actual = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if actual != digest {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            python,
            launcher,
            engine,
            digest,
        })
    }
}
impl AppScanWorker for SandboxPreviewWorker {
    fn preview(&self, request: &ScanRequest) -> Result<ScanResult> {
        let data = serde_json::to_vec(request).map_err(|_| SetupError::Invalid)?;
        if data.len() > 16 * 1024 * 1024 {
            return Err(SetupError::Capacity);
        }
        let mut child = Command::new(&self.python)
            .arg("-I")
            .arg(&self.launcher)
            .arg(&self.engine)
            .arg(&self.digest)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| SetupError::Unavailable)?;
        let mut stdin = child.stdin.take().ok_or(SetupError::Unavailable)?;
        let writer = std::thread::spawn(move || stdin.write_all(&data));
        let stdout = child.stdout.take().ok_or(SetupError::Unavailable)?;
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let start = std::time::Instant::now();
        let success = loop {
            if start.elapsed() > std::time::Duration::from_secs(50) {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            match child.try_wait() {
                Ok(Some(s)) => break s.success(),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };
        let written = writer.join().map_err(|_| SetupError::Unavailable)?.is_ok();
        let bytes = reader
            .join()
            .map_err(|_| SetupError::Unavailable)?
            .map_err(|_| SetupError::Unavailable)?;
        if !success || !written || bytes.len() > 1024 * 1024 {
            return Err(SetupError::Unavailable);
        }
        serde_json::from_slice(&bytes).map_err(|_| SetupError::Unavailable)
    }
}
