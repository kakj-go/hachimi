PRAGMA foreign_keys = ON;

CREATE TABLE plugin_revisions (
    plugin_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    root_path TEXT NOT NULL,
    plugin_status TEXT NOT NULL CHECK(plugin_status IN (
        'disabled', 'enabled', 'needs_attention', 'invalid'
    )),
    status TEXT NOT NULL CHECK(status IN (
        'staged', 'validated', 'activating', 'healthy',
        'failed', 'superseded', 'removed'
    )),
    health_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(plugin_id, revision)
);

CREATE INDEX plugin_revision_status_idx
ON plugin_revisions(plugin_id, status, updated_at_ms);

CREATE TABLE plugin_revision_heads (
    plugin_id TEXT PRIMARY KEY NOT NULL,
    current_revision TEXT NOT NULL,
    known_good_revision TEXT,
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(plugin_id, current_revision)
        REFERENCES plugin_revisions(plugin_id, revision),
    FOREIGN KEY(plugin_id, known_good_revision)
        REFERENCES plugin_revisions(plugin_id, revision)
);

CREATE TABLE plugin_lifecycle_journal (
    id TEXT PRIMARY KEY NOT NULL,
    plugin_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN (
        'install', 'update', 'enable', 'disable',
        'rollback', 'uninstall', 'reconcile'
    )),
    phase TEXT NOT NULL CHECK(phase IN (
        'stage', 'validate', 'permission_review', 'activate',
        'health_check', 'commit', 'rollback'
    )),
    status TEXT NOT NULL CHECK(status IN (
        'in_progress', 'committed', 'rolled_back', 'failed'
    )),
    source_revision TEXT,
    candidate_revision TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX plugin_lifecycle_reconcile_idx
ON plugin_lifecycle_journal(status, updated_at_ms, plugin_id);

-- Existing installations become the first known-good revision. This keeps
-- upgrades from older schemas rollback-capable without rewriting bundle data.
INSERT INTO plugin_revisions(
    plugin_id, revision, manifest_json, content_hash, root_path, plugin_status,
    status, health_code, created_at_ms, updated_at_ms
)
SELECT
    plugin_id, content_hash, manifest_json, content_hash, root_path, status,
    CASE WHEN status = 'invalid' THEN 'failed' ELSE 'healthy' END,
    CASE WHEN status = 'invalid' THEN 'legacy_invalid_installation' ELSE NULL END,
    installed_at_ms, updated_at_ms
FROM plugin_installations;

INSERT INTO plugin_revision_heads(
    plugin_id, current_revision, known_good_revision, updated_at_ms
)
SELECT
    plugin_id, content_hash,
    CASE WHEN status = 'invalid' THEN NULL ELSE content_hash END,
    updated_at_ms
FROM plugin_installations;

-- Normalize the released preview state vocabulary while the decoder remains
-- backwards-compatible with databases copied before this migration completed.
UPDATE plugin_contribution_runtime
SET runtime_state = CASE runtime_state
    WHEN 'ready' THEN 'active'
    WHEN 'needs_attention' THEN 'degraded'
    ELSE runtime_state
END;
