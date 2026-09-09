CREATE TABLE IF NOT EXISTS supervisor (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    pid        INTEGER NOT NULL,
    start_time INTEGER,
    started_at TEXT NOT NULL
);
