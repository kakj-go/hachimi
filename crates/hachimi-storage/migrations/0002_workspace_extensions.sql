PRAGMA foreign_keys = ON;

CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    root_path TEXT NOT NULL UNIQUE,
    git_root TEXT,
    trusted INTEGER NOT NULL DEFAULT 0 CHECK (trusted IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE workspace_checkouts (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('local', 'managed_worktree')),
    path TEXT NOT NULL UNIQUE,
    base_revision TEXT,
    head_revision TEXT,
    status TEXT NOT NULL CHECK (status IN ('preparing', 'ready', 'dirty', 'cleanup_blocked', 'removed')),
    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE checkout_write_leases (
    checkout_id TEXT PRIMARY KEY NOT NULL REFERENCES workspace_checkouts(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK (run_generation > 0),
    acquired_at_ms INTEGER NOT NULL
);

CREATE TABLE process_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    checkout_id TEXT NOT NULL REFERENCES workspace_checkouts(id) ON DELETE RESTRICT,
    run_generation INTEGER,
    owner_client_id TEXT NOT NULL,
    command_summary TEXT NOT NULL,
    interactive INTEGER NOT NULL CHECK (interactive IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('starting', 'running', 'exited', 'terminated', 'failed', 'expired')),
    exit_code INTEGER,
    output_limit_bytes INTEGER NOT NULL CHECK (output_limit_bytes > 0),
    reconnect_expires_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX process_sessions_owner_idx
ON process_sessions(owner_client_id, status, updated_at_ms DESC);

CREATE TABLE review_runs (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    target_json TEXT NOT NULL,
    delivery TEXT NOT NULL CHECK (delivery IN ('inline', 'detached')),
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE review_findings (
    id TEXT PRIMARY KEY NOT NULL,
    review_id TEXT NOT NULL REFERENCES review_runs(id) ON DELETE CASCADE,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'error', 'critical')),
    file TEXT,
    line INTEGER CHECK (line IS NULL OR line > 0),
    message TEXT NOT NULL,
    evidence TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('open', 'acknowledged', 'resolved', 'dismissed'))
);

CREATE INDEX review_findings_review_idx
ON review_findings(review_id, severity, id);

CREATE TABLE run_file_baselines (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    path_key TEXT NOT NULL,
    display_path TEXT NOT NULL,
    baseline_hash TEXT,
    baseline_artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    previous_path TEXT,
    baseline_mode TEXT,
    baseline_size INTEGER,
    baseline_binary INTEGER NOT NULL DEFAULT 0,
    current_hash TEXT,
    current_mode TEXT,
    current_size INTEGER,
    current_binary INTEGER NOT NULL DEFAULT 0,
    change_kind TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(run_id, path_key)
);

CREATE TABLE run_diff_manifests (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    checkout_id TEXT NOT NULL REFERENCES workspace_checkouts(id) ON DELETE CASCADE,
    snapshot_json TEXT NOT NULL,
    artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    generated_at_ms INTEGER NOT NULL
);

CREATE INDEX run_file_baselines_updated_idx
ON run_file_baselines(run_id, updated_at_ms, path_key);

CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    transport_kind TEXT NOT NULL CHECK (transport_kind IN ('stdio', 'streamable_http')),
    command TEXT NOT NULL,
    args_json TEXT NOT NULL DEFAULT '[]',
    cwd TEXT,
    url TEXT,
    auth_reference TEXT,
    read_only_tools_json TEXT NOT NULL DEFAULT '[]',
    startup_timeout_ms INTEGER NOT NULL CHECK (startup_timeout_ms > 0),
    request_timeout_ms INTEGER NOT NULL CHECK (request_timeout_ms > 0),
    max_message_bytes INTEGER NOT NULL CHECK (max_message_bytes BETWEEN 4096 AND 16777216),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE mcp_headers (
    server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE,
    value TEXT,
    secret INTEGER NOT NULL DEFAULT 0 CHECK (secret IN (0, 1)),
    credential_reference TEXT,
    configured INTEGER NOT NULL DEFAULT 0 CHECK (configured IN (0, 1)),
    PRIMARY KEY (server_id, name),
    CHECK ((secret = 0 AND credential_reference IS NULL) OR (secret = 1 AND value IS NULL AND credential_reference IS NOT NULL))
);

