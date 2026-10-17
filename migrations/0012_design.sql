-- HLD facts and accepted interpretations are separate immutable records.
CREATE TABLE design_versions (
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    design_id TEXT NOT NULL,
    scope_id INTEGER NOT NULL REFERENCES scopes(id),
    onboarding_context_id TEXT NOT NULL,
    input_fingerprint TEXT NOT NULL,
    payload BLOB NOT NULL CHECK(length(payload) <= 1048576),
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
    PRIMARY KEY(repository_id, design_id)
);

-- The cursor is mutable; each review revision remains an immutable audit entry.
CREATE TABLE design_reviews (
    repository_id INTEGER NOT NULL,
    review_id TEXT NOT NULL,
    design_id TEXT NOT NULL,
    principal TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    payload BLOB NOT NULL CHECK(length(payload) <= 1048576),
    PRIMARY KEY(repository_id, review_id),
    FOREIGN KEY(repository_id, design_id) REFERENCES design_versions(repository_id, design_id)
);
CREATE TABLE design_review_history (
    repository_id INTEGER NOT NULL,
    review_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 0),
    actor TEXT NOT NULL,
    recorded_at_ms INTEGER NOT NULL CHECK(recorded_at_ms >= 0),
    payload BLOB NOT NULL CHECK(length(payload) <= 1048576),
    PRIMARY KEY(repository_id, review_id, revision),
    FOREIGN KEY(repository_id, review_id) REFERENCES design_reviews(repository_id, review_id)
);

CREATE TABLE design_accepted (
    repository_id INTEGER NOT NULL,
    accepted_id TEXT NOT NULL,
    design_id TEXT NOT NULL,
    review_id TEXT NOT NULL,
    review_revision INTEGER NOT NULL CHECK(review_revision >= 0),
    interpretation TEXT NOT NULL CHECK(interpretation IN ('as_built', 'intended')),
    actor TEXT NOT NULL,
    accepted_at_ms INTEGER NOT NULL CHECK(accepted_at_ms >= 0),
    payload BLOB NOT NULL CHECK(length(payload) <= 1048576),
    PRIMARY KEY(repository_id, accepted_id),
    FOREIGN KEY(repository_id, design_id) REFERENCES design_versions(repository_id, design_id),
    FOREIGN KEY(repository_id, review_id, review_revision)
        REFERENCES design_review_history(repository_id, review_id, revision)
);
CREATE TABLE design_defaults (
    repository_id INTEGER NOT NULL,
    interpretation TEXT NOT NULL CHECK(interpretation IN ('as_built', 'intended')),
    accepted_id TEXT NOT NULL,
    selected_by TEXT NOT NULL,
    selected_at_ms INTEGER NOT NULL CHECK(selected_at_ms >= 0),
    PRIMARY KEY(repository_id, interpretation),
    FOREIGN KEY(repository_id, accepted_id) REFERENCES design_accepted(repository_id, accepted_id)
);
