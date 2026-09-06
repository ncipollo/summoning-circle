mod cli;

use clap::Parser;

use cli::Cli;

#[tokio::main]
async fn main() {
    if let Err(error) = cli::route(Cli::parse()).await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
