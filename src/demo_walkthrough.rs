//! Demo-only walkthrough planning surface.
//!
//! This module exists to demonstrate how a pull request changes a system. It
//! maps a review request to a walkthrough stage, an ordered set of steps, and
//! an explicit residual-risk statement. It is intentionally isolated from the
//! review pipeline and has no product behavior.

/// The deepest stage a walkthrough reaches for a review request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkthroughStage {
    /// The request has no observed contract change.
    Map,
    /// The request changes contracts without asking for exact source.
    Behavior,
    /// The request asks to pin exact source.
    Source,
}

/// A request to plan a walkthrough for one review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkthroughRequest {
    /// Number of observed contract changes in the review.
    pub changed_contracts: usize,
    /// Number of findings the review leaves uncertain.
    pub uncertain_findings: usize,
    /// Whether the walkthrough must pin exact source.
    pub include_source: bool,
}

/// A planned walkthrough for one review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkthroughPlan {
    /// The deepest stage the plan reaches.
    pub stage: WalkthroughStage,
    /// Ordered steps to show during the walkthrough.
    pub steps: Vec<&'static str>,
    /// What the walkthrough still cannot settle.
    pub residual_risk: &'static str,
}

/// Why a walkthrough cannot be planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkthroughError {
    /// The review has no observed contract change to walk through.
    EmptyChange,
    /// The review leaves more uncertainty than a walkthrough can present.
    TooManyUncertainFindings {
        /// Number of uncertain findings in the request.
        count: usize,
    },
}

/// Highest number of uncertain findings a walkthrough presents.
pub const MAX_UNCERTAIN_FINDINGS: usize = 3;

/// Plans a walkthrough for one review request.
///
/// # Errors
///
/// Returns [`WalkthroughError::EmptyChange`] when the request has no observed
/// contract change and [`WalkthroughError::TooManyUncertainFindings`] when the
/// request leaves more than [`MAX_UNCERTAIN_FINDINGS`] findings uncertain.
pub fn plan_walkthrough(request: WalkthroughRequest) -> Result<WalkthroughPlan, WalkthroughError> {
    let stage = stage_for(&request);
    if stage == WalkthroughStage::Map {
        return Err(WalkthroughError::EmptyChange);
    }
    if request.uncertain_findings > MAX_UNCERTAIN_FINDINGS {
        return Err(WalkthroughError::TooManyUncertainFindings {
            count: request.uncertain_findings,
        });
    }

    let mut steps = vec!["Open the pinned comparison", "Read the HLD delta"];
    if request.changed_contracts > 1 {
        steps.push("Follow the changed contracts");
    }
    if request.include_source {
        steps.push("Pin the exact source");
    }

    Ok(WalkthroughPlan {
        stage,
        steps,
        residual_risk: residual_risk(request.uncertain_findings),
    })
}

fn stage_for(request: &WalkthroughRequest) -> WalkthroughStage {
    if request.include_source {
        WalkthroughStage::Source
    } else if request.changed_contracts > 0 {
        WalkthroughStage::Behavior
    } else {
        WalkthroughStage::Map
    }
}

fn residual_risk(uncertain_findings: usize) -> &'static str {
    match uncertain_findings {
        0 => "No unresolved difference observed",
        1..=2 => "A bounded difference remains explicit",
        _ => "The walkthrough stops at the evidence boundary",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        changed_contracts: usize,
        uncertain_findings: usize,
        include_source: bool,
    ) -> WalkthroughRequest {
        WalkthroughRequest {
            changed_contracts,
            uncertain_findings,
            include_source,
        }
    }

    #[test]
    fn empty_change_is_refused() {
        let result = plan_walkthrough(request(0, 0, false));
        assert_eq!(result, Err(WalkthroughError::EmptyChange));
    }

    #[test]
    fn uncertainty_above_the_bound_is_refused() {
        let result = plan_walkthrough(request(4, 4, false));
        assert_eq!(
            result,
            Err(WalkthroughError::TooManyUncertainFindings { count: 4 })
        );
    }

    #[test]
    fn uncertainty_at_the_bound_is_presented() {
        let plan = plan_walkthrough(request(4, 3, false)).expect("bounded uncertainty is allowed");
        assert_eq!(
            plan.residual_risk,
            "The walkthrough stops at the evidence boundary"
        );
    }

    #[test]
    fn source_extends_the_plan() {
        let plan = plan_walkthrough(request(2, 1, true)).expect("source plan is valid");
        assert_eq!(plan.stage, WalkthroughStage::Source);
        assert!(plan.steps.contains(&"Pin the exact source"));
        assert_eq!(plan.residual_risk, "A bounded difference remains explicit");
    }

    #[test]
    fn behavior_plan_omits_source() {
        let plan = plan_walkthrough(request(2, 0, false)).expect("behavior plan is valid");
        assert_eq!(plan.stage, WalkthroughStage::Behavior);
        assert!(!plan.steps.contains(&"Pin the exact source"));
        assert_eq!(plan.residual_risk, "No unresolved difference observed");
    }

    #[test]
    fn single_contract_skips_contract_following() {
        let plan = plan_walkthrough(request(1, 0, false)).expect("single contract is valid");
        assert_eq!(
            plan.steps,
            vec!["Open the pinned comparison", "Read the HLD delta"]
        );
    }
}
