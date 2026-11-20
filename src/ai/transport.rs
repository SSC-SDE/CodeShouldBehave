//! One bounded JSON transport for AI providers. No retries, no redirects, no
//! ambient proxy and a hard response cap.

use serde_json::Value;
use std::time::Duration;

pub(crate) const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

pub(crate) fn post_json(
    endpoint: &str,
    key: Option<&str>,
    body: &Value,
    timeout: Duration,
) -> Result<Value, String> {
    validate_endpoint(endpoint)?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(false)
            .proxy(None)
            .max_redirects(0)
            .max_redirects_will_error(true)
            .max_response_header_size(16 * 1024)
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .build(),
    );
    let mut request = agent
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", "CodeShouldBehave-csb")
        .config()
        .timeout_global(Some(timeout))
        .build();
    if let Some(key) = key {
        let mut value = ureq::http::HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| "provider_invalid_key")?;
        value.set_sensitive(true);
        request = request.header("Authorization", value);
    }
    let body = serde_json::to_vec(body).map_err(|_| "provider_invalid_request")?;
    let mut response = request
        .content_type("application/json")
        .send(body)
        .map_err(map_error)?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err("provider_auth".into()),
        429 => return Err("provider_capacity".into()),
        _ => return Err("provider_unavailable".into()),
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|_| "provider_response_too_large")?;
    serde_json::from_slice(&bytes).map_err(|_| "provider_invalid_json".into())
}

fn validate_endpoint(endpoint: &str) -> Result<(), String> {
    let uri: ureq::http::Uri = endpoint.parse().map_err(|_| "provider_invalid_endpoint")?;
    let scheme = uri.scheme_str().ok_or("provider_invalid_endpoint")?;
    let host = uri.host().ok_or("provider_invalid_endpoint")?;
    match scheme {
        "https" => Ok(()),
        "http" if is_loopback(host) => Ok(()),
        "http" => Err("provider_insecure_endpoint".into()),
        _ => Err("provider_invalid_endpoint".into()),
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn map_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(_) => "provider_timeout".into(),
        ureq::Error::StatusCode(401 | 403) => "provider_auth".into(),
        ureq::Error::StatusCode(429) => "provider_capacity".into(),
        _ => "provider_unavailable".into(),
    }
}
