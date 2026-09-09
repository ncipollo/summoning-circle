pub mod record;
mod supervisor;

use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};

pub use record::{ProcessRecord, ProcessStatus, SupervisorRecord, TrackedPid};

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

    /// Marks a process as running with the given pid and start time, bumping
    /// `restart_count` if it had exited or been stopped (e.g. by a config-triggered restart).
    pub async fn mark_running(&self, name: &str, pid: u32, start_time: Option<i64>) -> Result<()> {
        let result = sqlx::query(
            "UPDATE processes
                SET pid = ?1,
                    start_time = ?2,
                    status = 'running',
                    started_at = ?3,
                    updated_at = ?3,
                    restart_count = restart_count +
                        CASE WHEN status IN ('exited', 'stopped') THEN 1 ELSE 0 END
             WHERE name = ?4",
        )
        .bind(pid)
        .bind(start_time)
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

    /// Updates a tracked process's kind/command in place, leaving its status, pid, and
    /// restart_count untouched. Used when a live config change alters an existing process's
    /// definition, just ahead of restarting it.
    pub async fn update_definition(&self, name: &str, kind: &str, command: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE processes SET kind = ?, command = ?, updated_at = ? WHERE name = ?",
        )
        .bind(kind)
        .bind(command)
        .bind(Utc::now())
        .bind(name)
        .execute(&self.pool)
        .await
        .with_context(|| format!("could not update definition for '{name}'"))?;

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
                    start_time = NULL,
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
                    start_time = NULL,
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

    /// Lists the pid and start time of every process that currently has a stored pid,
    /// used to find orphans left behind by a previous supervisor.
    pub async fn tracked_pids(&self) -> Result<Vec<TrackedPid>> {
        sqlx::query_as("SELECT pid, start_time FROM processes WHERE pid IS NOT NULL")
            .fetch_all(&self.pool)
            .await
            .context("could not list tracked pids")
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

    /// Removes a single tracked process by name, used when a live config change drops an entry.
    pub async fn remove(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM processes WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await
            .with_context(|| format!("could not remove process '{name}'"))?;

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
            .mark_running("api", 123, Some(456))
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
            .mark_running("api", 123, Some(456))
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
            .mark_running("api", 456, Some(789))
            .await
            .expect("relaunch mark_running should succeed");
        let after_relaunch = store.list().await.expect("list should succeed");
        assert_eq!(after_relaunch[0].pid, Some(456));
        assert_eq!(after_relaunch[0].restart_count, 1);
    }

    #[tokio::test]
    async fn mark_running_after_stopped_bumps_restart_count() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", 123, Some(456))
            .await
            .expect("mark_running should succeed");
        store
            .mark_stopped("api")
            .await
            .expect("mark_stopped should succeed");

        store
            .mark_running("api", 789, Some(111))
            .await
            .expect("relaunch mark_running should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records[0].restart_count, 1);
    }

    #[tokio::test]
    async fn update_definition_changes_command_without_touching_status_or_pid() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", 123, Some(456))
            .await
            .expect("mark_running should succeed");

        store
            .update_definition("api", "shell", "cargo run --release")
            .await
            .expect("update_definition should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records[0].command, "cargo run --release");
        assert_eq!(records[0].status, ProcessStatus::Running);
        assert_eq!(records[0].pid, Some(123));
    }

    #[tokio::test]
    async fn update_definition_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .update_definition("ghost", "shell", "echo hi")
            .await
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn mark_running_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .mark_running("ghost", 1, None)
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
            .mark_running("api", 123, Some(456))
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

    async fn start_time_of(store: &Store, name: &str) -> Option<i64> {
        sqlx::query_scalar("SELECT start_time FROM processes WHERE name = ?")
            .bind(name)
            .fetch_one(&store.pool)
            .await
            .expect("query should succeed")
    }

    #[tokio::test]
    async fn mark_running_persists_start_time() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");

        store
            .mark_running("api", 123, Some(456))
            .await
            .expect("mark_running should succeed");

        assert_eq!(start_time_of(&store, "api").await, Some(456));
    }

    #[tokio::test]
    async fn mark_exited_clears_start_time() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", 123, Some(456))
            .await
            .expect("mark_running should succeed");

        store
            .mark_exited("api", Some(1))
            .await
            .expect("mark_exited should succeed");

        assert_eq!(start_time_of(&store, "api").await, None);
    }

    #[tokio::test]
    async fn mark_stopped_clears_start_time() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", 123, Some(456))
            .await
            .expect("mark_running should succeed");

        store
            .mark_stopped("api")
            .await
            .expect("mark_stopped should succeed");

        assert_eq!(start_time_of(&store, "api").await, None);
    }

    #[tokio::test]
    async fn tracked_pids_skips_records_without_a_pid() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .upsert(&ProcessRecord::starting("tunnel", "shell", "ssh -N"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", 123, Some(456))
            .await
            .expect("mark_running should succeed");

        let pids = store
            .tracked_pids()
            .await
            .expect("tracked_pids should succeed");

        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0].pid, 123);
        assert_eq!(pids[0].start_time, Some(456));
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
    async fn remove_drops_only_the_named_process() {
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

        store.remove("api").await.expect("remove should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "tunnel");
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
