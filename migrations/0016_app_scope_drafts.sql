-- Inactive setup only. This is not an analysis grant or publication consent.
CREATE TABLE app_setup_scope_previews (
    id TEXT PRIMARY KEY CHECK(length(id)=64),
    app_id INTEGER NOT NULL,
    installation_id INTEGER NOT NULL,
    repository_id INTEGER NOT NULL,
    actor_id INTEGER NOT NULL CHECK(actor_id>0),
    expected_revision INTEGER NOT NULL CHECK(expected_revision>=0),
    expires_at_s INTEGER NOT NULL,
    document TEXT NOT NULL CHECK(length(document)<=524288),
    FOREIGN KEY(app_id,installation_id,repository_id)
      REFERENCES app_setup_repositories(app_id,installation_id,repository_id)
) STRICT;
CREATE INDEX app_scope_preview_expiry ON app_setup_scope_previews(expires_at_s);
CREATE TABLE app_setup_scopes (
    app_id INTEGER NOT NULL,
    installation_id INTEGER NOT NULL,
    repository_id INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    actor_id INTEGER NOT NULL CHECK(actor_id>0),
    confirmed_at_s INTEGER NOT NULL,
    activation TEXT NOT NULL DEFAULT 'off' CHECK(activation='off'),
    document TEXT NOT NULL CHECK(length(document)<=524288),
    PRIMARY KEY(app_id,installation_id,repository_id,revision),
    FOREIGN KEY(app_id,installation_id,repository_id)
      REFERENCES app_setup_repositories(app_id,installation_id,repository_id)
) STRICT;
CREATE TRIGGER app_setup_scopes_immutable_update BEFORE UPDATE ON app_setup_scopes
BEGIN SELECT RAISE(ABORT,'scope history is immutable'); END;
