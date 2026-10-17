-- Immutable accepted-only records; selecting one never modifies an existing review.
CREATE TABLE onboarding_accepted_contexts (
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    context_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    session_revision INTEGER NOT NULL CHECK(session_revision >= 0),
    actor TEXT NOT NULL,
    accepted_at_ms INTEGER NOT NULL CHECK(accepted_at_ms >= 0),
    payload BLOB NOT NULL CHECK(length(payload) <= 1048576),
    PRIMARY KEY(repository_id, context_id)
);
CREATE TABLE onboarding_defaults (
    repository_id INTEGER PRIMARY KEY REFERENCES repositories(id),
    context_id TEXT NOT NULL,
    selected_by TEXT NOT NULL,
    selected_at_ms INTEGER NOT NULL CHECK(selected_at_ms >= 0),
    FOREIGN KEY(repository_id, context_id)
        REFERENCES onboarding_accepted_contexts(repository_id, context_id)
);
