use std::path::PathBuf;

use clap::Parser;
use irmin::{DownloadHandle, SophonProgress};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::game::GameId;
use crate::launcher::Launcher;
use crate::operations::Operations;

/// Elysiae game manager CLI.
#[derive(Parser)]
#[command(name = "elysiae", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available subcommands. When omitted, the app launches in TUI mode.
#[derive(clap::Subcommand)]
pub enum Commands {
    /// Download a game from scratch.
    Download {
        game: GameId,
        #[arg(long, default_value = "en-us")]
        lang: String,
        #[arg(long)]
        path: PathBuf,
    },
    /// Update an installed game to the latest version.
    Update { game: GameId },
    /// Download a preinstall patch for an upcoming version.
    Preinstall { game: GameId },
    /// Apply a previously downloaded preinstall patch.
    ApplyPreinstall { game: GameId },
    /// Verify integrity of installed game files.
    Verify { game: GameId },
    /// Launch the game through Proton.
    Launch { game: GameId },
    /// Check whether an update is available.
    CheckUpdate { game: GameId },
}

/// Executes the given CLI command against the provided config.
pub async fn run_cli(cmd: Commands, config: &mut Config) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Commands::Download { game, lang, path } => {
            let client = reqwest::Client::new();
            let ops = Operations::new(client);
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = path.to_string_lossy();
            ops.download(game, &lang, &path_str, &handle, tx).await?;

            // Persist the install path for future operations
            let gc = config.game_config(game);
            gc.install_path = Some(path);
            gc.vo_lang = lang;
            config.save()?;

            println!("Download complete.");
        }
        Commands::Update { game } => {
            let gc = config.game_config(game).clone();
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let client = reqwest::Client::new();
            let ops = Operations::new(client);
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = install_path.to_string_lossy();
            ops.update(game, &gc.vo_lang, &path_str, &handle, tx).await?;
            println!("Update complete.");
        }
        Commands::Preinstall { game } => {
            let gc = config.game_config(game).clone();
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let client = reqwest::Client::new();
            let ops = Operations::new(client);
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = install_path.to_string_lossy();
            ops.preinstall(game, &gc.vo_lang, &path_str, &handle, tx).await?;
            println!("Preinstall download complete.");
        }
        Commands::ApplyPreinstall { game } => {
            let gc = config.game_config(game).clone();
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let client = reqwest::Client::new();
            let ops = Operations::new(client);
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            // check_update gives us the preinstall_tag needed for apply
            let path_str = install_path.to_string_lossy();
            let info = ops.check_update(game, &gc.vo_lang, &path_str).await?;
            let tag = info.preinstall_tag.ok_or("no preinstall tag available")?;
            ops.apply_preinstall(&tag, &path_str, &handle, tx).await?;
            println!("Preinstall applied.");
        }
        Commands::Verify { game } => {
            let gc = config.game_config(game).clone();
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let client = reqwest::Client::new();
            let ops = Operations::new(client);
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = install_path.to_string_lossy();
            ops.verify(game, &gc.vo_lang, &path_str, tx).await?;
            println!("Verification complete.");
        }
        Commands::Launch { game } => {
            let gc = config.game_config(game).clone();
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let data_dir = dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                .join("elysiae-tui");

            let launcher = Launcher::new(data_dir);
            let (log_tx, _log_rx) = tokio::sync::mpsc::channel(16);
            launcher.launch(game, install_path, log_tx)?;
        }
        Commands::CheckUpdate { game } => {
            let gc = config.game_config(game).clone();
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let client = reqwest::Client::new();
            let ops = Operations::new(client);

            let path_str = install_path.to_string_lossy();
            let info = ops.check_update(game, &gc.vo_lang, &path_str).await?;

            if info.update_available {
                println!("Update available: {} -> {}", info.current_tag.as_deref().unwrap_or("unknown"), info.remote_tag);
            } else {
                println!("Already up to date ({})", info.remote_tag);
            }
            if info.preinstall_available {
                let tag = info.preinstall_tag.as_deref().unwrap_or("unknown");
                println!("Preinstall available: {tag}");
            }
        }
    }
    Ok(())
}

fn print_progress(p: &SophonProgress) {
    match p {
        SophonProgress::FetchingManifest => {
            println!("Fetching manifest...");
        }
        SophonProgress::CalculatingDownloads { checked_files, total_files } => {
            println!("Calculating: {checked_files}/{total_files} files");
        }
        SophonProgress::Downloading { downloaded_bytes, total_bytes, speed_bps, .. } => {
            let pct = if *total_bytes > 0 { *downloaded_bytes * 100 / *total_bytes } else { 0 };
            let speed_mb = *speed_bps / 1_000_000.0;
            println!("Downloading: {pct}% ({speed_mb:.1} MB/s)");
        }
        SophonProgress::Paused { downloaded_bytes, total_bytes } => {
            let pct = if *total_bytes > 0 { *downloaded_bytes * 100 / *total_bytes } else { 0 };
            println!("Paused: {pct}%");
        }
        SophonProgress::Assembling { assembled_files, total_files } => {
            println!("Assembling: {assembled_files}/{total_files} files");
        }
        SophonProgress::CheckingFiles { checked_files, total_files } => {
            println!("Checking: {checked_files}/{total_files} files");
        }
        SophonProgress::Verifying { scanned_files, total_files, error_count } => {
            println!("Verifying: {scanned_files}/{total_files} files ({error_count} errors)");
        }
        SophonProgress::ApplyingPreinstall { applied_files, total_files } => {
            println!("Applying preinstall: {applied_files}/{total_files} files");
        }
        SophonProgress::InstallingPlugins { current_plugin, total_plugins } => {
            println!("Installing plugin: {current_plugin} ({total_plugins} total)");
        }
        SophonProgress::InstallingSdks { current_sdk, total_sdks } => {
            println!("Installing SDK: {current_sdk} ({total_sdks} total)");
        }
        SophonProgress::DownloadingPlugin { name, downloaded_bytes, total_bytes } => {
            let pct = if *total_bytes > 0 { *downloaded_bytes * 100 / *total_bytes } else { 0 };
            println!("Downloading plugin {name}: {pct}%");
        }
        SophonProgress::Warning { message } => {
            println!("Warning: {message}");
        }
        SophonProgress::Error { message } => {
            eprintln!("Error: {message}");
        }
        SophonProgress::Finished => {
            println!("Finished.");
        }
    }
}
