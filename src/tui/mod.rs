mod actions;
mod input;

use std::collections::HashMap;
use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use irmin::SophonProgress;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::app::App;
use crate::backgrounds::Backgrounds;
use crate::config::Config;
use crate::game::GameId;
use crate::quadrant::QuadrantImage;
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
    let mut term_size = crossterm::terminal::size().unwrap_or((80, 24));

    // Load installed tags
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

    // Try loading backgrounds from quadrant cache (instant, <1ms)
    load_from_cache(&mut app, term_size);

    // Spawn background sync + encode for any missing games
    let cache_dir = bg_cache_dir();
    let qcache_dir = quadrant_cache_dir();
    let (bg_tx, mut bg_rx) = oneshot::channel::<HashMap<GameId, QuadrantImage>>();
    let bg_client = client.clone();
    let bg_term_size = term_size;
    tokio::spawn(async move {
        let mut backgrounds = Backgrounds::new(cache_dir);
        backgrounds.sync(&bg_client).await;
        let encoded = tokio::task::spawn_blocking(move || {
            encode_missing(&backgrounds, bg_term_size, &qcache_dir)
        }).await.unwrap_or_default();
        let _ = bg_tx.send(encoded);
    });

    check_resume_state(&mut app);

    // Main event loop — TUI is interactive immediately
    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Receive lazily-encoded backgrounds when ready
        if let Ok(new_bgs) = bg_rx.try_recv() {
            for (game, img) in new_bgs {
                app.backgrounds.entry(game).or_insert(img);
            }
            // Replace with a dummy channel
            let (_tx, rx) = oneshot::channel();
            bg_rx = rx;
            drop(_tx);
        }

        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if app.error_message.is_some() {
                    app.dismiss_error();
                    continue;
                }
                if app.status_message.is_some() {
                    app.dismiss_status();
                    continue;
                }

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

        // Re-encode on terminal resize
        if let Ok(new_size) = crossterm::terminal::size() {
            if new_size != term_size {
                term_size = new_size;
                app.backgrounds.clear();
                load_from_cache(&mut app, term_size);
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

/// Loads pre-encoded quadrant caches for the current terminal size.
fn load_from_cache(app: &mut App, term_size: (u16, u16)) {
    let dir = quadrant_cache_dir();
    let (cols, rows) = term_size;
    for game in GameId::ALL {
        if app.backgrounds.contains_key(&game) {
            continue;
        }
        let path = dir.join(format!("{}_{cols}x{rows}.qcache", game.as_str()));
        if let Ok(mut img) = QuadrantImage::read_cache(&path) {
            img.darken();
            app.backgrounds.insert(game, img);
        }
    }
}

/// Encodes backgrounds that aren't already cached, saves to disk.
fn encode_missing(
    backgrounds: &Backgrounds,
    term_size: (u16, u16),
    qcache_dir: &std::path::Path,
) -> HashMap<GameId, QuadrantImage> {
    let _ = std::fs::create_dir_all(qcache_dir);
    let (cols, rows) = term_size;
    let mut map = HashMap::new();

    for game in GameId::ALL {
        let cache_path = qcache_dir.join(format!("{}_{cols}x{rows}.qcache", game.as_str()));
        // Skip if cache already exists (load_from_cache handled it)
        if cache_path.exists() {
            continue;
        }

        let Some(path) = backgrounds.get(game) else { continue };

        // Prefer thumbnail if available
        let thumb_path = path.with_file_name("bg_thumb.png");
        let load_path = if thumb_path.exists() { &thumb_path } else { path };

        let Ok(reader) = image::ImageReader::open(load_path) else { continue };
        let Ok(img) = reader.decode() else { continue };

        // Resize source to exactly fit the quadrant pixel grid.
        // No aspect correction — source images are landscape (16:9) and terminals
        // are visually similar. resize_exact avoids cropping/zooming artifacts.
        let grid_w = (cols as u32) * 2;
        let grid_h = (rows as u32) * 2;
        let resized = img.resize_exact(grid_w, grid_h, image::imageops::FilterType::Lanczos3);
        let rgb = resized.to_rgb8();

        let mut encoded = QuadrantImage::encode(&rgb, cols, rows);
        let _ = encoded.write_cache(&cache_path);
        encoded.darken();
        map.insert(game, encoded);
    }

    // Also generate thumbnails for future fast loads
    for game in GameId::ALL {
        let Some(path) = backgrounds.get(game) else { continue };
        let thumb_path = path.with_file_name("bg_thumb.png");
        if thumb_path.exists() {
            continue;
        }
        let Ok(reader) = image::ImageReader::open(path) else { continue };
        let Ok(img) = reader.decode() else { continue };
        let thumb = img.resize_to_fill(480, 270, image::imageops::FilterType::Lanczos3);
        let _ = thumb.save(&thumb_path);
    }

    map
}

fn bg_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.cache"))
        .join("elysiae-tui")
        .join("backgrounds")
}

fn quadrant_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.cache"))
        .join("elysiae-tui")
        .join("quadrant")
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
