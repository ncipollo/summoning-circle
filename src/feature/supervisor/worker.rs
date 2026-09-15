use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use tokio::sync::watch;
use tokio::time;
use tracing::info;

use super::backoff::Backoff;
use super::policy::Policy;
use super::repository::ProcessRepository;
use super::shell::Summon;
use crate::feature::proc;

/// What ended a single launch attempt.
enum Ended {
    /// `shutdown` fired; the child (if any) has already been stopped.
    Shutdown,
    /// The child exited on its own, after running for `uptime`.
    Exited { uptime: Duration },
    /// A pause raced the launch: the worker stopped its own child rather than let it run.
    Paused,
}

/// Spawns, watches, and relaunches a single process until `shutdown` fires. A paused process
/// is neither launched nor relaunched until it's resumed.
pub async fn run(
    name: &str,
    summon: &dyn Summon,
    repository: &ProcessRepository,
    mut shutdown: watch::Receiver<bool>,
    policy: &Policy,
) -> Result<()> {
    let mut backoff = Backoff::new(policy);

    while !*shutdown.borrow() {
        if !wait_while_paused(name, repository, &mut shutdown, policy).await? {
            return Ok(());
        }

        match launch(name, summon, repository, &mut shutdown, policy).await? {
            Ended::Shutdown => return Ok(()),
            Ended::Paused => continue,
            Ended::Exited { uptime } => {
                let delay = backoff.after_exit(uptime);
                tokio::select! {
                    () = time::sleep(delay) => {}
                    _ = shutdown.changed() => return Ok(()),
                }
            }
        }
    }

    Ok(())
}

/// Waits out a pause on `name`, returning `true` once it's clear to launch (whether or not it
/// was ever paused) or `false` if `shutdown` fired first. Only polls the store while actually
/// paused — an unpaused worker never touches it here.
async fn wait_while_paused(
    name: &str,
    repository: &ProcessRepository,
    shutdown: &mut watch::Receiver<bool>,
    policy: &Policy,
) -> Result<bool> {
    if !repository.is_paused(name).await? {
        return Ok(true);
    }

    repository.record_stopped(name).await?;
    info!(name, "process paused");

    while repository.is_paused(name).await? {
        tokio::select! {
            () = time::sleep(policy.pause_poll) => {}
            _ = shutdown.changed() => return Ok(false),
        }
    }

    info!(name, "process resumed");
    Ok(true)
}

/// Spawns one child and runs it to completion: exit, shutdown, or a pause that raced the
/// spawn closely enough that this launch never got a chance to run unpaused.
async fn launch(
    name: &str,
    summon: &dyn Summon,
    repository: &ProcessRepository,
    shutdown: &mut watch::Receiver<bool>,
    policy: &Policy,
) -> Result<Ended> {
    let mut child = summon.spawn()?;
    let pid = child.id().context("spawned child is missing a pid")?;
    repository
        .record_running(name, pid, proc::start_time(pid))
        .await?;
    info!(name, pid, "launched process");

    if repository.is_paused(name).await? {
        stop_child(&mut child, pid, policy).await;
        repository.record_stopped(name).await?;
        info!(name, "stopped process after a pause raced the launch");
        return Ok(Ended::Paused);
    }

    let started_at = Instant::now();
    tokio::select! {
        status = child.wait() => {
            let status = status?;
            info!(name, code = status.code(), "process exited");
            repository.record_exited(name, status.code()).await?;
            Ok(Ended::Exited { uptime: started_at.elapsed() })
        }
        _ = shutdown.changed() => {
            stop_child(&mut child, pid, policy).await;
            repository.record_stopped(name).await?;
            info!(name, "stopped process");
            Ok(Ended::Shutdown)
        }
    }
}

/// Asks the child to terminate gracefully, escalating to SIGKILL after the policy's grace period.
async fn stop_child(child: &mut tokio::process::Child, pid: u32, policy: &Policy) {
    proc::terminate(pid);

    tokio::select! {
        _ = child.wait() => {}
        _ = time::sleep(policy.shutdown_grace) => {
            proc::kill(pid);
            let _ = child.wait().await;
        }
    }
}
