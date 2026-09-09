use anyhow::{Result, bail};
use summoning_circle::feature::proc::control::SystemControl;
use summoning_circle::feature::ps;
use summoning_circle::feature::restart;
use summoning_circle::feature::store::Store;
use summoning_circle::feature::supervisor::policy::Policy;

use crate::cli::context::Context;

pub async fn run(context: &Context, name: &str) -> Result<()> {
    if !context.db_path.exists() {
        bail!("no tracked process named '{name}'");
    }

    let store = Store::open(&context.db_path).await?;
    let records = ps::resolve(store.list().await?);
    let supervisor = store.supervisor().await?;

    let (outcome, supervisor_alive) = restart::restart(
        &records,
        name,
        supervisor,
        Policy::default().shutdown_grace,
        &SystemControl,
    )
    .await?;
    print!("{}", restart::render(&outcome, supervisor_alive));

    Ok(())
}
