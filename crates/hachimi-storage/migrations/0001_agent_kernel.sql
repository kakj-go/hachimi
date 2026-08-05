PRAGMA foreign_keys = ON;

CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    context_kind TEXT NOT NULL CHECK (context_kind IN ('general', 'project', 'avatar')),
    context_json TEXT NOT NULL,
    entry_profile TEXT NOT NULL CHECK (entry_profile IN ('workbench', 'pet_conversation')),
    title TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    parent_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    source_run_id TEXT,
    next_sequence INTEGER NOT NULL DEFAULT 1 CHECK (next_sequence > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX sessions_list_cursor_idx
ON sessions(archived, pinned DESC, updated_at_ms DESC, id ASC);

CREATE TABLE runs (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'preparing', 'running', 'waiting_approval', 'waiting_user_input',
        'cancelling', 'succeeded', 'failed', 'timed_out', 'cancelled', 'interrupted', 'lost'
    )),
    purpose TEXT NOT NULL CHECK (purpose IN ('task', 'review', 'automation')),
    origin_json TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation > 0),
    configuration_json TEXT NOT NULL,
    requested_capabilities_json TEXT NOT NULL,
    negotiated_capabilities_json TEXT NOT NULL,
    provider_capability_probe_json TEXT NOT NULL,
    capability_degradations_json TEXT NOT NULL,
    failure_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX runs_session_created_idx ON runs(session_id, created_at_ms DESC, id);
CREATE INDEX runs_active_session_idx ON runs(session_id, status, created_at_ms DESC, id);

CREATE TABLE run_events (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    payload_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (session_id, sequence)
);

CREATE TABLE transcript_items (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    kind TEXT NOT NULL CHECK (kind IN (
        'user', 'assistant', 'reasoning', 'tool_execution', 'plan', 'approval',
        'user_input_request', 'command_execution', 'file_change', 'mcp_call',
        'dynamic_tool_call', 'collab_tool_call', 'context_compaction', 'review', 'system_context'
    )),
    status TEXT NOT NULL CHECK (status IN ('pending', 'in_progress', 'completed', 'failed', 'interrupted')),
    payload_json TEXT NOT NULL,
    relations_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, sequence)
);

CREATE TABLE attachments (
    id TEXT PRIMARY KEY NOT NULL,
    content_hash TEXT NOT NULL UNIQUE,
    original_name TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    managed_path TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE run_attachments (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE RESTRICT,
    PRIMARY KEY (run_id, attachment_id)
);

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    content_hash TEXT,
    managed_path TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at_ms INTEGER NOT NULL
);

CREATE TABLE approval_requests (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    tool_call_id TEXT NOT NULL,
    run_generation INTEGER NOT NULL CHECK (run_generation > 0),
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'denied', 'expired', 'cancelled')),
    action TEXT NOT NULL,
    resource TEXT NOT NULL,
    parameter_hash TEXT NOT NULL,
    risk_summary TEXT NOT NULL,
    target_host TEXT NOT NULL,
    required_scopes_json TEXT NOT NULL,
    grant_scope TEXT NOT NULL CHECK (grant_scope IN ('once', 'session', 'timed_lease')),
    uses_remaining INTEGER NOT NULL CHECK (uses_remaining >= 0),
    requester_principal TEXT NOT NULL,
    resolved_by TEXT,
    expires_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    resolved_at_ms INTEGER
);

CREATE INDEX approvals_pending_idx ON approval_requests(status, expires_at_ms);

CREATE TABLE user_input_requests (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK (run_generation > 0),
    item_id TEXT NOT NULL REFERENCES transcript_items(id) ON DELETE CASCADE,
    questions_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'resolved', 'cancelled', 'expired', 'interrupted')),
    expires_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    resolved_at_ms INTEGER,
    resolved_by TEXT
);

CREATE INDEX user_input_pending_run_idx
ON user_input_requests(run_id, status, created_at_ms ASC);

CREATE TABLE proposed_plans (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision > 0),
    goal TEXT NOT NULL,
    assumptions_json TEXT NOT NULL,
    steps_json TEXT NOT NULL,
    affected_resources_json TEXT NOT NULL,
    verification_json TEXT NOT NULL,
    risks_json TEXT NOT NULL,
    open_questions_json TEXT NOT NULL,
    content_markdown TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('proposed', 'accepted', 'superseded')),
    accepted_run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    accepted_at_ms INTEGER,
    UNIQUE(session_id, revision)
);

