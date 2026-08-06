CREATE TABLE channel_identity_transfer_requests (
    id TEXT PRIMARY KEY NOT NULL,
    link_code_id TEXT NOT NULL UNIQUE REFERENCES channel_identity_link_codes(id) ON DELETE CASCADE,
    source_external_identity_id TEXT NOT NULL REFERENCES channel_external_identities(id) ON DELETE CASCADE,
    target_external_identity_id TEXT NOT NULL REFERENCES channel_external_identities(id) ON DELETE CASCADE,
    source_group_id TEXT REFERENCES channel_identity_groups(id) ON DELETE SET NULL,
    target_group_id TEXT REFERENCES channel_identity_groups(id) ON DELETE SET NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    status TEXT NOT NULL CHECK(status IN ('pending', 'completed', 'cancelled')),
    expires_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX channel_identity_transfer_pending_idx
ON channel_identity_transfer_requests(status, expires_at_ms, updated_at_ms);
