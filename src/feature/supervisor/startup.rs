use std::time::Duration;

use anyhow::{Result, bail};
use tokio::time::{self, Instant};
use tracing::{info, warn};

use super::policy::Policy;
use super::repository::ProcessRepository;
use super::signals;
use crate::feature::proc;
use crate::feature::store::TrackedPid;

/// Abstracts liveness checks and signaling so the claim sequence can be tested
/// without touching real processes.
pub trait ProcessControl: Send + Sync {
    fn is_alive(&self, pid: u32) -> bool;
    fn start_time(&self, pid: u32) -> Option<i64>;
    fn terminate(&self, pid: u32);
    fn kill(&self, pid: u32);
}

/// Delegates to the real OS-facing liveness checks and signals.
pub struct SystemControl;

impl ProcessControl for SystemControl {
    fn is_alive(&self, pid: u32) -> bool {
        proc::is_alive(pid)
    }

    fn start_time(&self, pid: u32) -> Option<i64> {
        proc::start_time(pid)
    }

    fn terminate(&self, pid: u32) {
        signals::terminate(pid);
    }

    fn kill(&self, pid: u32) {
        signals::kill(pid);
    }
}

/// Ensures no other supervisor owns the store, reclaims any orphaned children left
/// behind by a previous crashed supervisor, then claims ownership for this process.
pub async fn claim(repository: &ProcessRepository, policy: &Policy) -> Result<()> {
    claim_with(repository, policy, &SystemControl).await
}

/// Whether a stored `(pid, start_time)` still identifies the same OS process, given its
/// currently observed start time. Pids get recycled, so a pid alone is never enough.
#[derive(Debug, PartialEq, Eq)]
enum Identity {
    /// The pid isn't running at all.
    Dead,
    /// The pid is running and its start time matches what we recorded.
    Matches,
    /// The pid is running, but its start time doesn't match (or we never recorded one) —
    /// almost certainly a recycled pid now used by an unrelated process.
    Mismatched,
}

fn classify(expected_start_time: Option<i64>, actual_start_time: Option<i64>) -> Identity {
    match (expected_start_time, actual_start_time) {
        (_, None) => Identity::Dead,
        (Some(expected), Some(actual)) if expected == actual => Identity::Matches,
        _ => Identity::Mismatched,
    }
}

async fn claim_with(
    repository: &ProcessRepository,
    policy: &Policy,
    control: &dyn ProcessControl,
) -> Result<()> {
    ensure_no_live_supervisor(repository, control).await?;
    reclaim_orphans(repository, policy, control).await?;

    let pid = std::process::id();
    repository
        .claim_supervisor(pid, control.start_time(pid))
        .await
}

async fn ensure_no_live_supervisor(
    repository: &ProcessRepository,
    control: &dyn ProcessControl,
) -> Result<()> {
    let Some(claim) = repository.supervisor().await? else {
        return Ok(());
    };

    if classify(claim.start_time, control.start_time(claim.pid)) == Identity::Matches {
        bail!(
            "another summoning-circle supervisor is already running (pid {})",
            claim.pid
        );
    }

    Ok(())
}

async fn reclaim_orphans(
    repository: &ProcessRepository,
    policy: &Policy,
    control: &dyn ProcessControl,
) -> Result<()> {
    for tracked in repository.tracked_pids().await? {
        reclaim_one(tracked, policy, control).await;
    }

    Ok(())
}

async fn reclaim_one(tracked: TrackedPid, policy: &Policy, control: &dyn ProcessControl) {
    match classify(tracked.start_time, control.start_time(tracked.pid)) {
        Identity::Dead => {}
        Identity::Mismatched => {
            warn!(
                pid = tracked.pid,
                "skipping pid reused by an unrelated process"
            );
        }
        Identity::Matches => {
            stop_orphan(tracked.pid, policy, control).await;
            info!(pid = tracked.pid, "terminated orphaned process");
        }
    }
}

