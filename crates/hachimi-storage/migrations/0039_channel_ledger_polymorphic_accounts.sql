-- The durable Channel ledger is shared by formal platform accounts and local
-- built-in providers. A foreign key to only integration_provider_accounts
-- rejected loopback and plugin provider traffic, so account ownership is
-- validated by the Gateway provider registry instead.
ALTER TABLE channel_ingress RENAME TO channel_ingress_legacy_account_fk;

CREATE TABLE channel_ingress (
    provider_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    external_message_id TEXT NOT NULL,
    address_json TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    normalized_payload_json TEXT,
    status TEXT NOT NULL CHECK(status IN (
        'accepted', 'rejected', 'claimed', 'run_created', 'completed', 'needs_attention'
    )),
    claim_token TEXT,
    claim_expires_at_ms INTEGER,
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    authorization_id TEXT REFERENCES channel_authorizations(id) ON DELETE SET NULL,
    authorization_revision INTEGER,
    grant_snapshot_json TEXT NOT NULL DEFAULT '{"skillIds":[],"mcpServerIds":[],"connectorSelections":[],"readOnlyWorkspaceRoots":[],"networkHosts":[]}',
    result_code TEXT NOT NULL,
    provider_receipt TEXT,
    received_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(provider_id, account_id, external_message_id)
);

INSERT INTO channel_ingress SELECT * FROM channel_ingress_legacy_account_fk;
DROP TABLE channel_ingress_legacy_account_fk;

CREATE INDEX channel_ingress_claim_idx
ON channel_ingress(status, claim_expires_at_ms, received_at_ms, provider_id, account_id, external_message_id);

ALTER TABLE channel_outbox RENAME TO channel_outbox_legacy_account_fk;

CREATE TABLE channel_outbox (
    id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    address_json TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    reply_context_json TEXT,
    idempotency_key TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'claimed', 'delivered', 'retry_scheduled', 'permanent_failure', 'indeterminate'
    )),
    attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt >= 0),
    claim_token TEXT,
    claim_expires_at_ms INTEGER,
    next_attempt_at_ms INTEGER,
    error_code TEXT,
    provider_receipt TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    authorization_id TEXT REFERENCES channel_authorizations(id) ON DELETE SET NULL,
    authorization_revision INTEGER,
    account_config_revision INTEGER,
    reactive_external_message_id TEXT,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    final_item_id TEXT,
    part_index INTEGER CHECK(part_index IS NULL OR part_index >= 0),
    dispatched_at_ms INTEGER
);

INSERT INTO channel_outbox SELECT * FROM channel_outbox_legacy_account_fk;
DROP TABLE channel_outbox_legacy_account_fk;

CREATE INDEX channel_outbox_claim_idx
ON channel_outbox(status, next_attempt_at_ms, claim_expires_at_ms, created_at_ms, id);

CREATE INDEX channel_outbox_authorization_idx
ON channel_outbox(account_id, authorization_id, authorization_revision, status);

CREATE INDEX channel_outbox_run_item_idx
ON channel_outbox(run_id, final_item_id, part_index);
