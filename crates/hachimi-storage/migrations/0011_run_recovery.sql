PRAGMA foreign_keys = ON;

ALTER TABLE side_effect_executions
ADD COLUMN recovery_policy TEXT NOT NULL DEFAULT 'non_replayable'
CHECK (recovery_policy IN ('read_only_replayable', 'idempotent_with_receipt', 'non_replayable'));

CREATE TABLE run_step_checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    run_generation INTEGER NOT NULL CHECK (run_generation > 0),
    step_index INTEGER NOT NULL CHECK (step_index >= 0),
    phase TEXT NOT NULL CHECK (phase IN (
        'sampling', 'tool_prepared', 'tool_claimed', 'tool_dispatched',
        'tool_completed', 'projection_committed'
    )),
    tool_call_id TEXT,
    tool_name TEXT,
    side_effect_execution_id TEXT REFERENCES side_effect_executions(id) ON DELETE SET NULL,
    recovery_policy TEXT NOT NULL CHECK (recovery_policy IN (
        'read_only_replayable', 'idempotent_with_receipt', 'non_replayable'
    )),
    parameter_hash TEXT,
    world_revision TEXT NOT NULL,
    provider_revision TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE(run_id, run_generation, step_index, phase, tool_call_id)
);

CREATE INDEX run_step_checkpoints_latest_idx
ON run_step_checkpoints(run_id, run_generation DESC, step_index DESC, created_at_ms DESC);

CREATE TABLE run_recoveries (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    previous_status TEXT NOT NULL CHECK (previous_status IN (
        'preparing', 'running', 'waiting_approval', 'waiting_user_input', 'cancelling'
    )),
    interrupted_generation INTEGER NOT NULL CHECK (interrupted_generation > 0),
    resume_generation INTEGER NOT NULL CHECK (resume_generation = interrupted_generation + 1),
    state TEXT NOT NULL CHECK (state IN (
        'eligible_auto', 'awaiting_user', 'resuming', 'resumed', 'abandoned', 'failed'
    )),
    reason_code TEXT NOT NULL,
    checkpoint_id TEXT REFERENCES run_step_checkpoints(id) ON DELETE SET NULL,
    side_effect_execution_id TEXT REFERENCES side_effect_executions(id) ON DELETE SET NULL,
    decision_action TEXT CHECK (decision_action IN (
        'resume_safe_remainder', 'confirm_effect_succeeded',
        'retry_idempotent_effect', 'abandon_run'
    )),
    decision_idempotency_key TEXT,
    resolved_by TEXT,
    resolved_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(run_id, interrupted_generation)
);

CREATE INDEX run_recoveries_pending_idx
ON run_recoveries(state, updated_at_ms, run_id);
