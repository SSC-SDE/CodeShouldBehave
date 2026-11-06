-- Recovery is an explicit terminal decision, never a retry or accounting reset.
CREATE TABLE app_recovery_decisions (
 job_id TEXT NOT NULL REFERENCES app_analysis_jobs(id),
 app_id INTEGER NOT NULL,
 decision TEXT NOT NULL CHECK(decision IN ('reconciled','reconciled_progress','abandoned')),
 check_id INTEGER,
 actor TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 128),
 reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 1024),
 created_at INTEGER NOT NULL,
 analysis_attempts INTEGER NOT NULL CHECK(analysis_attempts>=0),
 delivery_attempts INTEGER NOT NULL CHECK(delivery_attempts>=0),
 create_attempted INTEGER NOT NULL CHECK(create_attempted IN (0,1)),
 PRIMARY KEY(job_id,decision),
 CHECK((decision IN ('reconciled','reconciled_progress') AND check_id>0) OR (decision='abandoned' AND check_id IS NULL))
) STRICT;
CREATE TRIGGER app_recovery_identity BEFORE INSERT ON app_recovery_decisions
WHEN NOT EXISTS(SELECT 1 FROM app_analysis_jobs j WHERE j.id=NEW.job_id AND j.app_id=NEW.app_id
 AND j.attempts=NEW.analysis_attempts AND j.delivery_attempts=NEW.delivery_attempts
 AND j.delivery_attempted=NEW.create_attempted)
BEGIN SELECT RAISE(ABORT,'recovery must bind exact retained job counters'); END;
CREATE TRIGGER app_recovery_immutable BEFORE UPDATE ON app_recovery_decisions
BEGIN SELECT RAISE(ABORT,'recovery decisions are immutable'); END;
CREATE TRIGGER app_recovery_retained BEFORE DELETE ON app_recovery_decisions
BEGIN SELECT RAISE(ABORT,'recovery decisions are retained'); END;
CREATE TRIGGER app_recovery_terminal BEFORE UPDATE ON app_analysis_jobs
WHEN EXISTS(SELECT 1 FROM app_recovery_decisions r WHERE r.job_id=OLD.id AND
 ((r.decision='abandoned' AND NEW.state!='cancelled') OR
  (r.decision='reconciled' AND (NEW.state!='delivered' OR NEW.check_id IS NOT r.check_id))))
BEGIN SELECT RAISE(ABORT,'operator recovery decision is terminal'); END;
CREATE TRIGGER app_attempts_monotonic BEFORE UPDATE ON app_analysis_jobs
WHEN NEW.attempts<OLD.attempts OR NEW.delivery_attempts<OLD.delivery_attempts
 OR NEW.delivery_attempted<OLD.delivery_attempted
 OR NEW.id!=OLD.id OR NEW.app_id!=OLD.app_id OR NEW.input!=OLD.input
 OR NEW.installation_id!=OLD.installation_id OR NEW.repository_id!=OLD.repository_id
 OR NEW.pr_number!=OLD.pr_number OR NEW.consent_digest!=OLD.consent_digest
 OR NEW.created_at!=OLD.created_at
 OR (OLD.check_id IS NOT NULL AND NEW.check_id IS NOT OLD.check_id)
BEGIN SELECT RAISE(ABORT,'job identity and lifetime attempts cannot be reset'); END;
CREATE TRIGGER app_attempt_history_retained BEFORE DELETE ON app_analysis_jobs
BEGIN SELECT RAISE(ABORT,'job lifetime attempts are retained'); END;
CREATE TRIGGER app_budget_history_retained BEFORE DELETE ON app_request_budgets
BEGIN SELECT RAISE(ABORT,'request reservations cannot be forgotten'); END;
CREATE TRIGGER app_consent_history_retained BEFORE DELETE ON app_publication_consents
BEGIN SELECT RAISE(ABORT,'consent revocations cannot be forgotten'); END;
CREATE TRIGGER app_publication_history_retained BEFORE DELETE ON app_report_publications
BEGIN SELECT RAISE(ABORT,'publication withdrawals cannot be forgotten'); END;
