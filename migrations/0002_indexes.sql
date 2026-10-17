CREATE INDEX files_by_snapshot_coverage ON files(snapshot_id, coverage, path);
CREATE INDEX symbols_by_snapshot_path ON symbols(snapshot_id, path);
CREATE INDEX edges_outgoing ON edges(snapshot_id, from_symbol_id, kind);
CREATE INDEX edges_incoming ON edges(snapshot_id, to_symbol_id, kind);
CREATE INDEX jobs_claim_order ON jobs(state, priority DESC, available_at_ms, id);
CREATE INDEX grants_lookup ON grants(principal, repository_id, action, revoked_at_ms);
CREATE INDEX reviews_by_repository_proposal ON reviews(repository_id, proposal_commit, created_at_ms);
CREATE INDEX audit_by_domain_time ON audit_events(security_domain, occurred_at_ms, id);
CREATE INDEX artifact_links_by_owner ON artifact_links(owner_kind, owner_id, purpose);
