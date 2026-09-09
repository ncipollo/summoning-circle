use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::policy::Policy;
use super::repository::ProcessRepository;
use super::shell;
use super::worker;
use crate::feature::config::process::ProcessEntry;

struct WorkerHandle {
    stop: watch::Sender<bool>,
    handle: JoinHandle<Result<()>>,
}

/// The set of currently running workers, each independently stoppable — unlike a single shared
/// shutdown receiver, which can only stop every worker at once.
pub struct WorkerSet {
    repository: ProcessRepository,
    log_dir: PathBuf,
    policy: Policy,
    workers: HashMap<String, WorkerHandle>,
}

impl WorkerSet {
    pub fn new(repository: ProcessRepository, log_dir: PathBuf, policy: Policy) -> Self {
        Self {
            repository,
            log_dir,
            policy,
            workers: HashMap::new(),
        }
    }

    /// Spawns a worker for `entry`, replacing any previous handle under the same name without
    /// stopping it — callers must `stop` first if that's what they want.
    pub fn spawn(&mut self, entry: &ProcessEntry) {
        let (stop_tx, stop_rx) = watch::channel(false);
        let name = entry.name.clone();
        let summon = shell::summon_for(entry, &self.log_dir);
        let repository = self.repository.clone();
        let policy = self.policy;

        let handle = tokio::spawn(async move {
            worker::run(&name, summon.as_ref(), &repository, stop_rx, &policy).await
        });

        self.workers.insert(
            entry.name.clone(),
            WorkerHandle {
                stop: stop_tx,
                handle,
            },
        );
    }

    /// Stops the named worker gracefully and waits for it to finish. A no-op if it isn't tracked.
    pub async fn stop(&mut self, name: &str) -> Result<()> {
        let Some(worker) = self.workers.remove(name) else {
            return Ok(());
        };

        let _ = worker.stop.send(true);
        worker.handle.await.expect("worker task should not panic")
    }

    /// Stops the named worker, then spawns it again with `entry`'s (new) definition.
    pub async fn restart(&mut self, entry: &ProcessEntry) -> Result<()> {
        self.stop(&entry.name).await?;
        self.spawn(entry);
        Ok(())
    }

    /// Stops every currently tracked worker, including ones added after construction.
    pub async fn stop_all(&mut self) -> Result<()> {
        let names: Vec<String> = self.workers.keys().cloned().collect();
        for name in names {
            self.stop(&name).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio::time::Instant;

    use super::*;
    use crate::feature::config::process::ProcessKind;
    use crate::feature::store::{ProcessRecord, ProcessStatus, Store};

    fn shell_entry(name: &str, command: &str) -> ProcessEntry {
        ProcessEntry {
            name: name.to_string(),
            kind: ProcessKind::Shell {
                command: command.to_string(),
                cwd: None,
                env: None,
            },
        }
    }

    fn test_policy() -> Policy {
        Policy {
            shutdown_grace: Duration::from_millis(200),
            ..Policy::default()
        }
    }

    async fn open_set(dir: &TempDir) -> (WorkerSet, Store) {
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let repository = ProcessRepository::new(store.clone());
        for name in ["api", "tunnel"] {
            repository
                .track_new(&ProcessRecord::starting(name, "shell", "unused"))
                .await
                .expect("track_new should succeed");
        }
        (
            WorkerSet::new(repository, dir.path().join("logs"), test_policy()),
            store,
        )
    }

    async fn wait_until(
        deadline: Duration,
        store: &Store,
        name: &str,
        condition: impl Fn(&ProcessRecord) -> bool,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed() < deadline {
            if let Ok(records) = store.list().await
                && records
                    .iter()
                    .find(|record| record.name == name)
                    .is_some_and(&condition)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn stop_marks_the_row_stopped_and_clears_the_pid() {
        let dir = TempDir::new().expect("temp dir should create");
        let (mut workers, store) = open_set(&dir).await;
        workers.spawn(&shell_entry("api", "sleep 5"));
        assert!(
            wait_until(Duration::from_secs(2), &store, "api", |record| {
                record.status == ProcessStatus::Running
            })
            .await
        );

        workers.stop("api").await.expect("stop should succeed");

        let records = store.list().await.expect("list should succeed");
        let api = records.iter().find(|record| record.name == "api").unwrap();
        assert_eq!(api.status, ProcessStatus::Stopped);
        assert_eq!(api.pid, None);
    }

    #[tokio::test]
    async fn stop_on_an_untracked_name_is_a_no_op() {
        let dir = TempDir::new().expect("temp dir should create");
        let (mut workers, _store) = open_set(&dir).await;

        workers
            .stop("ghost")
            .await
            .expect("stopping an untracked worker should be a no-op");
    }

    #[tokio::test]
    async fn restart_relaunches_with_a_new_pid_and_bumps_restart_count() {
        let dir = TempDir::new().expect("temp dir should create");
        let (mut workers, store) = open_set(&dir).await;
        workers.spawn(&shell_entry("api", "sleep 5"));
        assert!(
            wait_until(Duration::from_secs(2), &store, "api", |record| {
                record.status == ProcessStatus::Running
            })
            .await
        );
        let first_pid = store
            .list()
            .await
            .expect("list should succeed")
            .into_iter()
            .find(|record| record.name == "api")
            .unwrap()
            .pid;

        workers
            .restart(&shell_entry("api", "sleep 5"))
            .await
            .expect("restart should succeed");
        assert!(
            wait_until(Duration::from_secs(2), &store, "api", |record| {
                record.status == ProcessStatus::Running && record.pid != first_pid
            })
            .await
        );

        let api = store
            .list()
            .await
            .expect("list should succeed")
            .into_iter()
            .find(|record| record.name == "api")
            .unwrap();
        assert_eq!(api.restart_count, 1);
    }

    #[tokio::test]
    async fn stop_all_stops_every_worker_including_one_added_later() {
        let dir = TempDir::new().expect("temp dir should create");
        let (mut workers, store) = open_set(&dir).await;
        workers.spawn(&shell_entry("api", "sleep 5"));
        workers.spawn(&shell_entry("tunnel", "sleep 5"));
        assert!(
            wait_until(Duration::from_secs(2), &store, "tunnel", |record| {
                record.status == ProcessStatus::Running
            })
            .await
        );

        workers.stop_all().await.expect("stop_all should succeed");

        let records = store.list().await.expect("list should succeed");
        for record in &records {
            assert_eq!(
                record.status,
                ProcessStatus::Stopped,
                "{} not stopped",
                record.name
            );
        }
    }
}
