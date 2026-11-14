//! Stable, bounded review batches, separated by architectural responsibility.

use super::intake::Inventory;
use super::model::{Change, ChangeKind};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Lane {
    Implementation,
    Contract,
    Dependency,
    Data,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTask<'a> {
    pub change: &'a Change,
    pub weight: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewBatch<'a> {
    pub lane: Lane,
    pub tasks: Vec<ReviewTask<'a>>,
    pub weight: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanLimits {
    pub max_tasks_per_batch: usize,
    pub max_weight_per_batch: u64,
    pub max_batches: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    ZeroLimit,
    ChangeTooHeavy(String),
    TooManyBatches,
    WeightOverflow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewPlan<'a> {
    batches: Vec<ReviewBatch<'a>>,
    total_weight: u64,
}

impl<'a> ReviewPlan<'a> {
    /// Each change appears once. Batches never mix lanes or exceed either limit.
    pub fn build(inventory: &'a Inventory, limits: PlanLimits) -> Result<Self, PlanError> {
        if limits.max_tasks_per_batch == 0 || limits.max_weight_per_batch == 0 || limits.max_batches == 0 {
            return Err(PlanError::ZeroLimit);
        }
        let mut plan = Self { batches: Vec::new(), total_weight: 0 };
        for lane in [Lane::Implementation, Lane::Contract, Lane::Dependency, Lane::Data] {
            for change in inventory.changes().iter().filter(|change| lane_for(change) == lane) {
                let weight = change.review_weight();
                if weight > limits.max_weight_per_batch {
                    return Err(PlanError::ChangeTooHeavy(change.path().into()));
                }
                let fits = plan.batches.last().is_some_and(|batch| {
                    batch.lane == lane
                        && batch.tasks.len() < limits.max_tasks_per_batch
                        && weight <= limits.max_weight_per_batch - batch.weight
                });
                if !fits {
                    if plan.batches.len() >= limits.max_batches {
                        return Err(PlanError::TooManyBatches);
                    }
                    plan.batches.push(ReviewBatch { lane, tasks: Vec::new(), weight: 0 });
                }
                let batch = plan.batches.last_mut().expect("a fitting batch was created");
                batch.weight += weight;
                batch.tasks.push(ReviewTask { change, weight });
                plan.total_weight = plan.total_weight.checked_add(weight).ok_or(PlanError::WeightOverflow)?;
            }
        }
        Ok(plan)
    }

    pub fn batches(&self) -> &[ReviewBatch<'a>] {
        &self.batches
    }

    pub fn task_count(&self) -> usize {
        self.batches.iter().map(|batch| batch.tasks.len()).sum()
    }

    pub fn total_weight(&self) -> u64 {
        self.total_weight
    }
}

pub fn lane_for(change: &Change) -> Lane {
    match change.kind {
        ChangeKind::Implementation => Lane::Implementation,
        ChangeKind::PublicContract => Lane::Contract,
        ChangeKind::Dependency => Lane::Dependency,
        ChangeKind::DataLayout => Lane::Data,
    }
}
