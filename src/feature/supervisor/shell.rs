use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use tokio::process::{Child, Command};

use crate::feature::config::process::{ProcessEntry, ProcessKind};

/// Something the supervisor can launch as a child process.
pub trait Summon: Send + Sync {
    fn spawn(&self) -> Result<Child>;
}

/// Builds the `Summon` for a config entry. Exhaustive on `ProcessKind` so a new variant is a
/// compile error here, rather than a silent no-op at runtime.
pub fn summon_for(entry: &ProcessEntry, log_dir: &Path) -> Box<dyn Summon> {
    match &entry.kind {
        ProcessKind::Shell { command, cwd, env } => Box::new(ShellSummon {
            command: command.clone(),
            cwd: cwd.clone(),
            env: env.clone(),
            log_path: log_dir.join(format!("{}.log", entry.name)),
        }),
    }
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
}
