PRAGMA foreign_keys = ON;

CREATE TABLE browser_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('embedded')),
    storage_key TEXT NOT NULL UNIQUE,
    data_epoch INTEGER NOT NULL DEFAULT 1 CHECK(data_epoch > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO browser_profiles(id, kind, storage_key, data_epoch, created_at_ms, updated_at_ms)
VALUES('embedded-default', 'embedded', 'default', 1, 0, 0);

CREATE TABLE browser_workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    owner_session_id TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES browser_profiles(id) ON DELETE RESTRICT,
    active_tab_id TEXT,
    runtime_state TEXT NOT NULL CHECK(runtime_state IN ('dormant', 'starting', 'ready', 'failed')),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE browser_tabs (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES browser_workspaces(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    title TEXT NOT NULL,
    favicon_token TEXT,
    loading INTEGER NOT NULL DEFAULT 0 CHECK(loading IN (0, 1)),
    can_go_back INTEGER NOT NULL DEFAULT 0 CHECK(can_go_back IN (0, 1)),
    can_go_forward INTEGER NOT NULL DEFAULT 0 CHECK(can_go_forward IN (0, 1)),
    runtime_loaded INTEGER NOT NULL DEFAULT 0 CHECK(runtime_loaded IN (0, 1)),
    navigation_error_json TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    input_epoch INTEGER NOT NULL DEFAULT 1 CHECK(input_epoch > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX browser_tabs_workspace_idx
ON browser_tabs(workspace_id, updated_at_ms DESC, id ASC);

CREATE TABLE browser_history (
    profile_id TEXT NOT NULL REFERENCES browser_profiles(id) ON DELETE CASCADE,
    canonical_url TEXT NOT NULL,
    url TEXT NOT NULL,
    title TEXT NOT NULL,
    visit_count INTEGER NOT NULL DEFAULT 1 CHECK(visit_count > 0),
    first_visited_at_ms INTEGER NOT NULL,
    last_visited_at_ms INTEGER NOT NULL,
    PRIMARY KEY(profile_id, canonical_url)
);

CREATE INDEX browser_history_recent_idx
ON browser_history(profile_id, last_visited_at_ms DESC, canonical_url ASC);

CREATE TABLE browser_automation_leases (
    id TEXT PRIMARY KEY NOT NULL,
    surface TEXT NOT NULL CHECK(surface IN ('embedded', 'external_chrome')),
    workspace_id TEXT REFERENCES browser_workspaces(id) ON DELETE SET NULL,
    tab_id TEXT REFERENCES browser_tabs(id) ON DELETE SET NULL,
    owner_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK(run_generation >= 0),
    capabilities_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending', 'active', 'suspended', 'expired', 'failed')),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    expires_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX browser_automation_leases_active_idx
ON browser_automation_leases(owner_session_id, status, updated_at_ms DESC, id ASC);

CREATE TABLE browser_downloads (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES browser_workspaces(id) ON DELETE CASCADE,
    tab_id TEXT NOT NULL REFERENCES browser_tabs(id) ON DELETE CASCADE,
    source_url TEXT NOT NULL,
    suggested_name TEXT NOT NULL,
    destination TEXT,
    status TEXT NOT NULL CHECK(status IN ('pending', 'in_progress', 'completed', 'cancelled', 'failed')),
    received_bytes INTEGER NOT NULL DEFAULT 0 CHECK(received_bytes >= 0),
    total_bytes INTEGER CHECK(total_bytes IS NULL OR total_bytes >= 0),
    sha256 TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

ALTER TABLE session_sources ADD COLUMN browser_tab_id TEXT REFERENCES browser_tabs(id) ON DELETE SET NULL;

UPDATE session_sources SET browser_session_id = NULL;
DELETE FROM browser_permission_requests;
DELETE FROM browser_network_rules;
DELETE FROM browser_site_permissions;
DELETE FROM browser_sessions;

UPDATE desktop_control_sessions
SET active_browser_session_id = NULL,
    control_state = CASE WHEN control_state IN ('observing', 'controlling') THEN 'stopped' ELSE control_state END;

UPDATE browser_automation_leases
SET status = 'expired', revision = revision + 1, updated_at_ms = 0
WHERE status IN ('pending', 'active', 'suspended');
