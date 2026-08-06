PRAGMA foreign_keys = ON;

-- This formal model replaces the unshipped development-only Channel tables.
DROP TABLE IF EXISTS channel_attachment_metadata;
DROP TABLE IF EXISTS channel_media_secrets;
DROP TABLE IF EXISTS channel_ingress;
DROP TABLE IF EXISTS channel_deliveries;
DROP TABLE IF EXISTS channel_provider_accounts;

-- Sandboxed third-party Channels remain separate from the five formal product
-- integrations. They share the same verified-message contract but not account UI.
CREATE TABLE channel_provider_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL REFERENCES channel_provider_manifests(provider_id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    tenant_key TEXT NOT NULL,
    credential_ref TEXT,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0, 1)),
    state TEXT NOT NULL CHECK(state IN (
        'draft', 'awaiting_auth', 'starting', 'healthy', 'degraded',
        'needs_attention', 'revoked', 'removing'
    )),
    config_json TEXT NOT NULL DEFAULT '{}',
    credential_revision INTEGER NOT NULL DEFAULT 1 CHECK(credential_revision > 0),
    config_revision INTEGER NOT NULL DEFAULT 1 CHECK(config_revision > 0),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE integration_provider_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL CHECK(provider_id IN (
        'dingtalk', 'feishu', 'wecom_ai_bot', 'wecom_app', 'wechat_ilink'
    )),
    display_name TEXT NOT NULL,
    tenant_key TEXT NOT NULL,
    tenant_identity_hash TEXT NOT NULL,
    transport TEXT NOT NULL CHECK(transport IN (
        'encrypted_callback', 'stream', 'long_connection', 'web_socket', 'qr_long_poll'
    )),
    state TEXT NOT NULL CHECK(state IN (
        'draft', 'awaiting_auth', 'starting', 'healthy', 'degraded',
        'needs_attention', 'revoked', 'removing'
    )),
    diagnostic TEXT,
    connector_account_id TEXT REFERENCES connector_accounts(id) ON DELETE SET NULL,
    credential_ref TEXT,
    credential_fingerprint TEXT,
    api_access_enabled INTEGER NOT NULL DEFAULT 0 CHECK(api_access_enabled IN (0, 1)),
    messaging_enabled INTEGER NOT NULL DEFAULT 0 CHECK(messaging_enabled IN (0, 1)),
    config_json TEXT NOT NULL DEFAULT '{}',
    credential_revision INTEGER NOT NULL DEFAULT 1 CHECK(credential_revision > 0),
    config_revision INTEGER NOT NULL DEFAULT 1 CHECK(config_revision > 0),
    last_event_at_ms INTEGER,
    last_delivery_at_ms INTEGER,
    next_reconnect_at_ms INTEGER,
    consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK(consecutive_failures >= 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(provider_id, tenant_key, tenant_identity_hash)
);

CREATE INDEX integration_provider_account_state_idx
ON integration_provider_accounts(provider_id, state, updated_at_ms);

CREATE TABLE integration_ilink_qr_sessions (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    qrcode TEXT,
    qr_content TEXT,
    state TEXT NOT NULL CHECK(state IN ('waiting', 'scanned', 'expired', 'confirmed', 'cancelled')),
    expires_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

-- Enterprise API and managed-media state belongs to the formal provider
-- account. These are operation ledgers, not the removed route/account model.
CREATE TABLE enterprise_token_state (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    token_fingerprint TEXT,
    expires_at_ms INTEGER,
    refresh_after_ms INTEGER,
    last_result_code TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE enterprise_event_receipts (
    platform TEXT NOT NULL CHECK(platform IN ('dingtalk', 'feishu', 'wecom_app')),
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    tenant_identity_hash TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('accepted', 'duplicate', 'acknowledged', 'dead_letter')),
    result_code TEXT NOT NULL,
    received_at_ms INTEGER NOT NULL,
    acknowledged_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(platform, account_id, event_id)
);

CREATE INDEX enterprise_event_reconcile_idx
ON enterprise_event_receipts(status, received_at_ms, platform, account_id);

CREATE TABLE enterprise_operation_ledger (
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    operation TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('claimed', 'completed', 'indeterminate', 'failed')),
    provider_request_id TEXT,
    provider_result_id TEXT,
    result_json TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, idempotency_key)
);

