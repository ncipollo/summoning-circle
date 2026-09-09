mod backoff;
mod config_watch;
mod diff;
pub mod policy;
mod repository;
pub mod shell;
pub mod signals;
mod startup;
mod worker;
mod worker_set;

use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::watch;
use tracing::warn;

use policy::Policy;
use repository::ProcessRepository;
use worker_set::WorkerSet;

use crate::feature::config;
use crate::feature::config::Config;
use crate::feature::config::process::{ProcessEntry, ProcessKind};
use crate::feature::store::{ProcessRecord, Store};

/// Launches every configured process, keeps it alive, watches the config file for changes, and
/// shuts everything down on signal.
pub struct Supervisor {
    entries: Vec<ProcessEntry>,
    repository: ProcessRepository,
    log_dir: PathBuf,
    config_path: PathBuf,
    policy: Policy,
}

impl Supervisor {
    pub fn new(
        config: &Config,
        store: Store,
        log_dir: PathBuf,
        config_path: PathBuf,
        policy: Policy,
    ) -> Self {
        Self {
            entries: config.processes.clone(),
            repository: ProcessRepository::new(store),
            log_dir,
            config_path,
            policy,
        }
    }

    /// Runs until `shutdown` fires, then stops every child before returning.
    ///
    /// Before spawning anything, this refuses to start alongside another live supervisor
    /// and reclaims any orphaned children left behind by a previous crashed run. While
    /// running, it watches the config file and reconciles added, removed, and changed
    /// entries without disturbing anything else. The supervisor claim is always released
    /// before returning, even if a worker errors, so the next `run` doesn't mistake this
    /// process for still being alive.
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) -> Result<()> {
        startup::claim(&self.repository, &self.policy).await?;
        self.reconcile(&self.entries).await?;

        let mut entries = self.entries.clone();
        let mut workers =
            WorkerSet::new(self.repository.clone(), self.log_dir.clone(), self.policy);
        for entry in &entries {
            workers.spawn(entry);
        }

        let (_watcher, mut config_changes) =
            config_watch::watch(&self.config_path, self.policy.config_debounce)?;

        let outcome = loop {
            tokio::select! {
                _ = shutdown.changed() => break workers.stop_all().await,
                Some(()) = config_changes.recv() => {
                    if let Err(error) = self.apply_config_change(&mut entries, &mut workers).await {
                        warn!(%error, "ignoring config change");
                    }
                }
            }
        };

        self.repository.release_supervisor().await?;
        outcome
    }

    /// Reloads the config, diffs it against `entries`, and applies just the difference:
    /// removed entries are stopped and dropped from the store, added ones are tracked and
    /// spawned, and changed ones are gracefully restarted. Everything else is untouched.
    async fn apply_config_change(
        &self,
        entries: &mut Vec<ProcessEntry>,
        workers: &mut WorkerSet,
    ) -> Result<()> {
        let new_entries = config::load(&self.config_path)?.processes;
        let changes = diff::diff(entries, &new_entries);

        for name in &changes.removed {
            workers.stop(name).await?;
            self.repository.untrack(name).await?;
        }
        for entry in &changes.added {
            self.repository.track_new(&starting_record(entry)).await?;
            workers.spawn(entry);
        }
        for entry in &changes.changed {
            let (kind, command) = kind_and_command(entry);
            self.repository
                .update_definition(&entry.name, kind, command)
                .await?;
            workers.restart(entry).await?;
        }

        *entries = new_entries;
        Ok(())
    }

    async fn reconcile(&self, entries: &[ProcessEntry]) -> Result<()> {
        let records: Vec<ProcessRecord> = entries.iter().map(starting_record).collect();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        self.repository.reconcile(&records, &names).await
    }
}

fn kind_and_command(entry: &ProcessEntry) -> (&'static str, &str) {
    match &entry.kind {
        ProcessKind::Shell { command, .. } => ("shell", command.as_str()),
    }
}

