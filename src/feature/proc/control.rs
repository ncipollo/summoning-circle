use std::time::Duration;

use tokio::time::{self, Instant};

use crate::feature::proc;

/// Abstracts liveness checks and signaling so callers can be tested without touching
/// real processes.
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
        proc::terminate(pid);
    }

    fn kill(&self, pid: u32) {
        proc::kill(pid);
    }
}

/// Asks `pid` to terminate gracefully, escalating to SIGKILL after `grace` elapses.
/// Returns `true` if it took a SIGKILL to stop it.
pub async fn stop(pid: u32, grace: Duration, control: &dyn ProcessControl) -> bool {
    stop_all(&[pid], grace, control).await == vec![pid]
}

/// SIGTERMs every pid, polls until all are dead or `grace` elapses (paid once for the
/// whole batch, not per pid), then SIGKILLs whatever is still alive. Returns the pids
/// that had to be killed.
pub async fn stop_all(pids: &[u32], grace: Duration, control: &dyn ProcessControl) -> Vec<u32> {
    for &pid in pids {
        control.terminate(pid);
    }

    let deadline = Instant::now() + grace;
    while pids.iter().any(|&pid| control.is_alive(pid)) && Instant::now() < deadline {
        time::sleep(Duration::from_millis(10)).await;
    }

    let stragglers: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|&pid| control.is_alive(pid))
        .collect();
    for &pid in &stragglers {
        control.kill(pid);
    }
    stragglers
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct FakeControl {
        pub alive: Mutex<HashMap<u32, bool>>,
        pub start_times: HashMap<u32, i64>,
        pub dies_on_terminate: bool,
        pub terminated: Mutex<Vec<u32>>,
        pub killed: Mutex<Vec<u32>>,
    }

    impl FakeControl {
        pub fn alive_with_start_time(pid: u32, start_time: i64) -> Self {
            let mut control = Self::default();
            control.alive.lock().unwrap().insert(pid, true);
            control.start_times.insert(pid, start_time);
            control
        }

        pub fn dead(pid: u32) -> Self {
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

    #[tokio::test]
    async fn stop_all_terminates_pids_that_die_gracefully() {
        let control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(1, 0)
        };
        control.alive.lock().unwrap().insert(2, true);

        let killed = stop_all(&[1, 2], Duration::from_millis(50), &control).await;

        assert_eq!(*control.terminated.lock().unwrap(), vec![1, 2]);
        assert!(killed.is_empty());
    }

    #[tokio::test]
    async fn stop_all_escalates_only_the_pids_still_alive_after_grace() {
        let control = FakeControl::alive_with_start_time(1, 0);
        control.alive.lock().unwrap().insert(2, true);

        let killed = stop_all(&[1, 2], Duration::from_millis(20), &control).await;

        let mut killed = killed;
        killed.sort_unstable();
        assert_eq!(killed, vec![1, 2]);
    }

    #[tokio::test]
    async fn stop_all_with_no_pids_is_a_no_op() {
        let control = FakeControl::default();

        let killed = stop_all(&[], Duration::from_millis(20), &control).await;

        assert!(killed.is_empty());
        assert!(control.terminated.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn stop_returns_false_when_the_pid_dies_gracefully() {
        let control = FakeControl {
            dies_on_terminate: true,
            ..FakeControl::alive_with_start_time(1, 0)
        };

        assert!(!stop(1, Duration::from_millis(50), &control).await);
    }

    #[tokio::test]
    async fn stop_returns_true_when_the_pid_needed_a_kill() {
        let control = FakeControl::alive_with_start_time(1, 0);

        assert!(stop(1, Duration::from_millis(20), &control).await);
    }
}
