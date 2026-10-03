#[cfg(target_os = "macos")]
mod macos;

use std::path::Path;

use anyhow::Result;

#[cfg(not(target_os = "macos"))]
const UNSUPPORTED_PLATFORM_ERROR: &str = "tray is only supported on macOS";

/// Launches a persistent menu-bar icon reflecting the tracked processes at `db_path`, blocking
/// until the user quits it, at which point `on_exit` runs. There is no channel to the
/// supervisor: like every other command, this polls the same SQLite store directly (see
/// `feature::ps`).
#[cfg(target_os = "macos")]
pub fn run(db_path: &Path, on_exit: impl FnOnce() + 'static) -> Result<()> {
    macos::run(db_path, on_exit)
}

#[cfg(not(target_os = "macos"))]
pub fn run(_db_path: &Path, _on_exit: impl FnOnce() + 'static) -> Result<()> {
    anyhow::bail!(UNSUPPORTED_PLATFORM_ERROR)
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use std::path::Path;

    use super::run;

    #[test]
    fn errors_on_unsupported_platforms() {
        let error =
            run(Path::new("/tmp/circle.db"), || {}).expect_err("should be unsupported here");

        assert!(error.to_string().contains("only supported on macOS"));
    }
}
