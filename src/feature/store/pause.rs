use anyhow::{Context as _, Result, bail};
use chrono::Utc;

use super::Store;

impl Store {
    /// Marks a process as paused, so the supervisor stops relaunching it. Signaling the
    /// process itself is the caller's job; this only records the intent.
    pub async fn pause(&self, name: &str) -> Result<()> {
        set_paused(self, name, true).await
    }

    /// Clears a process's paused flag, letting the supervisor relaunch it again.
    pub async fn resume(&self, name: &str) -> Result<()> {
        set_paused(self, name, false).await
    }

    /// Whether `name` is currently paused. An unknown name (e.g. dropped from the config
    /// while paused) reads as `false`, so a worker polling this can never wedge.
    pub async fn is_paused(&self, name: &str) -> Result<bool> {
        let paused: Option<bool> =
            sqlx::query_scalar("SELECT paused FROM processes WHERE name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .with_context(|| format!("could not read paused state for '{name}'"))?;

        Ok(paused.unwrap_or(false))
    }
}

async fn set_paused(store: &Store, name: &str, paused: bool) -> Result<()> {
    let result = sqlx::query("UPDATE processes SET paused = ?, updated_at = ? WHERE name = ?")
        .bind(paused)
        .bind(Utc::now())
        .bind(name)
        .execute(&store.pool)
        .await
        .with_context(|| format!("could not update paused state for '{name}'"))?;

    if result.rows_affected() == 0 {
        bail!("no tracked process named '{name}'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::super::Store;
    use crate::feature::store::ProcessRecord;

    async fn open_store(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open")
    }

    #[tokio::test]
    async fn pause_then_resume_round_trips() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");

        assert!(!store.is_paused("api").await.expect("read should succeed"));

        store.pause("api").await.expect("pause should succeed");
        assert!(store.is_paused("api").await.expect("read should succeed"));

        store.resume("api").await.expect("resume should succeed");
        assert!(!store.is_paused("api").await.expect("read should succeed"));
    }

    #[tokio::test]
    async fn is_paused_on_unknown_name_reads_as_false() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        assert!(!store.is_paused("ghost").await.expect("read should succeed"));
    }

    #[tokio::test]
    async fn pause_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .pause("ghost")
            .await
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn resume_on_unknown_name_errors() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        let error = store
            .resume("ghost")
            .await
            .expect_err("unknown process should error");

        assert!(error.to_string().contains("ghost"));
    }

    #[tokio::test]
    async fn a_paused_flag_survives_an_upsert() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store.pause("api").await.expect("pause should succeed");

        store
            .upsert(&ProcessRecord::starting(
                "api",
                "shell",
                "cargo run --release",
            ))
            .await
            .expect("reconcile upsert should succeed");

        assert!(
            store.is_paused("api").await.expect("read should succeed"),
            "a reconcile upsert on supervisor startup should not clear an existing pause"
        );
    }
}
