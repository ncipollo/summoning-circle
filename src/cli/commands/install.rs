use anyhow::Result;

use crate::cli::context::Context;

pub fn run(context: &Context) -> Result<()> {
    println!(
        "install: not implemented yet (data dir: {}, db: {})",
        context.data_dir.display(),
        context.db_path.display()
    );
    Ok(())
}
