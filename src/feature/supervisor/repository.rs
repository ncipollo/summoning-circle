use anyhow::Result;

use crate::feature::store::{ProcessRecord, Store, SupervisorRecord, TrackedPid};

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

    pub async fn record_running(
        &self,
        name: &str,
        pid: u32,
        start_time: Option<i64>,
    ) -> Result<()> {
        self.store.mark_running(name, pid, start_time).await
    }

    pub async fn record_exited(&self, name: &str, exit_code: Option<i32>) -> Result<()> {
        self.store.mark_exited(name, exit_code).await
    }

    pub async fn record_stopped(&self, name: &str) -> Result<()> {
        self.store.mark_stopped(name).await
    }

    /// Records `pid` as the currently running supervisor, replacing any previous claim.
    pub async fn claim_supervisor(&self, pid: u32, start_time: Option<i64>) -> Result<()> {
        self.store.claim_supervisor(pid, start_time).await
    }

    /// Returns the current supervisor claim, if one has been recorded.
    pub async fn supervisor(&self) -> Result<Option<SupervisorRecord>> {
        self.store.supervisor().await
    }

    /// Clears the supervisor claim, so the next `run` sees no live owner.
    pub async fn release_supervisor(&self) -> Result<()> {
        self.store.release_supervisor().await
    }

    /// Lists the pid and start time of every process that currently has a stored pid.
    pub async fn tracked_pids(&self) -> Result<Vec<TrackedPid>> {
        self.store.tracked_pids().await
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
            .record_running("api", 123, Some(456))
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
            .record_running("api", 456, Some(789))
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

    #[tokio::test]
    async fn supervisor_claim_round_trips_then_releases() {
        let dir = TempDir::new().expect("temp dir should create");
        let (repository, _path) = open(&dir).await;

        assert_eq!(
            repository.supervisor().await.expect("read should succeed"),
            None
        );

        repository
            .claim_supervisor(123, Some(456))
            .await
            .expect("claim should succeed");
        let claim = repository
            .supervisor()
            .await
            .expect("read should succeed")
            .expect("claim should be present");
        assert_eq!(claim.pid, 123);

        repository
            .release_supervisor()
            .await
            .expect("release should succeed");
        assert_eq!(
            repository.supervisor().await.expect("read should succeed"),
            None
        );
    }

    #[tokio::test]
    async fn tracked_pids_reflects_running_processes() {
        let dir = TempDir::new().expect("temp dir should create");
        let (repository, _path) = open(&dir).await;
        repository
            .reconcile(
                &[ProcessRecord::starting("api", "shell", "cargo run")],
                &["api"],
            )
            .await
            .expect("reconcile should succeed");
        repository
            .record_running("api", 123, Some(456))
            .await
            .expect("record_running should succeed");

        let pids = repository
            .tracked_pids()
            .await
            .expect("tracked_pids should succeed");

        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0].pid, 123);
    }
}
