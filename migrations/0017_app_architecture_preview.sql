-- Derived evidence and inactive acceptance only. Never store fetched source here.
CREATE TABLE app_architecture_previews (
 id TEXT PRIMARY KEY CHECK(length(id)=64),
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 scope_revision INTEGER NOT NULL, actor_id INTEGER NOT NULL, expires_at_s INTEGER NOT NULL,
 document TEXT NOT NULL CHECK(length(document)<=2097152),
 FOREIGN KEY(app_id,installation_id,repository_id,scope_revision)
 REFERENCES app_setup_scopes(app_id,installation_id,repository_id,revision)
) STRICT;
CREATE TABLE app_architecture_acceptances (
 app_id INTEGER NOT NULL, installation_id INTEGER NOT NULL, repository_id INTEGER NOT NULL,
 scope_revision INTEGER NOT NULL, actor_id INTEGER NOT NULL, accepted_at_s INTEGER NOT NULL,
 context TEXT NOT NULL CHECK(length(context)<=2097152),
 activation TEXT NOT NULL DEFAULT 'off' CHECK(activation='off'),
 publication_consent INTEGER NOT NULL DEFAULT 0 CHECK(publication_consent=0),
 PRIMARY KEY(app_id,installation_id,repository_id,scope_revision),
 FOREIGN KEY(app_id,installation_id,repository_id,scope_revision)
 REFERENCES app_setup_scopes(app_id,installation_id,repository_id,revision)
) STRICT;
CREATE TRIGGER app_architecture_immutable BEFORE UPDATE ON app_architecture_acceptances
BEGIN SELECT RAISE(ABORT,'architecture acceptance is immutable'); END;
