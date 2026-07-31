PRAGMA foreign_keys = ON;

CREATE TABLE enterprise_integration_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    platform TEXT NOT NULL CHECK(platform IN ('wecom', 'ding_talk', 'feishu')),
    connector_account_id TEXT REFERENCES connector_accounts(id) ON DELETE SET NULL,
    channel_account_id TEXT REFERENCES channel_provider_accounts(id) ON DELETE SET NULL,
    tenant_identity_hash TEXT NOT NULL,
    ingress_mode TEXT NOT NULL CHECK(ingress_mode IN (
        'encrypted_callback', 'stream', 'long_connection'
    )),
    event_source_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN (
        'disabled', 'starting', 'healthy', 'rate_limited',
        'revoked', 'needs_attention', 'failed'
    )),
    diagnostic TEXT,
    credential_revision INTEGER NOT NULL DEFAULT 1 CHECK(credential_revision > 0),
    source_account_updated_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(platform, tenant_identity_hash, event_source_id)
);

CREATE INDEX enterprise_account_state_idx
ON enterprise_integration_accounts(platform, state, updated_at_ms);

CREATE TABLE enterprise_token_state (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES enterprise_integration_accounts(id) ON DELETE CASCADE,
    token_fingerprint TEXT,
    expires_at_ms INTEGER,
    refresh_after_ms INTEGER,
    last_result_code TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE enterprise_event_receipts (
    platform TEXT NOT NULL CHECK(platform IN ('wecom', 'ding_talk', 'feishu')),
    account_id TEXT NOT NULL REFERENCES enterprise_integration_accounts(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    tenant_identity_hash TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'accepted', 'duplicate', 'acknowledged', 'dead_letter'
    )),
    result_code TEXT NOT NULL,
    received_at_ms INTEGER NOT NULL,
    acknowledged_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(platform, account_id, event_id)
);

CREATE INDEX enterprise_event_reconcile_idx
ON enterprise_event_receipts(status, received_at_ms, platform, account_id);

CREATE TABLE enterprise_operation_ledger (
    account_id TEXT NOT NULL REFERENCES enterprise_integration_accounts(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    operation TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'claimed', 'completed', 'indeterminate', 'failed'
    )),
    provider_request_id TEXT,
    provider_result_id TEXT,
    result_json TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, idempotency_key)
);

CREATE TABLE enterprise_rate_limit_state (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES enterprise_integration_accounts(id) ON DELETE CASCADE,
    attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt >= 0),
    retry_after_ms INTEGER,
    last_error_code TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE enterprise_attachment_metadata (
    platform TEXT NOT NULL CHECK(platform IN ('wecom', 'ding_talk', 'feishu')),
    account_id TEXT NOT NULL REFERENCES enterprise_integration_accounts(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    file_name TEXT,
    mime_type TEXT,
    declared_size_bytes INTEGER CHECK(declared_size_bytes IS NULL OR declared_size_bytes >= 0),
    metadata_hash TEXT NOT NULL,
    artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(platform, account_id, event_id, remote_id)
);
