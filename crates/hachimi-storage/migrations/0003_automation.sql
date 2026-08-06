PRAGMA foreign_keys = ON;

CREATE TABLE schedule_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    prompt TEXT NOT NULL,
    schedule_json TEXT NOT NULL,
    entry_profile TEXT NOT NULL CHECK (entry_profile = 'workbench'),
    workload_override TEXT CHECK (workload_override IN ('general', 'coding', 'office')),
    context_template_json TEXT NOT NULL,
    tool_allowlist_json TEXT NOT NULL DEFAULT '[]',
    skill_allowlist_json TEXT NOT NULL DEFAULT '[]',
    skill_revisions_json TEXT NOT NULL DEFAULT '[]',
    mcp_tool_allowlist_json TEXT NOT NULL DEFAULT '[]',
    permission_config_json TEXT NOT NULL,
    permission_revision INTEGER NOT NULL CHECK (permission_revision > 0),
    timeout_ms INTEGER NOT NULL CHECK (timeout_ms > 0),
    misfire_policy TEXT NOT NULL CHECK (misfire_policy IN ('skip', 'catch_up_once')),
    delivery_policy TEXT NOT NULL CHECK (delivery_policy IN ('task_tab_only', 'task_tab_and_system_notification')),
    config_revision INTEGER NOT NULL CHECK (config_revision > 0),
    created_by TEXT NOT NULL,
    next_run_at_ms INTEGER,
    health TEXT NOT NULL CHECK (health IN ('healthy', 'needs_attention', 'invalid')),
    health_reason TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX schedules_due_idx
ON schedule_definitions(enabled, health, next_run_at_ms, id);

CREATE TABLE schedule_runtime_state (
    schedule_id TEXT PRIMARY KEY NOT NULL REFERENCES schedule_definitions(id) ON DELETE CASCADE,
    last_scheduled_for_ms INTEGER,
    last_invocation_key TEXT,
    active_task_run_id TEXT,
    timer_generation INTEGER NOT NULL CHECK (timer_generation >= 0),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE task_runs (
    id TEXT PRIMARY KEY NOT NULL,
    schedule_id TEXT REFERENCES schedule_definitions(id) ON DELETE SET NULL,
    schedule_revision INTEGER,
    trigger TEXT NOT NULL CHECK (trigger IN ('scheduled', 'manual', 'retry', 'catch_up')),
    scheduled_for_ms INTEGER,
    invocation_key TEXT NOT NULL UNIQUE,
    requester_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    execution_session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'preparing', 'running', 'needs_attention', 'succeeded',
        'failed', 'timed_out', 'cancelled', 'lost', 'skipped'
    )),
    progress_percent INTEGER CHECK (progress_percent BETWEEN 0 AND 100),
    result_summary TEXT,
    error_code TEXT,
    error_summary TEXT,
    artifact_ids_json TEXT NOT NULL DEFAULT '[]',
    delivery_status TEXT NOT NULL CHECK (delivery_status IN ('pending', 'not_requested', 'delivered', 'failed')),
    delivery_error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    finished_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX task_runs_schedule_created_idx
ON task_runs(schedule_id, created_at_ms DESC, id);
CREATE INDEX task_runs_active_idx
ON task_runs(status, created_at_ms, id);
