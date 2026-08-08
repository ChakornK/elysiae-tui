mod app;
mod atomic;
mod backgrounds;
mod cli;
mod components;
mod config;
mod disk;
mod game;
mod http;
mod launcher;
mod operations;
mod postinstall;
mod quadrant;
mod signal;
mod state;
mod tui;
mod ui;

use clap::Parser;
use cli::Cli;
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
            if let Err(e) = tui::run(config).await {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}
