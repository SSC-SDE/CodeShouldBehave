CREATE TABLE ai_audit_events (
    id INTEGER PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL,
    principal TEXT NOT NULL,
    destination TEXT NOT NULL,
    disclosure TEXT NOT NULL CHECK (disclosure IN ('code_content','symbols_only','derived_summary')),
    capability TEXT NOT NULL CHECK (capability IN ('disabled','fake_only','unavailable')),
    outcome TEXT NOT NULL CHECK (outcome IN ('prepared','disabled','accepted','rejected','cancelled')),
    provider TEXT,
    model TEXT,
    rule_version TEXT NOT NULL,
    evidence_count INTEGER NOT NULL CHECK (evidence_count >= 0),
    evidence_bytes INTEGER NOT NULL CHECK (evidence_bytes >= 0),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (bundle_id, outcome, created_at_ms)
);

CREATE INDEX ai_audit_by_review ON ai_audit_events(review_id, id);
