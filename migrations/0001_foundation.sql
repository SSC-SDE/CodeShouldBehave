CREATE TABLE repositories (
    id INTEGER PRIMARY KEY,
    security_domain TEXT NOT NULL,
    provider TEXT NOT NULL,
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE (security_domain, provider, owner, name)
);

CREATE TABLE grants (
    id INTEGER PRIMARY KEY,
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    principal TEXT NOT NULL,
    action TEXT NOT NULL,
    path_scope BLOB NOT NULL,
    revoked_at_ms INTEGER,
    UNIQUE (repository_id, principal, action, path_scope)
);

CREATE TABLE scopes (
    id INTEGER PRIMARY KEY,
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    version TEXT NOT NULL,
    canonical BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE (repository_id, version),
    UNIQUE (repository_id, canonical)
);

CREATE TABLE context_revisions (
    scope_id INTEGER NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    commit_id TEXT NOT NULL,
    path_scope BLOB NOT NULL,
    PRIMARY KEY (scope_id, position)
);

CREATE TABLE snapshots (
    id INTEGER PRIMARY KEY,
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    commit_id TEXT NOT NULL,
    scope_id INTEGER NOT NULL REFERENCES scopes(id),
    analyzer_version TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending','ready','partial','failed')),
    enumeration_complete INTEGER NOT NULL DEFAULT 0 CHECK (enumeration_complete IN (0,1)),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (repository_id, commit_id, scope_id, analyzer_version)
);

CREATE TABLE files (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    blob_id TEXT,
    coverage TEXT NOT NULL CHECK (coverage IN ('pending','indexed','unsupported','excluded','failed','deferred')),
    detail TEXT,
    PRIMARY KEY (snapshot_id, path)
);

CREATE TABLE symbols (
    id INTEGER PRIMARY KEY,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    stable_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    UNIQUE (snapshot_id, stable_key),
    FOREIGN KEY (snapshot_id, path) REFERENCES files(snapshot_id, path) ON DELETE CASCADE
);

CREATE TABLE edges (
    id INTEGER PRIMARY KEY,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    stable_key TEXT NOT NULL,
    from_symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
    to_symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    resolution TEXT NOT NULL,
    source_path TEXT NOT NULL,
    source_start_line INTEGER,
    source_start_column INTEGER,
    source_end_line INTEGER,
    source_end_column INTEGER,
    provenance BLOB NOT NULL,
    UNIQUE (snapshot_id, stable_key)
);

CREATE TABLE pr_heads (
    repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    provider_pr_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    head_commit TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (repository_id, provider_pr_id)
);

CREATE TABLE reviews (
    id TEXT PRIMARY KEY,
    repository_id INTEGER NOT NULL REFERENCES repositories(id),
    selected_base_commit TEXT NOT NULL,
    analysis_base_commit TEXT NOT NULL,
    proposal_commit TEXT NOT NULL,
    comparison_mode TEXT NOT NULL,
    scope_id INTEGER NOT NULL REFERENCES scopes(id),
    baseline_snapshot_id INTEGER REFERENCES snapshots(id),
    proposal_snapshot_id INTEGER REFERENCES snapshots(id),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (
        repository_id,
        selected_base_commit,
        analysis_base_commit,
        proposal_commit,
        comparison_mode,
        scope_id
    )
);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    priority INTEGER NOT NULL,
    state TEXT NOT NULL,
    available_at_ms INTEGER NOT NULL,
    lease_owner TEXT,
    lease_expires_at_ms INTEGER,
    lease_generation INTEGER NOT NULL DEFAULT 0,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    terminal_reason TEXT,
    UNIQUE (review_id, kind)
);

CREATE TABLE attempts (
    job_id INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    attempt INTEGER NOT NULL,
    lease_generation INTEGER NOT NULL,
    started_at_ms INTEGER NOT NULL,
    finished_at_ms INTEGER,
    outcome TEXT,
    PRIMARY KEY (job_id, attempt)
);

CREATE TABLE event_deliveries (
    provider TEXT NOT NULL,
    delivery_id TEXT NOT NULL,
    repository_id INTEGER REFERENCES repositories(id),
    accepted_at_ms INTEGER NOT NULL,
    body_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    PRIMARY KEY (provider, delivery_id)
);

CREATE TABLE artifact_refs (
    id INTEGER PRIMARY KEY,
    security_domain TEXT NOT NULL,
    digest TEXT NOT NULL,
    relative_path TEXT NOT NULL UNIQUE,
    byte_length INTEGER NOT NULL,
    format_version TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('ready','missing','corrupt')),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (security_domain, digest, format_version)
);

CREATE TABLE artifact_links (
    artifact_id INTEGER NOT NULL REFERENCES artifact_refs(id) ON DELETE CASCADE,
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    purpose TEXT NOT NULL,
    PRIMARY KEY (artifact_id, owner_kind, owner_id, purpose)
);

CREATE TABLE audit_events (
    id INTEGER PRIMARY KEY,
    security_domain TEXT NOT NULL,
    principal TEXT NOT NULL,
    action TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    occurred_at_ms INTEGER NOT NULL,
    detail BLOB NOT NULL
);
