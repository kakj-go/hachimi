PRAGMA foreign_keys = ON;

CREATE TABLE connector_webhook_events (
    account_id TEXT NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued', 'delivered')),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, event_id)
);

CREATE INDEX connector_webhook_claim_idx
ON connector_webhook_events(account_id, status, created_at_ms, event_id);

CREATE TABLE connector_poll_state (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    cursor INTEGER NOT NULL DEFAULT 0 CHECK(cursor >= 0),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE connector_retry_ledger (
    account_id TEXT NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    attempt INTEGER NOT NULL CHECK(attempt >= 0),
    next_attempt_at_ms INTEGER,
    last_error TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, idempotency_key)
);

CREATE TABLE connector_rate_limits (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    window_started_at_ms INTEGER NOT NULL,
    invocation_count INTEGER NOT NULL CHECK(invocation_count >= 0),
    updated_at_ms INTEGER NOT NULL
);

-- 0004 intentionally preserved the released connector_accounts table. This
-- additive override keeps the missing action_drift value backward compatible
-- without rebuilding a referenced table or invalidating existing migrations.
CREATE TABLE connector_health_overrides (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    health TEXT NOT NULL CHECK(health = 'action_drift'),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE mock_poll_inbox (
    message_id TEXT PRIMARY KEY NOT NULL,
    envelope_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued', 'drained')),
    received_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX mock_poll_inbox_claim_idx
ON mock_poll_inbox(status, received_at_ms, message_id);

CREATE TABLE computer_global_app_rules (
    app_id TEXT PRIMARY KEY NOT NULL,
    rule_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE browser_host_settings (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
    preferred_profile_kind TEXT NOT NULL CHECK(preferred_profile_kind IN ('isolated', 'chrome_extension')),
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO browser_host_settings(singleton, preferred_profile_kind, updated_at_ms)
VALUES(1, 'isolated', 0);
