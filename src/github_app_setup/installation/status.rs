//! Only update a check with this App's exact immutable job/head identity.
use super::*;

pub(super) fn valid_payload(payload: &Value) -> Result<()> {
    if payload["name"] != "CodeShouldBehave / Architecture"
        || payload["external_id"]
            .as_str()
            .and_then(|s| s.strip_prefix("csb-app-v1:"))
            .is_none_or(|s| !crate::github_app_setup::valid_secret(s))
        || payload["output"]["title"]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 256)
        || payload["output"]["summary"]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 8192)
    {
        return Err(SetupError::Denied);
    }
    oid(&payload["head_sha"])?;
    match payload["status"].as_str() {
        Some("queued" | "in_progress")
            if payload["conclusion"].is_null() && payload["details_url"].is_null() =>
        {
            Ok(())
        }
        Some("completed")
            if matches!(
                payload["conclusion"].as_str(),
                Some("success" | "neutral" | "failure")
            ) =>
        {
            Ok(())
        }
        _ => Err(SetupError::Denied),
    }
}
impl InstallationSession<'_> {
    pub(super) fn update_owned_check(
        &self,
        repository: i64,
        check_id: i64,
        payload: &Value,
    ) -> Result<i64> {
        let _guard = self
            .broker
            .admission
            .try_lock()
            .map_err(|_| SetupError::Capacity)?;
        self.credential.check(super::super::now()?)?;
        if repository != self.credential.repository || check_id <= 0 {
            return Err(SetupError::Denied);
        }
        valid_payload(payload)?;
        let path = format!("/repos/{}/{}/check-runs/{check_id}", self.owner, self.name);
        let current = self
            .broker
            .request("GET", &path, &self.credential.token, None)?;
        if current["id"].as_i64() != Some(check_id)
            || current["app"]["id"].as_i64() != Some(self.broker.app_id)
            || ["name", "head_sha", "external_id"]
                .iter()
                .any(|k| current[k] != payload[k])
        {
            return Err(SetupError::Conflict);
        }
        // GitHub's PATCH schema cannot change the immutable head SHA.
        let mut patch = payload.clone();
        patch
            .as_object_mut()
            .ok_or(SetupError::Invalid)?
            .remove("head_sha");
        let updated = self
            .broker
            .request("PATCH", &path, &self.credential.token, Some(&patch))?;
        self.credential.check(super::super::now()?)?;
        let id = check_matches(&updated, payload, self.broker.app_id)?;
        if id != check_id {
            return Err(SetupError::Conflict);
        }
        Ok(id)
    }
}
