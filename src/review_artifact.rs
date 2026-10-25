//! Versioned machine artifact; never scrape rendered HTML to obtain review evidence.
use crate::AnalysisResult;
use serde::Serialize;
use std::collections::BTreeSet;
#[derive(Serialize)]
pub struct ReviewArtifact {
    pub artifact_version: &'static str,
    pub review_id: String,
    pub selected_baseline: String,
    pub baseline: String,
    pub proposal: String,
    pub scope_version: String,
    pub scope_manifest: serde_json::Value,
    pub files: Vec<ArtifactFile>,
    pub architecture: ArtifactArchitecture,
    pub warnings: Vec<String>,
    pub coverage: ArtifactCoverage,
    pub hld_status: &'static str,
    pub lld_status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onboarding_context: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hld: Option<ArtifactHld>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lld: Option<crate::lld::LldReview>,
}
#[derive(Serialize)]
pub struct ArtifactHld {
    pub baseline: crate::design::DesignVersion,
    pub proposal: crate::design::DesignVersion,
    pub delta: crate::design::DesignDelta,
    pub sources: Vec<ArtifactDesignSource>,
    pub source_gaps: Vec<String>,
}
#[derive(Serialize)]
pub struct ArtifactDesignSource {
    pub evidence_id: String,
    pub commit: String,
    pub path: String,
    pub blob_id: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    pub truncated: bool,
}
#[derive(Serialize)]
pub struct ArtifactFile {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: String,
    pub additions: usize,
    pub deletions: usize,
}
#[derive(Serialize)]
pub struct ArtifactArchitecture {
    pub nodes: Vec<ArtifactNode>,
    pub edges: Vec<ArtifactEdge>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactNode {
    pub path: String,
    pub status: String,
    pub blob_before: Option<String>,
    pub blob_after: Option<String>,
    pub definitions_before: Vec<String>,
    pub definitions_after: Vec<String>,
    pub calls_before: Vec<String>,
    pub calls_after: Vec<String>,
    pub source_before: Option<String>,
    pub source_after: Option<String>,
    pub source_before_truncated: bool,
    pub source_after_truncated: bool,
}
#[derive(Serialize)]
pub struct ArtifactEdge {
    pub from: String,
    pub to: String,
    pub name: String,
    pub status: String,
    pub resolution: &'static str,
}
#[derive(Serialize)]
pub struct ArtifactCoverage {
    pub indexed: usize,
    pub unsupported: usize,
    pub excluded: usize,
    pub failed: usize,
    pub deferred: usize,
    pub enumeration_complete: bool,
}
impl ReviewArtifact {
    pub fn from_analysis(result: &AnalysisResult) -> Self {
        let review = &result.review;
        let architecture = &review.architecture;
        let paths: BTreeSet<_> = architecture
            .before
            .iter()
            .chain(&architecture.after)
            .map(|node| &node.path)
            .collect();
        let nodes = paths
            .into_iter()
            .map(|path| {
                let before = architecture.before.iter().find(|node| &node.path == path);
                let after = architecture.after.iter().find(|node| &node.path == path);
                let status = match (before, after) {
                    (None, _) => "added",
                    (_, None) => "removed",
                    (Some(b), Some(a)) if b.oid != a.oid => "modified",
                    _ => "unchanged",
                };
                let file = review.files.iter().find(|file| &file.path == path);
                let source_before = file
                    .filter(|_| before.is_some())
                    .map(|file| file.before.source.chars().take(16000).collect());
                let source_after = file
                    .filter(|_| after.is_some())
                    .map(|file| file.after.source.chars().take(16000).collect());
                ArtifactNode {
                    path: path.clone(),
                    status: status.into(),
                    blob_before: before.map(|node| node.oid.clone()),
                    blob_after: after.map(|node| node.oid.clone()),
                    definitions_before: before
                        .map(|node| node.definitions.iter().cloned().collect())
                        .unwrap_or_default(),
                    definitions_after: after
                        .map(|node| node.definitions.iter().cloned().collect())
                        .unwrap_or_default(),
                    calls_before: before
                        .map(|node| node.calls.iter().cloned().collect())
                        .unwrap_or_default(),
                    calls_after: after
                        .map(|node| node.calls.iter().cloned().collect())
                        .unwrap_or_default(),
                    source_before,
                    source_after,
                    source_before_truncated: file
                        .is_some_and(|file| file.before.source.chars().count() > 16000),
                    source_after_truncated: file
                        .is_some_and(|file| file.after.source.chars().count() > 16000),
                }
            })
            .collect();
        Self {
            artifact_version: "csb-review-artifact-v1",
            review_id: result.review_id.as_str().into(),
            selected_baseline: result.selected_baseline.as_str().into(),
            baseline: result.baseline.as_str().into(),
            proposal: result.proposal.as_str().into(),
            scope_version: result.scope_version.as_str().into(),
            scope_manifest: serde_json::json!({"primary_repository":{"security_domain":result.scope().primary_repository.security_domain().as_str(),"provider":result.scope().primary_repository.provider(),"owner":result.scope().primary_repository.owner(),"name":result.scope().primary_repository.name()},
                "include":result.scope().primary_paths.includes().collect::<Vec<_>>(),"exclude":result.scope().primary_paths.excludes().collect::<Vec<_>>(),"export_allowed":result.scope().export_allowed,
                "contexts":result.scope().contexts.iter().map(|pin|serde_json::json!({"name":pin.name,"commit":pin.commit.as_str(),"repository":{"security_domain":pin.repository.security_domain().as_str(),"provider":pin.repository.provider(),"owner":pin.repository.owner(),"name":pin.repository.name()},"include":pin.paths.includes().collect::<Vec<_>>(),"exclude":pin.paths.excludes().collect::<Vec<_>>()})).collect::<Vec<_>>() }),
            files: review
                .files
                .iter()
                .map(|file| ArtifactFile {
                    path: file.path.clone(),
                    old_path: None,
                    kind: match file.status.as_str() {
                        "A" => "added",
                        "D" => "deleted",
                        "R" => "renamed",
                        _ => "modified",
                    }
                    .into(),
                    additions: file.additions,
                    deletions: file.deletions,
                })
                .collect(),
            architecture: ArtifactArchitecture {
                nodes,
                edges: architecture
                    .edges
                    .iter()
                    .map(|(from, to, name, status)| ArtifactEdge {
                        from: from.clone(),
                        to: to.clone(),
                        name: name.clone(),
                        status: (*status).into(),
                        resolution: "name_only_candidate",
                    })
                    .collect(),
            },
            warnings: review
                .warnings
                .iter()
                .chain(&architecture.notes)
                .cloned()
                .collect(),
            coverage: ArtifactCoverage {
                indexed: result.coverage.indexed,
                unsupported: result.coverage.unsupported,
                excluded: result.coverage.excluded,
                failed: result.coverage.failed,
                deferred: result.coverage.deferred,
                enumeration_complete: result.coverage.enumeration_complete,
            },
            hld_status: "unavailable",
            lld_status: "unavailable",
            onboarding_context: None,
            hld: None,
            lld: None,
        }
    }
    pub fn generate_hld(
        &mut self,
        result: &AnalysisResult,
        repository: &std::path::Path,
        confirmed: &serde_json::Value,
        grants: &[crate::Grant],
        cancellation: &crate::Cancellation,
    ) -> Result<(), String> {
        use crate::design::{ConfirmedDesignContext, DesignOutcome, compare_hld, generate_hld};
        result.authorize(crate::Action::Graph, grants, None)?;
        result.authorize(crate::Action::Source, grants, None)?;
        let context = ConfirmedDesignContext::from_accepted(confirmed, result.scope())?;
        let baseline = generate_hld(
            repository,
            &result.baseline,
            result.scope(),
            &context,
            cancellation,
        )?;
        let proposal = generate_hld(
            repository,
            &result.proposal,
            result.scope(),
            &context,
            cancellation,
        )?;
        let delta = compare_hld(&baseline, &proposal)?;
        self.hld_status = match delta.outcome {
            DesignOutcome::ChangeDetected => "change_detected",
            DesignOutcome::NoChangeDetectedWithinScope => "no_change_detected_within_scope",
            DesignOutcome::Incomplete => "incomplete",
            DesignOutcome::Unavailable => "unavailable",
        };
        let (sources, source_gaps) =
            design_sources(repository, &baseline, &proposal, cancellation)?;
        self.hld = Some(ArtifactHld {
            baseline,
            proposal,
            delta,
            sources,
            source_gaps,
        });
        Ok(())
    }
    /// Retain bounded exact syntax inputs before an isolated mirror is released.
    /// The host never serves this private input bundle as a catalog response.
    pub fn generate_lld(
        &mut self,
        result: &AnalysisResult,
        repository: &std::path::Path,
        confirmed: &serde_json::Value,
        grants: &[crate::Grant],
        cancellation: &crate::Cancellation,
    ) -> Result<(), String> {
        result.authorize(crate::Action::Graph, grants, None)?;
        result.authorize(crate::Action::Source, grants, None)?;
        let context =
            crate::design::ConfirmedDesignContext::from_accepted(confirmed, result.scope())?;
        let hld = self
            .hld
            .as_ref()
            .ok_or("LLD requires source-backed HLD parents")?;
        let changed_paths = self
            .files
            .iter()
            .flat_map(|file| std::iter::once(file.path.clone()).chain(file.old_path.clone()))
            .collect::<BTreeSet<_>>();
        let baseline = crate::lld::generate_lld_prioritized(
            repository,
            &result.baseline,
            result.scope(),
            &context,
            &hld.baseline,
            cancellation,
            &changed_paths,
        )?;
        let proposal = crate::lld::generate_lld_prioritized(
            repository,
            &result.proposal,
            result.scope(),
            &context,
            &hld.proposal,
            cancellation,
            &changed_paths,
        )?;
        let delta = crate::lld::compare_lld(&baseline.catalog, &proposal.catalog);
        self.lld_status = match delta.outcome.as_str() {
            "change_detected" => "change_detected",
            "no_change_detected_within_scope" => "no_change_detected_within_scope",
            "incomplete" => "incomplete",
            _ => "unavailable",
        };
        self.lld = Some(crate::lld::LldReview {
            baseline,
            proposal,
            delta,
        });
        if serde_json::to_vec(self)
            .map_err(|_| "Cannot encode bounded LLD artifact")?
            .len()
            > 16 * 1024 * 1024
        {
            self.lld = None;
            self.lld_status = "unavailable";
            self.warnings.push("LLD syntax input exceeds the comparison artifact budget; ordinary scoped source/HLD review is retained and LLD is unavailable".into());
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|_| "Cannot encode review artifact")?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("Review artifact exceeds 16 MiB".into());
        }
        Ok(bytes)
    }
}

