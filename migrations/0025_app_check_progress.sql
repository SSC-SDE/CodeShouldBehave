-- One App-owned check follows the immutable comparison through its lifecycle.
-- The first create payload survives a lost response; retries never blindly POST.
CREATE TABLE app_check_progress (
 job_id TEXT PRIMARY KEY REFERENCES app_analysis_jobs(id),
 create_payload TEXT CHECK(create_payload IS NULL OR length(create_payload)<=16384),
 observed_state TEXT NOT NULL DEFAULT '',
 desired_digest TEXT NOT NULL CHECK(length(desired_digest)=64),
 delivered_digest TEXT,
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 3),
 next_at INTEGER NOT NULL
) STRICT;
CREATE TRIGGER app_check_create_payload_immutable BEFORE UPDATE OF create_payload ON app_check_progress
WHEN OLD.create_payload IS NOT NULL AND NEW.create_payload IS NOT OLD.create_payload
BEGIN SELECT RAISE(ABORT,'check create identity is immutable'); END;