fn starting_record(entry: &ProcessEntry) -> ProcessRecord {
    let (kind, command) = kind_and_command(entry);
    ProcessRecord::starting(entry.name.as_str(), kind, command)
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
            config_debounce: Duration::from_millis(20),
        }
    }

    fn write_config(path: &std::path::Path, contents: &str) {
        std::fs::write(path, contents).expect("config write should succeed");
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

    async fn wait_until_named(
        deadline: Duration,
        store: &Store,
        name: &str,
        condition: impl Fn(&crate::feature::store::ProcessRecord) -> bool,
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
    async fn relaunches_a_failing_process() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let config_path = dir.path().join("config.toml");
        write_config(
            &config_path,
            r#"
            [[process]]
            name = "flapper"
            type = "shell"
            command = "exit 1"
        "#,
        );
        let config =
            parse(&std::fs::read_to_string(&config_path).unwrap()).expect("config should parse");
        let supervisor = Supervisor::new(
            &config,
            store,
            dir.path().join("logs"),
            config_path,
            test_policy(),
        );
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
        let config_path = dir.path().join("config.toml");
        write_config(
            &config_path,
            r#"
            [[process]]
            name = "flapper"
            type = "shell"
            command = "exit 1"
        "#,
        );
        let config =
            parse(&std::fs::read_to_string(&config_path).unwrap()).expect("config should parse");
        let policy = Policy {
            initial_backoff: Duration::from_secs(10),
            ..test_policy()
        };
        let supervisor =
            Supervisor::new(&config, store, dir.path().join("logs"), config_path, policy);
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
        let config_path = dir.path().join("config.toml");
        write_config(
            &config_path,
            r#"
            [[process]]
            name = "sleeper"
            type = "shell"
            command = "sleep 5"
        "#,
        );
        let config =
            parse(&std::fs::read_to_string(&config_path).unwrap()).expect("config should parse");
        let supervisor = Supervisor::new(
            &config,
            store,
            dir.path().join("logs"),
            config_path,
            test_policy(),
        );
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
        let config_path = dir.path().join("config.toml");
        write_config(&config_path, "");
        let config = parse("").expect("empty config should parse");
        let supervisor = Supervisor::new(
            &config,
            store,
            dir.path().join("logs"),
            config_path,
            test_policy(),
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
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

    #[tokio::test]
    async fn editing_the_config_restarts_only_the_changed_process() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let config_path = dir.path().join("config.toml");
        write_config(
            &config_path,
            r#"
            [[process]]
            name = "api"
            type = "shell"
            command = "sleep 5"

            [[process]]
            name = "tunnel"
            type = "shell"
            command = "sleep 5"
        "#,
        );
        let config =
            parse(&std::fs::read_to_string(&config_path).unwrap()).expect("config should parse");
        let supervisor = Supervisor::new(
            &config,
            store,
            dir.path().join("logs"),
            config_path.clone(),
            test_policy(),
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let read_store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("reader store should open");

        let handle = tokio::spawn(supervisor.run(shutdown_rx));
        assert!(
            wait_until_named(Duration::from_secs(5), &read_store, "tunnel", |record| {
                record.status == ProcessStatus::Running
            })
            .await
        );
        let tunnel_pid_before = read_store
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|record| record.name == "tunnel")
            .unwrap()
            .pid;

        write_config(
            &config_path,
            r#"
            [[process]]
            name = "api"
            type = "shell"
            command = "sleep 6"

            [[process]]
            name = "tunnel"
            type = "shell"
            command = "sleep 5"
        "#,
        );

        assert!(
            wait_until_named(Duration::from_secs(5), &read_store, "api", |record| {
                record.command == "sleep 6" && record.restart_count >= 1
            })
            .await
        );
        let tunnel_pid_after = read_store
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|record| record.name == "tunnel")
            .unwrap()
            .pid;
        assert_eq!(
            tunnel_pid_before, tunnel_pid_after,
            "unrelated process should keep its pid"
        );

        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
        handle
            .await
            .expect("supervisor task should not panic")
            .expect("supervisor should exit cleanly");
    }

    #[tokio::test]
    async fn adding_and_removing_entries_updates_the_store() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let config_path = dir.path().join("config.toml");
        write_config(
            &config_path,
            r#"
            [[process]]
            name = "api"
            type = "shell"
            command = "sleep 5"
        "#,
        );
        let config =
            parse(&std::fs::read_to_string(&config_path).unwrap()).expect("config should parse");
        let supervisor = Supervisor::new(
            &config,
            store,
            dir.path().join("logs"),
            config_path.clone(),
            test_policy(),
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let read_store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("reader store should open");

        let handle = tokio::spawn(supervisor.run(shutdown_rx));
        assert!(
            wait_until_named(Duration::from_secs(5), &read_store, "api", |record| {
                record.status == ProcessStatus::Running
            })
            .await
        );

        write_config(
            &config_path,
            r#"
            [[process]]
            name = "worker"
            type = "shell"
            command = "sleep 5"
        "#,
        );

        assert!(
            wait_until_named(Duration::from_secs(5), &read_store, "worker", |record| {
                record.status == ProcessStatus::Running
            })
            .await
        );
        let start = Instant::now();
        let api_gone = loop {
            if start.elapsed() > Duration::from_secs(5) {
                break false;
            }
            let records = read_store.list().await.unwrap();
            if !records.iter().any(|record| record.name == "api") {
                break true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(api_gone, "removed entry should be dropped from the store");

        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
        handle
            .await
            .expect("supervisor task should not panic")
            .expect("supervisor should exit cleanly");
    }

    #[tokio::test]
    async fn an_invalid_config_edit_is_ignored() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let config_path = dir.path().join("config.toml");
        write_config(
            &config_path,
            r#"
            [[process]]
            name = "api"
            type = "shell"
            command = "sleep 5"
        "#,
        );
        let config =
            parse(&std::fs::read_to_string(&config_path).unwrap()).expect("config should parse");
        let supervisor = Supervisor::new(
            &config,
            store,
            dir.path().join("logs"),
            config_path.clone(),
            test_policy(),
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let read_store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("reader store should open");

        let handle = tokio::spawn(supervisor.run(shutdown_rx));
        assert!(
            wait_until_named(Duration::from_secs(5), &read_store, "api", |record| {
                record.status == ProcessStatus::Running
            })
            .await
        );
        let pid_before = read_store
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|record| record.name == "api")
            .unwrap()
            .pid;

        write_config(&config_path, "not valid toml [[[");
        tokio::time::sleep(Duration::from_millis(200)).await;

        let pid_after = read_store
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|record| record.name == "api")
            .unwrap()
            .pid;
        assert_eq!(
            pid_before, pid_after,
            "an invalid config edit should not touch running processes"
        );

        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
        handle
            .await
            .expect("supervisor task should not panic")
            .expect("supervisor should exit cleanly");
    }
}
