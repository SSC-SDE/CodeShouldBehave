CREATE TABLE change_sets (
    id TEXT PRIMARY KEY,
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    baseline_snapshot_id INTEGER NOT NULL REFERENCES snapshots(id),
    proposal_snapshot_id INTEGER NOT NULL REFERENCES snapshots(id),
    baseline_commit TEXT NOT NULL,
    proposal_commit TEXT NOT NULL,
    resolution_strategy TEXT NOT NULL CHECK (resolution_strategy IN ('local','full')),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (repository_id, baseline_snapshot_id, proposal_snapshot_id, id)
);

CREATE TABLE change_paths (
    change_set_id TEXT NOT NULL REFERENCES change_sets(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('added','deleted','modified','renamed')),
    path TEXT NOT NULL,
    old_path TEXT,
    parse_required INTEGER NOT NULL CHECK (parse_required IN (0,1)),
    PRIMARY KEY (change_set_id, position)
);

CREATE TABLE change_invalidations (
    change_set_id TEXT NOT NULL REFERENCES change_sets(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    path TEXT,
    target_path TEXT,
    PRIMARY KEY (change_set_id, position)
);

CREATE TABLE change_reused_paths (
    change_set_id TEXT NOT NULL REFERENCES change_sets(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    PRIMARY KEY (change_set_id, path)
);

CREATE TABLE change_affected_callers (
    change_set_id TEXT NOT NULL REFERENCES change_sets(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    PRIMARY KEY (change_set_id, path)
);

CREATE INDEX change_sets_by_proposal
    ON change_sets(repository_id, proposal_snapshot_id, baseline_snapshot_id);
CREATE INDEX change_paths_by_set_kind ON change_paths(change_set_id, kind, position);

ALTER TABLE reviews ADD COLUMN change_set_id TEXT REFERENCES change_sets(id);
