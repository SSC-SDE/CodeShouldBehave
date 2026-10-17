//! Credential-free check drafts. Hosted publication must separately recheck App
//! installation access, current PR commits and context grants before each write.
use crate::{RepositoryId, ReviewArtifact};
use serde::Serialize;

pub const ARCHITECTURE_CHECK_NAME: &str = "CodeShouldBehave / Architecture";

/// Fresh provider observation, not values obtained from the analysis artifact.
pub struct CheckTarget<'a> {
    pub repository: &'a RepositoryId,
    pub pr_number: u64,
    pub selected_baseline: &'a str,
    pub head: &'a str,
    /// Trusted operator configuration, never repository text or webhook content.
    pub dashboard_origin: &'a str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArchitectureCheckDraft {
    pub name: &'static str,
    pub head_sha: String,
    pub external_id: String,
    pub details_url: String,
    pub status: &'static str,
    pub conclusion: &'static str,
    pub output: CheckOutput,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CheckOutput {
    pub title: &'static str,
    pub summary: String,
}

fn hex_id(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn dashboard_origin(value: &str) -> Result<&str, String> {
    let authority = value
        .strip_prefix("https://")
        .ok_or("Dashboard origin must use HTTPS")?;
    let hostname = authority.strip_suffix(":443").unwrap_or(authority);
    if hostname.len() > 253
        || !hostname.contains('.')
        || hostname.parse::<std::net::IpAddr>().is_ok()
        || hostname.ends_with(".localhost")
        || hostname.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(
            "Dashboard origin must be an HTTPS DNS origin without paths or credentials".into(),
        );
    }
    Ok(value)
}

fn outcome(value: &str) -> Result<&str, String> {
    match value {
        "change_detected" | "no_change_detected_within_scope" | "incomplete" | "unavailable" => {
            Ok(value)
        }
        _ => Err("Invalid architecture capability outcome".into()),
    }
}

