-- Gateway supervision is a product invariant; the old per-user startup
-- switch is removed from the persisted schema after the runtime snapshot has
-- been introduced.
CREATE TABLE gateway_runtime_state_without_startup_switch (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton = 1),
    revision INTEGER NOT NULL CHECK(revision > 0),
    process_id INTEGER,
    last_heartbeat_ms INTEGER,
    updated_at_ms INTEGER NOT NULL,
    last_started_at_ms INTEGER,
    restart_attempt INTEGER NOT NULL DEFAULT 0,
    last_error_code TEXT
);

INSERT INTO gateway_runtime_state_without_startup_switch(
    singleton, revision, process_id, last_heartbeat_ms, updated_at_ms,
    last_started_at_ms, restart_attempt, last_error_code
)
SELECT singleton, revision, process_id, last_heartbeat_ms, updated_at_ms,
       last_started_at_ms, restart_attempt, last_error_code
FROM gateway_runtime_state;

DROP TABLE gateway_runtime_state;
ALTER TABLE gateway_runtime_state_without_startup_switch RENAME TO gateway_runtime_state;
