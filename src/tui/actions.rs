use std::io;
use std::path::PathBuf;

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use irmin::{DownloadHandle, SophonProgress};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::Sender;

use crate::app::App;
use crate::components::{ComponentManager, ComponentProgress};
use crate::game::GameId;
use crate::operations::Operations;

/// Fetches update info for the active game and stores it in app state.
pub async fn refresh_update_info(app: &mut App, client: &reqwest::Client) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    if let Some(ref path) = gc.install_path {
        let ops = Operations::new(client.clone());
        let path_str = path.to_string_lossy().to_string();
        if let Ok(info) = ops.check_update(game, &gc.vo_lang, &path_str).await {
            if let Some(gs) = app.games.get_mut(&game) {
                gs.update_info = Some(info);
            }
        }
    }
}

/// Spawns a fresh download task for the active game.
pub fn start_download(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    let path = match gc.install_path {
        Some(p) => p,
        None => {
            // Auto-assign a default install path
            let default = default_install_path(game);
            app.config.game_config(game).install_path = Some(default.clone());
            let _ = app.config.save();
            default
        }
    };
    let handle = DownloadHandle::new();
    app.start_download(game, handle.clone());
    spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Download);
}

/// Spawns an update task for the active game.
pub fn start_update(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    if let Some(ref path) = gc.install_path {
        let handle = DownloadHandle::new();
        app.start_download(game, handle.clone());
        spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Update);
    }
}

/// Spawns a preinstall download task for the active game.
pub fn start_preinstall(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    if let Some(ref path) = gc.install_path {
        let handle = DownloadHandle::new();
        app.start_download(game, handle.clone());
        spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Preinstall);
    }
}

/// Spawns a verify integrity task for the active game.
pub fn start_verify(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    if let Some(ref path) = gc.install_path {
        let handle = DownloadHandle::new();
        app.start_download(game, handle.clone());
        spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Verify);
    }
}

/// Resumes an interrupted download using irmin's saved state.
pub fn resume_download(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    if let Some(ref path) = gc.install_path {
        let handle = DownloadHandle::new();
        app.start_download(game, handle.clone());
        // Resume uses the same download function; irmin detects the state file automatically
        spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Download);
    }
}

/// Leaves the TUI, launches the game, then re-enters the TUI.
pub fn launch_game(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    let Some(ref path) = gc.install_path else {
        app.error_message = Some("No install path configured for this game.".to_owned());
        return Ok(());
    };

    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join("elysiae-tui");
    let launcher = crate::launcher::Launcher::new(data_dir);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    if let Err(e) = launcher.launch(game, path) {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        *terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        app.error_message = Some(format!("Launch failed: {e}"));
        return Ok(());
    }

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    *terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok(())
}

/// Installs missing components then launches the game.
/// If components need installing, shows progress overlay first.
pub fn prepare_and_launch(
    app: &mut App,
    client: &reqwest::Client,
    progress_tx: &Sender<SophonProgress>,
) {
    let game = app.active_game;
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join("elysiae-tui");

    use crate::components::{proton_available, jadeite_available};
    let needs_proton = !proton_available(&data_dir);
    let needs_jadeite = game.needs_jadeite() && !jadeite_available(&data_dir);

    if !needs_proton && !needs_jadeite {
        // Components ready — mark as ready to launch (handled by caller)
        app.ready_to_launch = true;
        return;
    }

    // Components missing — install them with progress, then signal ready to launch
    let handle = DownloadHandle::new();
    app.start_download(game, handle.clone());
    if let Some(ref mut dl) = app.download {
        dl.launch_on_complete = true;
    }
    let tx = progress_tx.clone();
    let client = client.clone();

    tokio::spawn(async move {
        if let Err(msg) = ensure_components(&client, &data_dir, game, &tx).await {
            let _ = tx.send(SophonProgress::Error { message: msg }).await;
            return;
        }
        let _ = tx.send(SophonProgress::Finished).await;
    });
}

enum Op {
    Download,
    Update,
    Preinstall,
    Verify,
}

