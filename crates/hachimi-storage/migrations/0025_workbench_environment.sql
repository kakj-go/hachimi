PRAGMA foreign_keys = ON;

CREATE TABLE session_environment_state (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    baseline_revision TEXT,
    managed_checkout_id TEXT REFERENCES workspace_checkouts(id) ON DELETE SET NULL,
    binding_revision INTEGER NOT NULL DEFAULT 1 CHECK(binding_revision > 0),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    inactive_head TEXT,
    inactive_status_fingerprint TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE session_sources (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK(kind IN ('upload', 'web')),
    origin TEXT NOT NULL CHECK(origin IN ('upload', 'browser', 'mcp', 'connector')),
    canonical_key TEXT NOT NULL,
    attachment_id TEXT REFERENCES attachments(id) ON DELETE CASCADE,
    url TEXT,
    title TEXT,
    browser_session_id TEXT REFERENCES browser_sessions(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    last_used_at_ms INTEGER NOT NULL,
    CHECK(
        (kind = 'upload' AND attachment_id IS NOT NULL AND url IS NULL)
        OR (kind = 'web' AND attachment_id IS NULL AND url IS NOT NULL)
    ),
    UNIQUE(session_id, canonical_key)
);

CREATE INDEX session_sources_recent_idx
ON session_sources(session_id, last_used_at_ms DESC, id ASC);

CREATE TABLE workbench_handoff_journal (
    id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    source_checkout_id TEXT NOT NULL REFERENCES workspace_checkouts(id) ON DELETE RESTRICT,
    target_checkout_id TEXT NOT NULL REFERENCES workspace_checkouts(id) ON DELETE RESTRICT,
    phase TEXT NOT NULL CHECK(phase IN (
        'prepared', 'destination_applied', 'source_cleaned', 'committed', 'rolled_back', 'failed'
    )),
    source_head TEXT,
    source_branch TEXT,
    source_status_fingerprint TEXT NOT NULL,
    target_head TEXT,
    target_branch TEXT,
    target_status_fingerprint TEXT NOT NULL,
    expected_binding_revision INTEGER NOT NULL CHECK(expected_binding_revision > 0),
    snapshot_path TEXT NOT NULL,
    snapshot_hash TEXT NOT NULL,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, idempotency_key)
);

CREATE INDEX workbench_handoff_reconcile_idx
ON workbench_handoff_journal(phase, updated_at_ms);
