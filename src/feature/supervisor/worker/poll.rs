use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::watch;
use tokio::time;
use tracing::info;

use super::Ended;
use crate::feature::proc;
use crate::feature::supervisor::daemon::Daemon;
use crate::feature::supervisor::policy::Policy;
use crate::feature::supervisor::repository::ProcessRepository;
use crate::feature::supervisor::shell::Summon;

/// Whether a freshly launched daemon was confirmed alive, or the launch ended before that
/// could happen.
enum Confirmed {
    Alive,
    Ended(Ended),
}

/// Spawns and monitors a daemon-kind process to completion. Unlike a signal-controlled
/// process, a daemon has no long-lived child handle to wait on — `start` commonly forks and
/// exits immediately — so liveness is polled via `status` instead, and shutdown runs `stop`
/// rather than signaling a pid.
pub async fn launch_daemon(
    name: &str,
    summon: &dyn Summon,
    daemon: &Daemon,
    repository: &ProcessRepository,
    shutdown: &mut watch::Receiver<bool>,
    policy: &Policy,
) -> Result<Ended> {
    if daemon.status_ok(policy.daemon_command_timeout).await {
        info!(name, "daemon already alive; adopting instead of starting");
        repository.record_running(name, None, None).await?;
    } else {
        match start_and_confirm(name, summon, daemon, repository, shutdown, policy).await? {
            Confirmed::Alive => {}
            Confirmed::Ended(ended) => return Ok(ended),
        }
    }

    // Mirrors `launch_signal`'s single post-launch check: `pause`'s CLI path writes the paused
    // flag before it reads the process list, so this check and that direct stop jointly cover
    // every interleaving without the poll loop below needing to watch for a pause itself.
    if repository.is_paused(name).await? {
        return stop_for_pause(name, daemon, repository, policy).await;
    }

    poll_until_dead(name, daemon, repository, shutdown, policy).await
}

/// Runs `start`, records the (best-effort) pid, and waits for the daemon to report itself
/// alive. A non-zero `start` exit is a failed launch, not a daemon that started and died.
async fn start_and_confirm(
    name: &str,
    summon: &dyn Summon,
    daemon: &Daemon,
    repository: &ProcessRepository,
    shutdown: &mut watch::Receiver<bool>,
    policy: &Policy,
) -> Result<Confirmed> {
    let mut child = summon.spawn()?;
    let pid = child.id();
    repository
        .record_running(name, pid, pid.and_then(proc::start_time))
        .await?;
    info!(name, ?pid, "ran daemon start command");

    tokio::select! {
        status = child.wait() => {
            let status = status?;
            if !status.success() {
                info!(name, code = status.code(), "daemon start command exited non-zero");
                repository.record_exited(name, status.code()).await?;
                return Ok(Confirmed::Ended(Ended::Exited { uptime: Duration::ZERO }));
            }
        }
        _ = shutdown.changed() => {
            daemon.stop(policy.daemon_command_timeout).await;
            repository.record_stopped(name).await?;
            return Ok(Confirmed::Ended(Ended::Shutdown));
        }
    }

    await_first_alive(name, daemon, repository, shutdown, policy).await
}

/// Polls `status` until it reports alive, bounded by `policy.daemon_start_grace`. Without this
/// bound, a daemon whose `start` forks correctly but whose `status` is slow to report life
/// would be declared dead and relaunched needlessly.
async fn await_first_alive(
    name: &str,
    daemon: &Daemon,
    repository: &ProcessRepository,
    shutdown: &mut watch::Receiver<bool>,
    policy: &Policy,
) -> Result<Confirmed> {
    let started = Instant::now();

    loop {
        if daemon.status_ok(policy.daemon_command_timeout).await {
            info!(name, "daemon is alive");
            return Ok(Confirmed::Alive);
        }
        if started.elapsed() >= policy.daemon_start_grace {
            info!(
                name,
                "daemon did not report alive within the start grace period"
            );
            repository.record_exited(name, None).await?;
            return Ok(Confirmed::Ended(Ended::Exited {
                uptime: Duration::ZERO,
            }));
        }

        tokio::select! {
            () = time::sleep(policy.status_poll) => {}
            _ = shutdown.changed() => {
                daemon.stop(policy.daemon_command_timeout).await;
                repository.record_stopped(name).await?;
                return Ok(Confirmed::Ended(Ended::Shutdown));
            }
        }
    }
}

/// Polls `status` at `policy.status_poll` until it reports dead or `shutdown` fires.
async fn poll_until_dead(
    name: &str,
    daemon: &Daemon,
    repository: &ProcessRepository,
    shutdown: &mut watch::Receiver<bool>,
    policy: &Policy,
) -> Result<Ended> {
    let started_at = Instant::now();

    loop {
        tokio::select! {
            () = time::sleep(policy.status_poll) => {
                if !daemon.status_ok(policy.daemon_command_timeout).await {
                    info!(name, "daemon status reports dead");
                    repository.record_exited(name, None).await?;
                    return Ok(Ended::Exited { uptime: started_at.elapsed() });
                }
            }
            _ = shutdown.changed() => {
                daemon.stop(policy.daemon_command_timeout).await;
                repository.record_stopped(name).await?;
                info!(name, "stopped daemon");
                return Ok(Ended::Shutdown);
            }
        }
    }
}

