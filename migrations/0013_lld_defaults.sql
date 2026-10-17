-- LLD defaults are per qualified detail; HLD repository defaults stay intact.
CREATE TABLE lld_design_defaults (
    repository_id INTEGER NOT NULL,
    logical_key TEXT NOT NULL CHECK(length(logical_key) <= 4096),
    interpretation TEXT NOT NULL CHECK(interpretation IN ('as_built', 'intended')),
    accepted_id TEXT NOT NULL,
    selected_by TEXT NOT NULL,
    selected_at_ms INTEGER NOT NULL CHECK(selected_at_ms >= 0),
    PRIMARY KEY(repository_id, logical_key, interpretation),
    FOREIGN KEY(repository_id, accepted_id) REFERENCES design_accepted(repository_id, accepted_id)
);
