PRAGMA foreign_keys = ON;

CREATE TABLE plugin_hook_subscriptions (
    plugin_id TEXT NOT NULL REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    contribution_id TEXT NOT NULL,
    event TEXT NOT NULL CHECK(event IN (
        'run.before', 'run.after',
        'tool.before', 'tool.after',
        'schedule.before', 'schedule.after'
    )),
    runtime_revision TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0, 1)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(plugin_id, contribution_id, event)
);

CREATE INDEX plugin_hook_event_idx
ON plugin_hook_subscriptions(event, enabled, plugin_id, contribution_id);

CREATE TABLE plugin_hook_executions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_id TEXT NOT NULL REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    contribution_id TEXT NOT NULL,
    event TEXT NOT NULL,
    session_id TEXT,
    run_id TEXT,
    run_generation INTEGER,
    subject_hash TEXT NOT NULL,
    result_code TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX plugin_hook_execution_lookup_idx
ON plugin_hook_executions(plugin_id, contribution_id, event, created_at_ms);

CREATE TABLE plugin_runtime_bindings (
    plugin_id TEXT NOT NULL REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    contribution_id TEXT NOT NULL,
    resource_kind TEXT NOT NULL CHECK(resource_kind IN (
        'mcp', 'scheduled_task_template', 'browser_extension',
        'asset', 'custom_ui', 'channel'
    )),
    resource_id TEXT NOT NULL,
    runtime_revision TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0, 1)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(plugin_id, contribution_id, resource_kind)
);
