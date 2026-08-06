PRAGMA foreign_keys = ON;

ALTER TABLE compaction_checkpoints
ADD COLUMN summary_source TEXT NOT NULL DEFAULT 'local'
CHECK (summary_source IN ('local', 'provider_remote', 'local_fallback'));

ALTER TABLE compaction_checkpoints
ADD COLUMN provider_endpoint_id TEXT REFERENCES provider_endpoints(id) ON DELETE SET NULL;

ALTER TABLE compaction_checkpoints
ADD COLUMN provider_account_id TEXT REFERENCES provider_accounts(id) ON DELETE SET NULL;

ALTER TABLE compaction_checkpoints
ADD COLUMN capability_revision TEXT;

ALTER TABLE compaction_checkpoints
ADD COLUMN fallback_reason TEXT;

CREATE INDEX compaction_checkpoints_provider_idx
ON compaction_checkpoints(provider_endpoint_id, summary_source, created_at_ms DESC);
