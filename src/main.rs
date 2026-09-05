mod cli;

use clap::Parser;

use cli::Cli;

fn main() {
    if let Err(error) = cli::route(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
