//! Real semantic-version parsing for explicitly supplied public-contract versions.
//! Version compatibility is metadata evidence, not proof of behavioral compatibility.

use super::intake::Inventory;
use semver::{Error, Version, VersionReq};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionedContract<'a> {
    pub path: &'a str,
    pub baseline: Version,
    pub candidate: Version,
    pub consumers: VersionReq,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compatibility {
    Compatible,
    Downgrade,
    OutsideConsumerRequirement,
    BreakingRelease,
}

impl<'a> VersionedContract<'a> {
    pub fn parse(path: &'a str, baseline: &str, candidate: &str, consumers: &str) -> Result<Self, Error> {
        Ok(Self {
            path,
            baseline: Version::parse(baseline)?,
            candidate: Version::parse(candidate)?,
            consumers: VersionReq::parse(consumers)?,
        })
    }

    /// Build metadata does not affect precedence; 0.x minor changes are breaking.
    pub fn assess(&self) -> Compatibility {
        if self.candidate.cmp_precedence(&self.baseline).is_lt() {
            Compatibility::Downgrade
        } else if !self.consumers.matches(&self.candidate) {
            Compatibility::OutsideConsumerRequirement
        } else if self.candidate.major != self.baseline.major
            || (self.baseline.major == 0 && self.candidate.minor != self.baseline.minor)
            || (self.baseline.major == 0 && self.baseline.minor == 0 && self.candidate.patch != self.baseline.patch)
        {
            Compatibility::BreakingRelease
        } else {
            Compatibility::Compatible
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractAssessment<'a> {
    pub contract: &'a VersionedContract<'a>,
    pub outcome: Compatibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityError {
    UnknownPath,
    NotPublicBoundary,
    DuplicatePath,
}

pub fn assess_contracts<'a>(
    inventory: &Inventory,
    contracts: &'a [VersionedContract<'a>],
) -> Result<Vec<ContractAssessment<'a>>, CompatibilityError> {
    let mut assessments = Vec::new();
    for contract in contracts {
        let index = inventory.changes().binary_search_by(|change| change.path().cmp(contract.path))
            .map_err(|_| CompatibilityError::UnknownPath)?;
        if !inventory.changes()[index].is_public_boundary() {
            return Err(CompatibilityError::NotPublicBoundary);
        }
        assessments.push(ContractAssessment { contract, outcome: contract.assess() });
    }
    assessments.sort_unstable_by_key(|assessment| assessment.contract.path);
    if assessments.windows(2).any(|pair| pair[0].contract.path == pair[1].contract.path) {
        return Err(CompatibilityError::DuplicatePath);
    }
    Ok(assessments)
}
