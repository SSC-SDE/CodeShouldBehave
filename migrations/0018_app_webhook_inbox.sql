-- Notifications are not analysis authority. No enabled state exists here.
CREATE TABLE app_webhook_inbox (
 app_id INTEGER NOT NULL, delivery_id TEXT NOT NULL,
 event TEXT NOT NULL, body_digest TEXT NOT NULL CHECK(length(body_digest)=64),
 installation_id INTEGER NOT NULL, repository_id INTEGER, pr_number INTEGER, check_id INTEGER,
 action TEXT NOT NULL, received_at_s INTEGER NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','leased','processed','blocked')),
 attempt INTEGER NOT NULL DEFAULT 0, lease_id TEXT, lease_expires_at_s INTEGER,
 PRIMARY KEY(app_id,delivery_id)
) STRICT;
CREATE INDEX app_webhook_pending ON app_webhook_inbox(app_id,state,received_at_s);
CREATE TABLE app_installation_blocks (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL,
 reason TEXT NOT NULL, changed_at_s INTEGER NOT NULL,
 PRIMARY KEY(app_id,installation_id)
) STRICT;
CREATE TABLE app_repository_blocks (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 reason TEXT NOT NULL, changed_at_s INTEGER NOT NULL,
 PRIMARY KEY(app_id,installation_id,repository_id)
) STRICT;
