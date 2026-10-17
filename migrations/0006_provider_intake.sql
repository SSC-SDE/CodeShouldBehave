CREATE TABLE pr_revisions (
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    provider_pr_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    review_id TEXT REFERENCES reviews(id),
    base_commit TEXT NOT NULL,
    head_commit TEXT NOT NULL,
    target_branch TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('open','draft','closed')),
    observed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (repository_id, provider_pr_id, generation)
);

ALTER TABLE event_deliveries ADD COLUMN failure_reason TEXT;

CREATE INDEX pr_revisions_current
    ON pr_revisions(repository_id, provider_pr_id, generation DESC);
CREATE INDEX event_deliveries_state
    ON event_deliveries(state, accepted_at_ms);
