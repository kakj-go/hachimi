PRAGMA foreign_keys = ON;

CREATE TABLE run_summaries (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN (
        'succeeded', 'failed', 'timed_out', 'cancelled', 'interrupted', 'lost'
    )),
    changed_files INTEGER NOT NULL CHECK (changed_files >= 0),
    additions INTEGER NOT NULL CHECK (additions >= 0),
    deletions INTEGER NOT NULL CHECK (deletions >= 0),
    files_json TEXT NOT NULL,
    diff_artifact_id TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    diff_unavailable INTEGER NOT NULL DEFAULT 0 CHECK (diff_unavailable IN (0, 1)),
    completed_at_ms INTEGER NOT NULL
);

CREATE INDEX run_summaries_completed_idx
ON run_summaries(completed_at_ms DESC, run_id);

ALTER TABLE user_input_requests
ADD COLUMN answers_display_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE agent_task_transcript_items (
    agent_task_id TEXT PRIMARY KEY NOT NULL REFERENCES agent_tasks(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL UNIQUE REFERENCES transcript_items(id) ON DELETE CASCADE
);
