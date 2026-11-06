-- Discovery never creates or enables execution grants. All numbers are hints;
-- fresh consent-fenced reconciliation alone establishes analysis identity.
CREATE TABLE app_pr_discovery_cursors (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 grant_fingerprint TEXT NOT NULL CHECK(length(grant_fingerprint)=64),
 next_page INTEGER NOT NULL DEFAULT 1 CHECK(next_page>0),
 sweep INTEGER NOT NULL DEFAULT 0 CHECK(sweep>=0),
 last_number INTEGER NOT NULL DEFAULT 0 CHECK(last_number>=0),
 next_scan_at INTEGER NOT NULL DEFAULT 0,
 turn INTEGER NOT NULL DEFAULT 0,
 lease TEXT, lease_until INTEGER NOT NULL DEFAULT 0,
 last_pass_at INTEGER, last_error TEXT,
 PRIMARY KEY(app_id,installation_id,repository_id)
) STRICT;
CREATE TABLE app_pr_discovery_pending (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 pr_number INTEGER NOT NULL CHECK(pr_number>0),
 available_at INTEGER NOT NULL DEFAULT 0, turn INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(app_id,installation_id,repository_id,pr_number),
 FOREIGN KEY(app_id,installation_id,repository_id)
 REFERENCES app_pr_discovery_cursors(app_id,installation_id,repository_id)
) STRICT;
CREATE INDEX app_pr_discovery_due ON app_pr_discovery_pending(app_id,installation_id,repository_id,available_at,turn,pr_number);
