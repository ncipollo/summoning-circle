use anyhow::Result;

use crate::context::Context;

/// Launches configured processes and keeps them alive in the foreground.
pub fn execute(_context: &Context) -> Result<()> {
    println!("run: not yet implemented");
    Ok(())
}
