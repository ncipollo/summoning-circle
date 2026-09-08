pub mod agent;

#[cfg(target_os = "macos")]
mod macos;

use std::path::Path;

use anyhow::Result;

#[cfg(not(target_os = "macos"))]
const UNSUPPORTED_PLATFORM_ERROR: &str = "install is only supported on macOS (launchd)";

#[cfg(target_os = "macos")]
pub fn install(log_dir: &Path, config_override: Option<&Path>) -> Result<()> {
    macos::install(log_dir, config_override)
}

#[cfg(not(target_os = "macos"))]
pub fn install(_log_dir: &Path, _config_override: Option<&Path>) -> Result<()> {
    anyhow::bail!(UNSUPPORTED_PLATFORM_ERROR)
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    macos::uninstall()
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall() -> Result<()> {
    anyhow::bail!(UNSUPPORTED_PLATFORM_ERROR)
}
