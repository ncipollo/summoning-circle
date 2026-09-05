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
