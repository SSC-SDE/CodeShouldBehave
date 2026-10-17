CREATE TABLE review_annotations (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    proposal_commit TEXT NOT NULL,
    owner_principal TEXT NOT NULL,
    visibility TEXT NOT NULL CHECK (visibility IN ('private','shared')),
    kind TEXT NOT NULL CHECK (kind IN ('reviewed','bookmark','question','note','handoff')),
    target_path TEXT,
    target_key TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active','resolved','outdated')),
    version INTEGER NOT NULL CHECK (version > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (review_id, owner_principal, kind, target_key)
);

CREATE INDEX review_annotations_visible
    ON review_annotations(review_id, visibility, owner_principal, updated_at_ms, id);

CREATE TABLE review_annotation_audit (
    id INTEGER PRIMARY KEY,
    annotation_id TEXT NOT NULL REFERENCES review_annotations(id) ON DELETE CASCADE,
    review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    actor_principal TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('created','updated','resolved','reopened','handed_off')),
    from_version INTEGER,
    to_version INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX review_annotation_audit_by_review
    ON review_annotation_audit(review_id, id);
