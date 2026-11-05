//! Explicitly mounted setup routes; not connected to the existing workspace host.
use super::*;
use axum::{
    Json, Router,
    extract::Request,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
const SESSION: &str = "__Host-csb-app-session";
const BROWSER: &str = "__Host-csb-app-setup";

pub fn disabled_router() -> Router {
    Router::new().route("/v1/github-app/capability",get(||async{Json(json!({"profile":"github-app","available":false,"activation":"off","reason":"host_and_provider_not_configured"}))}))
        .route("/v1/github-app/login",get(||async{failure(SetupError::Unavailable)}))
        .layer(middleware::from_fn(security_headers))
}
pub fn router<P: GitHubSetupProvider>(host: Arc<AppSetupHost<P>>) -> Router {
    Router::new()
        .route("/v1/github-app/capability",get(||async{Json(json!({"profile":"github-app","available":true,"activation":"off","scope_confirmation_available":false,"checks_available":false}))}))
        .route("/v1/github-app/login",get(login::<P>))
        .route("/v1/github-app/callback",get(callback::<P>))
        .route("/v1/github-app/repositories",get(repositories::<P>).post(select::<P>))
        .route("/v1/github-app/logout",post(logout::<P>))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn(security_headers))
        .with_state(host)
}
async fn security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginQuery {
    installation_id: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Callback {
    state: String,
    code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    installation_id: i64,
    repository_id: i64,
}
fn cookie(headers: &HeaderMap, name: &str) -> Result<String> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for part in header
            .to_str()
            .map_err(|_| SetupError::Authentication)?
            .split(';')
        {
            if let Some((k, v)) = part.trim().split_once('=') {
                if k == name {
                    if found.is_some() || !valid_secret(v) {
                        return Err(SetupError::Authentication);
                    }
                    found = Some(v.to_string());
                }
            }
        }
    }
    found.ok_or(SetupError::Authentication)
}
fn origin<P: GitHubSetupProvider>(headers: &HeaderMap, host: &AppSetupHost<P>) -> Result<()> {
    let values: Vec<_> = headers.get_all(header::ORIGIN).iter().collect();
    if values.len() != 1 || values[0].to_str().ok() != Some(host.config.origin.as_str()) {
        return Err(SetupError::Denied);
    }
    Ok(())
}
fn set_cookie(response: &mut Response, name: &str, value: &str, age: i64) {
    if let Ok(v) = HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={age}"
    )) {
        response.headers_mut().append(header::SET_COOKIE, v);
    }
}
fn failure(error: SetupError) -> Response {
    let (status, code) = match error {
        SetupError::Invalid => (StatusCode::BAD_REQUEST, "invalid_setup_request"),
        SetupError::Authentication => (StatusCode::UNAUTHORIZED, "github_authentication_required"),
        SetupError::Denied => (StatusCode::FORBIDDEN, "repository_setup_not_authorized"),
        SetupError::Capacity => (StatusCode::TOO_MANY_REQUESTS, "setup_capacity_reached"),
        SetupError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "github_setup_unavailable"),
    };
    (status, Json(json!({"error":code,"activation":"off"}))).into_response()
}
async fn login<P: GitHubSetupProvider>(
    State(host): State<Arc<AppSetupHost<P>>>,
    Query(q): Query<LoginQuery>,
) -> Response {
    match tokio::task::spawn_blocking(move || host.begin_login(q.installation_id)).await {
        Ok(Ok(start)) => {
            let mut response = StatusCode::SEE_OTHER.into_response();
            let Ok(location) = HeaderValue::from_str(&start.authorize_url) else {
                return failure(SetupError::Unavailable);
            };
            response.headers_mut().insert(header::LOCATION, location);
            set_cookie(&mut response, BROWSER, &start.browser_cookie, STATE_TTL);
            response
        }
        Ok(Err(e)) => failure(e),
        Err(_) => failure(SetupError::Unavailable),
    }
}
async fn callback<P: GitHubSetupProvider>(
    State(host): State<Arc<AppSetupHost<P>>>,
    headers: HeaderMap,
    Query(q): Query<Callback>,
) -> Response {
    let browser = match cookie(&headers, BROWSER) {
        Ok(v) => v,
        Err(e) => return failure(e),
    };
    let previous = cookie(&headers, SESSION).ok();
    match tokio::task::spawn_blocking(move || {
        let complete = host.finish_login(&q.state, &browser, &q.code)?;
        if let Some(old) = previous {
            host.logout(&old)?;
        }
        Ok::<_, SetupError>(complete)
    })
    .await
    {
        Ok(Ok(complete)) => {
            // Repository picker UI comes next; callback returns only non-sensitive capability.
            let mut response=Json(json!({"authenticated":true,"activation":"off","next":"/v1/github-app/repositories"})).into_response();
            set_cookie(
                &mut response,
                SESSION,
                &complete.session_cookie,
                SESSION_TTL,
            );
            set_cookie(&mut response, BROWSER, "", 0);
            response
        }
        Ok(Err(e)) => failure(e),
        Err(_) => failure(SetupError::Unavailable),
    }
}
async fn repositories<P: GitHubSetupProvider>(
    State(host): State<Arc<AppSetupHost<P>>>,
    headers: HeaderMap,
) -> Response {
    let token = match cookie(&headers, SESSION) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    match tokio::task::spawn_blocking(move || host.repositories(&token)).await {
        Ok(Ok(rows)) => Json(json!({"repositories":rows,"activation":"off"})).into_response(),
        Ok(Err(e)) => failure(e),
        Err(_) => failure(SetupError::Unavailable),
    }
}
async fn select<P: GitHubSetupProvider>(
    State(host): State<Arc<AppSetupHost<P>>>,
    headers: HeaderMap,
    Json(q): Json<Selection>,
) -> Response {
    if let Err(e) = origin(&headers, &host) {
        return failure(e);
    }
    let token = match cookie(&headers, SESSION) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    match tokio::task::spawn_blocking(move || {
        host.select_repository(&token, q.installation_id, q.repository_id)
    })
    .await
    {
        Ok(Ok(row)) => {
            Json(json!({"repository":row,"activation":"off","scan_started":false})).into_response()
        }
        Ok(Err(e)) => failure(e),
        Err(_) => failure(SetupError::Unavailable),
    }
}
async fn logout<P: GitHubSetupProvider>(
    State(host): State<Arc<AppSetupHost<P>>>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = origin(&headers, &host) {
        return failure(e);
    }
    let token = match cookie(&headers, SESSION) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    match host.logout(&token) {
        Ok(()) => {
            let mut r = StatusCode::NO_CONTENT.into_response();
            set_cookie(&mut r, SESSION, "", 0);
            r
        }
        Err(e) => failure(e),
    }
}
