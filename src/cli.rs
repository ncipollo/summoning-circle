mod commands;
mod context;
mod info;

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};

use commands::{install, ps, run as run_command};
use context::Context;

#[derive(Debug, Parser)]
#[command(name = "summoning-circle", version, about)]
pub struct Cli {
    /// Path to the process config file
    #[arg(long, short, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// Print documentation and exit. Bare --info lists the available
    /// topics; --info <TOPIC> prints one.
    #[arg(long, value_name = "TOPIC", num_args = 0..=1)]
    pub info: Option<Option<String>>,
    #[command(subcommand)]
    pub command: Option<Command>,
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

pub async fn route(cli: Cli) -> Result<()> {
    if let Some(topic) = cli.info {
        return info::run(topic.as_deref());
    }

    let command = cli
        .command
        .ok_or_else(|| anyhow!("a subcommand is required: install, run, ps"))?;
    let context = Context::new(cli.config)?;

    match command {
        Command::Install => install::run(&context),
        Command::Run => run_command::run(&context).await,
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
        assert_eq!(cli.command, Some(Command::Ps));
    }

    #[test]
    fn defaults_config_to_none() {
        let cli = Cli::parse_from(["summoning-circle", "run"]);

        assert_eq!(cli.config, None);
        assert_eq!(cli.command, Some(Command::Run));
    }

    #[test]
    fn rejects_unknown_subcommand() {
        let result = Cli::try_parse_from(["summoning-circle", "bogus"]);

        assert!(result.is_err());
    }

    #[test]
    fn parses_bare_info_flag_without_subcommand() {
        let cli = Cli::parse_from(["summoning-circle", "--info"]);

        assert_eq!(cli.info, Some(None));
        assert_eq!(cli.command, None);
    }

    #[test]
    fn parses_info_flag_with_topic() {
        let cli = Cli::parse_from(["summoning-circle", "--info", "usage"]);

        assert_eq!(cli.info, Some(Some("usage".to_string())));
    }

    #[test]
    fn parses_subcommand_without_info() {
        let cli = Cli::parse_from(["summoning-circle", "ps"]);

        assert_eq!(cli.info, None);
        assert_eq!(cli.command, Some(Command::Ps));
    }
}
