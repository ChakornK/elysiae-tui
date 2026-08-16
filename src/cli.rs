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
        #[arg(long)]
        tag: Option<String>,
    },
    /// Update an installed game to the latest version.
    Update {
        game: GameId,
        #[arg(long)]
        lang: Option<String>,
    },
    /// Download a preinstall patch for an upcoming version.
    Preinstall {
        game: GameId,
        #[arg(long)]
        lang: Option<String>,
    },
    /// Apply a previously downloaded preinstall patch.
    ApplyPreinstall {
        game: GameId,
        #[arg(long)]
        lang: Option<String>,
    },
    /// Verify integrity of installed game files.
    Verify {
        game: GameId,
        #[arg(long)]
        lang: Option<String>,
    },
    /// Launch the game through Proton.
    Launch { game: GameId },
    /// Check whether an update is available.
    CheckUpdate {
        game: GameId,
        #[arg(long)]
        lang: Option<String>,
    },
    /// Resume an interrupted download.
    Resume { game: GameId },
}

/// Executes the given CLI command against the provided config.
pub async fn run_cli(cmd: Commands, config: &mut Config) -> Result<(), Box<dyn std::error::Error>> {
    let client = crate::http::build_client();
    match cmd {
        Commands::Download { game, lang, path, tag: _tag } => {
            let ops = Operations::new(client.clone(), crate::config::app_data_dir());
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = path.to_string_lossy();
            tokio::select! {
                result = ops.download(game, &lang, &path_str, &handle, tx) => {
                    result?;
                    // Persist the install path for future operations
                    let gc = config.game_config(game);
                    gc.install_path = Some(path);
                    gc.vo_langs = vec![lang];
                    config.save()?;
                    println!("Download complete.");
                }
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\ninterrupted, exiting...");
                }
            }
        }
        Commands::Update { game, lang } => {
            let gc = config.game_config(game).clone();
            let vo_lang = match lang {
                Some(l) => {
                    crate::config::validate_vo_lang(&l).map_err(|e| e.to_string())?;
                    l
                }
                None => gc.primary_vo_lang().to_owned(),
            };
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let ops = Operations::new(client.clone(), crate::config::app_data_dir());
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = install_path.to_string_lossy();
            tokio::select! {
                result = ops.update(game, &vo_lang, &path_str, &handle, tx) => {
                    result?;
                    println!("Update complete.");
                }
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\ninterrupted, exiting...");
                }
            }
        }
        Commands::Preinstall { game, lang } => {
            let gc = config.game_config(game).clone();
            let vo_lang = match lang {
                Some(l) => {
                    crate::config::validate_vo_lang(&l).map_err(|e| e.to_string())?;
                    l
                }
                None => gc.primary_vo_lang().to_owned(),
            };
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let ops = Operations::new(client.clone(), crate::config::app_data_dir());
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = install_path.to_string_lossy();
            tokio::select! {
                result = ops.preinstall(game, &vo_lang, &path_str, &handle, tx) => {
                    result?;
                    println!("Preinstall download complete.");
                }
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\ninterrupted, exiting...");
                }
            }
        }
        Commands::ApplyPreinstall { game, lang } => {
            let gc = config.game_config(game).clone();
            let vo_lang = match lang {
                Some(l) => {
                    crate::config::validate_vo_lang(&l).map_err(|e| e.to_string())?;
                    l
                }
                None => gc.primary_vo_lang().to_owned(),
            };
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let ops = Operations::new(client.clone(), crate::config::app_data_dir());
            let handle = DownloadHandle::new();
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            // check_update gives us the preinstall_tag needed for apply
            let path_str = install_path.to_string_lossy();
            let info = ops.check_update(game, &vo_lang, &path_str).await?;
            let tag = info.preinstall_tag.ok_or("no preinstall tag available")?;
            irmin::game_installer::validate_asset_name(&tag)
                .map_err(|e| format!("invalid preinstall tag: {e}"))?;
            ops.apply_preinstall(game, &vo_lang, &tag, &path_str, &handle, tx).await?;
            println!("Preinstall applied.");
        }
        Commands::Verify { game, lang } => {
            let gc = config.game_config(game).clone();
            let vo_lang = match lang {
                Some(l) => {
                    crate::config::validate_vo_lang(&l).map_err(|e| e.to_string())?;
                    l
                }
                None => gc.primary_vo_lang().to_owned(),
            };
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let ops = Operations::new(client.clone(), crate::config::app_data_dir());
            let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);

            tokio::spawn(async move {
                while let Some(p) = rx.recv().await {
                    print_progress(&p);
                }
            });

            let path_str = install_path.to_string_lossy();
            ops.verify(game, &vo_lang, &path_str, tx).await?;
            println!("Verification complete.");
        }
        Commands::Launch { game } => {
            let gc = config.game_config(game).clone();
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let data_dir = dirs::data_dir()
                .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
                .join("elysiae-tui");

            let launcher = Launcher::new(data_dir);
            let (log_tx, _log_rx) = tokio::sync::mpsc::channel(16);
            launcher.launch(game, install_path, log_tx)?;
        }
        Commands::CheckUpdate { game, lang } => {
            let gc = config.game_config(game).clone();
            let vo_lang = match lang {
                Some(l) => {
                    crate::config::validate_vo_lang(&l).map_err(|e| e.to_string())?;
                    l
                }
                None => gc.primary_vo_lang().to_owned(),
            };
            let install_path = gc.install_path.as_ref().ok_or("no install path configured for this game")?;

            let ops = Operations::new(client.clone(), crate::config::app_data_dir());

            let path_str = install_path.to_string_lossy();
            let info = ops.check_update(game, &vo_lang, &path_str).await?;

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
        Commands::Resume { game } => {
            let data_dir = crate::config::app_data_dir();
            let state = irmin::load_download_state(&data_dir, game.as_str())
                .ok_or("no interrupted download found for this game")?;
            println!(
                "resuming {:?} for {} from {}",
                state.download_type,
                game.as_str(),
                state.output_path
            );

            let ops = Operations::new(client.clone(), data_dir);
            let handle = irmin::DownloadHandle::new();
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            let output = state.output_path.clone();

            let result = tokio::select! {
                r = async {
                    match state.download_type {
                        irmin::DownloadType::Fresh => ops.download(game, &state.vo_lang, &output, &handle, tx).await,
                        irmin::DownloadType::Update => ops.update(game, &state.vo_lang, &output, &handle, tx).await,
                        irmin::DownloadType::Preinstall => ops.preinstall(game, &state.vo_lang, &output, &handle, tx).await,
                    }
                } => r,
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\ninterrupted, progress saved");
                    return Ok(());
                }
            };

            // Drain remaining progress
            while let Ok(p) = rx.try_recv() {
                print_progress(&p);
            }
            result.map_err(|e| e.to_string())?;
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
            let pct = if *total_bytes > 0 { (*downloaded_bytes as f64 / *total_bytes as f64) * 100.0 } else { 0.0 };
            let speed_mb = *speed_bps / 1_000_000.0;
            println!("Downloading: {pct:.1}% ({speed_mb:.1} MB/s)");
        }
        SophonProgress::Paused { downloaded_bytes, total_bytes } => {
            let pct = if *total_bytes > 0 { (*downloaded_bytes as f64 / *total_bytes as f64) * 100.0 } else { 0.0 };
            println!("Paused: {pct:.1}%");
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
            let pct = if *total_bytes > 0 { (*downloaded_bytes as f64 / *total_bytes as f64) * 100.0 } else { 0.0 };
            println!("Downloading plugin {name}: {pct:.1}%");
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
