use anyhow::{Context as _, Result};
use chrono::Utc;

use super::Store;
use super::record::SupervisorRecord;

impl Store {
    /// Records `pid` as the currently running supervisor, replacing any previous claim.
    pub async fn claim_supervisor(&self, pid: u32, start_time: Option<i64>) -> Result<()> {
        sqlx::query(
            "INSERT INTO supervisor (id, pid, start_time, started_at)
             VALUES (1, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                pid = excluded.pid,
                start_time = excluded.start_time,
                started_at = excluded.started_at",
        )
        .bind(pid)
        .bind(start_time)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .context("could not claim supervisor")?;

        Ok(())
    }

    /// Returns the current supervisor claim, if one has been recorded.
    pub async fn supervisor(&self) -> Result<Option<SupervisorRecord>> {
        sqlx::query_as("SELECT pid, start_time FROM supervisor WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .context("could not read supervisor claim")
    }

    /// Clears the supervisor claim, so the next `run` sees no live owner.
    pub async fn release_supervisor(&self) -> Result<()> {
        sqlx::query("DELETE FROM supervisor WHERE id = 1")
            .execute(&self.pool)
            .await
            .context("could not release supervisor claim")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::super::Store;

    async fn open_store(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open")
    }

    #[tokio::test]
    async fn claim_then_read_round_trips() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        store
            .claim_supervisor(123, Some(456))
            .await
            .expect("claim should succeed");
        let claim = store
            .supervisor()
            .await
            .expect("read should succeed")
            .expect("claim should be present");

        assert_eq!(claim.pid, 123);
        assert_eq!(claim.start_time, Some(456));
    }

    #[tokio::test]
    async fn second_claim_overwrites_the_first() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .claim_supervisor(123, Some(456))
            .await
            .expect("first claim should succeed");

        store
            .claim_supervisor(789, Some(1))
            .await
            .expect("second claim should succeed");
        let claim = store
            .supervisor()
            .await
            .expect("read should succeed")
            .expect("claim should be present");

        assert_eq!(claim.pid, 789);
        assert_eq!(claim.start_time, Some(1));
    }

    #[tokio::test]
    async fn no_claim_yet_reads_as_none() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;

        assert_eq!(store.supervisor().await.expect("read should succeed"), None);
    }

    #[tokio::test]
    async fn release_clears_the_claim() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .claim_supervisor(123, Some(456))
            .await
            .expect("claim should succeed");

        store
            .release_supervisor()
            .await
            .expect("release should succeed");

        assert_eq!(store.supervisor().await.expect("read should succeed"), None);
    }
}
