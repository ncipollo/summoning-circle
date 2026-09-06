CREATE TABLE IF NOT EXISTS processes (
    name           TEXT PRIMARY KEY,
    kind           TEXT NOT NULL,
    command        TEXT NOT NULL,
    pid            INTEGER,
    status         TEXT NOT NULL,
    restart_count  INTEGER NOT NULL DEFAULT 0,
    last_exit_code INTEGER,
    started_at     TEXT,
    updated_at     TEXT NOT NULL
);
