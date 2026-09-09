use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use crate::feature::launchd::agent::{self, LABEL};

pub fn install(log_dir: &Path, config_override: Option<&Path>) -> Result<()> {
    fs::create_dir_all(log_dir)
        .with_context(|| format!("could not create log directory at {}", log_dir.display()))?;

    let path = plist_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }

    let domain = gui_domain()?;
    if path.exists() {
        bootout(&domain);
    }

    let exe = std::env::current_exe().context("could not resolve the current executable path")?;
    let launch_agent = agent::build(&exe, config_override, log_dir);
    agent::write(&launch_agent, &path)?;

    bootstrap(&domain, &path)?;

    println!("Installed launch agent at {}", path.display());
    println!("Check status with: launchctl print {domain}/{LABEL}");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let path = plist_path()?;
    let domain = gui_domain()?;

    if !path.exists() {
        println!("No launch agent installed at {}", path.display());
        return Ok(());
    }

    bootout(&domain);
    fs::remove_file(&path).with_context(|| format!("could not remove {}", path.display()))?;
    println!("Uninstalled launch agent at {}", path.display());
    Ok(())
}

fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve the current user's home directory")?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn current_uid() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("could not resolve the current user id")?;
    if !output.status.success() {
        bail!("id -u exited with a failure status");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn gui_domain() -> Result<String> {
    Ok(format!("gui/{}", current_uid()?))
}

/// Loads the plist, preferring `bootstrap` (modern macOS) and falling back to
/// `load -w` when bootstrap isn't available.
fn bootstrap(domain: &str, path: &Path) -> Result<()> {
    let bootstrapped = Command::new("launchctl")
        .args(["bootstrap", domain])
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if bootstrapped {
        return Ok(());
    }

    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(path)
        .status()
        .context("could not run launchctl load")?;
    if !status.success() {
        bail!("launchctl load exited with a failure status");
    }
    Ok(())
}

/// Unloads the agent if it's currently loaded. Best effort: it's normal for this to
/// "fail" when the agent isn't loaded yet, so the exit status is ignored.
fn bootout(domain: &str) {
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{domain}/{LABEL}")])
        .status();
}
