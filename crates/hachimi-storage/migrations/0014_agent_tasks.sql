PRAGMA foreign_keys = ON;

CREATE TABLE agent_tasks (
    id TEXT PRIMARY KEY NOT NULL,
    root_task_id TEXT NOT NULL,
    root_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    parent_task_id TEXT REFERENCES agent_tasks(id) ON DELETE CASCADE,
    parent_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    parent_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    child_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    child_run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    depth INTEGER NOT NULL CHECK (depth BETWEEN 1 AND 3),
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'running', 'waiting', 'needs_attention',
        'succeeded', 'failed', 'cancelled'
    )),
    reserved_budget_json TEXT NOT NULL,
    usage_json TEXT NOT NULL,
    artifact_ids_json TEXT NOT NULL,
    result_summary TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    finished_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX agent_tasks_parent_idx
ON agent_tasks(parent_run_id, created_at_ms, id);

CREATE INDEX agent_tasks_root_status_idx
ON agent_tasks(root_run_id, status, updated_at_ms);

CREATE TABLE agent_task_messages (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES agent_tasks(id) ON DELETE CASCADE,
    sender_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    recipient_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    content TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 32000),
    created_at_ms INTEGER NOT NULL,
    delivered_at_ms INTEGER
);

CREATE INDEX agent_task_messages_recipient_idx
ON agent_task_messages(recipient_run_id, delivered_at_ms, created_at_ms, id);
