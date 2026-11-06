-- Consent is recorded separately from operator activation. No enabled grants.
CREATE TABLE app_publication_consents (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 revision INTEGER NOT NULL, actor_id INTEGER NOT NULL, created_at INTEGER NOT NULL,
 document TEXT NOT NULL, digest TEXT NOT NULL CHECK(length(digest)=64),
 revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
 PRIMARY KEY(app_id,installation_id,repository_id,revision)
) STRICT;
CREATE TRIGGER app_consent_immutable BEFORE UPDATE ON app_publication_consents
WHEN NEW.app_id!=OLD.app_id OR NEW.installation_id!=OLD.installation_id OR NEW.repository_id!=OLD.repository_id OR NEW.revision!=OLD.revision OR NEW.actor_id!=OLD.actor_id OR NEW.created_at!=OLD.created_at OR NEW.document!=OLD.document OR NEW.digest!=OLD.digest OR NEW.revoked<OLD.revoked
BEGIN SELECT RAISE(ABORT,'consent identity and revocation are immutable'); END;
CREATE TABLE app_request_budgets (
 app_id INTEGER NOT NULL, bucket TEXT NOT NULL, used INTEGER NOT NULL CHECK(used>=0),
 PRIMARY KEY(app_id,bucket)
) STRICT;
CREATE TRIGGER app_budget_monotonic BEFORE UPDATE ON app_request_budgets
WHEN NEW.used<OLD.used OR NEW.app_id!=OLD.app_id OR NEW.bucket!=OLD.bucket
BEGIN SELECT RAISE(ABORT,'request reservations cannot be refunded'); END;
CREATE TABLE app_dispatch_turns (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, turn INTEGER NOT NULL,
 PRIMARY KEY(app_id,installation_id)
) STRICT;
CREATE TABLE app_dispatch_mode (app_id INTEGER PRIMARY KEY, analysis_next INTEGER NOT NULL CHECK(analysis_next IN (0,1))) STRICT;
ALTER TABLE app_analysis_jobs ADD COLUMN delivery_attempts INTEGER NOT NULL DEFAULT 0;
CREATE TABLE app_notification_turns (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, turn INTEGER NOT NULL,
 PRIMARY KEY(app_id,installation_id)
) STRICT;
CREATE TABLE app_reconcile_schedule (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 pr_number INTEGER NOT NULL, next_at INTEGER NOT NULL,
 PRIMARY KEY(app_id,installation_id,repository_id,pr_number)
) STRICT;