CREATE TABLE enterprise_rate_limit_state (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt >= 0),
    retry_after_ms INTEGER,
    last_error_code TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE channel_attachment_metadata (
    platform TEXT NOT NULL CHECK(platform IN (
        'dingtalk', 'feishu', 'wecom_ai_bot', 'wecom_app', 'wechat_ilink'
    )),
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    resource_key TEXT,
    file_name TEXT,
    mime_type TEXT,
    declared_size_bytes INTEGER CHECK(declared_size_bytes IS NULL OR declared_size_bytes >= 0),
    expected_content_hash TEXT,
    metadata_hash TEXT NOT NULL,
    artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    download_status TEXT NOT NULL DEFAULT 'not_downloaded'
        CHECK(download_status IN ('not_downloaded', 'downloading', 'completed', 'failed', 'indeterminate')),
    content_hash TEXT,
    detected_mime_type TEXT,
    downloaded_size_bytes INTEGER CHECK(downloaded_size_bytes IS NULL OR downloaded_size_bytes >= 0),
    managed_attachment_id TEXT REFERENCES attachments(id) ON DELETE SET NULL,
    PRIMARY KEY(platform, account_id, event_id, remote_id)
);

CREATE INDEX channel_attachment_download_idx
ON channel_attachment_metadata(download_status, account_id, event_id, remote_id);

CREATE TABLE channel_media_secrets (
    platform TEXT NOT NULL CHECK(platform IN ('wecom_ai_bot', 'wechat_ilink')),
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    secret_ref TEXT NOT NULL UNIQUE,
    secret_fingerprint TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(platform, account_id, event_id, remote_id)
);

CREATE TABLE enterprise_event_mentions (
    platform TEXT NOT NULL CHECK(platform IN ('dingtalk', 'feishu', 'wecom_app')),
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    mention_index INTEGER NOT NULL CHECK(mention_index >= 0),
    mention_kind TEXT NOT NULL CHECK(mention_kind IN ('user', 'bot', 'all')),
    target_id TEXT,
    display_text TEXT,
    PRIMARY KEY(platform, account_id, event_id, mention_index),
    FOREIGN KEY(platform, account_id, event_id)
        REFERENCES enterprise_event_receipts(platform, account_id, event_id)
        ON DELETE CASCADE
);

CREATE TABLE channel_external_identities (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    tenant_key TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    display_name TEXT,
    identity_group_id TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(account_id, tenant_key, actor_id)
);

