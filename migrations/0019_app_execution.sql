-- No grants are created by migration or onboarding. Activation remains unavailable.
-- A future approved consent transaction must populate these separately from inactive acceptance.
CREATE TABLE app_execution_grants (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 scope_revision INTEGER NOT NULL, consent_digest TEXT NOT NULL CHECK(length(consent_digest)=64),
 engine_digest TEXT NOT NULL CHECK(length(engine_digest)=64),
 engine_metadata_digest TEXT NOT NULL CHECK(length(engine_metadata_digest)=64), policy TEXT NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
 PRIMARY KEY(app_id,installation_id,repository_id),
 FOREIGN KEY(app_id,installation_id,repository_id,scope_revision)
 REFERENCES app_architecture_acceptances(app_id,installation_id,repository_id,scope_revision)
) STRICT;
CREATE TABLE app_analysis_jobs (
 id TEXT PRIMARY KEY CHECK(length(id)=64),
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 pr_number INTEGER NOT NULL, consent_digest TEXT NOT NULL, input TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','leased','failed','cancelled','staged','published','delivered')),
 attempts INTEGER NOT NULL DEFAULT 0, lease TEXT, lease_until INTEGER,
 delivery_attempted INTEGER NOT NULL DEFAULT 0 CHECK(delivery_attempted IN (0,1)), check_id INTEGER,
 created_at INTEGER NOT NULL, available_at INTEGER NOT NULL
) STRICT;
CREATE INDEX app_jobs_pending ON app_analysis_jobs(state,available_at,created_at);
CREATE INDEX app_jobs_tenant ON app_analysis_jobs(app_id,installation_id,repository_id,pr_number);
CREATE TABLE app_report_intents (
 job_id TEXT PRIMARY KEY REFERENCES app_analysis_jobs(id),
 receipt TEXT NOT NULL CHECK(length(receipt)<=16384),
 receipt_digest TEXT NOT NULL UNIQUE CHECK(length(receipt_digest)=64),
 predecessor TEXT, created_at INTEGER NOT NULL,
 FOREIGN KEY(predecessor) REFERENCES app_report_intents(receipt_digest)
) STRICT;
CREATE TRIGGER app_intent_immutable BEFORE UPDATE ON app_report_intents
BEGIN SELECT RAISE(ABORT,'signed App intent is immutable'); END;
