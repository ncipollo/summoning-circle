use anyhow::Result;
use summoning_circle::feature::kill;
use summoning_circle::feature::proc::control::SystemControl;
use summoning_circle::feature::ps;
use summoning_circle::feature::store::Store;
use summoning_circle::feature::supervisor::policy::Policy;

use crate::cli::context::Context;

pub async fn run(context: &Context) -> Result<()> {
    if !context.db_path.exists() {
        println!("{}", ps::NO_PROCESSES);
        return Ok(());
    }

    let store = Store::open(&context.db_path).await?;
    let records = ps::resolve(store.list().await?);

    let outcomes = kill::kill_all(&records, Policy::default().shutdown_grace, &SystemControl).await;
    print!("{}", kill::render(&outcomes));

    Ok(())
}
