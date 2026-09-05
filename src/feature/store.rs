mod migrations;
pub mod record;

use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use chrono::Utc;
use rusqlite::Connection;

pub use record::{ProcessRecord, ProcessStatus};

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (creating if needed) the database at `path`, applying any pending migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create data dir for {}", path.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("could not open database at {}", path.display()))?;
        conn.busy_timeout(Duration::from_secs(5))
            .with_context(|| format!("could not set busy timeout for {}", path.display()))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
            .with_context(|| format!("could not enable WAL mode for {}", path.display()))?;
        migrations::apply(&conn)
            .with_context(|| format!("could not migrate database at {}", path.display()))?;

        Ok(Self { conn })
    }

    /// Inserts a new record, or replaces an existing one with the same name.
    pub fn upsert(&self, record: &ProcessRecord) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO processes
                    (name, kind, command, pid, status, restart_count, last_exit_code, started_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(name) DO UPDATE SET
                    kind = excluded.kind,
                    command = excluded.command,
                    pid = excluded.pid,
                    status = excluded.status,
                    restart_count = excluded.restart_count,
                    last_exit_code = excluded.last_exit_code,
                    started_at = excluded.started_at,
                    updated_at = excluded.updated_at",
                (
                    &record.name,
                    &record.kind,
                    &record.command,
                    record.pid,
                    record.status.as_str(),
                    record.restart_count,
                    record.last_exit_code,
                    record.started_at.map(|value| value.to_rfc3339()),
                    record.updated_at.to_rfc3339(),
                ),
            )
            .with_context(|| format!("could not save process '{}'", record.name))?;

        Ok(())
    }

    /// Marks a process as running with the given pid, bumping `restart_count` if it had exited.
    pub fn mark_running(&self, name: &str, pid: u32) -> Result<()> {
        let updated = self
            .conn
            .execute(
                "UPDATE processes
                    SET pid = ?1,
                        status = 'running',
                        started_at = ?2,
                        updated_at = ?2,
                        restart_count = restart_count + CASE WHEN status = 'exited' THEN 1 ELSE 0 END
                 WHERE name = ?3",
                (pid, Utc::now().to_rfc3339(), name),
            )
            .with_context(|| format!("could not mark '{name}' as running"))?;

        if updated == 0 {
            bail!("no tracked process named '{name}'");
        }
        Ok(())
    }

    /// Marks a process as exited, clearing its pid and recording the exit code.
    pub fn mark_exited(&self, name: &str, exit_code: Option<i32>) -> Result<()> {
        let updated = self
            .conn
            .execute(
                "UPDATE processes
                    SET pid = NULL,
                        status = 'exited',
                        last_exit_code = ?1,
                        updated_at = ?2
                 WHERE name = ?3",
                (exit_code, Utc::now().to_rfc3339(), name),
            )
            .with_context(|| format!("could not mark '{name}' as exited"))?;

        if updated == 0 {
            bail!("no tracked process named '{name}'");
        }
        Ok(())
    }

    /// Lists all tracked processes, ordered by name.
    pub fn list(&self) -> Result<Vec<ProcessRecord>> {
        let mut statement = self
            .conn
            .prepare("SELECT * FROM processes ORDER BY name")
            .context("could not prepare process listing")?;
        let records = statement
            .query_map([], ProcessRecord::from_row)
            .context("could not list processes")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("could not read a process row")?;

        Ok(records)
    }

    /// Removes tracked processes whose name is not in `keep`.
    pub fn remove_missing(&self, keep: &[&str]) -> Result<()> {
        let placeholders = keep.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM processes WHERE name NOT IN ({placeholders})");

        self.conn
            .execute(&sql, rusqlite::params_from_iter(keep))
            .context("could not remove stale processes")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{ProcessRecord, ProcessStatus, Store};

    fn open_store(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("nested").join("circle.db")).expect("store should open")
    }

    #[test]
    fn open_creates_missing_parent_dir() {
        let dir = TempDir::new().expect("temp dir should create");

        let _store = open_store(&dir);

        assert!(dir.path().join("nested").join("circle.db").exists());
    }

    #[test]
    fn opening_twice_does_not_error() {
        let dir = TempDir::new().expect("temp dir should create");
        let path = dir.path().join("circle.db");

        Store::open(&path).expect("first open should succeed");
        let store = Store::open(&path).expect("second open should succeed");

        assert!(store.list().expect("list should succeed").is_empty());
    }

    #[test]
    fn upsert_then_list_round_trips() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);
        let record = ProcessRecord::starting("api", "shell", "cargo run");

        store.upsert(&record).expect("upsert should succeed");
        let records = store.list().expect("list should succeed");

        assert_eq!(records, vec![record]);
    }

    #[test]
    fn upsert_replaces_existing_record() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);
        store
            .upsert(&ProcessRecord::starting("api", "shell", "one"))
            .expect("first upsert should succeed");

        store
            .upsert(&ProcessRecord::starting("api", "shell", "two"))
            .expect("second upsert should succeed");
        let records = store.list().expect("list should succeed");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].command, "two");
    }

    #[test]
    fn mark_running_sets_pid_and_status() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .expect("upsert should succeed");

        store
            .mark_running("api", 123)
            .expect("mark_running should succeed");
        let records = store.list().expect("list should succeed");

        assert_eq!(records[0].pid, Some(123));
        assert_eq!(records[0].status, ProcessStatus::Running);
        assert_eq!(records[0].restart_count, 0);
    }

    #[test]
    fn mark_running_after_exit_bumps_restart_count() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .expect("upsert should succeed");
        store
            .mark_running("api", 123)
            .expect("mark_running should succeed");

        store
            .mark_exited("api", Some(1))
            .expect("mark_exited should succeed");
        let after_exit = store.list().expect("list should succeed");
        assert_eq!(after_exit[0].pid, None);
        assert_eq!(after_exit[0].status, ProcessStatus::Exited);
        assert_eq!(after_exit[0].last_exit_code, Some(1));
        assert_eq!(after_exit[0].restart_count, 0);

        store
            .mark_running("api", 456)
            .expect("relaunch mark_running should succeed");
        let after_relaunch = store.list().expect("list should succeed");
        assert_eq!(after_relaunch[0].pid, Some(456));
        assert_eq!(after_relaunch[0].restart_count, 1);
    }

    #[test]
    fn mark_running_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);

        let error = store
            .mark_running("ghost", 1)
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[test]
    fn mark_exited_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);

        let error = store
            .mark_exited("ghost", None)
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[test]
    fn remove_missing_drops_absent_names() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);
        store
            .upsert(&ProcessRecord::starting("api", "shell", "one"))
            .expect("upsert should succeed");
        store
            .upsert(&ProcessRecord::starting("tunnel", "shell", "two"))
            .expect("upsert should succeed");

        store
            .remove_missing(&["api"])
            .expect("remove_missing should succeed");
        let records = store.list().expect("list should succeed");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "api");
    }

    #[test]
    fn remove_missing_with_empty_keep_clears_table() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir);
        store
            .upsert(&ProcessRecord::starting("api", "shell", "one"))
            .expect("upsert should succeed");

        store
            .remove_missing(&[])
            .expect("remove_missing should succeed");

        assert!(store.list().expect("list should succeed").is_empty());
    }

    #[test]
    fn a_second_handle_sees_writes_from_the_first() {
        let dir = TempDir::new().expect("temp dir should create");
        let path = dir.path().join("circle.db");
        let writer = Store::open(&path).expect("writer should open");
        let reader = Store::open(&path).expect("reader should open");

        writer
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .expect("upsert should succeed");

        let records = reader.list().expect("reader list should succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "api");
    }
}
