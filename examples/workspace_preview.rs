#![cfg(feature = "server")]

use flowdiff::{HttpBackend, HttpFailure, HttpOperation, router};
use serde_json::{Value, json};
use std::sync::Arc;

const TOKEN: &str = "local-preview-session-token";

struct PreviewBackend;

impl HttpBackend for PreviewBackend {
    fn execute(&self, bearer: &str, operation: HttpOperation) -> Result<Value, HttpFailure> {
        if bearer != TOKEN {
            return Err(HttpFailure::Authentication);
        }
        let repository = json!({
            "security_domain": "preview",
            "provider": "fixture",
            "owner": "codeshouldbehave",
            "name": "payments-api",
        });
        let revision = |generation, review_id: &str, base: &str, head: &str, state: &str| {
            json!({
                "repository_id": 1,
                "repository": repository,
                "provider_pr_id": "PR-1842",
                "generation": generation,
                "review_id": review_id,
                "base_commit": base,
                "head_commit": head,
                "target_branch": "main",
                "state": state,
                "priority": 80,
                "observed_at_ms": 1_790_900_000_000_i64,
            })
        };
        match operation {
            HttpOperation::ListPrs { .. } => Ok(json!({
                "api_version": "v1",
                "items": [revision(2, "review-preview", &"a".repeat(40), &"b".repeat(40), "open")],
                "next_cursor": null,
            })),
            HttpOperation::ListRevisions { .. } => Ok(json!({
                "api_version": "v1",
                "items": [
                    revision(1, "review-old", &"c".repeat(40), &"d".repeat(40), "open"),
                    revision(2, "review-preview", &"a".repeat(40), &"b".repeat(40), "open"),
                ],
                "next_cursor": null,
            })),
            HttpOperation::CreateReview { request } => {
                let exact = request.get("selected_baseline").and_then(Value::as_str)
                    == Some("cccccccccccccccccccccccccccccccccccccccc")
                    && request.get("proposal").and_then(Value::as_str)
                        == Some("dddddddddddddddddddddddddddddddddddddddd");
                Ok(json!({
                    "api_version": "v1", "review_id": if exact { "review-comparison" } else { "review-preview" }, "job_id": "analysis:1", "created": exact,
                }))
            }
            HttpOperation::ReviewDetails { review_id } => {
                let (baseline, proposal) = if review_id == "review-comparison" {
                    ("c", "d")
                } else {
                    ("a", "b")
                };
                Ok(json!({
                    "api_version": "v1", "review_id": review_id,
                    "selected_baseline": baseline.repeat(40), "analysis_baseline": baseline.repeat(40),
                    "proposal": proposal.repeat(40), "comparison_mode": "exact", "scope_version": "preview-v1",
                    "stale": false, "capabilities": ["graph", "source", "export"],
                }))
            }
            HttpOperation::ReviewProgress { review_id } => Ok(json!({
                "api_version": "v1", "review_id": review_id, "job_state": "completed",
                "attempt_count": 1, "terminal_reason": null, "coverage": [["indexed", 42], ["partial", 2]], "stale": false,
            })),
            HttpOperation::ChangedFiles { review_id, .. } => Ok(json!({
                "api_version": "v1", "review_id": review_id,
                "items": [
                    {"path":"src/payments.rs","old_path":null,"kind":"modified","additions":18,"deletions":7},
                    {"path":"src/gateway/client.rs","old_path":null,"kind":"added","additions":64,"deletions":0},
                    {"path":"src/events.rs","old_path":null,"kind":"modified","additions":9,"deletions":3},
                    {"path":"config/routes.toml","old_path":null,"kind":"modified","additions":4,"deletions":2}
                ],
                "next_cursor": null,
            })),
            HttpOperation::Findings { review_id, .. } => Ok(json!({
                "api_version": "v1", "review_id": review_id,
                "items": [
                    {"id":1,"kind":"call_edge_added","resolution":"resolved","confidence":0.94,"source_path":"src/payments.rs","source_start_line":43,"source_end_line":43},
                    {"id":2,"kind":"unchanged_caller_may_observe_new_error","resolution":"ambiguous","confidence":0.61,"source_path":"src/events.rs","source_start_line":18,"source_end_line":24}
                ],
                "next_cursor": null,
            })),
            HttpOperation::ReviewAnnotations { review_id } => Ok(json!({
                "api_version":"v1","review_id":review_id,"items":[],
                "delta":{"prior_review_id":"review-old","prior_proposal":"d".repeat(40),"changed_paths":["src/payments.rs","src/events.rs"],"truncated":false}
            })),
            HttpOperation::ReviewAnnotationAudit { review_id } => Ok(json!({
                "api_version":"v1","review_id":review_id,"items":[]
            })),
            HttpOperation::WriteReviewAnnotation { review_id, request } => Ok(json!({
                "id":request["id"],"review_id":review_id,"proposal_commit":"b".repeat(40),
                "owner_principal":"preview-reviewer","visibility":request["visibility"],
                "kind":request["kind"],"target_path":request["target_path"],
                "target_key":request["target_key"],"body":request["body"],
                "status":request["status"],"version":request["expected_version"].as_i64().unwrap_or(0)+1,
                "updated_at_ms":1_790_900_000_000_i64
            })),
            HttpOperation::Graph { query, .. }
                if query.get("symbol").and_then(Value::as_str).is_some() =>
            {
                Ok(json!({
                    "api_version": "v1", "kind": "neighborhood",
                    "edges": [{"id":1,"stable_key":"payment-to-gateway","from_symbol":"process_payment","to_symbol":"create_intent","kind":"call","confidence":0.94,"resolution":"resolved","source_path":"src/payments.rs"}],
                    "next_cursor": null, "omitted": 0, "coverage": [["resolved", 1]],
                }))
            }
            HttpOperation::Graph { query, .. }
                if query.get("path").and_then(Value::as_str).is_some() =>
            {
                let baseline = query.get("side").and_then(Value::as_str) == Some("baseline");
                Ok(json!({
                    "api_version": "v1", "kind": "symbols",
                    "symbols": [{"stable_key":if baseline {"process_payment-v1"} else {"process_payment-v2"},"correspondence_key":"payment","path":"src/payments.rs","kind":"function","name":"process_payment","start_line":41,"start_column":1,"end_line":45,"end_column":2,"blob_id":if baseline {"preview-before"} else {"preview-after"}}],
                    "next_cursor": null,
                }))
            }
            HttpOperation::Graph { .. } => Ok(json!({
                "api_version": "v1", "kind": "children",
                "paths": [["src/payments.rs", "file"], ["src/gateway", "component"], ["src/events.rs", "file"], ["config/routes.toml", "file"]],
                "next_cursor": null,
            })),
            HttpOperation::Source { review_id, query } => {
                let baseline = query.get("side").and_then(Value::as_str) == Some("baseline");
                let (commit, blob, text) = if baseline {
                    (
                        "a",
                        "preview-before",
                        "pub async fn process_payment(req: Payment) {\n  validate(&req)?;\n  let payment = ledger.capture(req).await?;\n  publish(payment.id).await?;\n}",
                    )
                } else {
                    (
                        "b",
                        "preview-after",
                        "pub async fn process_payment(req: Payment) {\n  validate(&req)?;\n  let intent = gateway.create(req).await?;\n  publish(intent.id).await?;\n}",
                    )
                };
                Ok(json!({
                    "api_version": "v1", "review_id": review_id, "commit": commit.repeat(40),
                    "blob_id": blob, "path": "src/payments.rs", "start_line": 41, "end_line": 45,
                    "text": text,
                }))
            }
            _ => Err(HttpFailure::DeniedOrMissing),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3102")
        .await
        .map_err(|error| format!("Cannot bind preview: {error}"))?;
    println!("Workspace: http://127.0.0.1:3102/workspace");
    println!("Session token: {TOKEN}");
    let application = router(Arc::new(PreviewBackend))
        .route(
            "/phase2-pilot",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("phase2_pilot.html"))
            }),
        )
        .route(
            "/phase2-usability-replay",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("phase2_usability_replay.html"))
            }),
        )
        .route(
            "/phase2-responsive-replay",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("phase2_responsive_replay.html"))
            }),
        )
        .route(
            "/phase1-browser-replay",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("phase1_browser_replay.html"))
            }),
        )
        .route(
            "/phase2-navigation-replay",
            axum::routing::get(|| async {
                axum::response::Html(include_str!("phase2_navigation_replay.html"))
            }),
        );
    axum::serve(listener, application)
        .await
        .map_err(|error| format!("Preview failed: {error}"))
}
