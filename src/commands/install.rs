use anyhow::Result;

use crate::context::Context;

pub fn run(context: &Context) -> Result<()> {
    println!(
        "install: not implemented yet (data dir: {})",
        context.data_dir.display()
    );
    Ok(())
}