CREATE TABLE channel_access_policies (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    dm_policy TEXT NOT NULL CHECK(dm_policy IN ('pairing', 'allowlist', 'open', 'disabled')),
    allowlist_actor_ids_json TEXT NOT NULL DEFAULT '[]',
    grant_ceiling_json TEXT NOT NULL DEFAULT '{"skillIds":[],"mcpServerIds":[],"connectorSelections":[],"readOnlyWorkspaceRoots":[],"networkHosts":[]}',
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE channel_authorizations (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    target TEXT NOT NULL CHECK(target IN ('dm_identity', 'group_conversation')),
    tenant_key TEXT NOT NULL,
    chat_kind TEXT NOT NULL CHECK(chat_kind IN ('dm', 'group')),
    chat_id TEXT NOT NULL,
    topic_id TEXT,
    actor_id TEXT,
    group_history_policy TEXT CHECK(group_history_policy IS NULL OR group_history_policy IN ('shared', 'per_sender')),
    topic_policy TEXT NOT NULL CHECK(topic_policy IN ('inherit_group', 'isolate_topic')),
    mention_policy TEXT NOT NULL CHECK(mention_policy IN ('required', 'all_messages', 'disabled')),
    grant_json TEXT NOT NULL DEFAULT '{"skillIds":[],"mcpServerIds":[],"connectorSelections":[],"readOnlyWorkspaceRoots":[],"networkHosts":[]}',
    source TEXT NOT NULL CHECK(source IN ('pairing', 'manual', 'identity_link')),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    CHECK((target = 'dm_identity' AND chat_kind = 'dm' AND actor_id IS NOT NULL AND group_history_policy IS NULL)
       OR (target = 'group_conversation' AND chat_kind = 'group' AND actor_id IS NULL AND group_history_policy IS NOT NULL)),
    UNIQUE(account_id, tenant_key, chat_kind, chat_id, topic_id, actor_id)
);

CREATE INDEX channel_authorization_lookup_idx
ON channel_authorizations(account_id, tenant_key, chat_kind, chat_id, enabled);

CREATE TABLE channel_pairing_codes (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL UNIQUE,
    target TEXT NOT NULL CHECK(target IN ('dm_identity', 'group_conversation')),
    group_history_policy TEXT CHECK(group_history_policy IS NULL OR group_history_policy IN ('shared', 'per_sender')),
    topic_policy TEXT NOT NULL CHECK(topic_policy IN ('inherit_group', 'isolate_topic')),
    mention_policy TEXT NOT NULL CHECK(mention_policy IN ('required', 'all_messages', 'disabled')),
    grant_json TEXT NOT NULL DEFAULT '{"skillIds":[],"mcpServerIds":[],"connectorSelections":[],"readOnlyWorkspaceRoots":[],"networkHosts":[]}',
    expires_at_ms INTEGER NOT NULL,
    consumed_at_ms INTEGER,
    consumed_authorization_id TEXT REFERENCES channel_authorizations(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL,
    CHECK((target = 'dm_identity' AND group_history_policy IS NULL)
       OR (target = 'group_conversation' AND group_history_policy IS NOT NULL))
);

CREATE INDEX channel_pairing_expiry_idx
ON channel_pairing_codes(account_id, expires_at_ms, consumed_at_ms);

CREATE TABLE channel_pairing_attempts (
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL,
    failure_count INTEGER NOT NULL DEFAULT 0 CHECK(failure_count >= 0),
    cooldown_until_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, actor_id)
);

CREATE TABLE channel_identity_groups (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE channel_identity_group_members (
    identity_group_id TEXT NOT NULL REFERENCES channel_identity_groups(id) ON DELETE CASCADE,
    external_identity_id TEXT NOT NULL UNIQUE REFERENCES channel_external_identities(id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(identity_group_id, external_identity_id)
);

CREATE TABLE channel_identity_link_codes (
    id TEXT PRIMARY KEY NOT NULL,
    source_external_identity_id TEXT NOT NULL REFERENCES channel_external_identities(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL UNIQUE,
    expires_at_ms INTEGER NOT NULL,
    consumed_at_ms INTEGER,
    consumed_identity_group_id TEXT REFERENCES channel_identity_groups(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX channel_identity_link_expiry_idx
ON channel_identity_link_codes(expires_at_ms, consumed_at_ms);

CREATE TABLE channel_session_bindings (
    binding_key_hash TEXT PRIMARY KEY NOT NULL,
    binding_key_json TEXT NOT NULL,
    account_id TEXT REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    authorization_id TEXT REFERENCES channel_authorizations(id) ON DELETE CASCADE,
    authorization_revision INTEGER NOT NULL CHECK(authorization_revision > 0),
    identity_group_id TEXT REFERENCES channel_identity_groups(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX channel_session_binding_session_idx
ON channel_session_bindings(session_id, updated_at_ms);

CREATE TABLE channel_ingress (
    provider_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
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

CREATE INDEX channel_ingress_claim_idx
ON channel_ingress(status, claim_expires_at_ms, received_at_ms, provider_id, account_id, external_message_id);

CREATE TABLE channel_outbox (
    id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
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
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX channel_outbox_claim_idx
ON channel_outbox(status, next_attempt_at_ms, claim_expires_at_ms, created_at_ms, id);

CREATE TABLE channel_route_secrets (
    account_id TEXT NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    conversation_hash TEXT NOT NULL,
    secret_ref TEXT NOT NULL UNIQUE,
    token_fingerprint TEXT,
    expires_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(account_id, conversation_hash)
);

CREATE TABLE integration_lifecycle_journal (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN (
        'upsert', 'remove', 'credential_rotation', 'reconcile'
    )),
    phase TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('in_progress', 'committed', 'failed', 'deferred_cleanup')),
    credential_ref TEXT,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX integration_lifecycle_reconcile_idx
ON integration_lifecycle_journal(status, updated_at_ms, account_id);

CREATE TABLE integration_secret_cleanup_queue (
    secret_ref TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt >= 0),
    next_attempt_at_ms INTEGER NOT NULL,
    error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
