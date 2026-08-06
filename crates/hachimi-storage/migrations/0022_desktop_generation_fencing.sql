PRAGMA foreign_keys = ON;

ALTER TABLE browser_sessions
ADD COLUMN owner_run_generation INTEGER NOT NULL DEFAULT 1 CHECK(owner_run_generation > 0);

UPDATE browser_sessions
SET owner_run_generation = COALESCE(
    (SELECT generation FROM runs WHERE runs.id = browser_sessions.owner_run_id),
    1
);