impl ReviewArtifact {
    /// Produces a minimal, source-free completed-check payload; makes no request.
    /// Existing structural artifacts correctly produce `neutral`, not `success`.
    pub fn architecture_check_draft(
        &self,
        target: &CheckTarget<'_>,
    ) -> Result<ArchitectureCheckDraft, String> {
        if target.repository.provider() != "github"
            || target.pr_number == 0
            || target.pr_number > i64::MAX as u64
            || self.artifact_version != "csb-review-artifact-v1"
            || !hex_id(&self.review_id, 64)
            || !hex_id(&self.selected_baseline, 40)
            || !hex_id(&self.baseline, 40)
            || !hex_id(&self.proposal, 40)
            || self.scope_version.is_empty()
        {
            return Err("Invalid GitHub check artifact identity".into());
        }
        let repository = &self.scope_manifest["primary_repository"];
        for (key, expected) in [
            (
                "security_domain",
                target.repository.security_domain().as_str(),
            ),
            ("provider", target.repository.provider()),
            ("owner", target.repository.owner()),
            ("name", target.repository.name()),
        ] {
            if repository[key].as_str() != Some(expected) {
                return Err("Check target does not match artifact repository".into());
            }
        }
        if self.selected_baseline != target.selected_baseline || self.proposal != target.head {
            return Err("Superseded comparison; refuse check publication draft".into());
        }
        let origin = dashboard_origin(target.dashboard_origin)?;
        let hld = outcome(self.hld_status)?;
        let lld = outcome(self.lld_status)?;
        let complete =
            |value: &str| matches!(value, "change_detected" | "no_change_detected_within_scope");
        // A positive structural delta can coexist with unsupported or still
        // lazy behavior. Status labels and the older path index cannot upgrade
        // those gaps into a successful completed architecture check.
        let design_covered =
            self.hld.as_ref().is_some_and(|pair| {
                pair.baseline.coverage.complete && pair.proposal.coverage.complete
            }) && self.lld.as_ref().is_some_and(|pair| {
                [
                    &pair.baseline.catalog.coverage,
                    &pair.proposal.catalog.coverage,
                ]
                .iter()
                .all(|coverage| {
                    coverage.inventory_complete
                        && coverage.structure_complete
                        && coverage.unsupported == 0
                        && coverage.failed == 0
                        && coverage.deferred == 0
                        && coverage.local_flow == "complete_supported_profile"
                })
            });
        let covered = self.coverage.enumeration_complete
            && self.coverage.unsupported == 0
            && self.coverage.failed == 0
            && self.coverage.deferred == 0
            && self.warnings.is_empty()
            && design_covered;
        let ready = complete(hld) && complete(lld) && covered;
        Ok(ArchitectureCheckDraft {
            name: ARCHITECTURE_CHECK_NAME,
            head_sha: self.proposal.clone(),
            external_id: format!(
                "csb-architecture-v1:pr-{}:{}",
                target.pr_number, self.review_id
            ),
            details_url: format!(
                "{origin}/workspace#review=pr-{}-{}&layout=architecture&side=diff{}",
                target.pr_number,
                self.review_id,
                if self.hld.is_some() {
                    "&design=hld"
                } else {
                    ""
                }
            ),
            status: "completed",
            conclusion: if ready { "success" } else { "neutral" },
            output: CheckOutput {
                title: if ready {
                    "Architecture analysis completed"
                } else {
                    "Architecture analysis is incomplete or unavailable"
                },
                // Fixed vocabulary only: no repo labels, source paths, excerpts,
                // counts, warning text, related repos or model explanations.
                summary: format!(
                    "HLD: {hld}. LLD: {lld}. Coverage: {}. Open the authenticated dashboard for permitted evidence. This is analysis status, not merge approval.",
                    if covered {
                        "complete within scope"
                    } else {
                        "partial"
                    }
                ),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecurityDomain;
    use crate::review_artifact::{ArtifactArchitecture, ArtifactCoverage};
    use serde_json::json;

    fn fixture() -> (RepositoryId, ReviewArtifact) {
        let repository = RepositoryId::new(
            SecurityDomain::new("test").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let artifact = ReviewArtifact {
            artifact_version: "csb-review-artifact-v1",
            review_id: "c".repeat(64),
            selected_baseline: "a".repeat(40),
            baseline: "d".repeat(40),
            proposal: "b".repeat(40),
            scope_version: "scope-1".into(),
            scope_manifest: json!({"primary_repository":{"security_domain":"test","provider":"github","owner":"owner","name":"repo"},"contexts":[{"name":"PRIVATE_SENTINEL"}]}),
            files: vec![],
            architecture: ArtifactArchitecture {
                nodes: vec![],
                edges: vec![],
            },
            warnings: vec![],
            coverage: ArtifactCoverage {
                indexed: 1,
                unsupported: 0,
                excluded: 9,
                failed: 0,
                deferred: 0,
                enumeration_complete: true,
            },
            hld_status: "unavailable",
            lld_status: "unavailable",
            onboarding_context: None,
            hld: None,
            lld: None,
        };
        (repository, artifact)
    }
    fn target(repository: &RepositoryId) -> CheckTarget<'_> {
        CheckTarget {
            repository,
            pr_number: 35,
            selected_baseline: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            head: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            dashboard_origin: "https://csb.example.com",
        }
    }

    #[test]
    fn path_diagrams_are_neutral_and_link_to_the_exact_architecture_view() {
        let (repository, artifact) = fixture();
        let draft = artifact
            .architecture_check_draft(&target(&repository))
            .unwrap();
        assert_eq!(draft.conclusion, "neutral");
        assert_eq!(draft.head_sha, artifact.proposal);
        assert_eq!(
            draft.details_url,
            format!(
                "https://csb.example.com/workspace#review=pr-35-{}&layout=architecture&side=diff",
                "c".repeat(64)
            )
        );
        assert!(
            !serde_json::to_string(&draft)
                .unwrap()
                .contains("PRIVATE_SENTINEL")
        );
    }

    #[test]
    fn completed_status_labels_cannot_bypass_missing_or_lazy_design_coverage() {
        let (repository, mut artifact) = fixture();
        artifact.hld_status = "change_detected";
        artifact.lld_status = "no_change_detected_within_scope";
        assert_eq!(
            artifact
                .architecture_check_draft(&target(&repository))
                .unwrap()
                .conclusion,
            "neutral"
        );
        for (hld, lld) in [
            ("incomplete", "change_detected"),
            ("change_detected", "unavailable"),
        ] {
            artifact.hld_status = hld;
            artifact.lld_status = lld;
            assert_eq!(
                artifact
                    .architecture_check_draft(&target(&repository))
                    .unwrap()
                    .conclusion,
                "neutral"
            );
        }
        artifact.hld_status = "change_detected";
        artifact.lld_status = "change_detected";
        artifact.coverage.enumeration_complete = false;
        assert_eq!(
            artifact
                .architecture_check_draft(&target(&repository))
                .unwrap()
                .conclusion,
            "neutral"
        );
        artifact.coverage.enumeration_complete = true;
        artifact
            .warnings
            .push("SECRET_SENTINEL src/private.rs https://private.invalid".into());
        let draft = artifact
            .architecture_check_draft(&target(&repository))
            .unwrap();
        assert_eq!(draft.conclusion, "neutral");
        assert!(
            !serde_json::to_string(&draft)
                .unwrap()
                .contains("SECRET_SENTINEL")
        );
        artifact.warnings.clear();
        for gap in ["unsupported", "failed", "deferred"] {
            artifact.coverage.unsupported = usize::from(gap == "unsupported");
            artifact.coverage.failed = usize::from(gap == "failed");
            artifact.coverage.deferred = usize::from(gap == "deferred");
            assert_eq!(
                artifact
                    .architecture_check_draft(&target(&repository))
                    .unwrap()
                    .conclusion,
                "neutral"
            );
        }
        artifact.hld_status = "ready";
        assert!(
            artifact
                .architecture_check_draft(&target(&repository))
                .is_err()
        );
    }

    #[test]
    fn stale_heads_baselines_wrong_repositories_and_unsafe_origins_are_denied() {
        let (repository, artifact) = fixture();
        let mut request = target(&repository);
        request.head = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        assert!(artifact.architecture_check_draft(&request).is_err());
        request = target(&repository);
        request.selected_baseline = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        assert!(artifact.architecture_check_draft(&request).is_err());
        let wrong = RepositoryId::new(
            SecurityDomain::new("other").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        assert!(artifact.architecture_check_draft(&target(&wrong)).is_err());
        for origin in [
            "http://localhost:3195",
            "https://127.0.0.1",
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com?token=secret",
            "https://example.com#fragment",
            "https://example.com:3195",
            "https://foo.localhost",
            "https://example.com\n",
        ] {
            request = target(&repository);
            request.dashboard_origin = origin;
            assert!(
                artifact.architecture_check_draft(&request).is_err(),
                "{origin}"
            );
        }
        request = target(&repository);
        request.pr_number = 0;
        assert!(artifact.architecture_check_draft(&request).is_err());
        request = target(&repository);
        request.dashboard_origin = "https://csb.example.com:443";
        assert!(artifact.architecture_check_draft(&request).is_ok());
    }
}
