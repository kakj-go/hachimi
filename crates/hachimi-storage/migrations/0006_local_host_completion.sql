PRAGMA foreign_keys = ON;

ALTER TABLE schedule_definitions
ADD COLUMN host_grant_json TEXT NOT NULL DEFAULT '{"connectors":[],"browser":null,"computerUnattended":false}';

CREATE TABLE browser_network_rules (
    browser_session_id TEXT NOT NULL,
    origin TEXT NOT NULL,
    rule_kind TEXT NOT NULL CHECK(rule_kind IN ('document', 'resource')),
    allow_private_network INTEGER NOT NULL DEFAULT 0 CHECK(allow_private_network IN (0, 1)),
    expires_at_ms INTEGER,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(browser_session_id, origin, rule_kind)
);

CREATE TABLE browser_permission_requests (
    id TEXT PRIMARY KEY NOT NULL,
    browser_session_id TEXT NOT NULL,
    owner_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    origin TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    network_kind TEXT NOT NULL CHECK(network_kind IN ('document', 'resource')),
    private_network INTEGER NOT NULL DEFAULT 0 CHECK(private_network IN (0, 1)),
    status TEXT NOT NULL CHECK(status IN ('pending', 'allowed', 'denied', 'expired')),
    expected_browser_revision INTEGER NOT NULL CHECK(expected_browser_revision >= 0),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX browser_permission_pending_idx
ON browser_permission_requests(browser_session_id, status, created_at_ms);

CREATE TABLE plugin_contribution_runtime (
    plugin_id TEXT NOT NULL REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    contribution_id TEXT NOT NULL,
    contribution_kind TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    runtime_revision TEXT NOT NULL,
    runtime_state TEXT NOT NULL,
    diagnostic TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(plugin_id, contribution_id)
);

CREATE TABLE channel_provider_manifests (
    provider_id TEXT PRIMARY KEY NOT NULL,
    plugin_id TEXT REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    manifest_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0, 1)),
    config_revision INTEGER NOT NULL DEFAULT 1 CHECK(config_revision > 0),
    health TEXT NOT NULL,
    diagnostic TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE channel_provider_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL REFERENCES channel_provider_manifests(provider_id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    secret_ref TEXT,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0, 1)),
    route_allowlist_json TEXT NOT NULL,
    config_revision INTEGER NOT NULL DEFAULT 1 CHECK(config_revision > 0),
    updated_at_ms INTEGER NOT NULL
);
