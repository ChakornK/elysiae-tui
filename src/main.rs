#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

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

fn init_logging() {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().expect("HOME must be set").join(".local/share"))
        .join("elysiae-tui")
        .join("logs");
    let _ = std::fs::create_dir_all(&data_dir);
    let file_appender = tracing_appender::rolling::Builder::new()
        .max_log_files(3)
        .rotation(tracing_appender::rolling::Rotation::NEVER)
        .filename_prefix("elysiae-tui")
        .filename_suffix("log")
        .build(&data_dir)
        .expect("failed to create log file");
    tracing_subscriber::fmt()
        .with_writer(file_appender)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ELYSIAE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .init();
}

#[tokio::main]
async fn main() {
    init_logging();
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
