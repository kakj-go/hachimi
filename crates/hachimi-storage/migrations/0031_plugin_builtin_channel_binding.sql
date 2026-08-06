PRAGMA foreign_keys = ON;

CREATE TABLE plugin_runtime_bindings_next (
    plugin_id TEXT NOT NULL REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    contribution_id TEXT NOT NULL,
    resource_kind TEXT NOT NULL CHECK(resource_kind IN (
        'mcp', 'scheduled_task_template', 'browser_extension',
        'asset', 'custom_ui', 'channel', 'builtin_channel'
    )),
    resource_id TEXT NOT NULL,
    runtime_revision TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0, 1)),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(plugin_id, contribution_id, resource_kind)
);

INSERT INTO plugin_runtime_bindings_next(
    plugin_id,
    contribution_id,
    resource_kind,
    resource_id,
    runtime_revision,
    metadata_json,
    enabled,
    updated_at_ms
)
SELECT
    plugin_id,
    contribution_id,
    resource_kind,
    resource_id,
    runtime_revision,
    metadata_json,
    enabled,
    updated_at_ms
FROM plugin_runtime_bindings;

DROP TABLE plugin_runtime_bindings;
ALTER TABLE plugin_runtime_bindings_next RENAME TO plugin_runtime_bindings;
