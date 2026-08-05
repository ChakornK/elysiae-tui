mod app;
mod cli;
mod components;
mod config;
mod game;
mod launcher;
mod operations;
mod ui;

use clap::Parser;
use cli::{Cli, Commands};
use config::Config;

#[tokio::main]
async fn main() {
    let cli_args = Cli::parse();
    let mut config = Config::load();

    match cli_args.command {
        Some(cmd) => {
            if let Err(e) = cli::run_cli(cmd, &mut config).await {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        None => {
            if let Err(e) = run_tui(config) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn run_tui(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // TUI event loop will be implemented in a later task
    let _app = app::App::new(config);
    println!("TUI mode not yet implemented. Use subcommands for now.");
    Ok(())
}
