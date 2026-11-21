//! Isolation boundary for the server/Focus AI extension.
//!
//! This module is deliberately provider-free. It computes capability, validates
//! local-only destinations, keeps a separate admission gate and stores explicit
//! disclosure consent. It never reads storage, starts a worker or opens a socket.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

pub const CONTRACT_VERSION: &str = "ai-extension-v1";
pub const POLICY_REVISION: &str = "ai-extension-policy-v1";
pub const DEFAULT_MODEL: &str = "qwen2.5-coder:7b";
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";
pub const MAX_MODEL_BYTES: usize = 128;
pub const MAX_DESTINATION_BYTES: usize = 512;
pub const MAX_EVIDENCE_ITEMS: usize = 16;
pub const MAX_EVIDENCE_BYTES: usize = 16 * 1024;
pub const MAX_INPUT_TOKENS: usize = 4096;
pub const MAX_GENERATED_TOKENS: usize = 512;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const MAX_ADMISSIONS_PER_HOUR: u32 = 10;
pub const MAX_CONCURRENCY: usize = 1;
pub const CACHE_ENTRIES: usize = 128;
pub const CACHE_BYTES: usize = 16 * 1024 * 1024;
pub const CACHE_TTL_SECONDS: u64 = 3600;
pub const DEADLINE_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiCapabilityState {
    Disabled,
    Unconfigured,
    Unavailable,
    Ready,
}

impl AiCapabilityState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Unconfigured => "unconfigured",
            Self::Unavailable => "unavailable",
            Self::Ready => "ready",
        }
    }

    fn dispatch_available(self) -> bool {
        self == Self::Ready
    }
}

/// Disabled unless an operator explicitly enables and configures it. This type
/// never contacts Ollama; capability computation is pure configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiExtensionPolicy {
    enabled: bool,
    model: Option<String>,
    endpoint: Option<String>,
    unavailable: bool,
}

impl Default for AiExtensionPolicy {
    fn default() -> Self {
        Self::disabled()
    }
}

impl AiExtensionPolicy {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            model: None,
            endpoint: None,
            unavailable: false,
        }
    }

    /// Enables the extension only when the caller passes explicit configuration.
    /// Missing model or endpoint is allowed and reports `unconfigured` without
    /// touching a provider.
    pub fn configure(
        enabled: bool,
        model: Option<String>,
        endpoint: Option<String>,
    ) -> Result<Self, String> {
        if !enabled {
            return Ok(Self::disabled());
        }
        if let Some(model) = model.as_deref() {
            validate_model(model)?;
        }
        let endpoint = endpoint.map(|value| {
            let value = value.trim_end_matches('/').to_owned();
            validate_local_endpoint(&value).map(|_| value)
        });
        Ok(Self {
            enabled: true,
            model,
            endpoint: match endpoint {
                Some(Ok(value)) => Some(value),
                Some(Err(error)) => return Err(error),
                None => None,
            },
            unavailable: false,
        })
    }

    /// Marks a configured provider as unavailable after an observed failure.
    /// Callers must not use this to probe a provider. It does not change the
    /// deterministic review.
    pub fn mark_unavailable(mut self) -> Self {
        self.unavailable = true;
        self
    }

    pub fn state(&self) -> AiCapabilityState {
        if !self.enabled {
            AiCapabilityState::Disabled
        } else if self.model.is_none() || self.endpoint.is_none() {
            AiCapabilityState::Unconfigured
        } else if self.unavailable {
            AiCapabilityState::Unavailable
        } else {
            AiCapabilityState::Ready
        }
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    pub fn capability_json(&self) -> Value {
        let state = self.state();
        let mut value = json!({
            "schema": "csb-ai-capability-v1",
            "api_version": "v1",
            "contract_version": CONTRACT_VERSION,
            "capability": state.as_str(),
            "dispatch_available": state.dispatch_available(),
            "features": {
                "explain_hld_boundary": true,
                "hld_only": true,
                "streaming": false,
                "lld_explanations": false,
                "whole_pr_narratives": false
            },
            "limits": {
                "concurrency_global": MAX_CONCURRENCY,
                "concurrency_per_tenant": MAX_CONCURRENCY,
                "deadline_seconds": DEADLINE_SECONDS,
                "max_evidence_items": MAX_EVIDENCE_ITEMS,
                "max_evidence_bytes": MAX_EVIDENCE_BYTES,
                "max_input_tokens": MAX_INPUT_TOKENS,
                "max_generated_tokens": MAX_GENERATED_TOKENS,
                "max_response_bytes": MAX_RESPONSE_BYTES,
                "admissions_per_hour": MAX_ADMISSIONS_PER_HOUR,
                "cache_entries": CACHE_ENTRIES,
                "cache_bytes": CACHE_BYTES,
                "cache_ttl_seconds": CACHE_TTL_SECONDS
            },
            "policy_revision": POLICY_REVISION
        });
        if matches!(
            state,
            AiCapabilityState::Ready | AiCapabilityState::Unavailable
        ) {
            value["provider"] = json!({
                "name": "ollama",
                "model": self.model.as_deref().unwrap_or(""),
                "model_digest": Value::Null,
                "destination": self.endpoint.as_deref().unwrap_or("")
            });
        }
        value
    }
}

