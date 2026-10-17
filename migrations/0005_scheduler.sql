ALTER TABLE pr_heads ADD COLUMN current_review_id TEXT REFERENCES reviews(id);

ALTER TABLE jobs ADD COLUMN lane TEXT NOT NULL DEFAULT 'warm';
ALTER TABLE jobs ADD COLUMN provider_pr_id TEXT;
ALTER TABLE jobs ADD COLUMN pr_generation INTEGER;
ALTER TABLE jobs ADD COLUMN superseded_by_review_id TEXT REFERENCES reviews(id);
ALTER TABLE jobs ADD COLUMN enqueued_at_ms INTEGER NOT NULL DEFAULT 0;

CREATE TABLE current_results (
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    provider_pr_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    review_id TEXT NOT NULL REFERENCES reviews(id),
    job_id INTEGER NOT NULL REFERENCES jobs(id),
    lease_generation INTEGER NOT NULL,
    result_identity TEXT NOT NULL,
    published_at_ms INTEGER NOT NULL,
    PRIMARY KEY (repository_id, provider_pr_id, kind)
);

CREATE INDEX jobs_scheduler_claim
    ON jobs(state, lane, available_at_ms, priority DESC, id);
CREATE INDEX jobs_repository_state
    ON jobs(state, review_id, lane);
CREATE INDEX attempts_by_outcome ON attempts(outcome, finished_at_ms, job_id);
