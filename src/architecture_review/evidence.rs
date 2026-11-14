//! Caller-supplied observations, not execution of checks or claims of authorization.

use super::intake::Inventory;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CheckKind {
    Syntax,
    Behavior,
    Compatibility,
    Security,
    Migration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckOutcome {
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence<'a> {
    pub path: &'a str,
    pub check: CheckKind,
    pub outcome: CheckOutcome,
    pub source: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceError {
    UnknownPath,
    DuplicateCheck,
    InvalidSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceCoverage {
    pub recorded_checks: usize,
    pub checked_paths: usize,
    pub total_paths: usize,
}

pub struct EvidenceLedger<'a> {
    inventory: &'a Inventory,
    entries: BTreeMap<(&'a str, CheckKind), &'a Evidence<'a>>,
}

impl<'a> EvidenceLedger<'a> {
    pub fn for_inventory(inventory: &'a Inventory) -> Self {
        Self { inventory, entries: BTreeMap::new() }
    }

    /// Duplicate observations are rejected, never last-writer-wins.
    pub fn record(&mut self, evidence: &'a Evidence<'a>) -> Result<(), EvidenceError> {
        if self.inventory.changes().binary_search_by(|change| change.path().cmp(evidence.path)).is_err() {
            return Err(EvidenceError::UnknownPath);
        }
        if evidence.source.trim().is_empty()
            || evidence.source.len() > 1024
            || evidence.source.chars().any(char::is_control)
        {
            return Err(EvidenceError::InvalidSource);
        }
        let key = (evidence.path, evidence.check);
        match self.entries.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(evidence);
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(_) => Err(EvidenceError::DuplicateCheck),
        }
    }

    pub fn get(&self, path: &str, check: CheckKind) -> Option<&Evidence<'a>> {
        self.entries.get(&(path, check)).copied()
    }

    pub fn observations(&self) -> impl Iterator<Item = &Evidence<'a>> {
        self.entries.values().copied()
    }

    pub fn coverage(&self) -> EvidenceCoverage {
        let mut checked_paths = 0;
        let mut previous = None;
        for (path, _) in self.entries.keys() {
            if previous != Some(path) {
                checked_paths += 1;
                previous = Some(path);
            }
        }
        EvidenceCoverage {
            recorded_checks: self.entries.len(),
            checked_paths,
            total_paths: self.inventory.changes().len(),
        }
    }

    pub(crate) fn inventory(&self) -> &'a Inventory {
        self.inventory
    }
}
