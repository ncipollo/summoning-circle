use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use tokio::process::{Child, Command};

use super::daemon::Daemon;
use crate::feature::config::process::{ProcessEntry, ProcessKind};

/// Something the supervisor can launch as a child process.
pub trait Summon: Send + Sync {
    fn spawn(&self) -> Result<Child>;

    /// How this process kind is monitored and stopped once launched. Every impl must state
    /// this explicitly (no default) so a future kind can't silently inherit signal semantics,
    /// matching the exhaustive match in `summon_for` below.
    fn lifecycle(&self) -> Lifecycle<'_>;
}

/// How a spawned process is watched for exit and asked to stop.
pub enum Lifecycle<'a> {
    /// Watched by holding the child handle; stopped with SIGTERM then SIGKILL.
    Signal,
    /// Watched by polling `status`; stopped by running `stop`.
    Daemon(&'a Daemon),
}

/// Builds the `Summon` for a config entry. Exhaustive on `ProcessKind` so a new variant is a
/// compile error here, rather than a silent no-op at runtime.
pub fn summon_for(entry: &ProcessEntry, log_dir: &Path) -> Box<dyn Summon> {
    let log_path = log_dir.join(format!("{}.log", entry.name));
    match &entry.kind {
        ProcessKind::Shell { command, cwd, env } => shell_summon(command, cwd, env, log_path),
        ProcessKind::Daemon {
            start,
            stop,
            status,
        } => daemon_summon(&entry.name, start, stop, status, log_path),
    }
}

fn shell_summon(
    command: &str,
    cwd: &Option<PathBuf>,
    env: &Option<std::collections::BTreeMap<String, String>>,
    log_path: PathBuf,
) -> Box<dyn Summon> {
    Box::new(ShellSummon {
        command: command.to_string(),
        // Defaults to the user's home directory, as documented in the README.
        cwd: cwd.clone().or_else(dirs::home_dir),
        env: env.clone(),
        log_path,
    })
}

fn daemon_summon(
    name: &str,
    start: &str,
    stop: &str,
    status: &str,
    log_path: PathBuf,
) -> Box<dyn Summon> {
    Box::new(DaemonSummon {
        start_command: start.to_string(),
        log_path: log_path.clone(),
        daemon: Daemon::new(name, stop, status, log_path),
    })
}

struct ShellSummon {
    command: String,
    cwd: Option<PathBuf>,
    env: Option<std::collections::BTreeMap<String, String>>,
    log_path: PathBuf,
}

impl ShellSummon {
    fn open_log(&self) -> Result<File> {
        if let Some(parent) = self.log_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("could not create log dir for {}", self.log_path.display())
            })?;
        }

        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .with_context(|| format!("could not open log file at {}", self.log_path.display()))
    }
}

impl Summon for ShellSummon {
    fn spawn(&self) -> Result<Child> {
        let stdout = self.open_log()?;
        let stderr = stdout.try_clone().with_context(|| {
            format!(
                "could not duplicate log handle for {}",
                self.log_path.display()
            )
        })?;

        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(&self.command)
            .stdin(std::process::Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            // Own process group so we can signal the whole tree (including anything the shell
            // command forks) instead of just the `sh` pid.
            .process_group(0);

        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        if let Some(env) = &self.env {
            command.envs(env);
        }

        command
            .spawn()
            .with_context(|| format!("could not launch command '{}'", self.command))
    }

    fn lifecycle(&self) -> Lifecycle<'_> {
        Lifecycle::Signal
    }
}

struct DaemonSummon {
    start_command: String,
    log_path: PathBuf,
    daemon: Daemon,
}

impl DaemonSummon {
    fn open_log(&self) -> Result<File> {
        if let Some(parent) = self.log_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("could not create log dir for {}", self.log_path.display())
            })?;
        }

        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .with_context(|| format!("could not open log file at {}", self.log_path.display()))
    }
}

