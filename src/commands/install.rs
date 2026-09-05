use anyhow::Result;

use crate::context::Context;

/// Installs summoning-circle as a user launch agent.
pub fn execute(_context: &Context) -> Result<()> {
    println!("install: not yet implemented");
    Ok(())
}
