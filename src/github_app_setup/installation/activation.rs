//! Fresh operator activation checks, separate from browser authorization.
use super::*;
use crate::github_app_setup::{AppActivationProvider, CurrentActivationAccess};

/// Caller must reserve five durable tickets and hold `begin_pass(5)` before
/// invoking the operator API. No provider response is cached across invocations.
pub struct BrokerActivationProvider<'a> {
    broker: &'a GitHubInstallationBroker,
}
impl<'a> BrokerActivationProvider<'a> {
    pub fn new(broker: &'a GitHubInstallationBroker) -> Self {
        Self { broker }
    }
}
impl AppActivationProvider for BrokerActivationProvider<'_> {
    fn current_activation_access(
        &self,
        app: i64,
        installation: i64,
        repository: i64,
        actor: i64,
    ) -> Result<CurrentActivationAccess> {
        if app != self.broker.app_id || actor <= 0 {
            return Err(SetupError::Denied);
        }
        let credential = self.broker.issue(installation, repository)?;
        let _guard = self
            .broker
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        let started = Instant::now();
        credential.check(crate::github_app_setup::now()?)?;
        let repo = self.broker.request(
            "GET",
            &format!("/repositories/{repository}"),
            &credential.token,
            None,
        )?;
        let owner = repo["owner"]["login"]
            .as_str()
            .filter(|s| slug(s))
            .ok_or(SetupError::Denied)?;
        let name = repo["name"]
            .as_str()
            .filter(|s| slug(s))
            .ok_or(SetupError::Denied)?;
        if repo["id"].as_i64() != Some(repository)
            || repo["private"].as_bool() != Some(false)
            || repo["archived"].as_bool() != Some(false)
        {
            return Err(SetupError::Denied);
        }
        let user =
            self.broker
                .request("GET", &format!("/user/{actor}"), &credential.token, None)?;
        let login = user["login"]
            .as_str()
            .filter(|s| slug(s))
            .ok_or(SetupError::Denied)?;
        if user["id"].as_i64() != Some(actor) {
            return Err(SetupError::Denied);
        }
        let permission = self.broker.request(
            "GET",
            &format!("/repos/{owner}/{name}/collaborators/{login}/permission"),
            &credential.token,
            None,
        )?;
        if permission["user"]["id"].as_i64() != Some(actor)
            || permission["permission"].as_str() != Some("admin")
            || started.elapsed() >= Duration::from_secs(30)
        {
            return Err(SetupError::Denied);
        }
        credential.check(crate::github_app_setup::now()?)?;
        Ok(CurrentActivationAccess {
            app_id: app,
            installation_id: installation,
            repository_id: repository,
            actor_id: actor,
            owner: owner.into(),
            name: name.into(),
            public: true,
            archived: false,
            suspended: false,
            actor_admin: true,
            contents: "read".into(),
            pull_requests: "read".into(),
            checks: "write".into(),
        })
    }
}
