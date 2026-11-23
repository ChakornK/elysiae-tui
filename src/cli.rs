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

/// Dispatches a parsed CLI command.
pub async fn run_cli(cmd: Commands, config: &mut Config) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Commands::Download { game, lang, path } => run_download(config, game, lang, path).await,
        Commands::Update { game, lang } => run_update(config, game, lang).await,
        Commands::Preinstall { game, lang } => run_preinstall(config, game, lang).await,
        Commands::ApplyPreinstall { game, lang } => run_apply_preinstall(config, game, lang).await,
        Commands::Verify { game, lang } => run_verify(config, game, lang).await,
        Commands::Launch { game } => run_launch(config, game),
        Commands::CheckUpdate { game, lang } => run_check_update(config, game, lang).await,
        Commands::Resume { game } => run_resume(config, game).await,
    }
}

/// Creates an Operations bound to the shared client and data dir.
fn new_operations() -> Operations {
    Operations::new(crate::http::build_client(), crate::config::app_data_dir())
}

/// Resolves the VO language (validating an explicit `--lang`) and the
/// install path for a command targeting an installed game.
fn resolve_target(
    config: &mut Config,
    game: GameId,
    lang: Option<String>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let gc = config.game_config(game).clone();
    let vo_lang = match lang {
        Some(l) => {
            crate::config::validate_vo_lang(&l).map_err(|e| e.to_string())?;
            l
        }
        None => gc.primary_vo_lang().to_owned(),
    };
    let path = gc
        .install_path
        .as_ref()
        .ok_or("no install path configured for this game")?
        .to_string_lossy()
        .to_string();
    Ok((vo_lang, path))
}

/// Spawns a task that prints progress events as they arrive.
fn spawn_progress_printer() -> mpsc::Sender<SophonProgress> {
    let (tx, mut rx) = mpsc::channel::<SophonProgress>(64);
    tokio::spawn(async move {
        while let Some(p) = rx.recv().await {
            print_progress(&p);
        }
    });
    tx
}

async fn run_download(
    config: &mut Config,
    game: GameId,
    lang: String,
    path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let ops = new_operations();
    let handle = DownloadHandle::new();
    let tx = spawn_progress_printer();
    let path_str = path.to_string_lossy().to_string();

    let result = tokio::select! {
        r = ops.download(game, &lang, &path_str, &handle, tx) => r,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\ninterrupted, exiting...");
            return Ok(());
        }
    };
    result?;
    // Persist the install path for future operations
    let gc = config.game_config(game);
    gc.install_path = Some(path);
    gc.vo_langs = vec![lang];
    config.save()?;
    println!("Download complete.");
    Ok(())
}

async fn run_update(
    config: &mut Config,
    game: GameId,
    lang: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (vo_lang, path) = resolve_target(config, game, lang)?;
    let ops = new_operations();
    let handle = DownloadHandle::new();
    let tx = spawn_progress_printer();

    let result = tokio::select! {
        r = ops.update(game, &vo_lang, &path, &handle, tx) => r,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\ninterrupted, exiting...");
            return Ok(());
        }
    };
    result?;
    println!("Update complete.");
    Ok(())
}

async fn run_preinstall(
    config: &mut Config,
    game: GameId,
    lang: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (vo_lang, path) = resolve_target(config, game, lang)?;
    let ops = new_operations();
    let handle = DownloadHandle::new();
    let tx = spawn_progress_printer();

    let result = tokio::select! {
        r = ops.preinstall(game, &vo_lang, &path, &handle, tx) => r,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\ninterrupted, exiting...");
            return Ok(());
        }
    };
    result?;
    println!("Preinstall download complete.");
    Ok(())
}

async fn run_apply_preinstall(
    config: &mut Config,
    game: GameId,
    lang: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (vo_lang, path) = resolve_target(config, game, lang)?;
    let ops = new_operations();
    let handle = DownloadHandle::new();
    let tx = spawn_progress_printer();

    // check_update gives us the preinstall_tag needed for apply
    let info = ops.check_update(game, &vo_lang, &path).await?;
    let tag = info.preinstall_tag.ok_or("no preinstall tag available")?;
    irmin::game_installer::validate_asset_name(&tag)
        .map_err(|e| format!("invalid preinstall tag: {e}"))?;
    ops.apply_preinstall(game, &vo_lang, &tag, &path, &handle, tx).await?;
    println!("Preinstall applied.");
    Ok(())
}

async fn run_verify(
    config: &mut Config,
    game: GameId,
    lang: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (vo_lang, path) = resolve_target(config, game, lang)?;
    let ops = new_operations();
    let tx = spawn_progress_printer();
    ops.verify(game, &vo_lang, &path, tx).await?;
    println!("Verification complete.");
    Ok(())
}

fn run_launch(config: &mut Config, game: GameId) -> Result<(), Box<dyn std::error::Error>> {
    let install_path = config
        .game_config(game)
        .install_path
        .as_ref()
        .ok_or("no install path configured for this game")?;

    let launcher = Launcher::new(crate::config::app_data_dir());
    let (log_tx, _log_rx) = tokio::sync::mpsc::channel(16);
    launcher.launch(game, install_path, log_tx)?;
    Ok(())
}

async fn run_check_update(
    config: &mut Config,
    game: GameId,
    lang: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (vo_lang, path) = resolve_target(config, game, lang)?;
    let ops = new_operations();
    let info = ops.check_update(game, &vo_lang, &path).await?;

    if info.update_available {
        println!(
            "Update available: {} -> {}",
            info.current_tag.as_deref().unwrap_or("unknown"),
            info.remote_tag
        );
    } else {
        println!("Already up to date ({})", info.remote_tag);
    }
    if info.preinstall_available {
        let tag = info.preinstall_tag.as_deref().unwrap_or("unknown");
        println!("Preinstall available: {tag}");
    }
    Ok(())
}

async fn run_resume(config: &mut Config, game: GameId) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = crate::config::app_data_dir();
    let state = irmin::load_download_state(&data_dir, game.as_str())
        .ok_or("no interrupted download found for this game")?;
    println!(
        "resuming {:?} for {} from {}",
        state.download_type,
        game.as_str(),
        state.output_path
    );

    // Sync the configured install path with the resume path so the TUI
    // later detects and dispatches to the same place.
    let p = PathBuf::from(&state.output_path);
    let gc = config.game_config(game);
    if gc.install_path.as_deref() != Some(p.as_path()) {
        gc.install_path = Some(p);
        config.save()?;
    }

    let ops = Operations::new(crate::http::build_client(), data_dir);
    let handle = DownloadHandle::new();
    let tx = spawn_progress_printer();
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
    result.map_err(|e| e.to_string())?;
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
