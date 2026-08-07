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
        View::GameList | View::GameDetail | View::Downloading => {
            handle_main(app, key, client, progress_tx, terminal).await
        }
        View::Settings => handle_settings(app, key, client),
    }
}

/// Unified handler for the main view (tabs + game detail panel).
async fn handle_main(
    app: &mut App,
    key: KeyCode,
    client: &reqwest::Client,
    progress_tx: &Sender<SophonProgress>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Download controls take priority when active
    if app.download.is_some() {
        match key {
            KeyCode::Char('p') => {
                if let Some(ref mut dl) = app.download {
                    if dl.paused {
                        dl.handle.resume();
                        dl.paused = false;
                    } else {
                        dl.handle.pause();
                        dl.paused = true;
                    }
                }
                return Ok(());
            }
            KeyCode::Char('c') => {
                app.finish_download();
                return Ok(());
            }
            _ => {}
        }
    }

    match key {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Left => {
            app.prev_game();
            app.active_game = app.selected_game();
        }
        KeyCode::Right => {
            app.next_game();
            app.active_game = app.selected_game();
        }
        KeyCode::Tab => {
            app.next_game();
            app.active_game = app.selected_game();
        }
        KeyCode::BackTab => {
            app.prev_game();
            app.active_game = app.selected_game();
        }
        // Number keys to switch tabs directly
        KeyCode::Char(n @ '1'..='4') => {
            let idx = (n as usize) - ('1' as usize);
            if idx < crate::game::GameId::ALL.len() {
                app.game_list_index = idx;
                app.active_game = app.selected_game();
            }
        }
        // Primary action (Enter)
        KeyCode::Enter => {
            let game = app.selected_game();
            let status = app.games.get(&game);
            let installed = status.and_then(|s| s.installed_tag.as_ref()).is_some();
            let has_update = status
                .and_then(|s| s.update_info.as_ref())
                .is_some_and(|i| i.update_available);
            let has_resume = status.is_some_and(|s| s.has_resume);

            if has_update {
                actions::start_update(app, client, progress_tx);
            } else if has_resume || !installed {
                actions::start_download(app, client, progress_tx);
            } else {
                actions::launch_game(app, terminal)?;
            }
        }
        KeyCode::Char('v') => actions::start_verify(app, client, progress_tx),
        KeyCode::Char('s') => app.current_view = View::Settings,
        _ => {}
    }
    Ok(())
}

fn handle_settings(app: &mut App, key: KeyCode, client: &reqwest::Client) -> Result<(), Box<dyn std::error::Error>> {
    match key {
        KeyCode::Esc => {
            app.current_view = View::GameList;
        }
        KeyCode::Char('1') => {
            actions::install_component(app, client, "proton");
            app.status_message = Some("Installing Proton...".to_owned());
        }
        KeyCode::Char('2') => {
            actions::install_component(app, client, "jadeite");
            app.status_message = Some("Installing Jadeite...".to_owned());
        }
        _ => {}
    }
    Ok(())
}
