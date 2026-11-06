//! Durable queued/running/failure UI on the same immutable App-owned check.
use super::*;

fn progress_payload(id: &str, input: &AnalysisInput, state: &str) -> Result<Value> {
    let (status, title) = match state {
        "pending" => ("queued", "Queued · Scoped analysis pending"),
        "leased" => ("in_progress", "Running · Scoped analysis"),
        "staged" | "published" => ("in_progress", "Publishing · Report verification pending"),
        "failed" => (
            "completed",
            "Failed · Analysis or report delivery unavailable",
        ),
        _ => return Err(SetupError::Conflict),
    };
    let mut value = json!({"name":"CodeShouldBehave / Architecture",
        "head_sha":input.comparison.head,"external_id":format!("csb-app-v1:{id}"),
        "status":status,"output":{"title":title,
        "summary":format!("Exact baseline: `{}` → proposal: `{}`.\n\nNo completed architecture result is available. Static analysis does not approve merging. Failure or exhausted capacity requires operator review; no retry resets the lifetime budget.", input.comparison.selected_base,input.comparison.head)}});
    if state == "failed" {
        value["conclusion"] = json!("failure");
    }
    Ok(value)
}

impl AppExecutionQueue {
    /// Select one eligible progress update. Failed/incomplete checks never imply safety.
    pub fn pending_progress(&self) -> Result<Option<String>> {
        let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let at = now()?;
        let mut q = s.connection.prepare("SELECT j.id,j.input,j.state FROM app_analysis_jobs j JOIN app_execution_grants g ON g.app_id=j.app_id AND g.installation_id=j.installation_id AND g.repository_id=j.repository_id AND g.enabled=1 AND g.consent_digest=j.consent_digest LEFT JOIN app_check_progress p ON p.job_id=j.id WHERE j.app_id=?1 AND j.state IN ('pending','leased','staged','published','failed') AND (p.job_id IS NULL OR p.observed_state!=j.state OR ((p.delivered_digest IS NULL OR p.delivered_digest!=p.desired_digest) AND p.attempts<3 AND p.next_at<=?2)) ORDER BY coalesce(p.next_at,0),j.created_at,j.id").map_err(db)?;
        let rows = q
            .query_map(params![self.app_id, at], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(db)?;
        for row in rows {
            let (id, raw, state) = row.map_err(db)?;
            let input: AnalysisInput =
                serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
            match current_grant(&s.connection, &input) {
                Ok(_) => {}
                Err(SetupError::Denied) => continue,
                Err(e) => return Err(e),
            }
            let hash = sha256(
                &canonical_json(&progress_payload(&id, &input, &state)?)
                    .map_err(|_| SetupError::Invalid)?,
            );
            let progress: Option<(String,Option<String>,i64,i64)> = s.connection.query_row("SELECT desired_digest,delivered_digest,attempts,next_at FROM app_check_progress WHERE job_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db)?;
            if progress.is_none_or(|(desired, delivered, attempts, next)| {
                delivered.as_ref() != Some(&hash)
                    && (desired != hash || (attempts < 3 && next <= at))
            }) {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
    pub fn progress_configuration(&self, id: &str) -> Result<(AnalysisInput, PublicPolicy)> {
        let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let raw: String = s
            .connection
            .query_row(
                "SELECT input FROM app_analysis_jobs WHERE app_id=?1 AND id=?2",
                params![self.app_id, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?
            .ok_or(SetupError::Conflict)?;
        let input: AnalysisInput = serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
        let policy = current_grant(&s.connection, &input)?.policy;
        Ok((input, policy))
    }
    /// Three bounded attempts per observed status. Provider reservations are charged
    /// by the caller before entry; state transitions never refund job budgets.
    pub fn deliver_progress(
        &self,
        id: &str,
        provider: &dyn AppExecutionProvider,
        writer: &dyn AppCheckDelivery,
    ) -> Result<i64> {
        let (input, payload, hash) = {
            let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
            let tx = s
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db)?;
            let (raw, state): (String, String) = tx
                .query_row(
                    "SELECT input,state FROM app_analysis_jobs WHERE app_id=?1 AND id=?2",
                    params![self.app_id, id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(db)?
                .ok_or(SetupError::Conflict)?;
            let input: AnalysisInput =
                serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
            current_grant(&tx, &input)?;
            let payload = progress_payload(id, &input, &state)?;
            let hash = sha256(&canonical_json(&payload).map_err(|_| SetupError::Invalid)?);
            let at = now()?;
            tx.execute("INSERT INTO app_check_progress(job_id,desired_digest,observed_state,attempts,next_at) VALUES(?1,?2,?3,0,0) ON CONFLICT(job_id) DO UPDATE SET observed_state=excluded.observed_state,desired_digest=excluded.desired_digest,attempts=CASE WHEN desired_digest!=excluded.desired_digest THEN 0 ELSE attempts END,next_at=CASE WHEN desired_digest!=excluded.desired_digest THEN 0 ELSE next_at END",params![id,hash,state]).map_err(db)?;
            let changed=tx.execute("UPDATE app_check_progress SET attempts=attempts+1,next_at=?3+600 WHERE job_id=?1 AND desired_digest=?2 AND (delivered_digest IS NULL OR delivered_digest!=?2) AND attempts<3 AND next_at<=?3",params![id,hash,at]).map_err(db)?;
            if changed != 1 {
                return Err(SetupError::Conflict);
            }
            tx.commit().map_err(db)?;
            (input, payload, hash)
        };
        let check = self.send_check(id, &input, &payload, false, provider, writer)?;
        let s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        s.connection.execute("UPDATE app_check_progress SET delivered_digest=?2 WHERE job_id=?1 AND desired_digest=?2",params![id,hash]).map_err(db)?;
        Ok(check)
    }
    /// A quota/credential failure can happen before deliver_progress records an
    /// attempt. Persist its delay too, so one blocked job cannot monopolize UI IO.
    pub(crate) fn backoff_progress(&self, id: &str) -> Result<()> {
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let (raw, state): (String, String) = tx
            .query_row(
                "SELECT input,state FROM app_analysis_jobs WHERE app_id=?1 AND id=?2",
                params![self.app_id, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db)?
            .ok_or(SetupError::Conflict)?;
        if matches!(state.as_str(), "cancelled" | "delivered") {
            return Ok(());
        }
        let input: AnalysisInput = serde_json::from_str(&raw).map_err(|_| SetupError::Invalid)?;
        let hash = sha256(
            &canonical_json(&progress_payload(id, &input, &state)?)
                .map_err(|_| SetupError::Invalid)?,
        );
        tx.execute("INSERT INTO app_check_progress(job_id,desired_digest,observed_state,attempts,next_at) VALUES(?1,?2,?3,0,?4) ON CONFLICT(job_id) DO UPDATE SET observed_state=excluded.observed_state,desired_digest=excluded.desired_digest,attempts=CASE WHEN desired_digest!=excluded.desired_digest THEN 0 ELSE attempts END,next_at=max(next_at,excluded.next_at)",params![id,hash,state,now()?+600]).map_err(db)?;
        tx.commit().map_err(db)
    }
    /// All lifecycle writes share the original durable create fence. PATCH retries
    /// are idempotent; unknown POST outcomes only reconcile the exact first payload.
    pub(super) fn send_check(
        &self,
        id: &str,
        input: &AnalysisInput,
        payload: &Value,
        terminal: bool,
        provider: &dyn AppExecutionProvider,
        writer: &dyn AppCheckDelivery,
    ) -> Result<i64> {
        if input.app_id != self.app_id || input.id()? != id {
            return Err(SetupError::Denied);
        }
        let mut s = self.storage.lock().map_err(|_| SetupError::Unavailable)?;
        let tx = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        current_grant(&tx, input)?;
        recheck(input, provider)?;
        let (attempted, known, state):(bool,Option<i64>,String)=tx.query_row("SELECT delivery_attempted,check_id,state FROM app_analysis_jobs WHERE app_id=?1 AND id=?2",params![self.app_id,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db)?.ok_or(SetupError::Conflict)?;
        if terminal {
            if state == "delivered" {
                return known.ok_or(SetupError::Conflict);
            }
            if state != "published" {
                return Err(SetupError::Conflict);
            }
        } else if progress_payload(id, input, &state)? != *payload {
            return Err(SetupError::Conflict);
        }
        let initial: Option<String> = tx
            .query_row(
                "SELECT create_payload FROM app_check_progress WHERE job_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?
            .flatten();
        let initial_payload = initial
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|_| SetupError::Invalid)?;
        let found = match known {
            Some(check) => Some(check),
            None => writer.find(
                input.repository_id,
                &input.comparison.head,
                initial_payload.as_ref().unwrap_or(payload),
            )?,
        };
        let create = found.is_none();
        if create {
            if attempted {
                return Err(SetupError::Conflict);
            }
            let raw = serde_json::to_string(payload).map_err(|_| SetupError::Invalid)?;
            tx.execute("INSERT INTO app_check_progress(job_id,create_payload,desired_digest,attempts,next_at) VALUES(?1,?2,?3,0,0) ON CONFLICT(job_id) DO UPDATE SET create_payload=excluded.create_payload",params![id,raw,sha256(&canonical_json(payload).map_err(|_|SetupError::Invalid)?)]).map_err(db)?;
            tx.execute(
                "UPDATE app_analysis_jobs SET delivery_attempted=1 WHERE id=?1",
                [id],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        let gate = s
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        current_grant(&gate, input)?;
        recheck(input, provider)?;
        let current: String = gate
            .query_row(
                "SELECT state FROM app_analysis_jobs WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if current != state {
            return Err(SetupError::Conflict);
        }
        let check = if let Some(check) = found {
            writer.update(input.repository_id, check, payload)?
        } else {
            writer.create(input.repository_id, payload)?
        };
        if check <= 0 {
            return Err(SetupError::Invalid);
        }
        if terminal {
            gate.execute("UPDATE app_analysis_jobs SET state='delivered',check_id=?2 WHERE id=?1 AND state='published'",params![id,check]).map_err(db)?;
        } else {
            gate.execute(
                "UPDATE app_analysis_jobs SET check_id=?2 WHERE id=?1",
                params![id, check],
            )
            .map_err(db)?;
        }
        gate.commit().map_err(db)?;
        Ok(check)
    }
}
