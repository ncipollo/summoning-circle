use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "summoning-circle", version, about, long_about = None)]
pub struct Cli {
    /// Path to the config file. Defaults to the standard config location when omitted.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install summoning-circle as a user launch agent
    Install,
    /// Launch configured processes and keep them alive (foreground)
    Run,
    /// List processes currently tracked by summoning-circle
    Ps,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_and_ps_subcommand() {
        let cli = Cli::parse_from(["summoning-circle", "--config", "/tmp/x.toml", "ps"]);

        assert_eq!(cli.config, Some(PathBuf::from("/tmp/x.toml")));
        assert!(matches!(cli.command, Command::Ps));
    }

    #[test]
    fn parses_without_config() {
        let cli = Cli::parse_from(["summoning-circle", "run"]);

        assert_eq!(cli.config, None);
        assert!(matches!(cli.command, Command::Run));
    }
}
