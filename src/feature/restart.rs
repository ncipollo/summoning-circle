use std::time::Duration;

use anyhow::{Result, bail};

use crate::feature::kill::{self, Outcome};
use crate::feature::proc::control::{self, ProcessControl};
use crate::feature::store::{ProcessRecord, SupervisorRecord};

/// Looks up `name` among `records`, erroring with the same wording the store itself uses
/// for an unknown process name.
fn find<'a>(records: &'a [ProcessRecord], name: &str) -> Result<&'a ProcessRecord> {
    let Some(record) = records.iter().find(|record| record.name == name) else {
        bail!("no tracked process named '{name}'");
    };
    Ok(record)
}

/// Restarts the named process: signals it (escalating to SIGKILL after `grace`), and reports
/// whether a live supervisor is around to notice the exit and relaunch it.
pub async fn restart(
    records: &[ProcessRecord],
    name: &str,
    supervisor: Option<SupervisorRecord>,
    grace: Duration,
    control: &dyn ProcessControl,
) -> Result<(Outcome, bool)> {
    let record = find(records, name)?;
    let outcome = kill::kill_one(record, grace, control).await;
    let supervisor_alive = supervisor.is_some_and(|claim| {
        control::identifies_same_process(claim.pid, claim.start_time, control)
    });

    Ok((outcome, supervisor_alive))
}

/// Renders the outcome, adding a note when nothing will relaunch the process.
pub fn render(outcome: &Outcome, supervisor_alive: bool) -> String {
    let mut rendered = kill::render(std::slice::from_ref(outcome));

    if !supervisor_alive && matches!(outcome, Outcome::Signaled { .. } | Outcome::Killed { .. }) {
        rendered.push_str("no supervisor running; it will not be relaunched\n");
    }

    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::proc::control::tests::FakeControl;
    use crate::feature::store::ProcessStatus;

    fn record_with_status(status: ProcessStatus, pid: Option<u32>) -> ProcessRecord {
        ProcessRecord {
            status,
            pid,
            ..ProcessRecord::starting("api", "shell", "cargo run")
        }
    }

    #[tokio::test]
    async fn restart_errors_on_an_unknown_name() {
        let control = FakeControl::default();

        let error = restart(&[], "nope", None, Duration::from_millis(20), &control)
            .await
            .expect_err("unknown name should error");

        assert!(
            error
                .to_string()
                .contains("no tracked process named 'nope'")
        );
    }

    #[tokio::test]
    async fn restart_reports_no_supervisor_when_none_is_recorded() {
        let records = vec![record_with_status(ProcessStatus::Running, Some(1))];
        let control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(1, 0)
        };

        let (outcome, supervisor_alive) =
            restart(&records, "api", None, Duration::from_millis(50), &control)
                .await
                .expect("restart should succeed");

        assert!(matches!(outcome, Outcome::Signaled { pid: 1, .. }));
        assert!(!supervisor_alive);
    }

    #[tokio::test]
    async fn restart_reports_a_live_supervisor() {
        let records = vec![record_with_status(ProcessStatus::Running, Some(1))];
        let mut control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(1, 0)
        };
        control.alive.lock().unwrap().insert(999, true);
        control.start_times.insert(999, 111);
        let supervisor = SupervisorRecord {
            pid: 999,
            start_time: Some(111),
        };

        let (_outcome, supervisor_alive) = restart(
            &records,
            "api",
            Some(supervisor),
            Duration::from_millis(50),
            &control,
        )
        .await
        .expect("restart should succeed");

        assert!(supervisor_alive);
    }

    #[tokio::test]
    async fn restart_on_a_non_running_process_does_not_signal() {
        let records = vec![record_with_status(ProcessStatus::Stopped, None)];
        let control = FakeControl::default();

        let (outcome, _) = restart(&records, "api", None, Duration::from_millis(20), &control)
            .await
            .expect("restart should succeed");

        assert!(matches!(outcome, Outcome::NotRunning { .. }));
        assert!(control.terminated.lock().unwrap().is_empty());
    }

    #[test]
    fn render_notes_the_missing_supervisor_when_a_process_was_signaled() {
        let outcome = Outcome::Signaled {
            name: "api".to_string(),
            pid: 1,
        };

        let rendered = render(&outcome, false);

        assert!(rendered.contains("no supervisor running"));
    }

    #[test]
    fn render_omits_the_note_when_a_supervisor_is_alive() {
        let outcome = Outcome::Signaled {
            name: "api".to_string(),
            pid: 1,
        };

        let rendered = render(&outcome, true);

        assert!(!rendered.contains("no supervisor running"));
    }

    #[test]
    fn render_omits_the_note_for_a_process_that_was_not_running() {
        let outcome = Outcome::NotRunning {
            name: "api".to_string(),
            status: ProcessStatus::Stopped,
        };

        let rendered = render(&outcome, false);

        assert!(!rendered.contains("no supervisor running"));
    }
}
