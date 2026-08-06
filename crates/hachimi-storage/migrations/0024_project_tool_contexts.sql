CREATE TABLE project_tool_contexts (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    checkout_id TEXT NOT NULL REFERENCES workspace_checkouts(id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX project_tool_contexts_session_idx
ON project_tool_contexts(session_id);
