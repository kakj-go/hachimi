PRAGMA foreign_keys = ON;

CREATE TABLE enterprise_event_mentions (
    platform TEXT NOT NULL CHECK(platform IN ('wecom', 'ding_talk', 'feishu')),
    account_id TEXT NOT NULL REFERENCES enterprise_integration_accounts(id) ON DELETE CASCADE,
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

ALTER TABLE enterprise_attachment_metadata
ADD COLUMN download_status TEXT NOT NULL DEFAULT 'not_downloaded'
CHECK(download_status IN ('not_downloaded', 'downloading', 'completed', 'failed', 'indeterminate'));

ALTER TABLE enterprise_attachment_metadata ADD COLUMN content_hash TEXT;
ALTER TABLE enterprise_attachment_metadata ADD COLUMN detected_mime_type TEXT;
ALTER TABLE enterprise_attachment_metadata ADD COLUMN downloaded_size_bytes INTEGER CHECK(downloaded_size_bytes IS NULL OR downloaded_size_bytes >= 0);
ALTER TABLE enterprise_attachment_metadata ADD COLUMN managed_attachment_id TEXT REFERENCES attachments(id) ON DELETE SET NULL;

CREATE INDEX enterprise_attachment_download_idx
ON enterprise_attachment_metadata(download_status, account_id, event_id, remote_id);

