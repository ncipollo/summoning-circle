use anyhow::{Context as _, Result};
use tokio::signal::unix::SignalKind;
use tokio::sync::watch;

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
