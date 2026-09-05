use std::path::PathBuf;

use clap::{Parser, Subcommand};

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

#[cfg(test)]
mod tests;