async fn stop_for_pause(
    name: &str,
    daemon: &Daemon,
    repository: &ProcessRepository,
    policy: &Policy,
) -> Result<Ended> {
    daemon.stop(policy.daemon_command_timeout).await;
    repository.record_stopped(name).await?;
    info!(name, "stopped daemon after a pause raced the launch");
    Ok(Ended::Paused)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio::sync::watch;
    use tokio::time::Instant;

    use super::launch_daemon;
    use crate::feature::config::process::{ProcessEntry, ProcessKind};
    use crate::feature::store::{ProcessRecord, ProcessStatus, Store};
    use crate::feature::supervisor::policy::Policy;
    use crate::feature::supervisor::repository::ProcessRepository;
    use crate::feature::supervisor::shell::{self, Lifecycle};

    fn test_policy() -> Policy {
        Policy {
            status_poll: Duration::from_millis(20),
            daemon_start_grace: Duration::from_millis(200),
            daemon_command_timeout: Duration::from_secs(2),
            shutdown_grace: Duration::from_millis(200),
            ..Policy::default()
        }
    }

    fn daemon_entry(name: &str, start: &str, stop: &str, status: &str) -> ProcessEntry {
        ProcessEntry {
            name: name.to_string(),
            kind: ProcessKind::Daemon {
                start: start.to_string(),
                stop: stop.to_string(),
                status: status.to_string(),
            },
        }
    }

    async fn open_repository(dir: &TempDir) -> (ProcessRepository, Store) {
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        let repository = ProcessRepository::new(store.clone());
        repository
            .track_new(&ProcessRecord::starting("api", "daemon", "unused"))
            .await
            .expect("track_new should succeed");
        (repository, store)
    }

    #[tokio::test]
    async fn a_daemon_that_stays_alive_is_recorded_running_until_shutdown() {
        let dir = TempDir::new().expect("temp dir should create");
        let (repository, store) = open_repository(&dir).await;
        let entry = daemon_entry("api", "true", "true", "true");
        let log_dir = dir.path().to_path_buf();
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        let handle = tokio::spawn(async move {
            let summon = shell::summon_for(&entry, &log_dir);
            let Lifecycle::Daemon(daemon) = summon.lifecycle() else {
                panic!("expected a daemon lifecycle");
            };
            launch_daemon(
                "api",
                summon.as_ref(),
                daemon,
                &repository,
                &mut shutdown_rx,
                &test_policy(),
            )
            .await
        });

        let start = Instant::now();
        let running = loop {
            if start.elapsed() > Duration::from_secs(2) {
                break false;
            }
            let records = store.list().await.expect("list should succeed");
            if records
                .first()
                .is_some_and(|record| record.status == ProcessStatus::Running)
            {
                break true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(running, "expected the daemon to be recorded running");

        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");
        handle
            .await
            .expect("task should not panic")
            .expect("launch_daemon should exit cleanly");

        let records = store.list().await.expect("list should succeed");
        assert_eq!(records[0].status, ProcessStatus::Stopped);
    }

    #[tokio::test]
    async fn a_daemon_already_alive_is_adopted_without_running_start() {
        let dir = TempDir::new().expect("temp dir should create");
        let (repository, store) = open_repository(&dir).await;
        let marker = dir.path().join("started");
        let entry = daemon_entry(
            "api",
            &format!("touch {}", marker.display()),
            "true",
            "true",
        );
        let summon = shell::summon_for(&entry, dir.path());
        let Lifecycle::Daemon(daemon) = summon.lifecycle() else {
            panic!("expected a daemon lifecycle");
        };
        // Pre-signal shutdown so the poll loop entered right after adoption exits immediately
        // instead of polling `status` forever.
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        shutdown_tx
            .send(true)
            .expect("shutdown send should succeed");

        launch_daemon(
            "api",
            summon.as_ref(),
            daemon,
            &repository,
            &mut shutdown_rx,
            &test_policy(),
        )
        .await
        .expect("launch_daemon should exit cleanly");

        assert!(
            !marker.exists(),
            "an already-alive daemon should not be started again"
        );
        let records = store.list().await.expect("list should succeed");
        assert_eq!(records[0].pid, None);
    }

    #[tokio::test]
    async fn a_daemon_whose_status_fails_is_reported_exited() {
        let dir = TempDir::new().expect("temp dir should create");
        let (repository, store) = open_repository(&dir).await;
        let entry = daemon_entry("api", "true", "true", "false");
        let log_dir = dir.path().to_path_buf();
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let summon = shell::summon_for(&entry, &log_dir);
        let Lifecycle::Daemon(daemon) = summon.lifecycle() else {
            panic!("expected a daemon lifecycle");
        };

        let ended = launch_daemon(
            "api",
            summon.as_ref(),
            daemon,
            &repository,
            &mut shutdown_rx,
            &test_policy(),
        )
        .await
        .expect("launch_daemon should exit cleanly");

        assert!(matches!(ended, super::Ended::Exited { .. }));
        let records = store.list().await.expect("list should succeed");
        assert_eq!(records[0].status, ProcessStatus::Exited);
    }
}
