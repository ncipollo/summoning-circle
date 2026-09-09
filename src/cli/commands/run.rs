use anyhow::Result;
use summoning_circle::feature::config;
use summoning_circle::feature::store::Store;
use summoning_circle::feature::supervisor::policy::Policy;
use summoning_circle::feature::supervisor::{Supervisor, signals};
use tokio::sync::watch;

use crate::cli::context::Context;

pub async fn run(context: &Context) -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = config::load(&context.config_path)?;
    let store = Store::open(&context.db_path).await?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    tokio::spawn(signals::listen(shutdown_tx));

    let supervisor = Supervisor::new(
        &config,
        store,
        context.log_dir.clone(),
        context.config_path.clone(),
        Policy::default(),
    );
    supervisor.run(shutdown_rx).await
}
