//! Exact, credential-free Git object protocol for the networkless PR worker.
use super::{AnalysisInput, Result, SetupError};
use crate::public_report::{
    ExpectedComparison, PublicBundle, PublicManifest, PublicPolicy, export_public_report,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
pub const MAX_PR_INPUT: usize = 16 * 1024 * 1024;
pub const MAX_PR_OUTPUT: usize = 16 * 1024 * 1024;
pub const MAX_PR_BLOBS: usize = 64;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitObject {
    pub oid: String,
    pub kind: String,
    pub base64: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrWorkerRequest {
    pub input: AnalysisInput,
    pub policy: PublicPolicy,
    pub context: Value,
    pub objects: Vec<GitObject>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrWorkerOutput {
    pub manifest: PublicManifest,
    pub files: BTreeMap<String, String>,
}
impl PrWorkerOutput {
    pub fn into_bundle(self) -> Result<PublicBundle> {
        let bundle = PublicBundle {
            manifest: self.manifest,
            files: self
                .files
                .into_iter()
                .map(|(p, s)| (p, s.into_bytes()))
                .collect(),
        };
        bundle.validate().map_err(|_| SetupError::Invalid)?;
        Ok(bundle)
    }
}
pub trait AppPrWorker: Send + Sync {
    fn engine_digest(&self) -> &str;
    fn analyze(&self, request: &PrWorkerRequest) -> Result<PublicBundle>;
}
pub struct SandboxPrWorker {
    runner: super::SandboxPreviewWorker,
    digest: String,
}
impl SandboxPrWorker {
    pub fn new(
        python: std::path::PathBuf,
        launcher: std::path::PathBuf,
        engine: std::path::PathBuf,
        digest: String,
    ) -> Result<Self> {
        Ok(Self {
            runner: super::SandboxPreviewWorker::new(python, launcher, engine, digest.clone())?,
            digest,
        })
    }
}
impl AppPrWorker for SandboxPrWorker {
    fn engine_digest(&self) -> &str {
        &self.digest
    }
    fn analyze(&self, request: &PrWorkerRequest) -> Result<PublicBundle> {
        if request.input.engine_digest != self.digest {
            return Err(SetupError::Denied);
        }
        let data = serde_json::to_vec(request).map_err(|_| SetupError::Invalid)?;
        if data.len() > MAX_PR_INPUT {
            return Err(SetupError::Capacity);
        }
        let bytes = self.runner.run_protocol(data, MAX_PR_OUTPUT, 100)?;
        let output: PrWorkerOutput =
            serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
        output.into_bundle()
    }
}
pub(super) fn object_id(kind: &str, data: &[u8]) -> String {
    let mut hash = ring::digest::Context::new(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY);
    hash.update(format!("{kind} {}\0", data.len()).as_bytes());
    hash.update(data);
    hash.finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(super) fn object(kind: &str, bytes: &[u8]) -> GitObject {
    GitObject {
        kind: kind.into(),
        oid: object_id(kind, bytes),
        base64: STANDARD.encode(bytes),
    }
}
fn decode(request: &PrWorkerRequest) -> Result<BTreeMap<String, (String, Vec<u8>)>> {
    if request.objects.len() > 4096
        || serde_json::to_vec(request)
            .map_err(|_| SetupError::Invalid)?
            .len()
            > MAX_PR_INPUT
    {
        return Err(SetupError::Capacity);
    }
    let mut result = BTreeMap::new();
    let mut blobs = 0;
    for object in &request.objects {
        if !matches!(object.kind.as_str(), "commit" | "tree" | "blob")
            || !super::scope::oid(&object.oid)
        {
            return Err(SetupError::Invalid);
        }
        let bytes = STANDARD
            .decode(&object.base64)
            .map_err(|_| SetupError::Invalid)?;
        if bytes.len() > 256 * 1024
            || object_id(&object.kind, &bytes) != object.oid
            || result
                .insert(object.oid.clone(), (object.kind.clone(), bytes))
                .is_some()
        {
            return Err(SetupError::Invalid);
        }
        if object.kind == "blob" {
            blobs += 1;
        }
    }
    if blobs > MAX_PR_BLOBS {
        return Err(SetupError::Capacity);
    }
    Ok(result)
}
/// Validate reachability and path authorization before materializing *any* object.
fn validate_objects(
    request: &PrWorkerRequest,
    objects: &BTreeMap<String, (String, Vec<u8>)>,
) -> Result<()> {
    let scope = request.policy.validate().map_err(|_| SetupError::Denied)?;
    let mut reachable = BTreeSet::new();
    let mut allowed = BTreeSet::new();
    let mut entries = 0;
    for commit in BTreeSet::from([
        &request.input.comparison.merge_base,
        &request.input.comparison.head,
    ]) {
        let (kind, data) = objects.get(commit).ok_or(SetupError::Invalid)?;
        if kind != "commit" {
            return Err(SetupError::Invalid);
        }
        let text = std::str::from_utf8(data).map_err(|_| SetupError::Invalid)?;
        let tree = text
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("tree "))
            .filter(|s| super::scope::oid(s))
            .ok_or(SetupError::Invalid)?;
        reachable.insert(commit.clone());
        let mut stack = vec![(tree.to_owned(), String::new(), 0)];
        while let Some((oid, prefix, depth)) = stack.pop() {
            if depth > 32 {
                return Err(SetupError::Capacity);
            }
            reachable.insert(oid.clone());
            let (kind, data) = objects.get(&oid).ok_or(SetupError::Invalid)?;
            if kind != "tree" {
                return Err(SetupError::Invalid);
            }
            let mut rest = data.as_slice();
            let mut names = BTreeSet::new();
            while !rest.is_empty() {
                entries += 1;
                if entries > 4000 {
                    return Err(SetupError::Capacity);
                }
                let end = rest
                    .iter()
                    .position(|b| *b == 0)
                    .ok_or(SetupError::Invalid)?;
                let header = std::str::from_utf8(&rest[..end]).map_err(|_| SetupError::Invalid)?;
                let (mode, name) = header.split_once(' ').ok_or(SetupError::Invalid)?;
                if name.is_empty()
                    || name.contains('/')
                    || matches!(name, "." | ".." | ".git")
                    || !names.insert(name.to_owned())
                    || name.chars().any(char::is_control)
                {
                    return Err(SetupError::Invalid);
                }
                let raw = rest.get(end + 1..end + 21).ok_or(SetupError::Invalid)?;
                let child = raw.iter().map(|b| format!("{b:02x}")).collect::<String>();
                rest = &rest[end + 21..];
                let path = if prefix.is_empty() {
                    name.into()
                } else {
                    format!("{prefix}/{name}")
                };
                match mode {
                    "40000" => stack.push((child, path, depth + 1)),
                    "100644" | "100755" => {
                        if scope.allows(&path) && !crate::sensitive_path(&path) {
                            let (kind, bytes) = objects.get(&child).ok_or(SetupError::Invalid)?;
                            if kind != "blob"
                                || std::str::from_utf8(bytes).is_err()
                                || bytes.contains(&0)
                            {
                                return Err(SetupError::Invalid);
                            }
                            allowed.insert(child);
                        }
                    }
                    "120000" | "160000" => {}
                    _ => return Err(SetupError::Invalid),
                }
            }
        }
    }
    reachable.extend(allowed);
    if objects.keys().any(|id| !reachable.contains(id)) {
        return Err(SetupError::Denied);
    }
    Ok(())
}
/// Called by csb-app-pr-worker inside the sandbox. Never run repository scripts.
pub fn analyze_pr_objects(request: &PrWorkerRequest, root: &Path) -> Result<PrWorkerOutput> {
    use crate::{
        Action, Cancellation, ComparisonMode, Grant, PrincipalId, RepositoryId, ReviewRequest,
        ScopeRecord, ScopeVersion, SecurityDomain,
    };
    let input = &request.input;
    if input.schema != "csb-app-analysis-v1"
        || input.repository_id as u64 != request.policy.repository.id
        || !input.comparison.open
        || input.comparison.draft
        || input.comparison.repository_id != input.repository_id
        || input.comparison.number <= 0
        || request.policy.baseline_mode != "merge_base"
    {
        return Err(SetupError::Invalid);
    }
    let objects = decode(request)?;
    validate_objects(request, &objects)?;
    let repo = RepositoryId::new(
        SecurityDomain::new(format!("github-app-{}", input.installation_id))
            .map_err(|_| SetupError::Invalid)?,
        "github",
        &request.policy.repository.owner,
        &request.policy.repository.name,
    )
    .map_err(|_| SetupError::Invalid)?;
    let principal = PrincipalId::new("app-pr-worker").map_err(|_| SetupError::Invalid)?;
    let scope = ScopeRecord {
        version: ScopeVersion::new(input.scope_version()).map_err(|_| SetupError::Invalid)?,
        primary_repository: repo.clone(),
        primary_paths: request.policy.validate().map_err(|_| SetupError::Denied)?,
        contexts: vec![],
        contracts: BTreeSet::new(),
        export_allowed: true,
        model_destinations: BTreeSet::new(),
    };
    crate::design::ConfirmedDesignContext::from_accepted(&request.context, &scope)
        .map_err(|_| SetupError::Denied)?;
    let grants = vec![Grant {
        principal: principal.clone(),
        repository: repo,
        paths: scope.primary_paths.clone(),
        actions: BTreeSet::from([Action::Index, Action::Graph, Action::Source, Action::Export]),
    }];
    let mut directory = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory
        .create(root)
        .map_err(|_| SetupError::Unavailable)?;
    let result = (|| {
        let mirror = root.join("objects.git");
        std::fs::create_dir(&mirror).map_err(|_| SetupError::Unavailable)?;
        let run = |args: &[&str], bytes: Option<&[u8]>| -> Result<Vec<u8>> {
            let mut child = Command::new("git")
                .arg("-C")
                .arg(&mirror)
                .args([
                    "-c",
                    "init.templateDir=",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "protocol.allow=never",
                ])
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
            if let Some(bytes) = bytes {
                child
                    .stdin
                    .take()
                    .ok_or(SetupError::Unavailable)?
                    .write_all(bytes)
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
        run(&["init", "--bare", "--quiet"], None)?;
        for (oid, (kind, bytes)) in &objects {
            let got = run(&["hash-object", "-w", "-t", kind, "--stdin"], Some(bytes))?;
            if std::str::from_utf8(&got).ok().map(str::trim) != Some(oid) {
                return Err(SetupError::Invalid);
            }
        }
        std::fs::write(
            mirror.join("shallow"),
            format!(
                "{}\n{}\n",
                input.comparison.merge_base, input.comparison.head
            ),
        )
        .map_err(|_| SetupError::Unavailable)?;
        let cancel = Cancellation::default();
        let result = crate::analyze_review(
            &ReviewRequest {
                repository: mirror.clone(),
                baseline: input.comparison.merge_base.clone(),
                proposal: input.comparison.head.clone(),
                comparison: ComparisonMode::Exact,
                principal,
                scope,
                current_grants: grants.clone(),
                context_manifest: None,
                cache_dir: root.join("cache"),
            },
            &cancel,
        )
        .map_err(|_| SetupError::Unavailable)?;
        let mut artifact = crate::ReviewArtifact::from_analysis(&result);
        artifact
            .generate_hld(&result, &mirror, &request.context, &grants, &cancel)
            .map_err(|_| SetupError::Unavailable)?;
        artifact
            .generate_lld(&result, &mirror, &request.context, &grants, &cancel)
            .map_err(|_| SetupError::Unavailable)?;
        artifact.selected_baseline = input.comparison.selected_base.clone();
        artifact.review_id = input.id()?;
        let value = serde_json::to_value(artifact).map_err(|_| SetupError::Invalid)?;
        let bundle = export_public_report(
            &value,
            &request.policy,
            &ExpectedComparison {
                repository: request.policy.repository.clone(),
                pr_number: input.comparison.number as u64,
                selected_baseline: input.comparison.selected_base.clone(),
                baseline: input.comparison.merge_base.clone(),
                proposal: input.comparison.head.clone(),
            },
        )
        .map_err(|_| SetupError::Invalid)?;
        Ok(PrWorkerOutput {
            manifest: bundle.manifest,
            files: bundle
                .files
                .into_iter()
                .map(|(p, b)| {
                    String::from_utf8(b)
                        .map(|s| (p, s))
                        .map_err(|_| SetupError::Invalid)
                })
                .collect::<Result<_>>()?,
        })
    })();
    std::fs::remove_dir_all(root).map_err(|_| SetupError::Unavailable)?;
    result
}
