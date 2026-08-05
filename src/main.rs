mod app;
mod cli;
mod components;
mod config;
mod game;
mod launcher;
mod operations;
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
