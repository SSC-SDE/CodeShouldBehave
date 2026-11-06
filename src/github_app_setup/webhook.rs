//! Authenticated, bounded notification inbox. A delivery never grants analysis access.
use super::*;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde_json::{Value, json};
use std::sync::Arc;
const MAX_BODY: usize = 1024 * 1024;
pub struct AppWebhookInbox {
    storage: Mutex<Storage>,
    app_id: i64,
    secret: Vec<u8>,
}
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct WebhookReceipt {
    pub duplicate: bool,
    pub activation: &'static str,
}
#[derive(Debug)]
pub struct WebhookNotification {
    pub delivery_id: String,
    pub installation_id: i64,
    pub repository_id: Option<i64>,
    pub pr_number: Option<i64>,
    pub check_id: Option<i64>,
    pub event: String,
    pub action: String,
    pub lease_id: String,
}
impl AppWebhookInbox {
    pub fn new(storage: Storage, app_id: i64, secret: Vec<u8>) -> Result<Self> {
        if app_id <= 0 || secret.len() < 32 || secret.len() > 1024 {
            return Err(SetupError::Invalid);
        }
        Ok(Self {
            storage: Mutex::new(storage),
            app_id,
            secret,
        })
    }
    pub fn accept(
        &self,
        delivery: &str,
        event: &str,
        signature: &str,
        body: &[u8],
    ) -> Result<WebhookReceipt> {
        if body.len() > MAX_BODY
            || !delivery_id(delivery)
            || !matches!(
                event,
                "installation"
                    | "installation_repositories"
                    | "pull_request"
                    | "check_run"
                    | "repository"
                    | "ping"
            )
        {
            return Err(SetupError::Invalid);
        }
        crate::provider::verify_signature(&self.secret, body, signature)
            .map_err(|_| SetupError::Authentication)?;
        let payload: Value = serde_json::from_slice(body).map_err(|_| SetupError::Invalid)?;
        let installation = if event == "ping" {
            0
        } else {
            positive(&payload["installation"]["id"])?
        };
        if payload["installation"]["app_id"]
            .as_i64()
            .is_some_and(|id| id != self.app_id)
        {
            return Err(SetupError::Denied);
        }
        let action = if event == "ping" {
            "ping"
        } else {
            payload["action"]
                .as_str()
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 64
                        && s.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
                })
                .ok_or(SetupError::Invalid)?
        };
        let repository = if payload.get("repository").is_some() {
            Some(positive(&payload["repository"]["id"])?)
        } else {
            None
        };
        let pr = if event == "pull_request" {
            let n = positive(&payload["number"])?;
            if payload["pull_request"]["number"].as_i64() != Some(n) || repository.is_none() {
                return Err(SetupError::Invalid);
            }
            Some(n)
        } else {
            None
        };
        if event == "check_run"
            && (action != "rerequested"
                || payload["check_run"]["app"]["id"].as_i64() != Some(self.app_id))
        {
            return Err(SetupError::Denied);
        }
        let check_id = if event == "check_run" {
            Some(positive(&payload["check_run"]["id"])?)
        } else {
            None
        };
        // Never trust a webhook's commits or permissions; reconciliation fetches fresh
        // provider state. No source text, PR bodies, titles or user tokens are retained.
        let body_digest = digest(&format!("{event}\n{}", digest_bytes(body)));
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT body_digest FROM app_webhook_inbox WHERE app_id=?1 AND delivery_id=?2",
                params![self.app_id, delivery],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        if let Some(old) = existing {
            return if old == body_digest {
                Ok(WebhookReceipt {
                    duplicate: true,
                    activation: "off",
                })
            } else {
                Err(SetupError::Conflict)
            };
        }
        if event != "ping" {
            let installed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_setup_repositories WHERE app_id=?1 AND installation_id=?2)",params![self.app_id,installation],|r|r.get(0)).map_err(db)?;
            if !installed {
                return Err(SetupError::Denied);
            }
        }
        if let Some(repository) = repository {
            let known:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_setup_repositories WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3)",params![self.app_id,installation,repository],|r|r.get(0)).map_err(db)?;
            if !known {
                return Err(SetupError::Denied);
            }
        }
        let at = now()?;
        if event == "installation" && matches!(action, "deleted" | "suspend") {
            tx.execute("INSERT INTO app_installation_blocks VALUES(?1,?2,?3,?4) ON CONFLICT(app_id,installation_id) DO UPDATE SET reason=excluded.reason,changed_at_s=excluded.changed_at_s",params![self.app_id,installation,action,at]).map_err(db)?;
        }
        let mut removed = Vec::new();
        if event == "installation_repositories" && action == "removed" {
            let entries = payload["repositories_removed"]
                .as_array()
                .filter(|a| a.len() <= 1000)
                .ok_or(SetupError::Invalid)?;
            for entry in entries {
                removed.push(positive(&entry["id"])?);
            }
        }
        if event == "repository" && matches!(action, "deleted" | "privatized") {
            removed.push(repository.ok_or(SetupError::Invalid)?);
        }
        for id in removed {
            tx.execute("INSERT INTO app_repository_blocks SELECT ?1,?2,?3,?4,?5 WHERE EXISTS(SELECT 1 FROM app_setup_repositories WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3) ON CONFLICT(app_id,installation_id,repository_id) DO UPDATE SET reason=excluded.reason,changed_at_s=excluded.changed_at_s",params![self.app_id,installation,id,action,at]).map_err(db)?;
        }
        // Do not reactivate on 'added', 'unsuspend' or delayed 'created' notifications.
        // Fresh consent/reconciliation will be required by the later activation path.
        tx.execute("UPDATE app_webhook_inbox SET state='blocked',lease_id=NULL,lease_expires_at_s=NULL WHERE app_id=?1 AND (EXISTS(SELECT 1 FROM app_installation_blocks b WHERE b.app_id=app_webhook_inbox.app_id AND b.installation_id=app_webhook_inbox.installation_id) OR EXISTS(SELECT 1 FROM app_repository_blocks b WHERE b.app_id=app_webhook_inbox.app_id AND b.installation_id=app_webhook_inbox.installation_id AND b.repository_id=app_webhook_inbox.repository_id)) AND state IN ('pending','leased')",[self.app_id]).map_err(db)?;
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM app_webhook_inbox WHERE app_id=?1",
                [self.app_id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if count >= 1000 {
            // Persist safety blocks even when the inbox is full; GitHub may retry the notification.
            tx.commit().map_err(db)?;
            return Err(SetupError::Capacity);
        }
        let blocked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_installation_blocks WHERE app_id=?1 AND installation_id=?2) OR EXISTS(SELECT 1 FROM app_repository_blocks WHERE app_id=?1 AND installation_id=?2 AND repository_id=?3)",params![self.app_id,installation,repository],|r|r.get(0)).map_err(db)?;
        tx.execute("INSERT INTO app_webhook_inbox(app_id,delivery_id,event,body_digest,installation_id,repository_id,pr_number,check_id,action,received_at_s,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![self.app_id,delivery,event,body_digest,installation,repository,pr,check_id,action,at,if blocked{"blocked"}else if event=="ping"{"processed"}else{"pending"}]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(WebhookReceipt {
            duplicate: false,
            activation: "off",
        })
    }
    /// Claims metadata for reconciliation only. There is deliberately no analyzer,
    /// provider token, grant mutation or publication operation in this adapter.
    pub fn claim(&self) -> Result<Option<WebhookNotification>> {
        let mut storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let at = now()?;
        tx.execute("UPDATE app_webhook_inbox SET state=CASE WHEN attempt>=5 THEN 'blocked' ELSE 'pending' END,lease_id=NULL,lease_expires_at_s=NULL WHERE app_id=?1 AND state='leased' AND lease_expires_at_s<=?2",params![self.app_id,at]).map_err(db)?;
        type PendingRow = (
            String,
            i64,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            String,
            String,
        );
        let row:Option<PendingRow>=tx.query_row("SELECT delivery_id,installation_id,repository_id,pr_number,check_id,event,action FROM app_webhook_inbox WHERE app_id=?1 AND state='pending' ORDER BY attempt,received_at_s,delivery_id LIMIT 1",[self.app_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(db)?;
        let Some((delivery_id, installation_id, repository_id, pr_number, check_id, event, action)) =
            row
        else {
            tx.commit().map_err(db)?;
            return Ok(None);
        };
        let lease_id = secret()?;
        tx.execute("UPDATE app_webhook_inbox SET state='leased',attempt=attempt+1,lease_id=?3,lease_expires_at_s=?4 WHERE app_id=?1 AND delivery_id=?2",params![self.app_id,delivery_id,lease_id,at+60]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(Some(WebhookNotification {
            delivery_id,
            installation_id,
            repository_id,
            pr_number,
            check_id,
            event,
            action,
            lease_id,
        }))
    }
    pub fn finish(&self, notification: &WebhookNotification) -> Result<()> {
        let storage = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let changed=storage.connection.execute("UPDATE app_webhook_inbox SET state='processed',lease_id=NULL,lease_expires_at_s=NULL WHERE app_id=?1 AND delivery_id=?2 AND installation_id=?3 AND state='leased' AND lease_id=?4 AND lease_expires_at_s>?5",params![self.app_id,notification.delivery_id,notification.installation_id,notification.lease_id,now()?]).map_err(db)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(SetupError::Conflict)
        }
    }
}
fn digest_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn positive(v: &Value) -> Result<i64> {
    v.as_i64().filter(|v| *v > 0).ok_or(SetupError::Invalid)
}
fn delivery_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
pub fn webhook_router(inbox: Arc<AppWebhookInbox>) -> Router {
    Router::new()
        .route("/v1/github-app/webhook", post(deliver))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(inbox)
}
async fn deliver(
    State(inbox): State<Arc<AppWebhookInbox>>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let header = |name: &str| {
        let mut values = headers.get_all(name).iter();
        let value = values.next().and_then(|v| v.to_str().ok());
        if values.next().is_some() { None } else { value }
    };
    let (Some(id), Some(event), Some(signature)) = (
        header("x-github-delivery"),
        header("x-github-event"),
        header("x-hub-signature-256"),
    ) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_webhook_headers"})),
        );
    };
    let (id, event, signature) = (id.to_owned(), event.to_owned(), signature.to_owned());
    match tokio::task::spawn_blocking(move || inbox.accept(&id, &event, &signature, &body)).await {
        Ok(Ok(receipt)) => (StatusCode::ACCEPTED, Json(json!(receipt))),
        Ok(Err(e)) => (
            match e {
                SetupError::Authentication | SetupError::Denied => StatusCode::FORBIDDEN,
                SetupError::Capacity => StatusCode::TOO_MANY_REQUESTS,
                SetupError::Conflict => StatusCode::CONFLICT,
                SetupError::Invalid => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            },
            Json(json!({"error":"webhook_not_accepted"})),
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"webhook_not_accepted"})),
        ),
    }
}
