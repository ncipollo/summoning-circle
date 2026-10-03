mod cli;

use clap::Parser;

use cli::Cli;

fn main() {
    if let Err(error) = cli::run(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
