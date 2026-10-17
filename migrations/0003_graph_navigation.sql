ALTER TABLE symbols ADD COLUMN correspondence_key TEXT;
ALTER TABLE symbols ADD COLUMN start_column INTEGER;
ALTER TABLE symbols ADD COLUMN end_column INTEGER;

CREATE TABLE graph_paths (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    parent_path TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('component','module','file')),
    PRIMARY KEY (snapshot_id, path)
);

CREATE INDEX graph_paths_by_parent ON graph_paths(snapshot_id, parent_path, kind, path);
CREATE INDEX symbols_by_correspondence ON symbols(snapshot_id, correspondence_key);
CREATE INDEX edges_by_snapshot_id ON edges(snapshot_id, id);
