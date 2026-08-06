PRAGMA foreign_keys = ON;

ALTER TABLE agent_tasks ADD COLUMN execution_generation INTEGER NOT NULL DEFAULT 0 CHECK(execution_generation >= 0);
ALTER TABLE agent_tasks ADD COLUMN lease_owner TEXT;
ALTER TABLE agent_tasks ADD COLUMN lease_expires_at_ms INTEGER;
ALTER TABLE agent_tasks ADD COLUMN last_reconciled_at_ms INTEGER;

CREATE INDEX agent_tasks_reconcile_idx
ON agent_tasks(status, lease_expires_at_ms, updated_at_ms, id);

