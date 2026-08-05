use std::io;

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use irmin::{DownloadHandle, SophonProgress};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::Sender;

use crate::app::App;
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
    if let Some(ref path) = gc.install_path {
        let handle = DownloadHandle::new();
        app.start_download(game, handle.clone());
        spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Download);
    }
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

/// Leaves the TUI, launches the game, then re-enters the TUI.
pub fn launch_game(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    let Some(ref path) = gc.install_path else { return Ok(()) };

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join("elysiae-cli");
    let launcher = crate::launcher::Launcher::new(data_dir);
    if let Err(e) = launcher.launch(game, path) {
        eprintln!("launch failed: {e}");
    }

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    *terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok(())
}

enum Op {
    Download,
    Update,
    Preinstall,
    Verify,
}

fn spawn_operation(
    client: &reqwest::Client,
    game: crate::game::GameId,
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
        if let Err(e) = result {
            eprintln!("operation failed: {e}");
        }
    });
}
