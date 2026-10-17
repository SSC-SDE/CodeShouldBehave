use super::{canonical_json, sha256};
use crate::{PathScope, sensitive_path};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicRepository {
    pub id: u64,
    pub owner: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicPolicy {
    pub schema_version: String,
    pub repository: PublicRepository,
    pub public: bool,
    pub publication_consent: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub baseline_mode: String,
    pub max_details: usize,
}
#[derive(Clone, Debug)]
pub struct ExpectedComparison {
    pub repository: PublicRepository,
    pub pr_number: u64,
    pub selected_baseline: String,
    pub baseline: String,
    pub proposal: String,
}
pub(super) fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn safe_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.contains(['\\', ':', '%', '?', '#'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".." | ".git"))
    {
        return Err("Unsafe public relative path".into());
    }
    Ok(())
}
impl PublicPolicy {
    pub fn validate(&self) -> Result<PathScope, String> {
        let segment = |s: &str| {
            !s.is_empty()
                && s.len() <= 100
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && s != "."
                && s != ".."
        };
        if self.schema_version != "csb-public-policy-v1"
            || !self.public
            || !self.publication_consent
            || self.repository.id == 0
            || !segment(&self.repository.owner)
            || !segment(&self.repository.name)
            || !matches!(self.baseline_mode.as_str(), "exact" | "merge_base")
            || self.max_details > 100
            || self.include.len() > 100
            || self.exclude.len() > 100
        {
            return Err("Explicit bounded public policy/consent required".into());
        }
        for path in self.include.iter().chain(&self.exclude) {
            if path != "." {
                safe_path(path)?;
            }
        }
        let scope = PathScope::new(self.include.clone(), self.exclude.clone())?;
        if scope.includes().count() != self.include.len()
            || scope.excludes().count() != self.exclude.len()
        {
            return Err("Duplicate public policy path".into());
        }
        Ok(scope)
    }
    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.include.sort();
        normalized.exclude.sort();
        Ok(sha256(&canonical_json(&normalized)?))
    }
    pub(super) fn allows(&self, scope: &PathScope, path: &str) -> Result<(), String> {
        safe_path(path)?;
        if !scope.allows(path) || sensitive_path(path) {
            return Err("Public evidence outside approved non-sensitive paths".into());
        }
        Ok(())
    }
}
