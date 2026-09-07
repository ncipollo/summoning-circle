use std::time::Instant;

use anyhow::{Context as _, Result};
use tokio::sync::watch;
use tokio::time;
use tracing::info;

use super::backoff::Backoff;
use super::policy::Policy;
use super::repository::ProcessRepository;
use super::shell::Summon;
use super::signals;

/// Spawns, watches, and relaunches a single process until `shutdown` fires.
pub async fn run(
    name: &str,
    summon: &dyn Summon,
    repository: &ProcessRepository,
    mut shutdown: watch::Receiver<bool>,
    policy: &Policy,
) -> Result<()> {
    let mut backoff = Backoff::new(policy);

    while !*shutdown.borrow() {
        let mut child = summon.spawn()?;
        let pid = child.id().context("spawned child is missing a pid")?;
        repository.record_running(name, pid).await?;
        info!(name, pid, "launched process");

        let started_at = Instant::now();
        tokio::select! {
            status = child.wait() => {
                let status = status?;
                info!(name, code = status.code(), "process exited");
                repository.record_exited(name, status.code()).await?;

                let delay = backoff.after_exit(started_at.elapsed());
                tokio::select! {
                    () = time::sleep(delay) => {}
                    _ = shutdown.changed() => return Ok(()),
                }
            }
            _ = shutdown.changed() => {
                stop_child(&mut child, pid, policy).await;
                repository.record_stopped(name).await?;
                info!(name, "stopped process");
                return Ok(());
            }
        }
    }

    Ok(())
}

/// Asks the child to terminate gracefully, escalating to SIGKILL after the policy's grace period.
async fn stop_child(child: &mut tokio::process::Child, pid: u32, policy: &Policy) {
    signals::terminate(pid);

    tokio::select! {
        _ = child.wait() => {}
        _ = time::sleep(policy.shutdown_grace) => {
            let _ = child.kill().await;
        }
    }
}
