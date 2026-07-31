PRAGMA foreign_keys = ON;

ALTER TABLE schedule_definitions
ADD COLUMN stop_conditions_json TEXT NOT NULL DEFAULT '{}';

ALTER TABLE schedule_definitions
ADD COLUMN contribution_revisions_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE session_permission_configs (
    scope_key TEXT PRIMARY KEY NOT NULL,
    session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    entry_profile TEXT NOT NULL CHECK(entry_profile IN ('workbench', 'pet_conversation', 'desktop_control')),
    config_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    CHECK(
        (session_id IS NOT NULL AND scope_key = 'session:' || session_id)
        OR (session_id IS NULL AND scope_key = 'profile:' || entry_profile)
    )
);

CREATE TABLE browser_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    owner_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    owner_run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    record_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE browser_site_permissions (
    browser_session_id TEXT NOT NULL REFERENCES browser_sessions(id) ON DELETE CASCADE,
    origin TEXT NOT NULL,
    permission_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(browser_session_id, origin)
);

CREATE TABLE computer_app_rules (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    app_id TEXT NOT NULL,
    rule_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(session_id, app_id)
);

CREATE TABLE plugin_installations (
    plugin_id TEXT PRIMARY KEY NOT NULL,
    manifest_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    root_path TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('disabled', 'enabled', 'needs_attention', 'invalid')),
    diagnostics_json TEXT NOT NULL DEFAULT '[]',
    installed_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE connector_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    plugin_id TEXT NOT NULL REFERENCES plugin_installations(plugin_id) ON DELETE CASCADE,
    connector_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    secret_ref TEXT,
    revision_json TEXT NOT NULL,
    health TEXT NOT NULL CHECK(health IN (
        'healthy', 'revoked', 'schema_drift', 'host_identity_drift', 'rate_limited', 'failed'
    )),
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(plugin_id, connector_id, display_name)
);

CREATE TABLE connector_invocations (
    account_id TEXT NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    action TEXT NOT NULL,
    argument_hash TEXT NOT NULL,
    result_json TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, idempotency_key)
);

CREATE TABLE sample_crm_records (
    account_id TEXT NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    record_id TEXT NOT NULL,
    data_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, record_id)
);

CREATE TABLE channel_ingress (
    message_id TEXT PRIMARY KEY NOT NULL,
    route_key TEXT NOT NULL,
    envelope_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'accepted', 'duplicate', 'rejected', 'claimed', 'completed', 'needs_attention'
    )),
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    result_code TEXT NOT NULL,
    received_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX channel_ingress_claim_idx
ON channel_ingress(status, received_at_ms, message_id);

CREATE TABLE channel_deliveries (
    id TEXT PRIMARY KEY NOT NULL,
    route_key TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    text TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'claimed', 'delivered', 'retry_scheduled', 'failed'
    )),
    attempt INTEGER NOT NULL CHECK(attempt >= 0),
    next_attempt_at_ms INTEGER,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX channel_delivery_claim_idx
ON channel_deliveries(status, next_attempt_at_ms, created_at_ms, id);

CREATE TABLE channel_session_routes (
    route_key TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE gateway_runtime_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
    startup_registered INTEGER NOT NULL CHECK(startup_registered IN (0, 1)),
    revision INTEGER NOT NULL CHECK(revision > 0),
    process_id INTEGER,
    last_heartbeat_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO gateway_runtime_state(
    singleton, startup_registered, revision, process_id, last_heartbeat_ms, updated_at_ms
)
VALUES(1, 0, 1, NULL, NULL, 0);