impl Summon for DaemonSummon {
    fn spawn(&self) -> Result<Child> {
        let stdout = self.open_log()?;
        let stderr = stdout.try_clone().with_context(|| {
            format!(
                "could not duplicate log handle for {}",
                self.log_path.display()
            )
        })?;

        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(&self.start_command)
            .stdin(std::process::Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .process_group(0);
        if let Some(home) = dirs::home_dir() {
            command.current_dir(home);
        }

        command.spawn().with_context(|| {
            format!(
                "could not launch daemon start command '{}'",
                self.start_command
            )
        })
    }

    fn lifecycle(&self) -> Lifecycle<'_> {
        Lifecycle::Daemon(&self.daemon)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use tempfile::TempDir;

    use super::summon_for;
    use crate::feature::config::process::{ProcessEntry, ProcessKind};

    fn entry(
        name: &str,
        command: &str,
        cwd: Option<std::path::PathBuf>,
        env: Option<BTreeMap<String, String>>,
    ) -> ProcessEntry {
        ProcessEntry {
            name: name.to_string(),
            kind: ProcessKind::Shell {
                command: command.to_string(),
                cwd,
                env,
            },
        }
    }

    #[tokio::test]
    async fn writes_command_output_to_the_log_file() {
        let dir = TempDir::new().expect("temp dir should create");
        let entry = entry("api", "echo hi", None, None);

        let mut child = summon_for(&entry, dir.path())
            .spawn()
            .expect("spawn should succeed");
        child.wait().await.expect("wait should succeed");

        let contents =
            fs::read_to_string(dir.path().join("api.log")).expect("log file should exist");
        assert_eq!(contents.trim(), "hi");
    }

    #[tokio::test]
    async fn runs_in_the_configured_cwd() {
        let dir = TempDir::new().expect("temp dir should create");
        let work_dir = TempDir::new().expect("temp dir should create");
        let entry = entry("api", "pwd", Some(work_dir.path().to_path_buf()), None);

        let mut child = summon_for(&entry, dir.path())
            .spawn()
            .expect("spawn should succeed");
        child.wait().await.expect("wait should succeed");

        let contents =
            fs::read_to_string(dir.path().join("api.log")).expect("log file should exist");
        assert_eq!(
            contents.trim(),
            work_dir
                .path()
                .canonicalize()
                .expect("work dir should canonicalize")
                .to_string_lossy()
        );
    }

    #[tokio::test]
    async fn defaults_to_the_home_directory_when_no_cwd_is_configured() {
        let dir = TempDir::new().expect("temp dir should create");
        let entry = entry("api", "pwd", None, None);

        let mut child = summon_for(&entry, dir.path())
            .spawn()
            .expect("spawn should succeed");
        child.wait().await.expect("wait should succeed");

        let contents =
            fs::read_to_string(dir.path().join("api.log")).expect("log file should exist");
        let home = dirs::home_dir()
            .expect("test environment should have a home directory")
            .canonicalize()
            .expect("home dir should canonicalize");
        assert_eq!(contents.trim(), home.to_string_lossy());
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

    #[tokio::test]
    async fn a_shell_entry_is_stopped_by_signal() {
        let dir = TempDir::new().expect("temp dir should create");
        let entry = entry("api", "sleep 1", None, None);

        let summon = summon_for(&entry, dir.path());

        assert!(matches!(summon.lifecycle(), super::Lifecycle::Signal));
    }

    #[tokio::test]
    async fn a_daemon_entry_runs_start_and_is_monitored_via_status() {
        let dir = TempDir::new().expect("temp dir should create");
        let entry = daemon_entry("api", "echo starting", "true", "true");

        let summon = summon_for(&entry, dir.path());
        assert!(matches!(summon.lifecycle(), super::Lifecycle::Daemon(_)));

        let mut child = summon.spawn().expect("spawn should succeed");
        child.wait().await.expect("wait should succeed");

        let contents =
            fs::read_to_string(dir.path().join("api.log")).expect("log file should exist");
        assert_eq!(contents.trim(), "starting");
    }

    #[tokio::test]
    async fn a_daemon_start_command_defaults_to_the_home_directory() {
        let dir = TempDir::new().expect("temp dir should create");
        let entry = daemon_entry("api", "pwd", "true", "true");

        let mut child = summon_for(&entry, dir.path())
            .spawn()
            .expect("spawn should succeed");
        child.wait().await.expect("wait should succeed");

        let contents =
            fs::read_to_string(dir.path().join("api.log")).expect("log file should exist");
        let home = dirs::home_dir()
            .expect("test environment should have a home directory")
            .canonicalize()
            .expect("home dir should canonicalize");
        assert_eq!(contents.trim(), home.to_string_lossy());
    }
}
