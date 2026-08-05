mod actions;
mod input;

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use irmin::SophonProgress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::app::App;
use crate::config::Config;
use crate::game::GameId;
use crate::ui;

/// Runs the interactive TUI event loop.
pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config);
    let client = reqwest::Client::new();
    let (progress_tx, mut progress_rx) = mpsc::channel::<SophonProgress>(128);

    // Load installed tags for all configured games
    for game in GameId::ALL {
        let gc = app.config.game_config(game).clone();
        if let Some(ref path) = gc.install_path {
            let tag = irmin::game_installer::read_installed_tag(path);
            app.games.insert(game, crate::app::GameStatus {
                installed_tag: tag,
                update_info: None,
            });
        }
    }

    // Check for interrupted downloads
    check_resume_state(&mut app);

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Dismiss error/status on any keypress when shown
                if app.error_message.is_some() {
                    app.dismiss_error();
                    continue;
                }
                if app.status_message.is_some() {
                    app.dismiss_status();
                    continue;
                }

                // Handle resume prompt
                if app.show_resume_prompt {
                    match key.code {
                        crossterm::event::KeyCode::Char('y') => {
                            app.show_resume_prompt = false;
                            actions::resume_download(&mut app, &client, &progress_tx);
                        }
                        _ => {
                            app.show_resume_prompt = false;
                        }
                    }
                    continue;
                }

                input::handle_key(
                    &mut app,
                    key.code,
                    &client,
                    &progress_tx,
                    &mut terminal,
                )
                .await?;
            }
        }

        while let Ok(progress) = progress_rx.try_recv() {
            app.update_progress(progress);
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    app.config.selected_game = app.active_game;
    let _ = app.config.save();

    Ok(())
}

/// Checks all game directories for resume state files.
fn check_resume_state(app: &mut App) {
    for game in GameId::ALL {
        let gc = app.config.game_config(game).clone();
        if let Some(ref path) = gc.install_path {
            let path_str = path.to_string_lossy();
            if irmin::sophon_has_resume_state(&path_str) {
                app.show_resume_prompt = true;
                app.active_game = game;
                app.status_message = Some(format!(
                    "Interrupted download found for {}. Resume? (y/n)",
                    game.display_name()
                ));
                return;
            }
        }
    }
}
