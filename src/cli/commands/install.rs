use std::path::Path;

use anyhow::Result;
use summoning_circle::feature::launchd;

use crate::cli::context::Context;

pub fn run(context: &Context, uninstall: bool, config_override: Option<&Path>) -> Result<()> {
    if uninstall {
        launchd::uninstall()
    } else {
        launchd::install(&context.log_dir, config_override)
    }
}
