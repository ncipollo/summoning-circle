use anyhow::{Context as _, Result, bail};
use chrono::Utc;

use super::Store;

impl Store {
    /// Marks a process as running with the given pid and start time, bumping
    /// `restart_count` if it had exited or been stopped (e.g. by a config-triggered restart).
    /// `pid` is `None` when a daemon-kind process is adopted while already alive rather than
    /// freshly spawned — it has no pid we can vouch for.
    pub async fn mark_running(
        &self,
        name: &str,
        pid: Option<u32>,
        start_time: Option<i64>,
    ) -> Result<()> {
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
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::super::{ProcessRecord, ProcessStatus, Store};

    async fn open_store(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open")
    }

    async fn start_time_of(store: &Store, name: &str) -> Option<i64> {
        sqlx::query_scalar("SELECT start_time FROM processes WHERE name = ?")
            .bind(name)
            .fetch_one(&store.pool)
            .await
            .expect("query should succeed")
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
            .mark_running("api", Some(123), Some(456))
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
            .mark_running("api", Some(123), Some(456))
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
            .mark_running("api", Some(456), Some(789))
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
            .mark_running("api", Some(123), Some(456))
            .await
            .expect("mark_running should succeed");
        store
            .mark_stopped("api")
            .await
            .expect("mark_stopped should succeed");

        store
            .mark_running("api", Some(789), Some(111))
            .await
            .expect("relaunch mark_running should succeed");
        let records = store.list().await.expect("list should succeed");

        assert_eq!(records[0].restart_count, 1);
    }

    #[tokio::test]
    async fn mark_running_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .mark_running("ghost", Some(1), None)
            .await
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
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
            .mark_running("api", Some(123), Some(456))
            .await
            .expect("mark_running should succeed");

        assert_eq!(start_time_of(&store, "api").await, Some(456));
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
    async fn mark_exited_clears_start_time() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", Some(123), Some(456))
            .await
            .expect("mark_running should succeed");

        store
            .mark_exited("api", Some(1))
            .await
            .expect("mark_exited should succeed");

        assert_eq!(start_time_of(&store, "api").await, None);
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
            .mark_running("api", Some(123), Some(456))
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
    async fn mark_stopped_clears_start_time() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
            .mark_running("api", Some(123), Some(456))
            .await
            .expect("mark_running should succeed");

        store
            .mark_stopped("api")
            .await
            .expect("mark_stopped should succeed");

        assert_eq!(start_time_of(&store, "api").await, None);
    }
}