/// Local-only endpoint restriction. The server extension must never accept a
/// request-controlled destination, a remote host, a metadata address or a URL
/// with credentials, a path, a query or a fragment.
pub fn validate_local_endpoint(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_DESTINATION_BYTES
        || value.chars().any(char::is_control)
    {
        return Err("ai_endpoint_invalid".into());
    }
    if value.contains('@') || value.contains('?') || value.contains('#') {
        return Err("ai_endpoint_invalid".into());
    }
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| "ai_endpoint_not_loopback".to_owned())?;
    let authority = match rest.split_once('/') {
        Some((authority, "")) => authority,
        Some(_) => return Err("ai_endpoint_path_not_supported".into()),
        None => rest,
    };
    if authority.is_empty() {
        return Err("ai_endpoint_invalid".into());
    }
    let (host, port) = if let Some(value) = authority.strip_prefix('[') {
        value
            .split_once("]:")
            .ok_or_else(|| "ai_endpoint_invalid".to_owned())?
    } else {
        authority
            .rsplit_once(':')
            .ok_or_else(|| "ai_endpoint_invalid".to_owned())?
    };
    if port.is_empty()
        || !port.bytes().all(|byte| byte.is_ascii_digit())
        || port.parse::<u16>().ok().filter(|port| *port > 0).is_none()
    {
        return Err("ai_endpoint_invalid".into());
    }
    match host {
        "127.0.0.1" | "localhost" | "::1" => Ok(()),
        _ => Err("ai_endpoint_not_loopback".into()),
    }
}

fn validate_model(value: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > MAX_MODEL_BYTES
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err("ai_model_invalid".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Busy,
    BudgetExhausted,
}

impl AdmissionError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "ai_busy",
            Self::BudgetExhausted => "ai_budget_exhausted",
        }
    }
}

#[derive(Debug, Default)]
struct AdmissionState {
    active_tenant: Option<String>,
    windows: BTreeMap<String, (u64, u32)>,
}

/// Separate from the review storage and scheduler. Inference must never hold the
/// storage mutex or block deterministic review admission.
#[derive(Debug, Default)]
pub struct AiAdmission {
    state: Mutex<AdmissionState>,
}

#[derive(Debug)]
pub struct AiAdmissionPermit {
    admission: Arc<AiAdmission>,
    tenant: String,
}

impl Drop for AiAdmissionPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.admission.state.lock() {
            if state.active_tenant.as_deref() == Some(self.tenant.as_str()) {
                state.active_tenant = None;
            }
        }
    }
}

