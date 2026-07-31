PRAGMA foreign_keys = ON;

CREATE TABLE forge_repositories (
    remote_url_hash TEXT PRIMARY KEY NOT NULL,
    forge_kind TEXT NOT NULL CHECK (forge_kind IN (
        'github', 'gitlab', 'gitee', 'gitea_forgejo', 'unknown'
    )),
    api_base_url TEXT NOT NULL,
    owner TEXT NOT NULL,
    repository TEXT NOT NULL,
    secret_ref TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE forge_operations (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    run_generation INTEGER,
    operation_kind TEXT NOT NULL,
    remote_url_hash TEXT NOT NULL REFERENCES forge_repositories(remote_url_hash),
    source_ref TEXT,
    target_ref TEXT,
    commit_oid TEXT NOT NULL,
    expected_revision TEXT,
    approval_id TEXT REFERENCES approval_requests(id) ON DELETE SET NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'claimed', 'dispatched', 'confirmed', 'failed', 'indeterminate'
    )),
    result_json TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (session_id, idempotency_key)
);

CREATE INDEX forge_operations_reconcile_idx
ON forge_operations(status, updated_at_ms, id);
