mod cli;
mod commands;
mod context;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::context::Context;

fn main() {
    let cli = Cli::parse();

    if let Err(err) = run(cli) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let context = Context::resolve(cli.config)?;

    match cli.command {
        Command::Install => commands::install::execute(&context),
        Command::Run => commands::run::execute(&context),
        Command::Ps => commands::ps::execute(&context),
    }
}
