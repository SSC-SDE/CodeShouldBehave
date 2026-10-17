CREATE TABLE export_jobs (
    id INTEGER PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    requested_by TEXT NOT NULL,
    format_version TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued','ready','failed')),
    artifact_id INTEGER REFERENCES artifact_refs(id),
    failure_reason TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    UNIQUE (review_id, requested_by, format_version)
);

CREATE INDEX export_jobs_by_review ON export_jobs(review_id, id);
