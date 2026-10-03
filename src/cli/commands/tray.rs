use anyhow::Result;
use summoning_circle::feature::engine::Background;
use summoning_circle::feature::tray;

use crate::cli::context::Context;

/// Not `async`: on macOS, `tao`'s event loop takes over the calling thread for the tray's whole
/// lifetime and drives its own short-lived runtimes per poll tick (see `feature::tray::macos`),
/// so `cli::run` calls this before creating any Tokio runtime. The engine (same as `run`) lives
/// on a background thread with its own runtime.
pub fn run(context: &Context) -> Result<()> {
    tracing_subscriber::fmt::init();

    let engine = Background::spawn(context.engine_paths())?;
    tray::run(&context.db_path, move || engine.stop())
}
