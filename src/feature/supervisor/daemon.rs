use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, Result};
use tokio::process::Command;
use tokio::time;
use tracing::{info, warn};

use crate::feature::proc;

/// A daemon-kind process's `stop`/`status` commands, run exactly like `start`: via `sh -c`,
/// defaulting to the home directory. `stop`'s output is appended to the process's own log file,
/// matching `start`; `status` output is discarded since it runs on every poll and would
/// otherwise flood that file — its outcome is captured via `tracing` instead.
pub struct Daemon {
    name: String,
    stop_command: String,
    status_command: String,
    log_path: PathBuf,
}

impl Daemon {
    pub fn new(
        name: impl Into<String>,
        stop_command: impl Into<String>,
        status_command: impl Into<String>,
        log_path: PathBuf,
    ) -> Self {
        Self {
            name: name.into(),
            stop_command: stop_command.into(),
            status_command: status_command.into(),
            log_path,
        }
    }

    /// Whether the daemon is currently alive, per `status`'s exit code (0 = alive). A probe
    /// that exceeds `timeout` is treated as alive, with a warning, rather than as death: a
    /// hung health check must never cause a live daemon to be declared dead and relaunched.
    pub async fn status_ok(&self, timeout: Duration) -> bool {
        proc::daemon_status_ok(&self.name, &self.status_command, timeout).await
    }

    /// Runs the daemon's `stop` command, logging the interaction either way. There is no
    /// OS-level force-stop for a daemon: a stop that doesn't take is only ever retried by
    /// calling this again.
    pub async fn stop(&self, timeout: Duration) {
        info!(name = %self.name, command = %self.stop_command, "stopping daemon");

        let mut command = sh(&self.stop_command);
        if let Ok(log) = open_log(&self.log_path)
            && let Ok(err) = log.try_clone()
        {
            command.stdout(log).stderr(err);
        }

        match run(command, timeout).await {
            Some(status) if !status.success() => {
                warn!(name = %self.name, command = %self.stop_command, code = status.code(), "stop command exited non-zero");
            }
            Some(_) => {}
            None => {
                warn!(name = %self.name, command = %self.stop_command, "stop command timed out")
            }
        }
    }
}

fn sh(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command).stdin(Stdio::null());
    if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }
    cmd
}

async fn run(mut command: Command, timeout: Duration) -> Option<std::process::ExitStatus> {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            warn!(%error, "could not launch daemon command");
            return None;
        }
    };

    time::timeout(timeout, child.wait()).await.ok()?.ok()
}

fn open_log(log_path: &Path) -> Result<File> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create log dir for {}", log_path.display()))?;
    }

    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("could not open log file at {}", log_path.display()))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::Daemon;

    fn daemon(dir: &TempDir, stop: &str, status: &str) -> Daemon {
        Daemon::new("api", stop, status, dir.path().join("api.log"))
    }

    #[tokio::test]
    async fn status_ok_reflects_a_zero_exit_code() {
        let dir = TempDir::new().expect("temp dir should create");
        let daemon = daemon(&dir, "true", "true");

        assert!(daemon.status_ok(std::time::Duration::from_secs(2)).await);
    }

    #[tokio::test]
    async fn status_ok_reflects_a_nonzero_exit_code() {
        let dir = TempDir::new().expect("temp dir should create");
        let daemon = daemon(&dir, "true", "false");

        assert!(!daemon.status_ok(std::time::Duration::from_secs(2)).await);
    }

    #[tokio::test]
    async fn status_ok_treats_a_timeout_as_alive() {
        let dir = TempDir::new().expect("temp dir should create");
        let daemon = daemon(&dir, "true", "sleep 5");

        assert!(daemon.status_ok(std::time::Duration::from_millis(50)).await);
    }

    #[tokio::test]
    async fn stop_runs_the_stop_command() {
        let dir = TempDir::new().expect("temp dir should create");
        let marker = dir.path().join("stopped");
        let daemon = daemon(&dir, &format!("touch {}", marker.display()), "true");

        daemon.stop(std::time::Duration::from_secs(2)).await;

        assert!(marker.exists(), "stop command should have run");
    }

    #[tokio::test]
    async fn stop_appends_output_to_the_log_file() {
        let dir = TempDir::new().expect("temp dir should create");
        let daemon = daemon(&dir, "echo stopping", "true");

        daemon.stop(std::time::Duration::from_secs(2)).await;

        let contents =
            std::fs::read_to_string(dir.path().join("api.log")).expect("log file should exist");
        assert_eq!(contents.trim(), "stopping");
    }
}
