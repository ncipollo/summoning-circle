use anyhow::Result;

use crate::context::Context;

/// Lists processes currently tracked by summoning-circle.
pub fn execute(_context: &Context) -> Result<()> {
    println!("ps: not yet implemented");
    Ok(())
}
