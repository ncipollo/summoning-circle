use anyhow::Result;
use summoning_circle::feature::launchd;

pub fn run() -> Result<()> {
    launchd::uninstall()
}
