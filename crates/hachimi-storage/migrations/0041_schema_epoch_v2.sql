CREATE TABLE local_schema_epoch (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    epoch INTEGER NOT NULL CHECK (epoch = 2),
    initialized_at_ms INTEGER NOT NULL
);

INSERT INTO local_schema_epoch(singleton, epoch, initialized_at_ms)
VALUES (1, 2, unixepoch('subsec') * 1000);
