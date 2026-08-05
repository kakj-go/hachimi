PRAGMA foreign_keys = ON;

DELETE FROM embedded_browser_site_permissions WHERE scope = 'persisted';
DELETE FROM browser_site_permissions;

ALTER TABLE computer_control_sessions ADD COLUMN owner_run_id TEXT REFERENCES runs(id) ON DELETE SET NULL;
ALTER TABLE computer_control_sessions ADD COLUMN owner_run_generation INTEGER;
ALTER TABLE computer_control_sessions ADD COLUMN app_descriptor_json TEXT;
ALTER TABLE computer_control_sessions ADD COLUMN window_json TEXT;
ALTER TABLE computer_control_sessions ADD COLUMN latest_frame_json TEXT;
ALTER TABLE computer_control_sessions ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0);

CREATE TABLE browser_site_policies (
    origin TEXT PRIMARY KEY NOT NULL,
    decision TEXT NOT NULL CHECK(decision IN ('ask', 'allow', 'block')),
    capabilities_json TEXT NOT NULL,
    private_network INTEGER NOT NULL DEFAULT 0 CHECK(private_network IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE host_access_requests (
    id TEXT PRIMARY KEY NOT NULL,
    owner_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK(run_generation > 0),
    target_kind TEXT NOT NULL CHECK(target_kind IN ('browser', 'computer')),
    target_key TEXT NOT NULL,
    surface TEXT,
    target_json TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    private_network INTEGER NOT NULL DEFAULT 0 CHECK(private_network IN (0, 1)),
    status TEXT NOT NULL CHECK(status IN ('pending', 'allowed', 'denied', 'expired')),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE UNIQUE INDEX host_access_requests_pending_idx
ON host_access_requests(owner_run_id, target_kind, target_key, COALESCE(surface, ''))
WHERE status = 'pending';

CREATE INDEX host_access_requests_session_idx
ON host_access_requests(owner_session_id, status, created_at_ms DESC);

CREATE TABLE host_access_grants (
    id TEXT PRIMARY KEY NOT NULL,
    target_kind TEXT NOT NULL CHECK(target_kind IN ('browser', 'computer')),
    target_key TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('run', 'session')),
    scope_key TEXT NOT NULL,
    owner_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    owner_run_id TEXT REFERENCES runs(id) ON DELETE CASCADE,
    capabilities_json TEXT NOT NULL,
    allow_private_network INTEGER NOT NULL DEFAULT 0 CHECK(allow_private_network IN (0, 1)),
    expires_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(target_kind, target_key, scope_key)
);

CREATE INDEX host_access_grants_lookup_idx
ON host_access_grants(target_kind, target_key, scope, expires_at_ms);

CREATE TABLE computer_app_policies (
    identity_hash TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    descriptor_json TEXT NOT NULL,
    decision TEXT NOT NULL CHECK(decision IN ('ask', 'allow', 'block')),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE computer_host_settings (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
    automation_enabled INTEGER NOT NULL DEFAULT 1 CHECK(automation_enabled IN (0, 1)),
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO computer_host_settings(singleton, automation_enabled, updated_at_ms)
VALUES(1, 1, unixepoch('subsec') * 1000);

CREATE TABLE external_browser_lease_observations (
    lease_id TEXT PRIMARY KEY NOT NULL REFERENCES browser_automation_leases(id) ON DELETE CASCADE,
    owner_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    observation_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX external_browser_lease_observations_session_idx
ON external_browser_lease_observations(owner_session_id, updated_at_ms DESC);
