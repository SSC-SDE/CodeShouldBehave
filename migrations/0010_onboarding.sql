CREATE TABLE onboarding_sessions (
    security_domain TEXT NOT NULL,
    session_id TEXT NOT NULL,
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    scope_id INTEGER NOT NULL REFERENCES scopes(id),
    manifest_fingerprint TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    payload BLOB NOT NULL CHECK(length(payload) <= 1048576),
    PRIMARY KEY(security_domain, session_id)
);

CREATE TABLE onboarding_history (
    security_domain TEXT NOT NULL,
    session_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    actor TEXT NOT NULL,
    recorded_at_ms INTEGER NOT NULL CHECK(recorded_at_ms >= 0),
    payload BLOB NOT NULL CHECK(length(payload) <= 1048576),
    PRIMARY KEY(security_domain, session_id, revision),
    FOREIGN KEY(security_domain, session_id) REFERENCES onboarding_sessions(security_domain, session_id)
);
