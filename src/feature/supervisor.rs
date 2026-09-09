mod backoff;
pub mod policy;
mod repository;
pub mod shell;
pub mod signals;
mod startup;
mod worker;

use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use policy::Policy;
use repository::ProcessRepository;

use crate::feature::config::Config;
use crate::feature::config::process::{ProcessEntry, ProcessKind};
use crate::feature::store::{ProcessRecord, Store};

/// Launches every configured process, keeps it alive, and shuts everything down on signal.
pub struct Supervisor {
    entries: Vec<ProcessEntry>,
    repository: ProcessRepository,
    log_dir: PathBuf,
    policy: Policy,
}

impl Supervisor {
    pub fn new(config: &Config, store: Store, log_dir: PathBuf, policy: Policy) -> Self {
        Self {
            entries: config.processes.clone(),
            repository: ProcessRepository::new(store),
            log_dir,
            policy,
        }
    }

    /// Runs until `shutdown` fires, then stops every child before returning.
    ///
    /// Before spawning anything, this refuses to start alongside another live supervisor
    /// and reclaims any orphaned children left behind by a previous crashed run. The
    /// supervisor claim is always released before returning, even if a worker errors, so
    /// the next `run` doesn't mistake this process for still being alive.
    pub async fn run(self, shutdown: watch::Receiver<bool>) -> Result<()> {
        startup::claim(&self.repository, &self.policy).await?;

        self.reconcile().await?;

        let handles: Vec<_> = self
            .entries
            .iter()
            .map(|entry| self.spawn_worker(entry, shutdown.clone()))
            .collect();

        let mut outcome = Ok(());
        for handle in handles {
            if let Err(error) = handle.await.expect("worker task should not panic") {
                outcome = Err(error);
            }
        }

        self.repository.release_supervisor().await?;
        outcome
    }

    async fn reconcile(&self) -> Result<()> {
        let records: Vec<ProcessRecord> = self.entries.iter().map(starting_record).collect();
        let names: Vec<&str> = self
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();

        self.repository.reconcile(&records, &names).await
    }

    fn spawn_worker(
        &self,
        entry: &ProcessEntry,
        shutdown: watch::Receiver<bool>,
    ) -> JoinHandle<Result<()>> {
        let name = entry.name.clone();
        let summon = shell::summon_for(entry, &self.log_dir);
        let repository = self.repository.clone();
        let policy = self.policy;

        tokio::spawn(async move {
            worker::run(&name, summon.as_ref(), &repository, shutdown, &policy).await
        })
    }
}

fn starting_record(entry: &ProcessEntry) -> ProcessRecord {
    match &entry.kind {
        ProcessKind::Shell { command, .. } => {
            ProcessRecord::starting(entry.name.as_str(), "shell", command.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio::sync::watch;
    use tokio::time::Instant;

    use super::Supervisor;
    use super::policy::Policy;
    use crate::feature::config::parse;
    use crate::feature::store::{ProcessStatus, Store};

    fn test_policy() -> Policy {
        Policy {
            initial_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(50),
            uptime_reset: Duration::from_secs(60),
            shutdown_grace: Duration::from_millis(500),
        }
    }

    async fn wait_until(
        deadline: Duration,
        store: &Store,
        condition: impl Fn(&crate::feature::store::ProcessRecord) -> bool,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if let Ok(records) = store.list().await
                && records.first().is_some_and(&condition)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn relaunches_a_failing_process() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let config = parse(
            r#"
            [[process]]
            name = "flapper"
            type = "shell"
            command = "exit 1"
        "#,
        )
        .expect("config should parse");
        let supervisor = Supervisor::new(&config, store, dir.path().join("logs"), test_policy());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let read_store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("reader store should open");

        let handle = tokio::spawn(supervisor.run(shutdown_rx));

        let relaunched = wait_until(Duration::from_secs(5), &read_store, |record| {
            record.restart_count >= 1
        })
        .await;
        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
        handle
            .await
            .expect("supervisor task should not panic")
            .expect("supervisor should exit cleanly");

        assert!(
            relaunched,
            "expected the flapper to be relaunched at least once"
        );
    }

    #[tokio::test]
    async fn shutdown_interrupts_a_long_backoff_sleep() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let config = parse(
            r#"
            [[process]]
            name = "flapper"
            type = "shell"
            command = "exit 1"
        "#,
        )
        .expect("config should parse");
        let policy = Policy {
            initial_backoff: Duration::from_secs(10),
            ..test_policy()
        };
        let supervisor = Supervisor::new(&config, store, dir.path().join("logs"), policy);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let read_store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("reader store should open");

        let handle = tokio::spawn(supervisor.run(shutdown_rx));

        let exited = wait_until(Duration::from_secs(5), &read_store, |record| {
            record.status == ProcessStatus::Exited
        })
        .await;
        assert!(exited, "expected the flapper to exit and enter backoff");

        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown should not wait out the 10s backoff sleep")
            .expect("supervisor task should not panic")
            .expect("supervisor should exit cleanly");
    }

    #[tokio::test]
    async fn stops_a_running_process_on_shutdown() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let config = parse(
            r#"
            [[process]]
            name = "sleeper"
            type = "shell"
            command = "sleep 5"
        "#,
        )
        .expect("config should parse");
        let supervisor = Supervisor::new(&config, store, dir.path().join("logs"), test_policy());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let read_store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("reader store should open");

        let handle = tokio::spawn(supervisor.run(shutdown_rx));

        let running = wait_until(Duration::from_secs(5), &read_store, |record| {
            record.status == ProcessStatus::Running && record.pid.is_some()
        })
        .await;
        assert!(running, "expected the sleeper to reach running status");

        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
        handle
            .await
            .expect("supervisor task should not panic")
            .expect("supervisor should exit cleanly");

        let records = read_store.list().await.expect("list should succeed");
        assert_eq!(records[0].status, ProcessStatus::Stopped);
        assert_eq!(records[0].pid, None);
    }

    #[tokio::test]
    async fn reconcile_drops_entries_removed_from_config() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        store
            .upsert(&crate::feature::store::ProcessRecord::starting(
                "stale",
                "shell",
                "echo stale",
            ))
            .await
            .expect("upsert should succeed");
        let config = parse("").expect("empty config should parse");
        let supervisor = Supervisor::new(&config, store, dir.path().join("logs"), test_policy());
        let (_shutdown_tx, shutdown_rx) = watch::channel(true);

        supervisor
            .run(shutdown_rx)
            .await
            .expect("supervisor should exit cleanly");

        let read_store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("reader store should open");
        assert!(
            read_store
                .list()
                .await
                .expect("list should succeed")
                .is_empty()
        );
    }
}
