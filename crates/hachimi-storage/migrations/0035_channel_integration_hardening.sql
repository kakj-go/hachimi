-- Delivery side effects need enough provenance to distinguish a safe retry
-- from an operation that may already have reached the remote platform.
ALTER TABLE channel_outbox ADD COLUMN authorization_id TEXT REFERENCES channel_authorizations(id) ON DELETE SET NULL;
ALTER TABLE channel_outbox ADD COLUMN authorization_revision INTEGER;
ALTER TABLE channel_outbox ADD COLUMN account_config_revision INTEGER;
ALTER TABLE channel_outbox ADD COLUMN reactive_external_message_id TEXT;
ALTER TABLE channel_outbox ADD COLUMN run_id TEXT REFERENCES runs(id) ON DELETE SET NULL;
ALTER TABLE channel_outbox ADD COLUMN final_item_id TEXT;
ALTER TABLE channel_outbox ADD COLUMN part_index INTEGER CHECK(part_index IS NULL OR part_index >= 0);
ALTER TABLE channel_outbox ADD COLUMN dispatched_at_ms INTEGER;

CREATE INDEX channel_outbox_authorization_idx
ON channel_outbox(account_id, authorization_id, authorization_revision, status);

CREATE INDEX channel_outbox_run_item_idx
ON channel_outbox(run_id, final_item_id, part_index);

-- SQLite UNIQUE treats NULL values as distinct. Conversation addresses use
-- nullable topic/actor dimensions, so normalize them in the actual key.
CREATE UNIQUE INDEX channel_authorization_address_unique_idx
ON channel_authorizations(
    account_id,
    tenant_key,
    chat_kind,
    chat_id,
    COALESCE(topic_id, ''),
    COALESCE(actor_id, '')
);
