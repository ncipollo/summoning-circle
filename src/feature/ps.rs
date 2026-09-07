pub mod table;

use anyhow::Result;
use nix::errno::Errno;
use nix::sys::signal;
use nix::unistd::Pid;

use crate::feature::store::{ProcessRecord, ProcessStatus};

/// Printed when there is nothing to show: either the store has no records,
/// or (checked by the caller) the database file doesn't exist yet.
pub const NO_PROCESSES: &str =
    "No processes tracked. Run 'summoning-circle run' or 'summoning-circle install' first.";

/// Rewrites `Running` records whose pid is no longer alive to `Stale`,
/// without touching the store — the supervisor remains the only writer.
pub fn resolve(records: Vec<ProcessRecord>) -> Vec<ProcessRecord> {
    records
        .into_iter()
        .map(|record| resolve_status(record, is_alive))
        .collect()
}

fn resolve_status(mut record: ProcessRecord, alive: impl Fn(u32) -> bool) -> ProcessRecord {
    if record.status == ProcessStatus::Running && !record.pid.is_some_and(&alive) {
        record.status = ProcessStatus::Stale;
    }
    record
}

/// Checks whether `pid` is still alive by sending it no signal. Only
/// `ESRCH` ("no such process") is treated as dead; any other error (e.g.
/// `EPERM`, meaning the process exists but isn't ours to signal) means it's
/// still alive.
fn is_alive(pid: u32) -> bool {
    !matches!(
        signal::kill(Pid::from_raw(pid as i32), None),
        Err(Errno::ESRCH)
    )
}

/// Serializes the resolved records as a JSON array for scripting.
pub fn render_json(records: &[ProcessRecord]) -> Result<String> {
    Ok(serde_json::to_string_pretty(records)?)
}

#[cfg(test)]
mod tests {
    use super::resolve_status;
    use crate::feature::store::{ProcessRecord, ProcessStatus};

    fn record_with_status(status: ProcessStatus, pid: Option<u32>) -> ProcessRecord {
        ProcessRecord {
            status,
            pid,
            ..ProcessRecord::starting("api", "shell", "cargo run")
        }
    }

    #[test]
    fn running_with_dead_pid_becomes_stale() {
        let record = record_with_status(ProcessStatus::Running, Some(123));

        let resolved = resolve_status(record, |_| false);

        assert_eq!(resolved.status, ProcessStatus::Stale);
    }

    #[test]
    fn running_with_live_pid_stays_running() {
        let record = record_with_status(ProcessStatus::Running, Some(123));

        let resolved = resolve_status(record, |_| true);

        assert_eq!(resolved.status, ProcessStatus::Running);
    }

    #[test]
    fn running_without_pid_becomes_stale() {
        let record = record_with_status(ProcessStatus::Running, None);

        let resolved = resolve_status(record, |_| true);

        assert_eq!(resolved.status, ProcessStatus::Stale);
    }

    #[test]
    fn non_running_status_is_left_alone() {
        let record = record_with_status(ProcessStatus::Exited, None);

        let resolved = resolve_status(record, |_| false);

        assert_eq!(resolved.status, ProcessStatus::Exited);
    }

    #[test]
    fn render_json_includes_resolved_status() {
        let record = record_with_status(ProcessStatus::Stale, None);

        let json = super::render_json(&[record]).expect("serialization should succeed");

        assert!(json.contains("\"status\": \"stale\""));
    }
}
