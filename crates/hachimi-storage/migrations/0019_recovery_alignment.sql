-- no-transaction
PRAGMA foreign_keys = OFF;
PRAGMA legacy_alter_table = ON;
BEGIN IMMEDIATE;

ALTER TABLE runs RENAME TO runs_v28;

CREATE TABLE runs (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'preparing', 'running', 'waiting_approval', 'waiting_user_input',
        'recovering', 'waiting_recovery_decision', 'cancelling', 'succeeded',
        'failed', 'timed_out', 'cancelled', 'interrupted', 'lost'
    )),
    purpose TEXT NOT NULL CHECK (purpose IN ('task', 'review', 'automation')),
    origin_json TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation > 0),
    configuration_json TEXT NOT NULL,
    requested_capabilities_json TEXT NOT NULL,
    negotiated_capabilities_json TEXT NOT NULL,
    provider_capability_probe_json TEXT NOT NULL,
    capability_degradations_json TEXT NOT NULL,
    failure_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO runs
SELECT * FROM runs_v28;

DROP TABLE runs_v28;

CREATE INDEX runs_session_created_idx ON runs(session_id, created_at_ms DESC, id);
CREATE INDEX runs_active_session_idx ON runs(session_id, status, created_at_ms DESC, id);

ALTER TABLE run_step_checkpoints
ADD COLUMN revision_snapshot_json TEXT NOT NULL DEFAULT '{}';

COMMIT;
PRAGMA legacy_alter_table = OFF;
PRAGMA foreign_keys = ON;
