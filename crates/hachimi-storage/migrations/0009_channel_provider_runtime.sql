PRAGMA foreign_keys = ON;

ALTER TABLE channel_provider_manifests
ADD COLUMN contribution_enabled INTEGER NOT NULL DEFAULT 1
CHECK(contribution_enabled IN (0, 1));

CREATE INDEX channel_provider_runtime_idx
ON channel_provider_manifests(contribution_enabled, enabled, provider_id);
