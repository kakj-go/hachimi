PRAGMA foreign_keys = ON;

CREATE TABLE desktop_control_sessions (
    session_id TEXT PRIMARY KEY NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    active_browser_session_id TEXT REFERENCES browser_sessions(id) ON DELETE SET NULL,
    selected_app_id TEXT,
    selected_window_fingerprint TEXT,
    input_epoch INTEGER NOT NULL DEFAULT 0 CHECK(input_epoch >= 0),
    control_state TEXT NOT NULL CHECK(control_state IN (
        'observing', 'controlling', 'taken_over', 'needs_attention', 'stopped'
    )),
    last_observation_at_ms INTEGER,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX desktop_control_state_idx
ON desktop_control_sessions(control_state, updated_at_ms);

CREATE TABLE desktop_control_action_ledger (
    session_id TEXT NOT NULL REFERENCES desktop_control_sessions(session_id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK(generation > 0),
    action_id TEXT NOT NULL,
    action_kind TEXT NOT NULL,
    target_fingerprint_hash TEXT NOT NULL,
    observation_revision TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'prepared', 'approved', 'dispatched', 'completed', 'denied', 'indeterminate'
    )),
    result_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(session_id, action_id)
);

CREATE INDEX desktop_control_action_reconcile_idx
ON desktop_control_action_ledger(status, updated_at_ms, session_id);
