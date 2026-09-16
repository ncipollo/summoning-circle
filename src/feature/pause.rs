use std::time::Duration;

use anyhow::Result;

use crate::feature::kill::{self, Outcome};
use crate::feature::proc::control::{self, ProcessControl};
use crate::feature::ps;
use crate::feature::store::Store;

/// Pauses the named process: marks it paused, so the supervisor stops relaunching it, then
/// signals it if it's currently running. The flag is written before the process list is read,
/// so a worker racing to launch the same process is guaranteed to see the flag either just
/// before or just after recording its pid — never neither, which is what lets a worker's own
/// post-launch pause check (see `supervisor::worker::launch`) and this direct signal jointly
/// cover every case.
pub async fn pause(
    store: &Store,
    name: &str,
    grace: Duration,
    control: &dyn ProcessControl,
) -> Result<Outcome> {
    store.pause(name).await?;

    let records = ps::resolve(store.list().await?).await;
    let record = kill::find(&records, name)?;
    Ok(kill::kill_one(record, grace, control).await)
}

/// Resumes the named process, clearing its paused flag, and reports whether a live supervisor
/// is around to actually relaunch it.
pub async fn resume(store: &Store, name: &str, control: &dyn ProcessControl) -> Result<bool> {
    store.resume(name).await?;

    let supervisor = store.supervisor().await?;
    Ok(control::supervisor_alive(supervisor, control))
}

/// Renders a `pause` result: the signal outcome, then a line confirming the pause.
pub fn render_pause(name: &str, outcome: &Outcome) -> String {
    let mut rendered = kill::render(std::slice::from_ref(outcome));
    rendered.push_str(&format!("{name}  paused\n"));
    rendered
}

/// Renders a `resume` result, noting when no supervisor is around to relaunch it yet.
pub fn render_resume(name: &str, supervisor_alive: bool) -> String {
    let mut rendered = format!("{name}  resumed\n");
    if !supervisor_alive {
        rendered.push_str("no supervisor running; it will not be relaunched\n");
    }
    rendered
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::feature::proc::control::tests::FakeControl;
    use crate::feature::store::ProcessRecord;

    async fn open_store(dir: &TempDir) -> Store {
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        store
            .upsert(&ProcessRecord::starting("api", "shell", "cargo run"))
            .await
            .expect("upsert should succeed");
        store
    }

    #[tokio::test]
    async fn pause_marks_the_flag_and_signals_a_running_process() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store
            .mark_running("api", Some(1), Some(0))
            .await
            .expect("mark_running should succeed");
        let control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(1, 0)
        };

        let outcome = pause(&store, "api", Duration::from_millis(50), &control)
            .await
            .expect("pause should succeed");

        assert!(matches!(outcome, Outcome::Signaled { pid: Some(1), .. }));
        assert!(store.is_paused("api").await.expect("read should succeed"));
    }

    #[tokio::test]
    async fn pause_marks_the_flag_even_when_not_running() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        let control = FakeControl::default();

        let outcome = pause(&store, "api", Duration::from_millis(20), &control)
            .await
            .expect("pause should succeed");

        assert!(matches!(outcome, Outcome::NotRunning { .. }));
        assert!(store.is_paused("api").await.expect("read should succeed"));
    }

    #[tokio::test]
    async fn pause_errors_on_an_unknown_name() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        let control = FakeControl::default();

        let error = pause(&store, "ghost", Duration::from_millis(20), &control)
            .await
            .expect_err("unknown name should error");

        assert!(
            error
                .to_string()
                .contains("no tracked process named 'ghost'")
        );
    }

    #[tokio::test]
    async fn resume_clears_the_flag() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store.pause("api").await.expect("pause should succeed");
        let control = FakeControl::default();

        resume(&store, "api", &control)
            .await
            .expect("resume should succeed");

        assert!(!store.is_paused("api").await.expect("read should succeed"));
    }

    #[tokio::test]
    async fn resume_reports_no_live_supervisor() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store.pause("api").await.expect("pause should succeed");
        let control = FakeControl::default();

        let supervisor_alive = resume(&store, "api", &control)
            .await
            .expect("resume should succeed");

        assert!(!supervisor_alive);
    }

    #[tokio::test]
    async fn resume_reports_a_live_supervisor() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        store.pause("api").await.expect("pause should succeed");
        store
            .claim_supervisor(999, Some(111))
            .await
            .expect("claim should succeed");
        let control = FakeControl::alive_with_start_time(999, 111);

        let supervisor_alive = resume(&store, "api", &control)
            .await
            .expect("resume should succeed");

        assert!(supervisor_alive);
    }

    #[tokio::test]
    async fn resume_errors_on_an_unknown_name() {
        let dir = TempDir::new().expect("temp dir should create");
        let store = open_store(&dir).await;
        let control = FakeControl::default();

        let error = resume(&store, "ghost", &control)
            .await
            .expect_err("unknown name should error");

        assert!(
            error
                .to_string()
                .contains("no tracked process named 'ghost'")
        );
    }

    #[test]
    fn render_pause_includes_the_signal_line_and_a_paused_note() {
        let outcome = Outcome::Signaled {
            name: "api".to_string(),
            pid: Some(1),
        };

        let rendered = render_pause("api", &outcome);

        assert!(rendered.contains("api  signaled (pid 1)"));
        assert!(rendered.contains("api  paused"));
    }

    #[test]
    fn render_resume_notes_the_missing_supervisor() {
        let rendered = render_resume("api", false);

        assert!(rendered.contains("api  resumed"));
        assert!(rendered.contains("no supervisor running"));
    }

    #[test]
    fn render_resume_omits_the_note_when_a_supervisor_is_alive() {
        let rendered = render_resume("api", true);

        assert!(rendered.contains("api  resumed"));
        assert!(!rendered.contains("no supervisor running"));
    }
}
