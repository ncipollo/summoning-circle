use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{Debouncer, new_debouncer};
use tokio::sync::mpsc;
use tracing::warn;

/// Watches `path` for changes, debounced by `debounce` so a burst of editor events (write a
/// temp file, then rename) coalesces into one. The returned `Debouncer` must be kept alive for
/// as long as the watch should run; dropping it stops the watch.
pub fn watch(
    path: &Path,
    debounce: Duration,
) -> Result<(Debouncer<RecommendedWatcher>, mpsc::UnboundedReceiver<()>)> {
    let (tx, rx) = mpsc::unbounded_channel();

    let mut debouncer = new_debouncer(
        debounce,
        move |result: notify_debouncer_mini::DebounceEventResult| match result {
            Ok(_) => {
                let _ = tx.send(());
            }
            Err(error) => warn!(%error, "config file watch error"),
        },
    )
    .context("could not start config file watcher")?;

    debouncer
        .watcher()
        .watch(path, RecursiveMode::NonRecursive)
        .with_context(|| format!("could not watch config file at {}", path.display()))?;

    Ok((debouncer, rx))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio::time::timeout;

    use super::watch;

    #[tokio::test]
    async fn a_file_write_yields_a_debounced_event() {
        let dir = TempDir::new().expect("temp dir should create");
        let path = dir.path().join("config.toml");
        fs::write(&path, "initial").expect("initial write should succeed");

        let (_debouncer, mut changes) =
            watch(&path, Duration::from_millis(20)).expect("watch should start");

        fs::write(&path, "changed").expect("write should succeed");

        timeout(Duration::from_secs(5), changes.recv())
            .await
            .expect("should receive a change within the timeout")
            .expect("channel should not have closed");
    }

    #[tokio::test]
    async fn rapid_writes_coalesce_into_at_least_one_event() {
        let dir = TempDir::new().expect("temp dir should create");
        let path = dir.path().join("config.toml");
        fs::write(&path, "initial").expect("initial write should succeed");

        let (_debouncer, mut changes) =
            watch(&path, Duration::from_millis(50)).expect("watch should start");

        for i in 0..5 {
            fs::write(&path, format!("changed {i}")).expect("write should succeed");
        }

        timeout(Duration::from_secs(5), changes.recv())
            .await
            .expect("should receive a change within the timeout")
            .expect("channel should not have closed");
    }
}
