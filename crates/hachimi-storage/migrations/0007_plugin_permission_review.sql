PRAGMA foreign_keys = ON;

CREATE TABLE plugin_permission_diffs (
    plugin_id TEXT PRIMARY KEY NOT NULL REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    diff_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
