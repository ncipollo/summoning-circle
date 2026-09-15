use anyhow::{Result, bail};
use summoning_circle::feature::pause;
use summoning_circle::feature::proc::control::SystemControl;
use summoning_circle::feature::store::Store;
use summoning_circle::feature::supervisor::policy::Policy;

use crate::cli::context::Context;

pub async fn run(context: &Context, name: &str) -> Result<()> {
    if !context.db_path.exists() {
        bail!("no tracked process named '{name}'");
    }

    let store = Store::open(&context.db_path).await?;
    let outcome = pause::pause(
        &store,
        name,
        Policy::default().shutdown_grace,
        &SystemControl,
    )
    .await?;
    print!("{}", pause::render_pause(name, &outcome));

    Ok(())
}