impl AiAdmission {
    pub fn try_admit(
        self: &Arc<Self>,
        tenant: &str,
        principal: &str,
        now_seconds: u64,
    ) -> Result<AiAdmissionPermit, AdmissionError> {
        if !valid_label(tenant) || !valid_label(principal) {
            return Err(AdmissionError::Busy);
        }
        let mut state = self.state.lock().map_err(|_| AdmissionError::Busy)?;
        if state.active_tenant.is_some() {
            return Err(AdmissionError::Busy);
        }
        let window_key = format!("{tenant}\u{1f}{principal}");
        let entry = state.windows.entry(window_key).or_insert((now_seconds, 0));
        if now_seconds < entry.0 || now_seconds.saturating_sub(entry.0) >= 3600 {
            *entry = (now_seconds, 0);
        }
        if entry.1 >= MAX_ADMISSIONS_PER_HOUR {
            return Err(AdmissionError::BudgetExhausted);
        }
        entry.1 += 1;
        state.active_tenant = Some(tenant.to_owned());
        Ok(AiAdmissionPermit {
            admission: Arc::clone(self),
            tenant: tenant.to_owned(),
        })
    }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

/// Explicit disclosure consent for one review/evidence/destination/policy tuple.
/// The explanation endpoint must check this immediately before dispatch and on
/// every cache hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConsentKey {
    tenant: String,
    principal: String,
    review_id: String,
    evidence_id: String,
    destination: String,
    policy_revision: String,
}

impl AiConsentKey {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant: &str,
        principal: &str,
        review_id: &str,
        evidence_id: &str,
        destination: &str,
        policy_revision: &str,
    ) -> Result<Self, String> {
        for (kind, value) in [
            ("tenant", tenant),
            ("principal", principal),
            ("review", review_id),
            ("evidence", evidence_id),
            ("policy revision", policy_revision),
        ] {
            if !valid_label(value) {
                return Err(format!("ai_consent_invalid_{kind}"));
            }
        }
        validate_local_endpoint(destination)?;
        Ok(Self {
            tenant: tenant.to_owned(),
            principal: principal.to_owned(),
            review_id: review_id.to_owned(),
            evidence_id: evidence_id.to_owned(),
            destination: destination.to_owned(),
            policy_revision: policy_revision.to_owned(),
        })
    }

    fn encode(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.tenant,
            self.principal,
            self.review_id,
            self.evidence_id,
            self.destination,
            self.policy_revision
        )
    }
}

#[derive(Default)]
pub struct AiConsentStore {
    granted: Mutex<BTreeSet<String>>,
}

impl AiConsentStore {
    pub fn grant(&self, key: &AiConsentKey) -> Result<(), String> {
        self.granted
            .lock()
            .map_err(|_| "ai_consent_store_unavailable".to_owned())?
            .insert(key.encode());
        Ok(())
    }

    pub fn allows(&self, key: &AiConsentKey) -> bool {
        self.granted
            .lock()
            .map(|granted| granted.contains(&key.encode()))
            .unwrap_or(false)
    }

