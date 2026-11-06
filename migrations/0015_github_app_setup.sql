-- App identity scaffolding only. No consent, scans, jobs or activation are implied.
CREATE TABLE app_setup_oauth_states (
    state_digest TEXT PRIMARY KEY CHECK(length(state_digest) = 64),
    browser_digest TEXT NOT NULL CHECK(length(browser_digest) = 64),
    installation_hint INTEGER CHECK(installation_hint > 0),
    expires_at_s INTEGER NOT NULL CHECK(expires_at_s > 0)
) STRICT;
CREATE INDEX app_setup_state_expiry ON app_setup_oauth_states(expires_at_s);

CREATE TABLE app_setup_repositories (
    app_id INTEGER NOT NULL CHECK(app_id > 0),
    installation_id INTEGER NOT NULL CHECK(installation_id > 0),
    repository_id INTEGER NOT NULL CHECK(repository_id > 0),
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    selected_by_user_id INTEGER NOT NULL CHECK(selected_by_user_id > 0),
    selected_at_s INTEGER NOT NULL CHECK(selected_at_s > 0),
    state TEXT NOT NULL DEFAULT 'installed_unconfigured'
      CHECK(state = 'installed_unconfigured'),
    PRIMARY KEY(app_id, installation_id, repository_id)
) STRICT;