/// Asks `pid` to terminate gracefully, escalating to SIGKILL after the policy's grace
/// period. Mirrors `worker::stop_child`, but there is no `Child` handle to `wait()` on, so
/// liveness is polled instead.
async fn stop_orphan(pid: u32, policy: &Policy, control: &dyn ProcessControl) {
    control.terminate(pid);

    let deadline = Instant::now() + policy.shutdown_grace;
    while control.is_alive(pid) && Instant::now() < deadline {
        time::sleep(Duration::from_millis(10)).await;
    }

    if control.is_alive(pid) {
        control.kill(pid);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use tempfile::TempDir;

    use super::*;
    use crate::feature::store::{ProcessRecord, Store};

    #[derive(Default)]
    struct FakeControl {
        alive: Mutex<HashMap<u32, bool>>,
        start_times: HashMap<u32, i64>,
        dies_on_terminate: bool,
        terminated: Mutex<Vec<u32>>,
        killed: Mutex<Vec<u32>>,
    }

    impl FakeControl {
        fn alive_with_start_time(pid: u32, start_time: i64) -> Self {
            let mut control = Self::default();
            control.alive.lock().unwrap().insert(pid, true);
            control.start_times.insert(pid, start_time);
            control
        }

        fn dead(pid: u32) -> Self {
            let control = Self::default();
            control.alive.lock().unwrap().insert(pid, false);
            control
        }
    }

    impl ProcessControl for FakeControl {
        fn is_alive(&self, pid: u32) -> bool {
            *self.alive.lock().unwrap().get(&pid).unwrap_or(&false)
        }

        fn start_time(&self, pid: u32) -> Option<i64> {
            if self.is_alive(pid) {
                self.start_times.get(&pid).copied()
            } else {
                None
            }
        }

        fn terminate(&self, pid: u32) {
            self.terminated.lock().unwrap().push(pid);
            if self.dies_on_terminate {
                self.alive.lock().unwrap().insert(pid, false);
            }
        }

        fn kill(&self, pid: u32) {
            self.killed.lock().unwrap().push(pid);
            self.alive.lock().unwrap().insert(pid, false);
        }
    }

    fn test_policy() -> Policy {
        Policy {
            shutdown_grace: Duration::from_millis(20),
            ..Policy::default()
        }
    }

    async fn open_repository(dir: &TempDir) -> ProcessRepository {
        let store = Store::open(&dir.path().join("circle.db"))
            .await
            .expect("store should open");
        ProcessRepository::new(store)
    }

    #[test]
    fn classify_dead_pid_regardless_of_expectation() {
        assert_eq!(classify(Some(1), None), Identity::Dead);
        assert_eq!(classify(None, None), Identity::Dead);
    }

    #[test]
    fn classify_matching_start_times() {
        assert_eq!(classify(Some(42), Some(42)), Identity::Matches);
    }

    #[test]
    fn classify_mismatched_or_unrecorded_start_time() {
        assert_eq!(classify(Some(42), Some(99)), Identity::Mismatched);
        assert_eq!(classify(None, Some(99)), Identity::Mismatched);
    }

    #[tokio::test]
    async fn claim_succeeds_when_no_supervisor_is_recorded() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;

        claim_with(&repository, &test_policy(), &FakeControl::default())
            .await
            .expect("claim should succeed");

        assert!(
            repository
                .supervisor()
                .await
                .expect("read should succeed")
                .is_some()
        );
    }

    #[tokio::test]
    async fn claim_fails_when_a_live_supervisor_matches() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;
        repository
            .claim_supervisor(999, Some(111))
            .await
            .expect("seed claim should succeed");

        let error = claim_with(
            &repository,
            &test_policy(),
            &FakeControl::alive_with_start_time(999, 111),
        )
        .await
        .expect_err("claim should fail when another supervisor is live");

        assert!(error.to_string().contains("999"));
    }

    #[tokio::test]
    async fn claim_succeeds_when_the_stored_supervisor_pid_was_recycled() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;
        repository
            .claim_supervisor(999, Some(111))
            .await
            .expect("seed claim should succeed");

        claim_with(
            &repository,
            &test_policy(),
            &FakeControl::alive_with_start_time(999, 222),
        )
        .await
        .expect("claim should succeed against a recycled pid");
    }

    #[tokio::test]
    async fn claim_succeeds_when_the_stored_supervisor_is_dead() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;
        repository
            .claim_supervisor(999, Some(111))
            .await
            .expect("seed claim should succeed");

        claim_with(&repository, &test_policy(), &FakeControl::dead(999))
            .await
            .expect("claim should succeed against a dead supervisor");
    }

    #[tokio::test]
    async fn reclaim_terminates_a_matching_live_orphan() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;
        repository
            .reconcile(
                &[ProcessRecord::starting("api", "shell", "cargo run")],
                &["api"],
            )
            .await
            .expect("reconcile should succeed");
        repository
            .record_running("api", 42, Some(111))
            .await
            .expect("record_running should succeed");
        let control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(42, 111)
        };

        claim_with(&repository, &test_policy(), &control)
            .await
            .expect("claim should succeed");

        assert_eq!(*control.terminated.lock().unwrap(), vec![42]);
        assert!(control.killed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reclaim_escalates_to_kill_after_the_grace_period() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;
        repository
            .reconcile(
                &[ProcessRecord::starting("api", "shell", "cargo run")],
                &["api"],
            )
            .await
            .expect("reconcile should succeed");
        repository
            .record_running("api", 42, Some(111))
            .await
            .expect("record_running should succeed");
        let control = FakeControl::alive_with_start_time(42, 111);

        claim_with(&repository, &test_policy(), &control)
            .await
            .expect("claim should succeed");

        assert_eq!(*control.killed.lock().unwrap(), vec![42]);
    }

    #[tokio::test]
    async fn reclaim_skips_a_pid_whose_start_time_no_longer_matches() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;
        repository
            .reconcile(
                &[ProcessRecord::starting("api", "shell", "cargo run")],
                &["api"],
            )
            .await
            .expect("reconcile should succeed");
        repository
            .record_running("api", 42, Some(111))
            .await
            .expect("record_running should succeed");
        let control = FakeControl::alive_with_start_time(42, 999);

        claim_with(&repository, &test_policy(), &control)
            .await
            .expect("claim should succeed");

        assert!(control.terminated.lock().unwrap().is_empty());
        assert!(control.killed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reclaim_skips_a_pid_with_no_recorded_start_time() {
        let dir = TempDir::new().expect("temp dir should create");
        let repository = open_repository(&dir).await;
        repository
            .reconcile(
                &[ProcessRecord::starting("api", "shell", "cargo run")],
                &["api"],
            )
            .await
            .expect("reconcile should succeed");
        repository
            .record_running("api", 42, None)
            .await
            .expect("record_running should succeed");
        let control = FakeControl::alive_with_start_time(42, 111);

        claim_with(&repository, &test_policy(), &control)
            .await
            .expect("claim should succeed");

        assert!(control.terminated.lock().unwrap().is_empty());
        assert!(control.killed.lock().unwrap().is_empty());
    }
}
