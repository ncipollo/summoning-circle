use anyhow::Result;
use summoning_circle::feature::tray;

use crate::cli::context::Context;

/// Not `async`: on macOS, `tao`'s event loop takes over the calling thread for the tray's whole
/// lifetime and drives its own short-lived runtimes per poll tick (see `feature::tray::macos`)
/// rather than running inside the outer `#[tokio::main]` one.
pub fn run(context: &Context) -> Result<()> {
    tray::run(&context.db_path)
}
