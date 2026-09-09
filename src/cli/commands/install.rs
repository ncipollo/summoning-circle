use std::path::Path;

use anyhow::Result;
use summoning_circle::feature::launchd;

use crate::cli::context::Context;

pub fn run(context: &Context, config_override: Option<&Path>) -> Result<()> {
    launchd::install(&context.log_dir, config_override)
}
