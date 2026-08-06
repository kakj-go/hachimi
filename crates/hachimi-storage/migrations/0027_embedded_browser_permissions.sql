PRAGMA foreign_keys = ON;

ALTER TABLE browser_automation_leases ADD COLUMN external_session_id TEXT;

CREATE INDEX browser_automation_leases_external_session_idx
ON browser_automation_leases(external_session_id);

CREATE TABLE embedded_browser_site_permissions (
    id TEXT PRIMARY KEY NOT NULL,
    origin TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('once', 'session', 'persisted')),
    scope_key TEXT NOT NULL,
    owner_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    owner_run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    capabilities_json TEXT NOT NULL,
    allow_private_network INTEGER NOT NULL DEFAULT 0 CHECK(allow_private_network IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(origin, scope_key)
);

CREATE INDEX embedded_browser_site_permissions_lookup_idx
ON embedded_browser_site_permissions(origin, scope, expires_at_ms);

CREATE TABLE embedded_browser_permission_requests (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES browser_workspaces(id) ON DELETE CASCADE,
    tab_id TEXT NOT NULL REFERENCES browser_tabs(id) ON DELETE CASCADE,
    automation_lease_id TEXT REFERENCES browser_automation_leases(id) ON DELETE SET NULL,
    owner_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK(run_generation >= 0),
    origin TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    private_network INTEGER NOT NULL DEFAULT 0 CHECK(private_network IN (0, 1)),
    status TEXT NOT NULL CHECK(status IN ('pending', 'allowed', 'denied', 'expired')),
    expected_tab_revision INTEGER NOT NULL CHECK(expected_tab_revision > 0),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX embedded_browser_permission_requests_pending_idx
ON embedded_browser_permission_requests(owner_session_id, status, created_at_ms DESC);

CREATE UNIQUE INDEX embedded_browser_permission_requests_dedupe_idx
ON embedded_browser_permission_requests(owner_run_id, origin)
WHERE status = 'pending';
