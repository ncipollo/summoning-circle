mod commands;
mod context;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use commands::{install, ps, run as run_command};
use context::Context;

#[derive(Debug, Parser)]
#[command(name = "summoning-circle", version, about)]
pub struct Cli {
    /// Path to the process config file
    #[arg(long, short, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Install summoning-circle as a user launch agent
    Install,
    /// Launch configured processes and keep them alive (foreground)
    Run,
    /// List processes currently tracked by summoning-circle
    Ps,
}

pub fn route(cli: Cli) -> Result<()> {
    let context = Context::new(cli.config)?;

    match cli.command {
        Command::Install => install::run(&context),
        Command::Run => run_command::run(&context),
        Command::Ps => ps::run(&context),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::{Cli, Command};

    #[test]
    fn parses_config_and_subcommand() {
        let cli = Cli::parse_from(["summoning-circle", "--config", "/tmp/x.toml", "ps"]);

        assert_eq!(cli.config, Some(PathBuf::from("/tmp/x.toml")));
        assert_eq!(cli.command, Command::Ps);
    }

    #[test]
    fn defaults_config_to_none() {
        let cli = Cli::parse_from(["summoning-circle", "run"]);

        assert_eq!(cli.config, None);
        assert_eq!(cli.command, Command::Run);
    }

    #[test]
    fn rejects_unknown_subcommand() {
        let result = Cli::try_parse_from(["summoning-circle", "bogus"]);

        assert!(result.is_err());
    }
}
