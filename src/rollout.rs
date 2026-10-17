use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSwitch {
    ProviderIngestion,
    IncrementalAnalysis,
    AiEnrichment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RolloutStage {
    ObservationOnly,
    Cohort,
    Draining,
    RolledBack,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObservationWindow {
    pub completed_reviews: u64,
    pub stale_publications: u64,
    pub scope_leaks: u64,
    pub silent_incomplete_results: u64,
}

impl ObservationWindow {
    fn permits_expansion(self, minimum_reviews: u64) -> bool {
        self.completed_reviews >= minimum_reviews
            && self.stale_publications == 0
            && self.scope_leaks == 0
            && self.silent_incomplete_results == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReviewerSignals {
    pub reviewed_findings: u64,
    pub actionable_findings: u64,
    pub rejected_edges: u64,
    pub ambiguous_edges: u64,
}

pub struct RolloutControls {
    stage: RolloutStage,
    provider_ingestion: bool,
    incremental_analysis: bool,
    ai_enrichment: bool,
    authorized_repositories: BTreeSet<String>,
    pilot_reviewers: BTreeSet<String>,
    immutable_review_history: BTreeSet<String>,
    signals: ReviewerSignals,
}

impl RolloutControls {
    pub fn observation_only(repository: &str) -> Result<Self, String> {
        validate_identity("repository", repository)?;
        Ok(Self {
            stage: RolloutStage::ObservationOnly,
            provider_ingestion: true,
            incremental_analysis: true,
            ai_enrichment: false,
            authorized_repositories: BTreeSet::from([repository.into()]),
            pilot_reviewers: BTreeSet::new(),
            immutable_review_history: BTreeSet::new(),
            signals: ReviewerSignals::default(),
        })
    }

    pub fn stage(&self) -> RolloutStage {
        self.stage
    }

    pub fn switch_enabled(&self, feature: FeatureSwitch) -> bool {
        match feature {
            FeatureSwitch::ProviderIngestion => self.provider_ingestion,
            FeatureSwitch::IncrementalAnalysis => self.incremental_analysis,
            FeatureSwitch::AiEnrichment => self.ai_enrichment,
        }
    }

    pub fn set_switch(&mut self, feature: FeatureSwitch, enabled: bool) -> Result<(), String> {
        if feature == FeatureSwitch::AiEnrichment && enabled {
            return Err("Live AI remains disabled until the Phase 7 gate".into());
        }
        match feature {
            FeatureSwitch::ProviderIngestion => self.provider_ingestion = enabled,
            FeatureSwitch::IncrementalAnalysis => self.incremental_analysis = enabled,
            FeatureSwitch::AiEnrichment => self.ai_enrichment = false,
        }
        Ok(())
    }

    pub fn record_review(&mut self, review_id: &str) -> Result<(), String> {
        validate_identity("review", review_id)?;
        self.immutable_review_history.insert(review_id.into());
        Ok(())
    }

    pub fn review_history(&self) -> &BTreeSet<String> {
        &self.immutable_review_history
    }

    pub fn expand_cohort(
        &mut self,
        repositories: impl IntoIterator<Item = String>,
        reviewers: impl IntoIterator<Item = String>,
        window: ObservationWindow,
        minimum_reviews: u64,
    ) -> Result<(), String> {
        if self.stage != RolloutStage::ObservationOnly {
            return Err("Cohort expansion requires observation-only stage".into());
        }
        if minimum_reviews == 0 || !window.permits_expansion(minimum_reviews) {
            return Err("Observation window does not satisfy expansion gates".into());
        }
        let repositories = repositories
            .into_iter()
            .map(|value| validate_owned("repository", value))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let reviewers = reviewers
            .into_iter()
            .map(|value| validate_owned("reviewer", value))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if repositories.is_empty() || repositories.len() > 10 || reviewers.is_empty() {
            return Err(
                "Pilot cohort must contain 1–10 repositories and at least one reviewer".into(),
            );
        }
        self.authorized_repositories.extend(repositories);
        self.pilot_reviewers = reviewers;
        self.stage = RolloutStage::Cohort;
        Ok(())
    }

    pub fn record_reviewer_signals(&mut self, signals: ReviewerSignals) -> Result<(), String> {
        if signals.actionable_findings > signals.reviewed_findings {
            return Err("Actionable findings cannot exceed reviewed findings".into());
        }
        self.signals = signals;
        Ok(())
    }

    pub fn reviewer_signals(&self) -> ReviewerSignals {
        self.signals
    }

    pub fn begin_rollback(&mut self) {
        self.provider_ingestion = false;
        self.stage = RolloutStage::Draining;
    }

    pub fn finish_rollback(&mut self, diagnostics_preserved: bool) -> Result<(), String> {
        if self.stage != RolloutStage::Draining || !diagnostics_preserved {
            return Err("Rollback requires drained work and preserved diagnostics".into());
        }
        self.incremental_analysis = false;
        self.stage = RolloutStage::RolledBack;
        Ok(())
    }
}

fn validate_identity(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(format!("Invalid {kind} identity"))
    } else {
        Ok(())
    }
}

fn validate_owned(kind: &str, value: String) -> Result<String, String> {
    validate_identity(kind, &value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_expansion_requires_a_clean_window_and_ai_stays_disabled() {
        let mut rollout = RolloutControls::observation_only("synthetic/repository-1").unwrap();
        assert_eq!(rollout.stage(), RolloutStage::ObservationOnly);
        assert!(rollout.switch_enabled(FeatureSwitch::ProviderIngestion));
        assert!(!rollout.switch_enabled(FeatureSwitch::AiEnrichment));
        assert!(
            rollout
                .set_switch(FeatureSwitch::AiEnrichment, true)
                .is_err()
        );
        assert!(
            rollout
                .expand_cohort(
                    ["synthetic/repository-2".into()],
                    ["pilot-reviewer".into()],
                    ObservationWindow {
                        completed_reviews: 20,
                        stale_publications: 1,
                        ..ObservationWindow::default()
                    },
                    20,
                )
                .is_err()
        );
        rollout
            .expand_cohort(
                ["synthetic/repository-2".into()],
                ["pilot-reviewer".into()],
                ObservationWindow {
                    completed_reviews: 20,
                    ..ObservationWindow::default()
                },
                20,
            )
            .unwrap();
        assert_eq!(rollout.stage(), RolloutStage::Cohort);
    }

    #[test]
    fn switches_and_rollback_preserve_immutable_review_history() {
        let mut rollout = RolloutControls::observation_only("synthetic/repository-1").unwrap();
        rollout.record_review("review-1").unwrap();
        rollout.record_review("review-2").unwrap();
        rollout
            .set_switch(FeatureSwitch::ProviderIngestion, false)
            .unwrap();
        rollout
            .set_switch(FeatureSwitch::IncrementalAnalysis, false)
            .unwrap();
        assert_eq!(rollout.review_history().len(), 2);
        rollout.begin_rollback();
        assert_eq!(rollout.stage(), RolloutStage::Draining);
        assert!(rollout.finish_rollback(false).is_err());
        rollout.finish_rollback(true).unwrap();
        assert_eq!(rollout.stage(), RolloutStage::RolledBack);
        assert_eq!(rollout.review_history().len(), 2);
    }

    #[test]
    fn reviewer_signals_are_bounded_and_explicit() {
        let mut rollout = RolloutControls::observation_only("synthetic/repository-1").unwrap();
        assert!(
            rollout
                .record_reviewer_signals(ReviewerSignals {
                    reviewed_findings: 2,
                    actionable_findings: 3,
                    rejected_edges: 0,
                    ambiguous_edges: 0,
                })
                .is_err()
        );
        let signals = ReviewerSignals {
            reviewed_findings: 20,
            actionable_findings: 12,
            rejected_edges: 3,
            ambiguous_edges: 4,
        };
        rollout.record_reviewer_signals(signals).unwrap();
        assert_eq!(rollout.reviewer_signals(), signals);
    }
}
