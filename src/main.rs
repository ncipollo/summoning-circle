mod cli;
mod commands;
mod context;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};
use commands::{install, ps, run as run_command};
use context::Context;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let context = Context::new(cli.config)?;

    match cli.command {
        Command::Install => install::run(&context),
        Command::Run => run_command::run(&context),
        Command::Ps => ps::run(&context),
    }
}
