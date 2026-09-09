pub mod control;

use nix::errno::Errno;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use sysinfo::{Pid as SysPid, ProcessRefreshKind, ProcessesToUpdate, System};
use tracing::warn;

/// Checks whether `pid` is still alive by sending it no signal. Only
/// `ESRCH` ("no such process") is treated as dead; any other error (e.g.
/// `EPERM`, meaning the process exists but isn't ours to signal) means it's
/// still alive.
pub fn is_alive(pid: u32) -> bool {
    !matches!(
        signal::kill(Pid::from_raw(pid as i32), None),
        Err(Errno::ESRCH)
    )
}

/// Asks `pid`'s whole process group to shut down gracefully.
pub fn terminate(pid: u32) {
    send_to_group(pid, Signal::SIGTERM);
}

/// Forces `pid`'s whole process group to exit immediately.
pub fn kill(pid: u32) {
    send_to_group(pid, Signal::SIGKILL);
}

/// Sends `sig` to the process group led by `pid`, which each child is spawned into so that any
/// processes it forks are signaled too, not just the tracked pid itself.
fn send_to_group(pid: u32, sig: Signal) {
    if let Err(error) = signal::kill(Pid::from_raw(-(pid as i32)), sig) {
        warn!(pid, %sig, %error, "could not signal process group");
    }
}

/// Looks up the OS-reported start time (seconds since epoch) of `pid`, or
/// `None` if the process doesn't exist. Used to verify a stored pid still
/// refers to the process we recorded, since pids get recycled.
pub fn start_time(pid: u32) -> Option<i64> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[SysPid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system
        .process(SysPid::from_u32(pid))
        .map(|process| process.start_time() as i64)
}

#[cfg(test)]
mod tests {
    use super::start_time;

    #[test]
    fn start_time_returns_some_for_the_current_process() {
        let pid = std::process::id();

        assert!(start_time(pid).is_some());
    }

    #[test]
    fn start_time_returns_none_for_an_unused_pid() {
        assert_eq!(start_time(u32::MAX), None);
    }
}
