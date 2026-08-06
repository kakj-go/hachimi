PRAGMA foreign_keys = ON;

ALTER TABLE task_runs ADD COLUMN event_context_json TEXT;

CREATE TABLE schedule_event_ledger (
    source_kind TEXT NOT NULL CHECK (source_kind IN ('workspace', 'plugin', 'connector', 'channel', 'gateway')),
    source_principal TEXT NOT NULL,
    source_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    event_type TEXT NOT NULL,
    subject TEXT,
    labels_json TEXT NOT NULL DEFAULT '{}',
    resource_json TEXT,
    occurred_at_ms INTEGER NOT NULL,
    received_at_ms INTEGER NOT NULL,
    processing_status TEXT NOT NULL CHECK (processing_status IN ('accepted', 'replayed', 'conflict')),
    matched_schedule_count INTEGER NOT NULL DEFAULT 0 CHECK (matched_schedule_count >= 0),
    replay_count INTEGER NOT NULL DEFAULT 0 CHECK (replay_count >= 0),
    conflict_count INTEGER NOT NULL DEFAULT 0 CHECK (conflict_count >= 0),
    last_received_at_ms INTEGER NOT NULL,
    PRIMARY KEY (source_kind, source_principal, source_id, event_id)
);

CREATE INDEX schedule_event_ledger_received_idx
ON schedule_event_ledger(last_received_at_ms, source_kind, source_principal, source_id, event_id);

CREATE TABLE schedule_event_task_runs (
    source_kind TEXT NOT NULL,
    source_principal TEXT NOT NULL,
    source_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    schedule_id TEXT NOT NULL REFERENCES schedule_definitions(id) ON DELETE CASCADE,
    task_run_id TEXT NOT NULL UNIQUE REFERENCES task_runs(id) ON DELETE CASCADE,
    PRIMARY KEY (source_kind, source_principal, source_id, event_id, schedule_id),
    FOREIGN KEY (source_kind, source_principal, source_id, event_id)
        REFERENCES schedule_event_ledger(source_kind, source_principal, source_id, event_id)
        ON DELETE CASCADE
);
