use anyhow::Result;

use crate::context::Context;

pub fn run(context: &Context) -> Result<()> {
    println!(
        "ps: not implemented yet (config: {})",
        context.config_path.display()
    );
    Ok(())
}
