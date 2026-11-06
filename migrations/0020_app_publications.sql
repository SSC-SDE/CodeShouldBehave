-- App-owned report catalog. No public route is mounted by migration.
CREATE TABLE app_report_publications (
 app_id INTEGER NOT NULL, job_id TEXT PRIMARY KEY REFERENCES app_report_intents(job_id),
 report_id TEXT NOT NULL CHECK(length(report_id)=64),
 manifest_digest TEXT NOT NULL CHECK(length(manifest_digest)=64),
 expires_at INTEGER NOT NULL, withdrawn INTEGER NOT NULL DEFAULT 0 CHECK(withdrawn IN (0,1)),
 UNIQUE(app_id,report_id)
) STRICT;
CREATE TRIGGER app_publication_no_restore BEFORE UPDATE ON app_report_publications
WHEN OLD.withdrawn=1 AND NEW.withdrawn!=1
BEGIN SELECT RAISE(ABORT,'withdrawal cannot be reversed'); END;
CREATE TRIGGER app_publication_identity BEFORE UPDATE ON app_report_publications
WHEN OLD.app_id!=NEW.app_id OR OLD.job_id!=NEW.job_id OR OLD.report_id!=NEW.report_id OR OLD.manifest_digest!=NEW.manifest_digest OR OLD.expires_at!=NEW.expires_at
BEGIN SELECT RAISE(ABORT,'publication identity is immutable'); END;
