use anyhow::Result;

use crate::context::Context;

pub fn run(context: &Context) -> Result<()> {
    println!(
        "run: not implemented yet (config: {})",
        context.config_path.display()
    );
    Ok(())
}
