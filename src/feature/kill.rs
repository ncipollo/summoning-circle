use std::time::Duration;

use crate::feature::proc::control::{self, ProcessControl};
use crate::feature::store::{ProcessRecord, ProcessStatus};

/// What happened to one tracked process after a kill request.
#[derive(Debug)]
pub enum Outcome {
    /// Died on SIGTERM within the grace period.
    Signaled { name: String, pid: u32 },
    /// Still alive after the grace period, so it took a SIGKILL.
    Killed { name: String, pid: u32 },
    /// Nothing to signal: not currently running.
    NotRunning { name: String, status: ProcessStatus },
}

/// The pid to signal for `record`, or `None` if it isn't currently running. Records should
/// already be passed through `ps::resolve` so a `Running` record with a dead pid has been
/// rewritten to `Stale` and is skipped here rather than re-checked.
fn target(record: &ProcessRecord) -> Option<u32> {
    if record.status == ProcessStatus::Running {
        record.pid
    } else {
        None
    }
}

/// Signals every running process in `records`, paying the grace period once for the whole
/// batch rather than once per process.
pub async fn kill_all(
    records: &[ProcessRecord],
    grace: Duration,
    control: &dyn ProcessControl,
) -> Vec<Outcome> {
    let pids: Vec<u32> = records.iter().filter_map(target).collect();
    let stragglers = control::stop_all(&pids, grace, control).await;

    records
        .iter()
        .map(|record| match target(record) {
            None => Outcome::NotRunning {
                name: record.name.clone(),
                status: record.status,
            },
            Some(pid) if stragglers.contains(&pid) => Outcome::Killed {
                name: record.name.clone(),
                pid,
            },
            Some(pid) => Outcome::Signaled {
                name: record.name.clone(),
                pid,
            },
        })
        .collect()
}

/// Signals a single running process.
pub async fn kill_one(
    record: &ProcessRecord,
    grace: Duration,
    control: &dyn ProcessControl,
) -> Outcome {
    let Some(pid) = target(record) else {
        return Outcome::NotRunning {
            name: record.name.clone(),
            status: record.status,
        };
    };

    if control::stop(pid, grace, control).await {
        Outcome::Killed {
            name: record.name.clone(),
            pid,
        }
    } else {
        Outcome::Signaled {
            name: record.name.clone(),
            pid,
        }
    }
}

/// Renders one line per outcome, for `killall`'s and `restart`'s CLI output.
pub fn render(outcomes: &[Outcome]) -> String {
    outcomes.iter().map(render_one).collect()
}

fn render_one(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Signaled { name, pid } => format!("{name}  signaled (pid {pid})\n"),
        Outcome::Killed { name, pid } => format!("{name}  killed after grace period (pid {pid})\n"),
        Outcome::NotRunning { name, status } => {
            format!("{name}  not running ({})\n", status.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::proc::control::tests::FakeControl;

    fn record_with_status(status: ProcessStatus, pid: Option<u32>) -> ProcessRecord {
        ProcessRecord {
            status,
            pid,
            ..ProcessRecord::starting("api", "shell", "cargo run")
        }
    }

    #[test]
    fn target_returns_the_pid_of_a_running_record() {
        let record = record_with_status(ProcessStatus::Running, Some(42));

        assert_eq!(target(&record), Some(42));
    }

    #[test]
    fn target_skips_non_running_records() {
        for status in [
            ProcessStatus::Starting,
            ProcessStatus::Exited,
            ProcessStatus::Stopped,
            ProcessStatus::Stale,
        ] {
            let record = record_with_status(status, None);
            assert_eq!(target(&record), None, "status {status:?} should be skipped");
        }
    }

    #[tokio::test]
    async fn kill_all_signals_running_records_and_reports_the_rest() {
        let records = vec![
            record_with_status(ProcessStatus::Running, Some(1)),
            record_with_status(ProcessStatus::Stale, None),
        ];
        let control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(1, 0)
        };

        let outcomes = kill_all(&records, Duration::from_millis(50), &control).await;

        assert!(matches!(outcomes[0], Outcome::Signaled { pid: 1, .. }));
        assert!(matches!(
            outcomes[1],
            Outcome::NotRunning {
                status: ProcessStatus::Stale,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn kill_all_reports_a_straggler_as_killed() {
        let records = vec![record_with_status(ProcessStatus::Running, Some(1))];
        let control = FakeControl::alive_with_start_time(1, 0);

        let outcomes = kill_all(&records, Duration::from_millis(20), &control).await;

        assert!(matches!(outcomes[0], Outcome::Killed { pid: 1, .. }));
    }

    #[tokio::test]
    async fn kill_one_reports_not_running_without_signaling() {
        let record = record_with_status(ProcessStatus::Exited, None);
        let control = FakeControl::default();

        let outcome = kill_one(&record, Duration::from_millis(20), &control).await;

        assert!(matches!(outcome, Outcome::NotRunning { .. }));
        assert!(control.terminated.lock().unwrap().is_empty());
    }

    #[test]
    fn render_includes_one_line_per_outcome() {
        let outcomes = vec![
            Outcome::Signaled {
                name: "api".to_string(),
                pid: 1,
            },
            Outcome::Killed {
                name: "worker".to_string(),
                pid: 2,
            },
            Outcome::NotRunning {
                name: "tunnel".to_string(),
                status: ProcessStatus::Stale,
            },
        ];

        let rendered = render(&outcomes);

        assert!(rendered.contains("api  signaled (pid 1)"));
        assert!(rendered.contains("worker  killed after grace period (pid 2)"));
        assert!(rendered.contains("tunnel  not running (stale)"));
    }
}
