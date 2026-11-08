-- Operator-only activation. Migration creates no grant or approval.
ALTER TABLE app_execution_grants ADD COLUMN activation_revision INTEGER NOT NULL DEFAULT 0 CHECK(activation_revision>=0);
CREATE TABLE app_execution_activations (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 revision INTEGER NOT NULL CHECK(revision>0), consent_revision INTEGER NOT NULL,
 consent_digest TEXT NOT NULL CHECK(length(consent_digest)=64),
 approval_digest TEXT NOT NULL UNIQUE CHECK(length(approval_digest)=64),
 document TEXT NOT NULL, approval TEXT NOT NULL, engine_evidence TEXT NOT NULL,
 host_evidence TEXT NOT NULL, committed_at INTEGER NOT NULL,
 PRIMARY KEY(app_id,installation_id,repository_id,revision),
 FOREIGN KEY(app_id,installation_id,repository_id,consent_revision)
 REFERENCES app_publication_consents(app_id,installation_id,repository_id,revision)
) STRICT;
CREATE TRIGGER app_activation_immutable BEFORE UPDATE ON app_execution_activations
BEGIN SELECT RAISE(ABORT,'operator activation audit is immutable'); END;
CREATE TRIGGER app_activation_no_delete BEFORE DELETE ON app_execution_activations
BEGIN SELECT RAISE(ABORT,'operator activation audit cannot be deleted'); END;