CREATE INDEX proposed_plans_session_revision_idx
ON proposed_plans(session_id, revision DESC);

CREATE TABLE compaction_checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    previous_checkpoint_id TEXT REFERENCES compaction_checkpoints(id) ON DELETE SET NULL,
    covered_through_sequence INTEGER NOT NULL CHECK (covered_through_sequence > 0),
    reason TEXT NOT NULL CHECK (reason IN ('automatic', 'manual', 'reactive')),
    trigger TEXT NOT NULL CHECK (trigger IN ('auto', 'manual', 'provider_overflow')),
    phase TEXT NOT NULL CHECK (phase IN ('pre_run', 'mid_run', 'standalone')),
    implementation TEXT NOT NULL CHECK (implementation IN ('local', 'remote')),
    summary_json TEXT NOT NULL,
    quality_json TEXT NOT NULL,
    token_snapshot_json TEXT,
    trimmed_history_groups INTEGER NOT NULL CHECK (trimmed_history_groups >= 0),
    created_at_ms INTEGER NOT NULL,
    UNIQUE(session_id, covered_through_sequence)
);

CREATE INDEX compaction_checkpoints_session_created_idx
ON compaction_checkpoints(session_id, covered_through_sequence DESC);

CREATE TABLE run_usage_snapshots (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    billed_input_tokens INTEGER NOT NULL CHECK (billed_input_tokens >= 0),
    billed_output_tokens INTEGER NOT NULL CHECK (billed_output_tokens >= 0),
    active_context_tokens INTEGER NOT NULL CHECK (active_context_tokens >= 0),
    remaining_context_tokens INTEGER NOT NULL CHECK (remaining_context_tokens >= 0),
    count_source TEXT NOT NULL CHECK (count_source IN ('provider', 'tokenizer', 'conservative_estimate')),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE capability_grants (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    scope TEXT NOT NULL CHECK (scope IN ('session', 'run')),
    grant_json TEXT NOT NULL,
    source TEXT NOT NULL,
    expires_at_ms INTEGER,
    invalidated_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX capability_grants_active_idx
ON capability_grants(session_id, run_id, invalidated_at_ms, expires_at_ms);

CREATE TABLE sandbox_capability_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    backend TEXT NOT NULL,
    readiness TEXT NOT NULL CHECK (readiness IN ('unavailable', 'setup_required', 'degraded', 'ready')),
    report_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX sandbox_reports_run_idx
ON sandbox_capability_reports(run_id, created_at_ms DESC);

CREATE TABLE run_steers (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK (run_generation > 0),
    item_id TEXT REFERENCES transcript_items(id) ON DELETE SET NULL,
    input_text TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'consumed', 'interrupted')),
    created_at_ms INTEGER NOT NULL,
    consumed_at_ms INTEGER
);

CREATE INDEX run_steers_pending_idx
ON run_steers(run_id, run_generation, status, created_at_ms ASC);

CREATE TABLE side_effect_executions (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK (run_generation > 0),
    tool_call_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    parameter_hash TEXT NOT NULL,
    approval_id TEXT REFERENCES approval_requests(id) ON DELETE SET NULL,
    host_request_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('claimed', 'dispatched', 'succeeded', 'failed', 'cancelled', 'indeterminate')),
    result_code TEXT,
    result_artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    result_json TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(run_id, run_generation, idempotency_key)
);

CREATE INDEX side_effect_executions_active_idx
ON side_effect_executions(status, updated_at_ms);
CREATE INDEX side_effect_executions_tool_call_idx
ON side_effect_executions(run_id, tool_call_id);

CREATE TABLE audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    principal TEXT NOT NULL,
    session_id TEXT,
    run_id TEXT,
    run_generation INTEGER,
    operation TEXT NOT NULL,
    target_summary TEXT NOT NULL,
    decision TEXT NOT NULL,
    result_code TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX audit_events_run_idx ON audit_events(run_id, created_at_ms, id);

CREATE TABLE idempotency_records (
    principal TEXT NOT NULL,
    method TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (principal, method, idempotency_key)
);
