mod actions;
pub mod guard;
mod input;

use std::collections::HashMap;
use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use irmin::SophonProgress;
use irmin::game_installer::UpdateInfo;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use crate::app::App;
use crate::signal;
use crate::backgrounds::Backgrounds;
use crate::config::Config;
use crate::game::GameId;
use crate::operations::Operations;
use crate::quadrant::QuadrantImage;
use crate::ui;

/// Runs the interactive TUI event loop.
pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    guard::install_panic_hook();
    let _guard = guard::TerminalGuard::new()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config);
    let client = crate::http::build_client();
    let shutdown_rx = signal::spawn_signal_handler();
    let (progress_tx, mut progress_rx) = mpsc::channel::<SophonProgress>(128);
    let (log_tx, mut log_rx) = mpsc::channel::<String>(256);
    let mut term_size = crossterm::terminal::size().unwrap_or((80, 24));

    // Load installed tags and resume state
    let data_dir_for_state = crate::config::app_data_dir();
    for game in GameId::ALL {
        let gc = app.config.game_config(game).clone();
        if let Some(ref path) = gc.install_path {
            let tag = irmin::game_installer::read_installed_tag(path);
            let installed = tag.is_some();
            let has_resume = !installed
                && crate::state::has_partial_download(&data_dir_for_state, path, game.as_str());
            app.games.insert(game, crate::app::GameStatus {
                installed_tag: tag,
                update_info: None,
                has_resume,
            });
        }
    }

    // Sync component installation state from disk to config
    {
        use crate::components::{proton_available, jadeite_available, read_component_tag};
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
            .join("elysiae-tui");
        let proton_tag = read_component_tag(&data_dir, "proton")
            .or_else(|| if proton_available(&data_dir) { Some("installed".to_owned()) } else { None });
        let jadeite_tag = read_component_tag(&data_dir, "jadeite")
            .or_else(|| if jadeite_available(&data_dir) { Some("installed".to_owned()) } else { None });
        let cv = &mut app.config.installed_components;
        if cv.proton != proton_tag || cv.jadeite != jadeite_tag {
            cv.proton = proton_tag;
            cv.jadeite = jadeite_tag;
            let _ = app.config.save();
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

    // Spawn background update check for all installed games
    let (update_tx, mut update_rx) = oneshot::channel::<HashMap<GameId, UpdateInfo>>();
    let update_client = client.clone();
    let update_configs: Vec<_> = GameId::ALL
        .iter()
        .filter_map(|&game| {
            let gc = app.config.game_config(game).clone();
            let path = gc.install_path.as_ref()?.to_string_lossy().to_string();
            // Only check games that are actually installed
            if app.games.get(&game).and_then(|gs| gs.installed_tag.as_ref()).is_some() {
                Some((game, gc.primary_vo_lang().to_owned(), path))
            } else {
                None
            }
        })
        .collect();
    tokio::spawn(async move {
        let ops = Operations::new(update_client, crate::config::app_data_dir());
        let mut results = HashMap::new();
        for (game, vo_lang, path) in update_configs {
            if let Ok(info) = ops.check_update(game, &vo_lang, &path).await {
                results.insert(game, info);
            }
        }
        let _ = update_tx.send(results);
    });

    // Main event loop — TUI is interactive immediately
    loop {
        if *shutdown_rx.borrow() {
            if let Some(ref dl) = app.download {
                dl.handle.cancel();
            }
            break;
        }

        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Clear finished background transitions
        if app.bg_transition.as_ref().is_some_and(|t| t.is_done()) {
            app.bg_transition = None;
        }

        // Receive lazily-encoded backgrounds when ready
        if let Ok(new_bgs) = bg_rx.try_recv() {
            for (game, img) in new_bgs {
                // If the updated background is the one currently on screen,
                // crossfade from the old image; otherwise just swap it in.
                if app.selected_game() == game
                    && let Some(old) = app.backgrounds.get(&game)
                {
                    app.bg_transition = Some(crate::transition::BgTransition::new(old.clone()));
                }
                app.backgrounds.insert(game, img);
            }
            // Replace with a dummy channel
            let (_tx, rx) = oneshot::channel();
            bg_rx = rx;
            drop(_tx);
        }

        // Receive background update check results
        if let Ok(updates) = update_rx.try_recv() {
            for (game, info) in &updates {
                if let Some(gs) = app.games.get_mut(game) {
                    gs.update_info = Some(info.clone());
                }
            }
            // Auto-update/preinstall the first eligible game (one at a time)
            if app.download.is_none() {
                for (game, info) in &updates {
                    if info.update_available && app.config.auto_update {
                        let gc = app.config.game_config(*game).clone();
                        if gc.install_path.is_some() {
                            actions::start_update(&mut app, &client, &progress_tx);
                            break;
                        }
                    } else if info.preinstall_available && app.config.auto_preload {
                        let gc = app.config.game_config(*game).clone();
                        if gc.install_path.is_some() {
                            actions::start_preinstall(&mut app, &client, &progress_tx);
                            break;
                        }
                    }
                }
            }
            let (_tx, rx) = oneshot::channel();
            update_rx = rx;
            drop(_tx);
        }

        if event::poll(Duration::from_millis(33))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Ctrl+C always quits
            if key.code == crossterm::event::KeyCode::Char('c')
                && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            {
                break;
            }

            // Download controls (p/c) bypass the modal stack
            if app.download.is_some() && matches!(key.code,
                crossterm::event::KeyCode::Char('p') | crossterm::event::KeyCode::Char('c'))
            {
                input::handle_key(&mut app, key.code, &client, &progress_tx, &mut terminal).await?;
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

            // VO manager modal: handles input before dialog
            if app.vo_modal.is_some() {
                let lang_count = crate::config::VALID_LANGS.len();
                match key.code {
                    crossterm::event::KeyCode::Up => {
                        if let Some(ref mut m) = app.vo_modal {
                            m.cursor = if m.cursor == 0 { lang_count - 1 } else { m.cursor - 1 };
                        }
                    }
                    crossterm::event::KeyCode::Down => {
                        if let Some(ref mut m) = app.vo_modal {
                            m.cursor = if m.cursor >= lang_count - 1 { 0 } else { m.cursor + 1 };
                        }
                    }
                    crossterm::event::KeyCode::Char(' ') => {
                        if let Some(ref mut m) = app.vo_modal {
                            m.toggle_current();
                        }
                    }
                    crossterm::event::KeyCode::Enter => {
                        let modal = app.vo_modal.take().unwrap();
                        let new_langs = modal.selected_langs();
                        let game = modal.game;
                        let old_langs = app.config.game_config(game).vo_langs.clone();
                        app.config.game_config(game).vo_langs = new_langs.clone();
                        let _ = app.config.save();
                        actions::apply_vo_changes(
                            &mut app, &client, &progress_tx, game, &new_langs, &old_langs,
                        );
                    }
                    crossterm::event::KeyCode::Esc => {
                        app.vo_modal = None;
                    }
                    _ => {}
                }
                continue;
            }

            // Confirm dialog: arrow keys move selection, Enter confirms, Esc dismisses
            if app.dialog.is_some() {
                use crate::app::DialogKind;
                match key.code {
                    crossterm::event::KeyCode::Left => {
                        if let Some(ref mut d) = app.dialog { d.select_left(); }
                    }
                    crossterm::event::KeyCode::Right => {
                        if let Some(ref mut d) = app.dialog { d.select_right(); }
                    }
                    crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Enter => {
                        let dialog = app.dialog.take().unwrap();
                        if key.code == crossterm::event::KeyCode::Char('y') || dialog.confirmed() {
                            match dialog.kind {
                                DialogKind::CancelDownload => app.finish_download(),
                                DialogKind::UninstallGame(game) => {
                                    if let Err(e) = actions::uninstall_game(&mut app, game) {
                                        app.error_message = Some(e);
                                    }
                                }
                                DialogKind::UninstallComponent(ref name) => {
                                    if let Err(e) = actions::uninstall_component(&mut app, name) {
                                        app.error_message = Some(e);
                                    }
                                }
                            }
                        }
                    }
                    crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('n') => {
                        app.dialog = None;
                    }
                    _ => {}
                }
                continue;
            }

            // Any key dismisses help overlay
            if app.show_help {
                app.show_help = false;
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

        // Re-encode on terminal resize
        if let Ok(new_size) = crossterm::terminal::size()
            && new_size != term_size
        {
            term_size = new_size;
            app.backgrounds.clear();
            app.bg_transition = None;
            load_from_cache(&mut app, term_size);
        }

        while let Ok(progress) = progress_rx.try_recv() {
            app.update_progress(progress);
        }

        // Receive game launch log lines
        while let Ok(line) = log_rx.try_recv() {
            if line == "\x00__PROCESS_EXIT__" {
                app.game_running = false;
                continue;
            }
            app.launch_log.push_back(line);
            if app.launch_log.len() > 1000 {
                app.launch_log.pop_front();
            }
            // Auto-scroll to bottom
            let visible = 8usize;
            if app.launch_log.len() > visible {
                app.launch_log_scroll = app.launch_log.len() - visible;
            }
        }

        // Launch game when components are ready
        if app.ready_to_launch {
            app.ready_to_launch = false;
            if let Err(e) = actions::launch_game(&mut app, &mut terminal, &log_tx) {
                app.error_message = Some(format!("Launch failed: {e}"));
            }
        }

        if app.should_quit {
            break;
        }
    }

    app.config.selected_game = app.active_game;
    let _ = app.config.save();

    Ok(())
}

/// Loads pre-encoded quadrant caches for the current terminal size.
/// Cache files are keyed by source background, so a stale image from an older
/// source is never loaded — it simply doesn't match the current source name.
fn load_from_cache(app: &mut App, term_size: (u16, u16)) {
    let dir = quadrant_cache_dir();
    let (cols, rows) = term_size;
    for game in GameId::ALL {
        if app.backgrounds.contains_key(&game) {
            continue;
        }
        let Some(src) = read_source_marker(game) else { continue };
        let path = dir.join(format!("{}_{cols}x{rows}_{src}.qcache", game.as_str()));
        if let Ok(mut img) = QuadrantImage::read_cache(&path) {
            img.darken();
            app.backgrounds.insert(game, img);
        }
    }
}

/// Reads the remote filename marker (`bg.src`) for a game's cached background.
fn read_source_marker(game: GameId) -> Option<String> {
    let path = bg_cache_dir().join(game.as_str()).join("bg.src");
    let name = std::fs::read_to_string(path).ok()?;
    let name = name.trim();
    if name.is_empty() { None } else { Some(name.to_owned()) }
}

/// Encodes backgrounds that aren't already cached, saves to disk.
/// Quadrant cache files are versioned by source (`{game}_{w}x{h}_{src}.qcache`),
/// so a fresh remote source always produces a fresh cache and stale ones are
/// left behind (and swept below).
fn encode_missing(
    backgrounds: &Backgrounds,
    term_size: (u16, u16),
    qcache_dir: &std::path::Path,
) -> HashMap<GameId, QuadrantImage> {
    let _ = std::fs::create_dir_all(qcache_dir);
    let (cols, rows) = term_size;
    let mut map = HashMap::new();

    for game in GameId::ALL {
        let Some(path) = backgrounds.get(game) else { continue };
        let Some(src) = backgrounds.current_name(game) else { continue };

        let cache_path = qcache_dir.join(format!("{}_{cols}x{rows}_{src}.qcache", game.as_str()));
        // Source-versioned cache already exists — up to date.
        if cache_path.exists() {
            continue;
        }

        // Prefer thumbnail if available
        let thumb_path = path.with_file_name(format!("bg_thumb_{src}.png"));
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

    // Sweep stale caches: remove any `{game}_*x*.qcache` not produced from the
    // current source. Valid versioned caches at other sizes are kept.
    for game in GameId::ALL {
        let Some(src) = backgrounds.current_name(game) else { continue };
        let prefix = format!("{}_", game.as_str());
        let suffix = format!("_{src}.qcache");
        if let Ok(entries) = std::fs::read_dir(qcache_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&prefix)
                    && name.ends_with(".qcache")
                    && !name.ends_with(&suffix)
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    // Also generate thumbnails for future fast loads
    for game in GameId::ALL {
        let Some(path) = backgrounds.get(game) else { continue };
        let Some(src) = backgrounds.current_name(game) else { continue };
        let thumb_path = path.with_file_name(format!("bg_thumb_{src}.png"));
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
        .unwrap_or_else(|| crate::config::fallback_home_join(".cache"))
        .join("elysiae-tui")
        .join("backgrounds")
}

fn quadrant_cache_dir() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| crate::config::fallback_home_join(".cache"))
        .join("elysiae-tui")
        .join("quadrant")
}
