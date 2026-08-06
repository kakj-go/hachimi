CREATE TABLE agent_workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('managed', 'selected_directory')),
    owner_kind TEXT NOT NULL CHECK (owner_kind IN ('session', 'schedule')),
    owner_id TEXT NOT NULL,
    root_path TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ready', 'unavailable')),
    status_reason TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(owner_kind, owner_id)
);

CREATE INDEX agent_workspaces_owner_idx ON agent_workspaces(owner_kind, owner_id);

CREATE TABLE agent_permission_policies (
    owner_key TEXT PRIMARY KEY NOT NULL,
    policy_json TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE run_authority_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    snapshot_json TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
