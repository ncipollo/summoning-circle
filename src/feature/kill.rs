use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use tokio::process::Command;
use tokio::time;
use tracing::{info, warn};

use crate::feature::proc;
use crate::feature::proc::control::{self, ProcessControl};
use crate::feature::store::{ProcessRecord, ProcessStatus};

/// How often a daemon's `status` is polled while waiting for a `stop` to take effect.
const DAEMON_STOP_POLL: Duration = Duration::from_millis(100);

/// Looks up `name` among `records`, erroring with the same wording the store itself uses
/// for an unknown process name. Shared by `restart`, `pause`, and `resume`.
pub fn find<'a>(records: &'a [ProcessRecord], name: &str) -> Result<&'a ProcessRecord> {
    let Some(record) = records.iter().find(|record| record.name == name) else {
        bail!("no tracked process named '{name}'");
    };
    Ok(record)
}

/// What happened to one tracked process after a kill request.
#[derive(Debug)]
pub enum Outcome {
    /// Died within the grace period: signaled for a `Shell` process, or its `stop` command
    /// reported it dead for a `Daemon`.
    Signaled { name: String, pid: Option<u32> },
    /// Still alive after the grace period: SIGKILLed for a `Shell` process, or (there being no
    /// force-stop for a daemon) had its `stop` command re-invoked.
    Killed { name: String, pid: Option<u32> },
    /// Nothing to signal: not currently running.
    NotRunning { name: String, status: ProcessStatus },
}

/// What a record resolves to when deciding how (or whether) to stop it. A daemon is targeted
/// by its own commands regardless of any stored pid, since a live daemon's pid is just the
/// (often already-dead) `start` command that launched it — never something to signal.
enum Target<'a> {
    Pid(u32),
    Daemon { stop: &'a str, status: &'a str },
    None,
}

/// Records should already be passed through `ps::resolve` so a `Running` record that's
/// actually dead (checked by pid for a `Shell`, by `status` for a `Daemon`) has been rewritten
/// to `Stale` and is skipped here rather than re-checked.
fn target(record: &ProcessRecord) -> Target<'_> {
    if record.status != ProcessStatus::Running {
        return Target::None;
    }
    match (&record.stop_command, &record.status_command) {
        (Some(stop), Some(status)) => Target::Daemon { stop, status },
        _ => record.pid.map_or(Target::None, Target::Pid),
    }
}

/// Signals every running process in `records`. Shell processes pay the grace period once for
/// the whole batch (via `control::stop_all`); daemons each run their own `stop`/`status`
/// sequence concurrently with that batch and with each other, rather than serially.
pub async fn kill_all(
    records: &[ProcessRecord],
    grace: Duration,
    control: &dyn ProcessControl,
) -> Vec<Outcome> {
    let pids: Vec<u32> = records
        .iter()
        .filter_map(|record| match target(record) {
            Target::Pid(pid) => Some(pid),
            _ => None,
        })
        .collect();
    let daemon_handles: Vec<(String, _)> = records
        .iter()
        .filter_map(|record| match target(record) {
            Target::Daemon { stop, status } => Some((
                record.name.clone(),
                tokio::spawn(stop_daemon(
                    record.name.clone(),
                    stop.to_string(),
                    status.to_string(),
                    grace,
                )),
            )),
            _ => None,
        })
        .collect();

    let stragglers = control::stop_all(&pids, grace, control).await;
    let mut daemon_outcomes: HashMap<String, Outcome> = HashMap::new();
    for (name, handle) in daemon_handles {
        let outcome = handle.await.expect("daemon stop task should not panic");
        daemon_outcomes.insert(name, outcome);
    }

    records
        .iter()
        .map(|record| outcome_for(record, &stragglers, &mut daemon_outcomes))
        .collect()
}

