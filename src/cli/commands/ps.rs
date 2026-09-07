use anyhow::Result;
use chrono::Utc;
use summoning_circle::feature::ps::{self, table};
use summoning_circle::feature::store::Store;

use crate::cli::context::Context;

pub async fn run(context: &Context, json: bool) -> Result<()> {
    if !context.db_path.exists() {
        println!("{}", ps::NO_PROCESSES);
        return Ok(());
    }

    let store = Store::open(&context.db_path).await?;
    let records = ps::resolve(store.list().await?);

    if json {
        println!("{}", ps::render_json(&records)?);
    } else {
        print!("{}", table::render(&records, Utc::now()));
    }

    Ok(())
}
