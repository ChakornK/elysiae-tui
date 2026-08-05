use std::io;

use crossterm::event::KeyCode;
use irmin::SophonProgress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::Sender;

use crate::app::{App, View};

use super::actions;

/// Dispatches a keypress to the handler for the current view.
pub async fn handle_key(
    app: &mut App,
    key: KeyCode,
    client: &reqwest::Client,
    progress_tx: &Sender<SophonProgress>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    match app.current_view {
        View::GameList => handle_game_list(app, key, client).await,
        View::GameDetail => handle_game_detail(app, key, client, progress_tx, terminal).await,
        View::Downloading => handle_downloading(app, key),
        View::Settings => handle_settings(app, key),
    }
}

async fn handle_game_list(
    app: &mut App,
    key: KeyCode,
    client: &reqwest::Client,
) -> Result<(), Box<dyn std::error::Error>> {
    match key {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.prev_game(),
        KeyCode::Down | KeyCode::Char('j') => app.next_game(),
        KeyCode::Enter => {
            app.enter_game_detail();
            actions::refresh_update_info(app, client).await;
        }
        KeyCode::Char('s') => app.current_view = View::Settings,
        _ => {}
    }
    Ok(())
}

async fn handle_game_detail(
    app: &mut App,
    key: KeyCode,
    client: &reqwest::Client,
    progress_tx: &Sender<SophonProgress>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    match key {
        KeyCode::Esc => app.back(),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('d') => actions::start_download(app, client, progress_tx),
        KeyCode::Char('u') => actions::start_update(app, client, progress_tx),
        KeyCode::Char('l') => actions::launch_game(app, terminal)?,
        KeyCode::Char('v') => actions::start_verify(app, client, progress_tx),
        KeyCode::Char('p') => actions::start_preinstall(app, client, progress_tx),
        _ => {}
    }
    Ok(())
}

fn handle_downloading(app: &mut App, key: KeyCode) -> Result<(), Box<dyn std::error::Error>> {
    match key {
        KeyCode::Char('p') => {
            if let Some(ref mut dl) = app.download {
                dl.handle.pause();
                dl.paused = true;
            }
        }
        KeyCode::Char('r') => {
            if let Some(ref mut dl) = app.download {
                dl.handle.resume();
                dl.paused = false;
            }
        }
        KeyCode::Char('c') => app.finish_download(),
        _ => {}
    }
    Ok(())
}

fn handle_settings(app: &mut App, key: KeyCode) -> Result<(), Box<dyn std::error::Error>> {
    match key {
        KeyCode::Esc => app.back(),
        _ => {}
    }
    Ok(())
}
