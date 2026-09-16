pub mod table;

use std::time::Duration;

use anyhow::Result;

use crate::feature::proc;
use crate::feature::store::{ProcessRecord, ProcessStatus};

/// Printed when there is nothing to show: either the store has no records,
/// or (checked by the caller) the database file doesn't exist yet.
pub const NO_PROCESSES: &str =
    "No processes tracked. Run 'summoning-circle run' or 'summoning-circle install' first.";

/// How long a single daemon `status` check is given before it's treated as a timeout (and
/// so, per `proc::daemon_status_ok`, read as alive rather than dead).
const STATUS_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Rewrites `Running` records that are no longer actually alive to `Stale`, without touching
/// the store — the supervisor remains the only writer. A `Shell` record is checked by pid; a
/// `Daemon` record is checked by running its `status` command, since it's routinely pidless
/// (see `ProcessRecord::pid`) while genuinely alive.
pub async fn resolve(records: Vec<ProcessRecord>) -> Vec<ProcessRecord> {
    let mut resolved = Vec::with_capacity(records.len());
    for record in records {
        resolved.push(resolve_one(record).await);
    }
    resolved
}

async fn resolve_one(record: ProcessRecord) -> ProcessRecord {
    if record.status != ProcessStatus::Running {
        return record;
    }

    let alive = match &record.status_command {
        Some(status_command) => {
            proc::daemon_status_ok(&record.name, status_command, STATUS_CHECK_TIMEOUT).await
        }
        None => record.pid.is_some_and(proc::is_alive),
    };
    resolve_status(record, alive)
}

fn resolve_status(mut record: ProcessRecord, alive: bool) -> ProcessRecord {
    if !alive {
        record.status = ProcessStatus::Stale;
    }
    record
}

/// Serializes the resolved records as a JSON array for scripting.
pub fn render_json(records: &[ProcessRecord]) -> Result<String> {
    Ok(serde_json::to_string_pretty(records)?)
}

#[cfg(test)]
mod tests {
    use super::{resolve, resolve_status};
    use crate::feature::store::{ProcessRecord, ProcessStatus};

    fn record_with_status(status: ProcessStatus, pid: Option<u32>) -> ProcessRecord {
        ProcessRecord {
            status,
            pid,
            ..ProcessRecord::starting("api", "shell", "cargo run")
        }
    }

    fn daemon_record(status: ProcessStatus, status_command: &str) -> ProcessRecord {
        ProcessRecord {
            status,
            ..ProcessRecord::starting("api", "daemon", "unused")
                .with_daemon_commands("stop-cmd", status_command)
        }
    }

    #[test]
    fn resolve_status_marks_a_dead_record_stale() {
        let record = record_with_status(ProcessStatus::Running, Some(123));

        let resolved = resolve_status(record, false);

        assert_eq!(resolved.status, ProcessStatus::Stale);
    }

    #[test]
    fn resolve_status_leaves_an_alive_record_running() {
        let record = record_with_status(ProcessStatus::Running, Some(123));

        let resolved = resolve_status(record, true);

        assert_eq!(resolved.status, ProcessStatus::Running);
    }

    #[tokio::test]
    async fn a_shell_record_with_a_dead_pid_becomes_stale() {
        // `u32::MAX as i32` is -1, which POSIX treats as "every process in the group" rather
        // than an invalid pid, so a real (exited) pid is used instead of a sentinel value.
        let mut child = tokio::process::Command::new("true")
            .spawn()
            .expect("spawn should succeed");
        let pid = child.id().expect("spawned child should have a pid");
        child.wait().await.expect("wait should succeed");
        let record = record_with_status(ProcessStatus::Running, Some(pid));

        let resolved = resolve(vec![record]).await;

        assert_eq!(resolved[0].status, ProcessStatus::Stale);
    }

    #[tokio::test]
    async fn a_shell_record_without_a_pid_becomes_stale() {
        let record = record_with_status(ProcessStatus::Running, None);

        let resolved = resolve(vec![record]).await;

        assert_eq!(resolved[0].status, ProcessStatus::Stale);
    }

    #[tokio::test]
    async fn a_daemon_record_whose_status_reports_alive_stays_running() {
        let record = daemon_record(ProcessStatus::Running, "true");

        let resolved = resolve(vec![record]).await;

        assert_eq!(resolved[0].status, ProcessStatus::Running);
    }

    #[tokio::test]
    async fn a_daemon_record_whose_status_reports_dead_becomes_stale() {
        let record = daemon_record(ProcessStatus::Running, "false");

        let resolved = resolve(vec![record]).await;

        assert_eq!(resolved[0].status, ProcessStatus::Stale);
    }

    #[tokio::test]
    async fn a_daemon_record_is_checked_by_status_even_without_a_pid() {
        let mut record = daemon_record(ProcessStatus::Running, "true");
        record.pid = None;

        let resolved = resolve(vec![record]).await;

        assert_eq!(
            resolved[0].status,
            ProcessStatus::Running,
            "a pidless but alive daemon must not be marked stale"
        );
    }

    #[tokio::test]
    async fn non_running_status_is_left_alone() {
        let record = record_with_status(ProcessStatus::Exited, None);

        let resolved = resolve(vec![record]).await;

        assert_eq!(resolved[0].status, ProcessStatus::Exited);
    }

    #[test]
    fn render_json_includes_resolved_status() {
        let record = record_with_status(ProcessStatus::Stale, None);

        let json = super::render_json(&[record]).expect("serialization should succeed");

        assert!(json.contains("\"status\": \"stale\""));
    }

    #[test]
    fn render_json_includes_the_paused_flag() {
        let mut record = record_with_status(ProcessStatus::Stopped, None);
        record.paused = true;

        let json = super::render_json(&[record]).expect("serialization should succeed");

        assert!(json.contains("\"paused\": true"));
    }
}
