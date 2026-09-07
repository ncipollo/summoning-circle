use anyhow::Result;

use crate::feature::store::{ProcessRecord, Store};

/// Intention-revealing façade over the `Store`, used by the supervisor and its workers.
/// `Store` already clones cheaply and is safe to use concurrently, so this holds an owned
/// clone rather than wrapping it in a lock.
#[derive(Clone)]
pub struct ProcessRepository {
    store: Store,
}

impl ProcessRepository {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Upserts every record, then drops any process not present in `keep`.
    pub async fn reconcile(&self, records: &[ProcessRecord], keep: &[&str]) -> Result<()> {
        for record in records {
            self.store.upsert(record).await?;
        }
        self.store.remove_missing(keep).await
    }

    pub async fn record_running(&self, name: &str, pid: u32) -> Result<()> {
        self.store.mark_running(name, pid).await
    }

    pub async fn record_exited(&self, name: &str, exit_code: Option<i32>) -> Result<()> {
        self.store.mark_exited(name, exit_code).await
    }

    pub async fn record_stopped(&self, name: &str) -> Result<()> {
        self.store.mark_stopped(name).await
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::ProcessRepository;
    use crate::feature::store::{ProcessRecord, ProcessStatus, Store};

    async fn open(dir: &TempDir) -> (ProcessRepository, std::path::PathBuf) {
        let path = dir.path().join("circle.db");
        let store = Store::open(&path).await.expect("store should open");
        (ProcessRepository::new(store), path)
    }

    #[tokio::test]
    async fn reconcile_upserts_then_drops_missing() {
        let dir = TempDir::new().expect("temp dir should create");
        let (repository, path) = open(&dir).await;
        let records = vec![
            ProcessRecord::starting("api", "shell", "cargo run"),
            ProcessRecord::starting("tunnel", "shell", "ssh -N"),
        ];

        repository
            .reconcile(&records, &["api"])
            .await
            .expect("reconcile should succeed");

        let read_store = Store::open(&path).await.expect("reader store should open");
        let listed = read_store.list().await.expect("list should succeed");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "api");
    }

    #[tokio::test]
    async fn record_running_then_exited_then_stopped() {
        let dir = TempDir::new().expect("temp dir should create");
        let (repository, path) = open(&dir).await;
        repository
            .reconcile(
                &[ProcessRecord::starting("api", "shell", "cargo run")],
                &["api"],
            )
            .await
            .expect("reconcile should succeed");
        let read_store = Store::open(&path).await.expect("reader store should open");

        repository
            .record_running("api", 123)
            .await
            .expect("record_running should succeed");
        assert_eq!(
            read_store.list().await.expect("list should succeed")[0].status,
            ProcessStatus::Running
        );

        repository
            .record_exited("api", Some(1))
            .await
            .expect("record_exited should succeed");
        assert_eq!(
            read_store.list().await.expect("list should succeed")[0].status,
            ProcessStatus::Exited
        );

        repository
            .record_running("api", 456)
            .await
            .expect("record_running should succeed");
        repository
            .record_stopped("api")
            .await
            .expect("record_stopped should succeed");
        let record = &read_store.list().await.expect("list should succeed")[0];
        assert_eq!(record.status, ProcessStatus::Stopped);
        assert_eq!(record.pid, None);
    }
}