/// Retain only bounded, authorized evidence spans while the isolated mirror is
/// available. Unchanged files need this too; no later worktree fallback is used.
fn design_sources(
    repository: &std::path::Path,
    baseline: &crate::design::DesignVersion,
    proposal: &crate::design::DesignVersion,
    cancellation: &crate::Cancellation,
) -> Result<(Vec<ArtifactDesignSource>, Vec<String>), String> {
    use std::collections::BTreeMap;
    let mut blobs = BTreeMap::new();
    let ids = baseline
        .evidence
        .iter()
        .chain(&proposal.evidence)
        .map(|item| item.blob.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut total_read = 0usize;
    let mut gaps = BTreeSet::new();
    for chunk in ids.chunks(128) {
        if cancellation.is_cancelled() {
            return Err("HLD evidence read cancelled".into());
        }
        if total_read >= 8 * 1024 * 1024 {
            gaps.insert("HLD evidence source retention reached its 8 MiB read budget".into());
            break;
        }
        let values = crate::snapshot::read_chunk(
            repository,
            chunk,
            crate::snapshot::ReadLimits {
                objects_per_chunk: 128,
                bytes_per_object: 512 * 1024,
                bytes_per_chunk: 8 * 1024 * 1024 - total_read,
            },
            cancellation,
        );
        let values = match values {
            Ok(values) => values,
            Err(_) if cancellation.is_cancelled() => {
                return Err("HLD evidence read cancelled".into());
            }
            Err(_) => {
                gaps.insert("Some HLD evidence sources could not be retained within the bounded snapshot reader".into());
                break;
            }
        };
        for (id, value) in chunk.iter().zip(values) {
            if let crate::git::BatchBlob::Data(bytes) = value {
                total_read += bytes.len();
                if let Ok(text) = String::from_utf8(bytes) {
                    blobs.insert(id.clone(), text);
                }
            }
        }
    }
    let mut sources = Vec::new();
    let mut retained = 0usize;
    let mut seen = BTreeSet::new();
    for evidence in baseline.evidence.iter().chain(&proposal.evidence) {
        if !seen.insert(evidence.id.clone()) {
            continue;
        }
        if cancellation.is_cancelled() {
            return Err("HLD evidence read cancelled".into());
        }
        let Some(text) = blobs.get(&evidence.blob) else {
            gaps.insert("Some exact HLD evidence sources are missing, oversized, non-text or outside the read budget".into());
            continue;
        };
        let lines = text.lines().collect::<Vec<_>>();
        let end = evidence
            .end_line
            .min(evidence.start_line.saturating_add(199))
            .min(lines.len());
        if end < evidence.start_line {
            gaps.insert("Some HLD evidence spans are unavailable at the pinned blob".into());
            continue;
        }
        let mut source = lines[evidence.start_line - 1..end].join("\n");
        let truncated = end < evidence.end_line || source.len() > 64 * 1024;
        if source.len() > 64 * 1024 {
            let mut boundary = 64 * 1024;
            while !source.is_char_boundary(boundary) {
                boundary -= 1;
            }
            source.truncate(boundary);
        }
        if retained.saturating_add(source.len()) > 2 * 1024 * 1024 {
            gaps.insert("HLD evidence source retention reached its 2 MiB output budget".into());
            continue;
        }
        retained += source.len();
        sources.push(ArtifactDesignSource {
            evidence_id: evidence.id.clone(),
            commit: evidence.commit.as_str().into(),
            path: evidence.path.clone(),
            blob_id: evidence.blob.clone(),
            start_line: evidence.start_line,
            end_line: evidence.start_line + source.lines().count().saturating_sub(1),
            text: source,
            truncated,
        });
    }
    Ok((sources, gaps.into_iter().collect()))
}