fn outcome_for(
    record: &ProcessRecord,
    stragglers: &[u32],
    daemon_outcomes: &mut HashMap<String, Outcome>,
) -> Outcome {
    match target(record) {
        Target::None => Outcome::NotRunning {
            name: record.name.clone(),
            status: record.status,
        },
        Target::Pid(pid) if stragglers.contains(&pid) => Outcome::Killed {
            name: record.name.clone(),
            pid: Some(pid),
        },
        Target::Pid(pid) => Outcome::Signaled {
            name: record.name.clone(),
            pid: Some(pid),
        },
        Target::Daemon { .. } => daemon_outcomes
            .remove(&record.name)
            .expect("a daemon target should have a matching stop outcome"),
    }
}

/// Signals a single running process.
pub async fn kill_one(
    record: &ProcessRecord,
    grace: Duration,
    control: &dyn ProcessControl,
) -> Outcome {
    match target(record) {
        Target::None => Outcome::NotRunning {
            name: record.name.clone(),
            status: record.status,
        },
        Target::Pid(pid) => {
            if control::stop(pid, grace, control).await {
                Outcome::Killed {
                    name: record.name.clone(),
                    pid: Some(pid),
                }
            } else {
                Outcome::Signaled {
                    name: record.name.clone(),
                    pid: Some(pid),
                }
            }
        }
        Target::Daemon { stop, status } => {
            stop_daemon(
                record.name.clone(),
                stop.to_string(),
                status.to_string(),
                grace,
            )
            .await
        }
    }
}

/// Runs `stop`, then waits up to `grace` for `status` to report the daemon dead. There is no
/// OS-level force-stop for an arbitrary daemon: if it's still alive after the grace period,
/// the only escalation available is to run `stop` again.
async fn stop_daemon(
    name: String,
    stop_command: String,
    status_command: String,
    grace: Duration,
) -> Outcome {
    run_stop(&name, &stop_command, grace).await;

    if wait_until_dead(&name, &status_command, grace).await {
        return Outcome::Signaled { name, pid: None };
    }

    info!(
        name,
        "daemon still alive after grace period; re-invoking stop"
    );
    run_stop(&name, &stop_command, grace).await;
    Outcome::Killed { name, pid: None }
}

async fn wait_until_dead(name: &str, status_command: &str, grace: Duration) -> bool {
    let deadline = Instant::now() + grace;
    loop {
        if !proc::daemon_status_ok(name, status_command, grace).await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        time::sleep(DAEMON_STOP_POLL).await;
    }
}

async fn run_stop(name: &str, command: &str, timeout: Duration) {
    info!(name, command, "stopping daemon");
    match time::timeout(timeout, sh(command).status()).await {
        Ok(Ok(status)) if !status.success() => {
            warn!(
                name,
                command,
                code = status.code(),
                "stop command exited non-zero"
            );
        }
        Ok(Ok(_)) => {}
        Ok(Err(error)) => warn!(name, command, %error, "could not run stop command"),
        Err(_) => warn!(name, command, "stop command timed out"),
    }
}

fn sh(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Some(home) = dirs::home_dir() {
        cmd.current_dir(home);
    }
    cmd
}

/// Renders one line per outcome, for `killall`'s and `restart`'s CLI output.
pub fn render(outcomes: &[Outcome]) -> String {
    outcomes.iter().map(render_one).collect()
}

