use std::io;

use crossterm::event::KeyCode;
use irmin::SophonProgress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::Sender;

use crate::app::{App, View};
use crate::transition::BgTransition;

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
        View::GameList => {
            handle_main(app, key, client, progress_tx, terminal).await
        }
        View::Settings => handle_settings(app, key, client, progress_tx),
    }
}

/// Unified handler for the main view (tabs + game detail panel).
async fn handle_main(
    app: &mut App,
    key: KeyCode,
    client: &reqwest::Client,
    progress_tx: &Sender<SophonProgress>,
    _terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
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
                app.dialog = Some(crate::app::ConfirmDialog::cancel_download());
                return Ok(());
            }
            _ => {}
        }
    }

    match key {
        KeyCode::Char('q') => app.should_quit = true,
        // Scroll launch log
        KeyCode::Up if app.game_running && !app.launch_log.is_empty() => {
            app.launch_log_scroll = app.launch_log_scroll.saturating_sub(1);
        }
        KeyCode::Down if app.game_running && !app.launch_log.is_empty() => {
            let max_scroll = app.launch_log.len().saturating_sub(8);
            if app.launch_log_scroll < max_scroll {
                app.launch_log_scroll += 1;
            }
        }
        KeyCode::Left => {
            let old = app.selected_game();
            app.prev_game();
            app.active_game = app.selected_game();
            start_bg_transition(app, old);
        }
        KeyCode::Right => {
            let old = app.selected_game();
            app.next_game();
            app.active_game = app.selected_game();
            start_bg_transition(app, old);
        }
        KeyCode::Tab => {
            let old = app.selected_game();
            app.next_game();
            app.active_game = app.selected_game();
            start_bg_transition(app, old);
        }
        KeyCode::BackTab => {
            let old = app.selected_game();
            app.prev_game();
            app.active_game = app.selected_game();
            start_bg_transition(app, old);
        }
        KeyCode::Char(n @ '1'..='4') => {
            let idx = (n as usize) - ('1' as usize);
            if idx < crate::game::GameId::ALL.len() {
                let old = app.selected_game();
                app.game_list_index = idx;
                app.active_game = app.selected_game();
                start_bg_transition(app, old);
            }
        }
        KeyCode::Enter => {
            let game = app.selected_game();
            let status = app.games.get(&game);
            let installed = status.and_then(|s| s.installed_tag.as_ref()).is_some();
            let has_update = status
                .and_then(|s| s.update_info.as_ref())
                .is_some_and(|i| i.update_available);
            let has_resume = status.is_some_and(|s| s.has_resume);

            if app.download.is_none() {
                if has_resume {
                    actions::start_resume(app, client, progress_tx);
                } else if has_update {
                    actions::start_update(app, client, progress_tx);
                } else if installed {
                    if !(app.game_running && app.launch_log_game == Some(game)) {
                        actions::prepare_and_launch(app, client, progress_tx);
                    }
                } else {
                    actions::start_download(app, client, progress_tx);
                }
            }
        }
        // Verify: only when installed and no download active
        KeyCode::Char('v') => {
            let game = app.selected_game();
            let installed = app.games.get(&game)
                .and_then(|s| s.installed_tag.as_ref())
                .is_some();
            if installed && app.download.is_none() {
                actions::start_verify(app, client, progress_tx);
            }
        }
        // Preinstall: only when no download active and preinstall available
        KeyCode::Char('p') => {
            let game = app.selected_game();
            let has_preinstall = app.games.get(&game)
                .and_then(|s| s.update_info.as_ref())
                .is_some_and(|i| i.preinstall_available && !i.preinstall_downloaded);
            if app.download.is_none() && has_preinstall {
                actions::start_preinstall(app, client, progress_tx);
            }
        }
        // Apply preinstall: when preinstall is downloaded and update is available
        KeyCode::Char('a') => {
            let game = app.selected_game();
            let can_apply = app.games.get(&game)
                .and_then(|s| s.update_info.as_ref())
                .is_some_and(|i| i.update_available && i.preinstall_downloaded);
            if app.download.is_none() && can_apply {
                actions::apply_preinstall(app, client, progress_tx);
            }
        }
        KeyCode::Char('s') => {
            app.current_view = View::Settings;
            // Initialize cursor to first selectable item
            let items = crate::ui::build_settings_items_pub(app);
            app.settings.cursor = items.iter().position(|(s, _)| *s).unwrap_or(0);
            app.settings.item_count = items.len();
        }
        KeyCode::Char('?') => {
            app.show_help = true;
        }
        _ => {}
    }
    Ok(())
}

fn handle_settings(app: &mut App, key: KeyCode, _client: &reqwest::Client, _progress_tx: &Sender<SophonProgress>) -> Result<(), Box<dyn std::error::Error>> {
    match key {
        KeyCode::Esc => app.current_view = View::GameList,
        KeyCode::Up => settings_move_cursor(app, -1),
        KeyCode::Down => settings_move_cursor(app, 1),
        KeyCode::Enter => settings_activate(app),
        _ => {}
    }
    Ok(())
}

fn settings_move_cursor(app: &mut App, direction: i32) {
    let items = crate::ui::build_settings_items_pub(app);
    if items.is_empty() { return; }
    let len = items.len();
    let mut pos = app.settings.cursor as i32;
    loop {
        pos += direction;
        if pos < 0 { pos = len as i32 - 1; }
        if pos >= len as i32 { pos = 0; }
        if items[pos as usize].0 { break; }
        // Safety: at least one selectable item always exists (components)
    }
    app.settings.cursor = pos as usize;
}

fn settings_activate(app: &mut App) {
    let items = crate::ui::build_settings_items_pub(app);
    let Some((_selectable, kind)) = items.get(app.settings.cursor) else { return };
    match kind {
        crate::ui::SettingsAction::ManageVos(game) => {
            let game = *game;
            let gc = app.config.game_config(game);
            app.vo_modal = Some(crate::app::VoManagerModal::new(game, &gc.vo_langs));
        }
        crate::ui::SettingsAction::UninstallGame(game) => {
            let game = *game;
            app.dialog = Some(crate::app::ConfirmDialog::uninstall_game(&game.display_name(), game));
        }
        crate::ui::SettingsAction::UninstallComponent(name) => {
            app.dialog = Some(crate::app::ConfirmDialog::uninstall_component(name));
        }
        crate::ui::SettingsAction::None => {}
    }
}

/// Starts a background transition if old and new games differ and both have loaded backgrounds.
fn start_bg_transition(app: &mut App, old_game: crate::game::GameId) {
    let new_game = app.selected_game();
    if old_game != new_game
        && app.backgrounds.contains_key(&old_game)
        && app.backgrounds.contains_key(&new_game)
    {
        app.bg_transition = Some(BgTransition::new(old_game));
    }
}
