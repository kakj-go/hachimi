ALTER TABLE gateway_runtime_state ADD COLUMN last_started_at_ms INTEGER;
ALTER TABLE gateway_runtime_state ADD COLUMN restart_attempt INTEGER NOT NULL DEFAULT 0;
ALTER TABLE gateway_runtime_state ADD COLUMN last_error_code TEXT;

UPDATE gateway_runtime_state SET startup_registered = 1 WHERE singleton = 1;

UPDATE integration_provider_accounts
SET state = 'starting', diagnostic = NULL, consecutive_failures = 0,
    next_reconnect_at_ms = NULL, updated_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE state = 'needs_attention' AND diagnostic = 'integration_probe_failed';

CREATE TABLE channel_provider_runtime_health (
    provider_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    state TEXT NOT NULL,
    diagnostic TEXT,
    last_event_at_ms INTEGER,
    last_delivery_at_ms INTEGER,
    next_reconnect_at_ms INTEGER,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    config_revision INTEGER NOT NULL DEFAULT 0,
    observed_at_ms INTEGER NOT NULL,
    PRIMARY KEY(provider_id, account_id)
);

ALTER TABLE mcp_server_health ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mcp_server_health ADD COLUMN next_retry_at_ms INTEGER;