CREATE TABLE mcp_server_health (
    server_id TEXT PRIMARY KEY NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK (state IN ('disabled', 'stopped', 'starting', 'ready', 'failed')),
    server_name TEXT,
    server_version TEXT,
    protocol_version TEXT,
    tool_count INTEGER NOT NULL DEFAULT 0 CHECK (tool_count >= 0),
    error_code TEXT,
    checked_at_ms INTEGER NOT NULL
);

CREATE TABLE mcp_discovered_tools (
    server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    exposed_name TEXT NOT NULL,
    description TEXT,
    input_schema_json TEXT NOT NULL,
    schema_hash TEXT NOT NULL,
    host_identity_hash TEXT NOT NULL,
    validation_error TEXT,
    discovered_at_ms INTEGER NOT NULL,
    PRIMARY KEY (server_id, tool_name)
);

CREATE TABLE mcp_tool_overrides (
    server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (server_id, tool_name)
);

CREATE TABLE mcp_content_cache (
    server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    content_kind TEXT NOT NULL CHECK (content_kind IN ('resource', 'resource_template', 'prompt')),
    content_key TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    refreshed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (server_id, content_kind, content_key)
);

CREATE TABLE mcp_inventory_status (
    server_id TEXT PRIMARY KEY NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    errors_json TEXT NOT NULL DEFAULT '{}',
    stale INTEGER NOT NULL DEFAULT 0 CHECK (stale IN (0, 1)),
    refreshed_at_ms INTEGER NOT NULL
);

CREATE TABLE mcp_call_summaries (
    id TEXT PRIMARY KEY NOT NULL,
    server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    tool_name TEXT NOT NULL,
    outcome TEXT NOT NULL,
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE mcp_keyring_cleanup_queue (
    credential_reference TEXT PRIMARY KEY,
    attempt_count INTEGER NOT NULL CHECK (attempt_count >= 1),
    created_at_ms INTEGER NOT NULL,
    last_attempt_at_ms INTEGER NOT NULL
);

CREATE TABLE skills (
    id TEXT PRIMARY KEY NOT NULL,
    stable_path TEXT NOT NULL UNIQUE,
    namespace TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    source_scope TEXT NOT NULL CHECK (source_scope IN ('built_in', 'user', 'repo', 'system', 'admin')),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    content_hash TEXT NOT NULL,
    dependencies_json TEXT NOT NULL DEFAULT '[]',
    diagnostics_json TEXT NOT NULL DEFAULT '[]',
    entry_hash TEXT NOT NULL,
    tree_revision TEXT NOT NULL,
    indexed_at_ms INTEGER,
    description TEXT NOT NULL,
    interface_json TEXT NOT NULL DEFAULT 'null',
    policy_json TEXT NOT NULL DEFAULT '{}',
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX skills_lookup_idx
ON skills(enabled, source_scope, namespace, name, stable_path);

CREATE TABLE skill_file_index (
    skill_id TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('file', 'directory')),
    editor_kind TEXT NOT NULL CHECK (editor_kind IN ('markdown', 'text', 'unsupported')),
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT,
    modified_at_ms INTEGER NOT NULL,
    PRIMARY KEY (skill_id, relative_path)
);

CREATE TABLE skill_classifications (
    skill_id TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    content_revision TEXT NOT NULL,
    workload TEXT NOT NULL CHECK (workload IN ('general', 'coding', 'office')),
    confidence_basis_points INTEGER NOT NULL CHECK (confidence_basis_points BETWEEN 0 AND 10000),
    reason TEXT NOT NULL,
    classifier_revision TEXT NOT NULL,
    classified_at_ms INTEGER NOT NULL,
    PRIMARY KEY (skill_id, content_revision)
);

CREATE TABLE skill_activations (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    skill_id TEXT NOT NULL REFERENCES skills(id) ON DELETE RESTRICT,
    content_revision TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('explicit_selection', 'mention', 'model_read', 'built_in_discovery')),
    activated_at_step_revision INTEGER NOT NULL CHECK (activated_at_step_revision > 0),
    classified_workload TEXT NOT NULL CHECK (classified_workload IN ('general', 'coding', 'office')),
    created_at_ms INTEGER NOT NULL,
    UNIQUE(run_id, skill_id, content_revision)
);
