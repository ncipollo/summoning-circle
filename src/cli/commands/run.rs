use anyhow::Result;
use summoning_circle::feature::engine;
use summoning_circle::feature::supervisor::signals;
use tokio::sync::watch;

use crate::cli::context::Context;

pub async fn run(context: &Context) -> Result<()> {
    tracing_subscriber::fmt::init();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(signals::listen(shutdown_tx));

    engine::run(&context.engine_paths(), shutdown_rx).await
}
