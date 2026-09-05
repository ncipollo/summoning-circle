use anyhow::{Context as _, Result};
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &["CREATE TABLE IF NOT EXISTS processes (
        name           TEXT PRIMARY KEY,
        kind           TEXT NOT NULL,
        command        TEXT NOT NULL,
        pid            INTEGER,
        status         TEXT NOT NULL,
        restart_count  INTEGER NOT NULL DEFAULT 0,
        last_exit_code INTEGER,
        started_at     TEXT,
        updated_at     TEXT NOT NULL
    )"];

/// Applies any migrations that have not yet run, tracked by `schema_version`.
pub(super) fn apply(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)")
        .context("failed to create schema_version table")?;

    let current: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .context("failed to read schema version")?;
    let current = current as usize;

    if current >= MIGRATIONS.len() {
        return Ok(());
    }

    let tx = conn
        .unchecked_transaction()
        .context("failed to start migration transaction")?;
    for migration in &MIGRATIONS[current..] {
        tx.execute_batch(migration)
            .context("failed to apply migration")?;
    }
    tx.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [MIGRATIONS.len() as i64],
    )
    .context("failed to record schema version")?;
    tx.commit()
        .context("failed to commit migration transaction")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::apply;

    #[test]
    fn applying_twice_is_a_no_op() {
        let conn = Connection::open_in_memory().expect("in-memory connection should open");

        apply(&conn).expect("first migration should apply");
        apply(&conn).expect("second migration should be a no-op");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM processes", [], |row| row.get(0))
            .expect("processes table should exist");
        assert_eq!(count, 0);
    }
}
