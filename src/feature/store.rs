pub mod record;

use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};

pub use record::{ProcessRecord, ProcessStatus};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens (creating if needed) the database at `path`, applying any pending migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create data dir for {}", path.display()))?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePool::connect_with(options)
            .await
            .with_context(|| format!("could not open database at {}", path.display()))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .with_context(|| format!("could not migrate database at {}", path.display()))?;

        Ok(Self { pool })
    }

    /// Inserts a new record, or replaces an existing one with the same name.
    pub async fn upsert(&self, record: &ProcessRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO processes
                (name, kind, command, pid, status, restart_count, last_exit_code, started_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET
                kind = excluded.kind,
                command = excluded.command,
                pid = excluded.pid,
                status = excluded.status,
                restart_count = excluded.restart_count,
                last_exit_code = excluded.last_exit_code,
                started_at = excluded.started_at,
                updated_at = excluded.updated_at",
        )
        .bind(&record.name)
        .bind(&record.kind)
        .bind(&record.command)
        .bind(record.pid)
        .bind(record.status)
        .bind(record.restart_count)
        .bind(record.last_exit_code)
        .bind(record.started_at)
        .bind(record.updated_at)
        .execute(&self.pool)
        .await
        .with_context(|| format!("could not save process '{}'", record.name))?;

        Ok(())
    }

    /// Marks a process as running with the given pid, bumping `restart_count` if it had exited.
    pub async fn mark_running(&self, name: &str, pid: u32) -> Result<()> {
        let result = sqlx::query(
            "UPDATE processes
                SET pid = ?1,
                    status = 'running',
                    started_at = ?2,
                    updated_at = ?2,
                    restart_count = restart_count + CASE WHEN status = 'exited' THEN 1 ELSE 0 END
             WHERE name = ?3",
        )
        .bind(pid)
        .bind(Utc::now())
        .bind(name)
        .execute(&self.pool)
        .await
        .with_context(|| format!("could not mark '{name}' as running"))?;

        if result.rows_affected() == 0 {
            bail!("no tracked process named '{name}'");
        }
        Ok(())
    }

    /// Marks a process as exited, clearing its pid and recording the exit code.
    pub async fn mark_exited(&self, name: &str, exit_code: Option<i32>) -> Result<()> {
        let result = sqlx::query(
            "UPDATE processes
                SET pid = NULL,
                    status = 'exited',
                    last_exit_code = ?,
                    updated_at = ?
             WHERE name = ?",
        )
        .bind(exit_code)
        .bind(Utc::now())
        .bind(name)
        .execute(&self.pool)
        .await
        .with_context(|| format!("could not mark '{name}' as exited"))?;

        if result.rows_affected() == 0 {
            bail!("no tracked process named '{name}'");
        }
        Ok(())
    }

    /// Marks a process as stopped, clearing its pid.
    pub async fn mark_stopped(&self, name: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE processes
                SET pid = NULL,
                    status = 'stopped',
                    updated_at = ?
             WHERE name = ?",
        )
        .bind(Utc::now())
        .bind(name)
        .execute(&self.pool)
        .await
        .with_context(|| format!("could not mark '{name}' as stopped"))?;

        if result.rows_affected() == 0 {
            bail!("no tracked process named '{name}'");
        }
        Ok(())
    }

    /// Lists all tracked processes, ordered by name.
    pub async fn list(&self) -> Result<Vec<ProcessRecord>> {
        sqlx::query_as("SELECT * FROM processes ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .context("could not list processes")
    }

    /// Removes tracked processes whose name is not in `keep`.
    pub async fn remove_missing(&self, keep: &[&str]) -> Result<()> {
        let placeholders = keep.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM processes WHERE name NOT IN ({placeholders})");

        let query = keep
            .iter()
            .fold(sqlx::query(&sql), |query, name| query.bind(*name));
        query
            .execute(&self.pool)
            .await
            .context("could not remove stale processes")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{ProcessRecord, ProcessStatus, Store};

    async fn open_store(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("nested").join("circle.db"))
            .await
            .expect("store should open")
    }

    #[tokio::test]
    async fn open_creates_missing_parent_dir() {
        let dir = TempDir::new().expect("temp dir should create");

        let _store = open_store(&dir).await;

        assert!(dir.path().join("nested").join("circle.db").exists());
    }

    #[tokio::test]
    async fn opening_twice_does_not_error() {
        let dir = TempDir::new().expect("temp dir should create");
        let path = dir.path().join("circle.db");

        Store::open(&path).await.expect("first open should succeed");
        let store = Store::open(&path)
            .await
            .expect("second open should succeed");

        assert!(store.list().await.expect("list should succeed").is_empty());
    }

    #[tokio::test]
    async fn upsert_then_list_round_trips() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        let record = ProcessRecord::starting("api", "shell", "cargo run");

        store.upsert(&record).await.expect("upsert should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records, vec![record]);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_record() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "one"))
            .await
            .expect("first upsert should succeed");

        store
            .upsert(&ProcessRecord::starting("api", "shell", "two"))
            .await
            .expect("second upsert should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].command, "two");
    }

    #[tokio::test]
    async fn mark_running_sets_pid_and_status() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");

        store
            .mark_running("api", 123)
            .await
            .expect("mark_running should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records[0].pid, Some(123));
        assert_eq!(records[0].status, ProcessStatus::Running);
        assert_eq!(records[0].restart_count, 0);
    }

    #[tokio::test]
    async fn mark_running_after_exit_bumps_restart_count() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", 123)
            .await
            .expect("mark_running should succeed");

        store
            .mark_exited("api", Some(1))
            .await
            .expect("mark_exited should succeed");
        let after_exit = store.list().await.expect("list should succeed");
        assert_eq!(after_exit[0].pid, None);
        assert_eq!(after_exit[0].status, ProcessStatus::Exited);
        assert_eq!(after_exit[0].last_exit_code, Some(1));
        assert_eq!(after_exit[0].restart_count, 0);

        store
            .mark_running("api", 456)
            .await
            .expect("relaunch mark_running should succeed");
        let after_relaunch = store.list().await.expect("list should succeed");
        assert_eq!(after_relaunch[0].pid, Some(456));
        assert_eq!(after_relaunch[0].restart_count, 1);
    }

    #[tokio::test]
    async fn mark_running_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .mark_running("ghost", 1)
            .await
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn mark_stopped_clears_pid() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", 123)
            .await
            .expect("mark_running should succeed");

        store
            .mark_stopped("api")
            .await
            .expect("mark_stopped should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records[0].pid, None);
        assert_eq!(records[0].status, ProcessStatus::Stopped);
    }

    #[tokio::test]
    async fn mark_stopped_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .mark_stopped("ghost")
            .await
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn mark_exited_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .mark_exited("ghost", None)
            .await
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn remove_missing_drops_absent_names() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "one"))
            .await
            .expect("upsert should succeed");
        store
            .upsert(&ProcessRecord::starting("tunnel", "shell", "two"))
            .await
            .expect("upsert should succeed");

        store
            .remove_missing(&["api"])
            .await
            .expect("remove_missing should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "api");
    }

    #[tokio::test]
    async fn remove_missing_with_empty_keep_clears_table() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "one"))
            .await
            .expect("upsert should succeed");

        store
            .remove_missing(&[])
            .await
            .expect("remove_missing should succeed");

        assert!(store.list().await.expect("list should succeed").is_empty());
    }

    #[tokio::test]
    async fn a_second_handle_sees_writes_from_the_first() {
        let dir = TempDir::new().expect("temp dir should create");
        let path = dir.path().join("circle.db");
        let writer = Store::open(&path).await.expect("writer should open");
        let reader = Store::open(&path).await.expect("reader should open");

        writer
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");

        let records = reader.list().await.expect("reader list should succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "api");
    }
}
