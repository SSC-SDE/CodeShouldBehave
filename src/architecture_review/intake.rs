//! Deterministic intake with caller-owned limits; no repository or provider access.

use super::model::{Change, Risk};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntakeLimits {
    pub max_changes: usize,
    pub max_changed_lines: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntakeError {
    EmptyInventory,
    TooManyChanges,
    TooManyLines,
    DuplicatePath(String),
    EmptyChange(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inventory {
    changes: Vec<Change>,
    total_lines: u64,
}

impl Inventory {
    /// Consumes at most max_changes + 1 items, including for an unbounded iterator.
    pub fn collect(
        changes: impl IntoIterator<Item = Change>,
        limits: IntakeLimits,
    ) -> Result<Self, IntakeError> {
        let mut accepted = Vec::new();
        let mut total_lines = 0_u64;
        for change in changes {
            if accepted.len() >= limits.max_changes {
                return Err(IntakeError::TooManyChanges);
            }
            let lines = u64::from(change.added_lines) + u64::from(change.removed_lines);
            if lines == 0 {
                return Err(IntakeError::EmptyChange(change.path().into()));
            }
            total_lines = total_lines.checked_add(lines).ok_or(IntakeError::TooManyLines)?;
            if total_lines > limits.max_changed_lines {
                return Err(IntakeError::TooManyLines);
            }
            accepted.push(change);
        }
        if accepted.is_empty() {
            return Err(IntakeError::EmptyInventory);
        }
        accepted.sort_unstable_by(|left, right| left.path().cmp(right.path()));
        if let Some(pair) = accepted.windows(2).find(|pair| pair[0].path() == pair[1].path()) {
            return Err(IntakeError::DuplicatePath(pair[0].path().into()));
        }
        Ok(Self { changes: accepted, total_lines })
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn total_lines(&self) -> u64 {
        self.total_lines
    }

    pub fn boundary_count(&self) -> usize {
        self.changes.iter().filter(|change| change.is_public_boundary()).count()
    }

    /// Ordered low, medium, high; every accepted change contributes exactly once.
    pub fn risk_counts(&self) -> [usize; 3] {
        let mut counts = [0; 3];
        for change in &self.changes {
            counts[match change.risk { Risk::Low => 0, Risk::Medium => 1, Risk::High => 2 }] += 1;
        }
        counts
    }
}
