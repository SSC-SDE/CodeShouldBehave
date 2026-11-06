//! Explicitly mounted setup routes; not connected to the existing workspace host.
use super::*;
use axum::{
    Json, Router,
    extract::Request,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
const SESSION: &str = "__Host-csb-app-session";
const BROWSER: &str = "__Host-csb-app-setup";

pub fn disabled_router() -> Router {
    page_routes().route("/v1/github-app/capability",get(||async{Json(json!({"profile":"github-app","available":false,"activation":"off","reason":"host_and_provider_not_configured"}))}))
        .route("/v1/github-app/login",get(||async{failure(SetupError::Unavailable)}))
        .layer(middleware::from_fn(security_headers))
}
pub fn router<P: GitHubSetupProvider>(host: Arc<AppSetupHost<P>>) -> Router {
    page_routes()
        .route("/v1/github-app/capability",get(||async{Json(json!({"profile":"github-app","available":true,"activation":"off","scope_confirmation_available":true,"checks_available":false}))}))
        .route("/v1/github-app/login",get(login::<P>))
        .route("/v1/github-app/callback",get(callback::<P>))
        .route("/v1/github-app/repositories",get(repositories::<P>).post(select::<P>))
        .route("/v1/github-app/logout",post(logout::<P>))
        .route("/v1/github-app/inventory",post(inventory::<P>))
        .route("/v1/github-app/scope/preview",post(preview::<P>))
        .route("/v1/github-app/scope/confirm",post(confirm::<P>))
        .layer(DefaultBodyLimit::max(16384))
        .layer(middleware::from_fn(security_headers))
        .with_state(host)
}
async fn security_headers(req: Request, next: Next) -> Response {
    let page = req.uri().path() == "/github-app/setup";
    let mut response = next.run(req).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        (
            "content-security-policy",
            if page {
                "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
            } else {
                "default-src 'none'; frame-ancestors 'none'"
            },
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
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    error_uri: Option<String>,
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
        SetupError::Conflict => (StatusCode::CONFLICT, "scope_changed_review_again"),
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
    // GitHub cancellation fields are accepted but never reflected into HTML.
    if q.error.is_some()
        || q.error_description.is_some()
        || q.error_uri.is_some()
        || q.code.is_none()
    {
        return setup_redirect(Some("authorization_not_completed"), None);
    }
    let browser = match cookie(&headers, BROWSER) {
        Ok(v) => v,
        Err(_) => return setup_redirect(Some("sign_in_again"), None),
    };
    let previous = cookie(&headers, SESSION).ok();
    match tokio::task::spawn_blocking(move || {
        let complete = host.finish_login(
            &q.state,
            &browser,
            q.code.as_deref().ok_or(SetupError::Authentication)?,
        )?;
        if let Some(old) = previous {
            host.logout(&old)?;
        }
        Ok::<_, SetupError>(complete)
    })
    .await
    {
        Ok(Ok(complete)) => {
            let mut response = setup_redirect(None, complete.installation_hint);
            set_cookie(
                &mut response,
                SESSION,
                &complete.session_cookie,
                SESSION_TTL,
            );
            set_cookie(&mut response, BROWSER, "", 0);
            response
        }
        Ok(Err(_)) | Err(_) => setup_redirect(Some("sign_in_again"), None),
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

fn page_routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/github-app/setup",
            get(|| async { Html(include_str!("setup.html")) }),
        )
        .route(
            "/assets/app-setup.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("setup.js"),
                )
            }),
        )
        .route(
            "/assets/app-setup.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    format!(
                        "{}\n{}",
                        include_str!("../onboarding.css"),
                        include_str!("setup.css")
                    ),
                )
            }),
        )
}
fn setup_redirect(error: Option<&str>, hint: Option<i64>) -> Response {
    let location = if let Some(error) = error {
        format!("/github-app/setup?auth_error={error}")
    } else if let Some(id) = hint {
        format!("/github-app/setup?installation_id={id}")
    } else {
        "/github-app/setup".into()
    };
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(value) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    set_cookie(&mut response, BROWSER, "", 0);
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    installation_id: i64,
    repository_id: i64,
    preview_id: String,
    expected_revision: i64,
    acknowledge_limits: bool,
}
async fn inventory<P: GitHubSetupProvider>(
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
    match tokio::task::spawn_blocking(move||{
        let inventory=host.scope_inventory(&token,q.installation_id,q.repository_id)?;
        let saved=host.saved_scope(&token,q.installation_id,q.repository_id)?;
        let excluded=inventory.entries.iter().filter(|e|crate::sensitive_path(&e.path)||matches!(e.kind.as_str(),"symlink"|"submodule")).map(|e|e.path.clone()).collect::<Vec<_>>();
        Ok::<_,SetupError>(json!({"inventory":inventory,"saved":saved,"automatically_excluded":excluded,"activation":"off","source_scanned":false}))
    }).await {Ok(Ok(value))=>Json(value).into_response(),Ok(Err(e))=>failure(e),Err(_)=>failure(SetupError::Unavailable)}
}
async fn preview<P: GitHubSetupProvider>(
    State(host): State<Arc<AppSetupHost<P>>>,
    headers: HeaderMap,
    Json(q): Json<ScopeInput>,
) -> Response {
    if let Err(e) = origin(&headers, &host) {
        return failure(e);
    }
    let token = match cookie(&headers, SESSION) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    match tokio::task::spawn_blocking(move || host.preview_scope(&token, q)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => failure(e),
        Err(_) => failure(SetupError::Unavailable),
    }
}
async fn confirm<P: GitHubSetupProvider>(
    State(host): State<Arc<AppSetupHost<P>>>,
    headers: HeaderMap,
    Json(q): Json<Confirmation>,
) -> Response {
    if let Err(e) = origin(&headers, &host) {
        return failure(e);
    }
    let token = match cookie(&headers, SESSION) {
        Ok(t) => t,
        Err(e) => return failure(e),
    };
    match tokio::task::spawn_blocking(move || {
        host.confirm_scope(
            &token,
            q.installation_id,
            q.repository_id,
            &q.preview_id,
            q.expected_revision,
            q.acknowledge_limits,
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => failure(e),
        Err(_) => failure(SetupError::Unavailable),
    }
}
