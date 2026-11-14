//! Validated inputs for the opt-in, offline architecture-review demonstration.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Implementation,
    PublicContract,
    Dependency,
    DataLayout,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Change {
    path: String,
    pub kind: ChangeKind,
    pub risk: Risk,
    pub added_lines: u32,
    pub removed_lines: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeError {
    InvalidPath,
    EmptyChange,
}

impl Change {
    pub fn new(
        path: String,
        kind: ChangeKind,
        risk: Risk,
        added_lines: u32,
        removed_lines: u32,
    ) -> Result<Self, ChangeError> {
        if path.is_empty()
            || path.contains('\\')
            || path.contains(':')
            || path.chars().any(char::is_control)
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(ChangeError::InvalidPath);
        }
        if added_lines == 0 && removed_lines == 0 {
            return Err(ChangeError::EmptyChange);
        }
        Ok(Self { path, kind, risk, added_lines, removed_lines })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn is_public_boundary(&self) -> bool {
        !matches!(self.kind, ChangeKind::Implementation)
    }

    /// Weight is an estimate for batching, never an execution or resource grant.
    pub fn review_weight(&self) -> u64 {
        let lines = u64::from(self.added_lines) + u64::from(self.removed_lines);
        let risk = match self.risk {
            Risk::Low => 1,
            Risk::Medium => 2,
            Risk::High => 4,
        };
        lines.max(1) * risk * if self.is_public_boundary() { 2 } else { 1 }
    }
}