fn render_one(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Signaled {
            name,
            pid: Some(pid),
        } => format!("{name}  signaled (pid {pid})\n"),
        Outcome::Signaled { name, pid: None } => format!("{name}  signaled\n"),
        Outcome::Killed {
            name,
            pid: Some(pid),
        } => format!("{name}  killed after grace period (pid {pid})\n"),
        Outcome::Killed { name, pid: None } => format!("{name}  killed after grace period\n"),
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

    fn daemon_record(status: ProcessStatus, stop: &str, health: &str) -> ProcessRecord {
        ProcessRecord {
            status,
            ..ProcessRecord::starting("api", "daemon", "unused").with_daemon_commands(stop, health)
        }
    }

    #[test]
    fn find_returns_the_named_record() {
        let records = vec![record_with_status(ProcessStatus::Running, Some(1))];

        let found = find(&records, "api").expect("known name should be found");

        assert_eq!(found.name, "api");
    }

    #[test]
    fn find_errors_on_an_unknown_name() {
        let error = find(&[], "nope").expect_err("unknown name should error");

        assert!(
            error
                .to_string()
                .contains("no tracked process named 'nope'")
        );
    }

    #[test]
    fn target_returns_the_pid_of_a_running_record() {
        let record = record_with_status(ProcessStatus::Running, Some(42));

        assert!(matches!(target(&record), Target::Pid(42)));
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
            assert!(
                matches!(target(&record), Target::None),
                "status {status:?} should be skipped"
            );
        }
    }

    #[test]
    fn target_returns_daemon_commands_regardless_of_pid() {
        let mut record = daemon_record(ProcessStatus::Running, "stop-cmd", "status-cmd");
        record.pid = None;

        assert!(matches!(
            target(&record),
            Target::Daemon {
                stop: "stop-cmd",
                status: "status-cmd"
            }
        ));
    }

    #[test]
    fn target_skips_a_non_running_daemon() {
        let record = daemon_record(ProcessStatus::Stale, "stop-cmd", "status-cmd");

        assert!(matches!(target(&record), Target::None));
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

        assert!(matches!(
            outcomes[0],
            Outcome::Signaled { pid: Some(1), .. }
        ));
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

        assert!(matches!(outcomes[0], Outcome::Killed { pid: Some(1), .. }));
    }

    #[tokio::test]
    async fn kill_all_stops_a_daemon_alongside_shell_processes() {
        let records = vec![
            record_with_status(ProcessStatus::Running, Some(1)),
            daemon_record(ProcessStatus::Running, "true", "false"),
        ];
        let control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(1, 0)
        };

        let outcomes = kill_all(&records, Duration::from_secs(2), &control).await;

        assert!(matches!(
            outcomes[0],
            Outcome::Signaled { pid: Some(1), .. }
        ));
        assert!(matches!(outcomes[1], Outcome::Signaled { pid: None, .. }));
    }

    #[tokio::test]
    async fn kill_one_reports_not_running_without_signaling() {
        let record = record_with_status(ProcessStatus::Exited, None);
        let control = FakeControl::default();

        let outcome = kill_one(&record, Duration::from_millis(20), &control).await;

        assert!(matches!(outcome, Outcome::NotRunning { .. }));
        assert!(control.terminated.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn kill_one_stops_a_daemon_via_its_own_commands() {
        let record = daemon_record(ProcessStatus::Running, "true", "false");
        let control = FakeControl::default();

        let outcome = kill_one(&record, Duration::from_secs(2), &control).await;

        assert!(matches!(outcome, Outcome::Signaled { pid: None, .. }));
    }

    #[tokio::test]
    async fn kill_one_re_invokes_stop_when_a_daemon_outlives_the_grace_period() {
        let record = daemon_record(ProcessStatus::Running, "true", "true");
        let control = FakeControl::default();

        let outcome = kill_one(&record, Duration::from_millis(150), &control).await;

        assert!(matches!(outcome, Outcome::Killed { pid: None, .. }));
    }

    #[test]
    fn render_includes_one_line_per_outcome() {
        let outcomes = vec![
            Outcome::Signaled {
                name: "api".to_string(),
                pid: Some(1),
            },
            Outcome::Killed {
                name: "worker".to_string(),
                pid: Some(2),
            },
            Outcome::Signaled {
                name: "daemon".to_string(),
                pid: None,
            },
            Outcome::NotRunning {
                name: "tunnel".to_string(),
                status: ProcessStatus::Stale,
            },
        ];

        let rendered = render(&outcomes);

        assert!(rendered.contains("api  signaled (pid 1)"));
        assert!(rendered.contains("worker  killed after grace period (pid 2)"));
        assert!(rendered.contains("daemon  signaled\n"));
        assert!(rendered.contains("tunnel  not running (stale)"));
    }
}
