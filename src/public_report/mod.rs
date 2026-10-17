//! Explicit public projection. A private review artifact is never a public DTO.
//! This library checks consent and pinned inputs; the trusted CI controller must
//! independently establish that policy came from its approved base commit and
//! recheck current public visibility before publishing. It makes no network call.
mod policy;
mod project;
mod validate;
mod write;

pub use policy::{ExpectedComparison, PublicPolicy, PublicRepository};
pub use write::{read_json_file, verify_bundle, write_bundle};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const PUBLIC_SCHEMA: &str = "csb-public-report-v1";
pub const MAX_PUBLIC_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PUBLIC_FILES: usize = 1000;
pub const MAX_PUBLIC_SOURCE_BYTES: usize = 2 * 1024 * 1024;

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    // serde_json's map is sorted (the preserve_order feature is not enabled).
    serde_json::to_vec(&serde_json::to_value(value).map_err(|_| "Invalid public value")?)
        .map_err(|_| "Cannot encode public value".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicFile {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DetailBinding {
    pub entry: Option<String>,
    pub parent_hld: String,
    pub component: Option<String>,
    pub baseline_entry: Option<String>,
    pub correspondence_kind: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub evidence: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileSourceBinding {
    pub file_path: String,
    pub side: String,
    pub parent_hld: String,
    pub path: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicRoutes {
    pub artifact: String,
    pub hld: Option<String>,
    pub lld_catalog: Option<String>,
    pub lld_details: Vec<DetailBinding>,
    pub sources: Vec<DetailBinding>,
    pub file_sources: Vec<FileSourceBinding>,
    pub exports: Vec<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCoverage {
    pub complete: bool,
    pub gaps: Vec<String>,
    pub deferred_details: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicManifest {
    pub schema_version: String,
    pub review_id: String,
    pub public_report_id: String,
    pub analysis_review_id: String,
    pub repository: PublicRepository,
    pub pr_number: u64,
    pub selected_baseline: String,
    pub baseline: String,
    pub proposal: String,
    pub scope_version: String,
    pub policy_digest: String,
    pub engine: Value,
    pub files: Vec<PublicFile>,
    pub routes: PublicRoutes,
    pub coverage: PublicCoverage,
    pub capabilities: Value,
}
#[derive(Clone, Debug)]
pub struct PublicBundle {
    pub manifest: PublicManifest,
    /// Keys are generated relative paths, values are inert UTF-8 JSON/Markdown.
    pub files: BTreeMap<String, Vec<u8>>,
}
impl PublicManifest {
    pub fn identity(&self) -> Result<String, String> {
        let mut value = serde_json::to_value(self).map_err(|_| "Invalid public manifest")?;
        let object = value.as_object_mut().ok_or("Invalid public manifest")?;
        object.remove("review_id");
        object.remove("public_report_id");
        Ok(sha256(&canonical_json(&value)?))
    }
}
impl PublicBundle {
    pub fn validate(&self) -> Result<(), String> {
        let m = &self.manifest;
        if m.schema_version != PUBLIC_SCHEMA
            || m.public_report_id != m.identity()?
            || m.review_id != m.public_report_id
            || m.files.len() != self.files.len()
            || self.files.len() >= MAX_PUBLIC_FILES
            || self
                .files
                .values()
                .map(Vec::len)
                .sum::<usize>()
                .saturating_add(canonical_json(m)?.len())
                > MAX_PUBLIC_BYTES
        {
            return Err("Public bundle identity or resource limit invalid".into());
        }
        if m.repository.id == 0
            || m.pr_number == 0
            || ![&m.selected_baseline, &m.baseline, &m.proposal]
                .iter()
                .all(|v| policy::oid(v))
            || [
                m.analysis_review_id.as_str(),
                m.policy_digest.as_str(),
                m.public_report_id.as_str(),
            ]
            .iter()
            .any(|v| {
                v.len() != 64
                    || !v
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err("Invalid public manifest metadata".into());
        }
        let mut previous = None;
        for file in &m.files {
            policy::safe_path(&file.path)?;
            if previous.is_some_and(|p: &str| p >= file.path.as_str()) {
                return Err("Public file table must be sorted and unique".into());
            }
            previous = Some(&file.path);
            let bytes = self.files.get(&file.path).ok_or("Missing public file")?;
            if bytes.len() != file.bytes || sha256(bytes) != file.sha256 {
                return Err("Public file hash/length mismatch".into());
            }
        }
        for path in std::iter::once(&m.routes.artifact)
            .chain(m.routes.hld.iter())
            .chain(m.routes.lld_catalog.iter())
            .chain(m.routes.lld_details.iter().map(|r| &r.path))
            .chain(m.routes.sources.iter().map(|r| &r.path))
            .chain(m.routes.file_sources.iter().map(|r| &r.path))
        {
            if !self.files.contains_key(path) {
                return Err("Public route is not allowlisted".into());
            }
        }
        validate::routes(self)?;
        Ok(())
    }
}

/// Consumes bounded private JSON but emits only the separate public projection.
pub fn export_public_report(
    artifact: &Value,
    policy: &PublicPolicy,
    expected: &ExpectedComparison,
) -> Result<PublicBundle, String> {
    project::export(artifact, policy, expected)
}
