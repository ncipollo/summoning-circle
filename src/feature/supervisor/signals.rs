use anyhow::{Context as _, Result};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tokio::signal::unix::SignalKind;
use tokio::sync::watch;
use tracing::warn;

/// Waits for SIGTERM or SIGINT, then sends `true` once on `shutdown`.
pub async fn listen(shutdown: watch::Sender<bool>) -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
        .context("could not register SIGTERM handler")?;
    let mut interrupt = tokio::signal::unix::signal(SignalKind::interrupt())
        .context("could not register SIGINT handler")?;

    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }

    let _ = shutdown.send(true);
    Ok(())
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
