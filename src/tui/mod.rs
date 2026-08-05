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

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
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