fn spawn_operation(
    client: &reqwest::Client,
    game: GameId,
    vo_lang: String,
    path: String,
    handle: DownloadHandle,
    tx: Sender<SophonProgress>,
    op: Op,
) {
    let client = client.clone();
    tokio::spawn(async move {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join("elysiae-tui");

        // Auto-install components before any download operation
        if matches!(op, Op::Download | Op::Update | Op::Preinstall) {
            if let Err(msg) = ensure_components(&client, &data_dir, game, &tx).await {
                let _ = tx.send(SophonProgress::Error { message: msg }).await;
                return;
            }
        }

        let ops = Operations::new(client);
        let result = match op {
            Op::Download => ops.download(game, &vo_lang, &path, &handle, tx.clone()).await,
            Op::Update => ops.update(game, &vo_lang, &path, &handle, tx.clone()).await,
            Op::Preinstall => ops.preinstall(game, &vo_lang, &path, &handle, tx.clone()).await,
            Op::Verify => ops.verify(game, &vo_lang, &path, tx.clone()).await,
        };
        if let Err(e) = result {
            let _ = tx.send(SophonProgress::Error { message: e.to_string() }).await;
        }
    });
}

/// Installs Proton (and Jadeite for HKRPG) if not already present.
/// Reports progress via SophonProgress events through the same channel.
async fn ensure_components(
    client: &reqwest::Client,
    data_dir: &std::path::Path,
    game: GameId,
    tx: &Sender<SophonProgress>,
) -> Result<(), String> {
    use crate::components::{proton_available, jadeite_available};

    if !proton_available(data_dir) {
        install_component_with_progress(client, data_dir, "proton", "Installing Proton", tx)
            .await?;
    }

    if game.needs_jadeite() && !jadeite_available(data_dir) {
        install_component_with_progress(client, data_dir, "jadeite", "Installing Jadeite", tx)
            .await?;
    }

    Ok(())
}

/// Installs a single component, bridging ComponentProgress to SophonProgress.
async fn install_component_with_progress(
    client: &reqwest::Client,
    data_dir: &std::path::Path,
    component: &str,
    status_msg: &str,
    tx: &Sender<SophonProgress>,
) -> Result<(), String> {
    let _ = tx
        .send(SophonProgress::FetchingManifest)
        .await;

    let mgr = ComponentManager::new(client.clone(), data_dir.to_path_buf());
    let (comp_tx, mut comp_rx) = tokio::sync::mpsc::channel::<ComponentProgress>(64);

    let comp_name = component.to_owned();
    let install_handle = tokio::spawn(async move {
        if comp_name == "proton" {
            mgr.install_proton(comp_tx).await
        } else {
            mgr.install_jadeite(comp_tx).await
        }
    });

    // Bridge component progress to sophon progress for the overlay
    let status = status_msg.to_owned();
    while let Some(prog) = comp_rx.recv().await {
        match prog {
            ComponentProgress::Downloading {
                downloaded_bytes,
                total_bytes,
            } => {
                let _ = tx
                    .send(SophonProgress::Downloading {
                        downloaded_bytes,
                        total_bytes,
                        speed_bps: 0.0,
                        eta_seconds: 0.0,
                    })
                    .await;
            }
            ComponentProgress::Extracting => {
                let _ = tx
                    .send(SophonProgress::Downloading {
                        downloaded_bytes: 0,
                        total_bytes: 0,
                        speed_bps: 0.0,
                        eta_seconds: 0.0,
                    })
                    .await;
            }
            ComponentProgress::Finished { .. } => {}
            ComponentProgress::Error { message } => {
                return Err(format!("{}: {}", status, message));
            }
        }
    }

    match install_handle.await {
        Ok(Ok(_tag)) => Ok(()),
        Ok(Err(e)) => Err(format!("{}: {}", status_msg, e)),
        Err(e) => Err(format!("{}: task panicked: {}", status_msg, e)),
    }
}

fn default_install_path(game: GameId) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("elysiae-tui")
        .join("games")
        .join(game.as_str())
}
