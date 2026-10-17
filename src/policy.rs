use crate::{PathScope, RepositoryId, ScopeRecord};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrincipalId(String);

impl PrincipalId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            Err("Principal ID must contain 1–256 printable characters".into())
        } else {
            Ok(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Index,
    ConfirmArchitecture,
    Graph,
    Source,
    Export,
    SubmitModel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub principal: PrincipalId,
    pub repository: RepositoryId,
    pub actions: BTreeSet<Action>,
    pub paths: PathScope,
}

pub fn authorize(
    action: Action,
    principal: &PrincipalId,
    scope: &ScopeRecord,
    current_grants: &[Grant],
    path: Option<&str>,
) -> Result<(), String> {
    if action == Action::Export && !scope.export_allowed {
        return Err("Export is disabled by the immutable review scope".into());
    }
    if action == Action::SubmitModel && scope.model_destinations.is_empty() {
        return Err("Model submission is disabled by the immutable review scope".into());
    }
    authorize_repository(
        action,
        principal,
        &scope.primary_repository,
        current_grants,
        path,
        &scope.primary_paths,
    )
}

pub fn authorize_repository(
    action: Action,
    principal: &PrincipalId,
    repository: &RepositoryId,
    current_grants: &[Grant],
    path: Option<&str>,
    immutable_paths: &PathScope,
) -> Result<(), String> {
    let allowed = current_grants.iter().any(|grant| {
        grant.principal == *principal
            && grant.repository == *repository
            && grant.actions.contains(&action)
            && path.is_none_or(|path| immutable_paths.allows(path) && grant.paths.allows(path))
    });
    if allowed {
        Ok(())
    } else {
        Err("Access denied by current grants".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ScopeVersion, SecurityDomain};

    fn fixture() -> (PrincipalId, ScopeRecord, Vec<Grant>) {
        let principal = PrincipalId::new("reviewer-1").unwrap();
        let repository = RepositoryId::new(
            SecurityDomain::new("tenant-a").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        let scope = ScopeRecord {
            version: ScopeVersion::new("scope-1").unwrap(),
            primary_repository: repository.clone(),
            primary_paths: PathScope::new(vec!["src".into()], vec!["src/private".into()]).unwrap(),
            contexts: vec![],
            contracts: BTreeSet::new(),
            export_allowed: false,
            model_destinations: BTreeSet::new(),
        };
        let grants = vec![Grant {
            principal: principal.clone(),
            repository,
            actions: BTreeSet::from([Action::Index, Action::Source]),
            paths: PathScope::all(),
        }];
        (principal, scope, grants)
    }

    #[test]
    fn denies_excluded_paths_and_ungranted_actions() {
        let (principal, scope, grants) = fixture();
        assert!(
            authorize(
                Action::Source,
                &principal,
                &scope,
                &grants,
                Some("src/api.rs")
            )
            .is_ok()
        );
        assert!(
            authorize(
                Action::Source,
                &principal,
                &scope,
                &grants,
                Some("src/private/key.rs")
            )
            .is_err()
        );
        assert!(authorize(Action::Graph, &principal, &scope, &grants, None).is_err());
        assert!(authorize(Action::Export, &principal, &scope, &grants, None).is_err());
        assert!(authorize(Action::SubmitModel, &principal, &scope, &grants, None).is_err());
    }

    #[test]
    fn denies_other_principal_and_security_domain() {
        let (principal, mut scope, grants) = fixture();
        assert!(
            authorize(
                Action::Index,
                &PrincipalId::new("attacker").unwrap(),
                &scope,
                &grants,
                None
            )
            .is_err()
        );
        scope.primary_repository = RepositoryId::new(
            SecurityDomain::new("tenant-b").unwrap(),
            "github",
            "owner",
            "repo",
        )
        .unwrap();
        assert!(authorize(Action::Index, &principal, &scope, &grants, None).is_err());
    }
}
