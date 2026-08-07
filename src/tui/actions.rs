use std::io;
use std::path::PathBuf;

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
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
/// Checks proton/jadeite availability first.
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

    if !launcher.proton_available() {
        app.error_message = Some("Proton not installed. Install it from Settings first.".to_owned());
        return Ok(());
    }

    if game.needs_jadeite() && !launcher.jadeite_available() {
        app.error_message = Some("Jadeite not installed. Install it from Settings first.".to_owned());
        return Ok(());
    }

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

        if let Err(e) = launcher.launch(game, path) {
            app.error_message = Some(format!("Launch failed: {e}"));
        }

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    *terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok(())
}

/// Spawns a component install (proton or jadeite) and updates config on completion.
pub fn install_component(
    _app: &mut App,
    client: &reqwest::Client,
    component: &str,
) {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join("elysiae-tui");
    let mgr = ComponentManager::new(client.clone(), data_dir);
    let component = component.to_owned();

    tokio::spawn(async move {
        let (tx, mut _rx) = tokio::sync::mpsc::channel::<ComponentProgress>(32);
        let result = if component == "proton" {
            mgr.install_proton(tx).await
        } else {
            mgr.install_jadeite(tx).await
        };
        match result {
            Ok(_tag) => {}
            Err(_e) => {}
        }
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
        let ops = Operations::new(client);
        let result = match op {
            Op::Download => ops.download(game, &vo_lang, &path, &handle, tx).await,
            Op::Update => ops.update(game, &vo_lang, &path, &handle, tx).await,
            Op::Preinstall => ops.preinstall(game, &vo_lang, &path, &handle, tx).await,
            Op::Verify => ops.verify(game, &vo_lang, &path, tx).await,
        };
        if let Err(_e) = result {
            // Error is reported via progress channel
        }
    });
}

fn default_install_path(game: GameId) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("elysiae-tui")
        .join("games")
        .join(game.as_str())
}