    pub fn revoke(&self, key: &AiConsentKey) -> bool {
        self.granted
            .lock()
            .map(|mut granted| granted.remove(&key.encode()))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> AiExtensionPolicy {
        AiExtensionPolicy::configure(
            true,
            Some(DEFAULT_MODEL.into()),
            Some(DEFAULT_ENDPOINT.into()),
        )
        .unwrap()
    }

    #[test]
    fn disabled_is_default_and_has_no_provider_surface() {
        let policy = AiExtensionPolicy::default();
        let value = policy.capability_json();
        assert_eq!(policy.state(), AiCapabilityState::Disabled);
        assert_eq!(value["capability"], "disabled");
        assert_eq!(value["dispatch_available"], false);
        assert!(value.get("provider").is_none());
        assert_eq!(value["limits"]["max_evidence_items"], 16);
    }

    #[test]
    fn enabled_without_configuration_is_unconfigured_and_ready_is_explicit() {
        let missing = AiExtensionPolicy::configure(true, None, None).unwrap();
        assert_eq!(missing.state(), AiCapabilityState::Unconfigured);
        assert_eq!(missing.capability_json()["dispatch_available"], false);

        let ready = configured();
        assert_eq!(ready.state(), AiCapabilityState::Ready);
        let value = ready.capability_json();
        assert_eq!(value["capability"], "ready");
        assert_eq!(value["dispatch_available"], true);
        assert_eq!(value["provider"]["model"], DEFAULT_MODEL);
        assert_eq!(value["provider"]["destination"], DEFAULT_ENDPOINT);
        assert_eq!(value["provider"]["model_digest"], Value::Null);
    }

    #[test]
    fn unavailable_does_not_claim_dispatch() {
        let value = configured().mark_unavailable().capability_json();
        assert_eq!(value["capability"], "unavailable");
        assert_eq!(value["dispatch_available"], false);
        assert_eq!(value["provider"]["name"], "ollama");
    }

    #[test]
    fn endpoint_restriction_is_local_only() {
        for allowed in [
            "http://127.0.0.1:11434",
            "http://localhost:11434",
            "http://[::1]:11434",
            "http://127.0.0.1:11434/",
        ] {
            validate_local_endpoint(allowed).unwrap();
        }
        for rejected in [
            "https://127.0.0.1:11434",
            "http://192.168.1.10:11434",
            "http://169.254.169.254:80",
            "http://host.docker.internal:11434",
            "http://0.0.0.0:11434",
            "http://user@127.0.0.1:11434",
            "http://127.0.0.1:11434/api",
            "http://127.0.0.1:11434?x=1",
            "http://127.0.0.1",
        ] {
            assert!(
                validate_local_endpoint(rejected).is_err(),
                "accepted {rejected}"
            );
        }
    }

    #[test]
    fn admission_is_single_flight_and_hourly_bounded() {
        let admission = Arc::new(AiAdmission::default());
        let first = admission.try_admit("tenant", "reviewer", 100).unwrap();
        assert_eq!(
            admission.try_admit("tenant", "reviewer", 100).unwrap_err(),
            AdmissionError::Busy
        );
        drop(first);
        for offset in 1..MAX_ADMISSIONS_PER_HOUR {
            let permit = admission
                .try_admit("tenant", "reviewer", 100 + u64::from(offset))
                .unwrap();
            drop(permit);
        }
        assert_eq!(
            admission.try_admit("tenant", "reviewer", 200).unwrap_err(),
            AdmissionError::BudgetExhausted
        );
        let permit = admission.try_admit("tenant", "reviewer", 3700).unwrap();
        drop(permit);
        let permit = admission.try_admit("tenant", "reviewer", 3701).unwrap();
        drop(permit);
    }

    #[test]
    fn consent_binds_review_evidence_destination_and_policy() {
        let consent = AiConsentStore::default();
        let key = AiConsentKey::new(
            "tenant",
            "reviewer",
            "review-1",
            "evidence-1",
            DEFAULT_ENDPOINT,
            POLICY_REVISION,
        )
        .unwrap();
        assert!(!consent.allows(&key));
        consent.grant(&key).unwrap();
        assert!(consent.allows(&key));

        let other_review = AiConsentKey::new(
            "tenant",
            "reviewer",
            "review-2",
            "evidence-1",
            DEFAULT_ENDPOINT,
            POLICY_REVISION,
        )
        .unwrap();
        let other_policy = AiConsentKey::new(
            "tenant",
            "reviewer",
            "review-1",
            "evidence-1",
            DEFAULT_ENDPOINT,
            "ai-extension-policy-v2",
        )
        .unwrap();
        assert!(!consent.allows(&other_review));
        assert!(!consent.allows(&other_policy));
        assert!(consent.revoke(&key));
        assert!(!consent.allows(&key));
    }
}
