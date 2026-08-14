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
    // Pair: (fade_on_arrival, images). Startup sends `true` to fade in from
    // dark; a resize re-encode sends `false` so the image just reflows in place.
    let (bg_tx, mut bg_rx) = mpsc::channel::<(bool, HashMap<GameId, QuadrantImage>)>(4);
    let bg_client = client.clone();
    let bg_term_size = term_size;
    let startup_tx = bg_tx.clone();
    tokio::spawn(async move {
        let mut backgrounds = Backgrounds::new(cache_dir);
        backgrounds.sync(&bg_client).await;
        let encoded = tokio::task::spawn_blocking(move || {
            // Generate 480×270 raw-RGB previews so the encode step only reads
            // a small file instead of decoding the full ~4 MB webp.
            ensure_previews(&backgrounds);
            encode_missing(&backgrounds, bg_term_size, &qcache_dir)
        }).await.unwrap_or_default();
        let _ = startup_tx.send((true, encoded)).await;
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
        while let Ok((fade, new_bgs)) = bg_rx.try_recv() {
            // `fade` is set by the startup encode (fade in from dark) and cleared
            // by the resize re-encode (the image should just reflow, not crossfade).
            for (game, img) in new_bgs {
                if fade && app.selected_game() == game {
                    let from = app.backgrounds.get(&game).cloned().unwrap_or_else(|| {
                        QuadrantImage::dark_blank(img.width, img.height)
                    });
                    app.bg_transition = Some(crate::transition::BgTransition::new(from));
                }
                app.backgrounds.insert(game, img);
            }
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

            // Any backgrounds not already cached for the new size are encoded
            // in the background and delivered through the same channel.
            let cache_dir = bg_cache_dir();
            let qcache_dir = quadrant_cache_dir();
            let resize_tx = bg_tx.clone();
            let (cols, rows) = term_size;
            tokio::spawn(async move {
                let backgrounds = Backgrounds::new(cache_dir);
                let encoded = tokio::task::spawn_blocking(move || {
                    ensure_previews(&backgrounds);
                    encode_missing(&backgrounds, (cols, rows), &qcache_dir)
                }).await.unwrap_or_default();
                let _ = resize_tx.send((false, encoded)).await;
            });
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
        let path = dir.join(format!("{}_{cols}x{rows}_{src}.v2.qcache", game.as_str()));
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
/// Quadrant cache files are versioned by source (`{game}_{w}x{h}_{src}.v2.qcache`),
/// so a fresh remote source always produces a fresh cache and stale ones from
/// an older source are never loaded. The `v2` marks the aspect-preserving
/// (cover) encoder — older squished caches are ignored, not loaded.
fn encode_missing(
    backgrounds: &Backgrounds,
    term_size: (u16, u16),
    qcache_dir: &std::path::Path,
) -> HashMap<GameId, QuadrantImage> {
    let _ = std::fs::create_dir_all(qcache_dir);
    let (cols, rows) = term_size;
    let grid_w = (cols as u32) * 2;
    let grid_h = (rows as u32) * 2;

    let handles: Vec<_> = GameId::ALL
        .iter()
        .filter_map(|&game| {
            let src = backgrounds.current_name(game)?;
            let path = backgrounds.get(game)?;
            let cache_path =
                qcache_dir.join(format!("{}_{cols}x{rows}_{src}.v2.qcache", game.as_str()));
            if cache_path.exists() {
                return None;
            }
            let preview_path = path.with_file_name(format!("bg_{src}.preview"));
            Some(std::thread::spawn(move || -> Option<(GameId, QuadrantImage)> {
                // Read the 480×270 raw-RGB preview (generated by ensure_previews)
                // and cover-fit it to the quadrant grid, preserving aspect ratio.
                let data = std::fs::read(&preview_path).ok()?;
                let preview = image::RgbImage::from_raw(480, 270, data)?;
                let resized = cover_resize(&preview, grid_w, grid_h);
                let mut encoded = QuadrantImage::encode(&resized, cols, rows);
                let _ = encoded.write_cache(&cache_path);
                encoded.darken();
                Some((game, encoded))
            }))
        })
        .collect();

    handles
        .into_iter()
        .filter_map(|h| h.join().ok().flatten())
        .collect()
}

/// Resizes `src` to exactly fill `out_w × out_h`, cropping any overflow, so the
/// displayed image never distorts. Terminal cells are roughly twice as tall as
/// wide, so each quadrant grid-pixel is about half as wide as it is tall on
/// screen (pixel aspect ratio `PAR`). The visual area aspect is therefore
/// `out_w * PAR / out_h`; we first crop the source to that aspect (cover), then
/// resize to the grid — the image keeps its proportions and fills the terminal.
// ponytail: PAR fixed at 0.5 (typical monospace cell ~8×16px); raise/lower if a
// specific font measures off and the background looks skewed.
fn cover_resize(src: &image::RgbImage, out_w: u32, out_h: u32) -> image::RgbImage {
    const PAR: f64 = 0.5;
    let target_vis = (out_w as f64) * PAR / (out_h as f64);
    let (sw, sh) = (src.width(), src.height());
    let src_aspect = sw as f64 / sh as f64;
    // Cover: keep the larger dimension, shrink the other to match the target
    // visual aspect, cropping the excess from the center.
    let (crop_w, crop_h) = if src_aspect > target_vis {
        // Source visually wider than the area — fit height, crop the sides.
        let w = ((sh as f64) * target_vis).round() as u32;
        (w.max(1).min(sw), sh)
    } else {
        // Source visually taller than the area — fit width, crop top/bottom.
        let h = ((sw as f64) / target_vis).round() as u32;
        (sw, h.max(1).min(sh))
    };
    let x = (sw - crop_w) / 2;
    let y = (sh - crop_h) / 2;
    let cropped = image::imageops::crop_imm(src, x, y, crop_w, crop_h).to_image();
    image::imageops::resize(
        &cropped,
        out_w,
        out_h,
        image::imageops::FilterType::Triangle,
    )
}

/// Generates a 480×270 raw-RGB preview file for each game whose background
/// source is available but no preview has been cached yet. The preview is
/// decoded via libwebp (fast C decoder) and read by `encode_missing`, keeping
/// the display path well under 50 ms. Decoding happens once per source here.
fn ensure_previews(backgrounds: &Backgrounds) {
    let preview_w = 480u32;
    let preview_h = 270u32;
    let tasks: Vec<_> = GameId::ALL
        .iter()
        .filter_map(|&game| {
            let src = backgrounds.current_name(game)?;
            let path = backgrounds.get(game)?;
            let preview_path = path.with_file_name(format!("bg_{src}.preview"));
            if preview_path.exists() {
                return None;
            }
            let path = path.clone();
            Some(std::thread::spawn(move || {
                let rgb = crate::webp_fast::decode_scaled(&path, preview_w, preview_h)?;
                std::fs::write(&preview_path, rgb.as_raw()).ok()?;
                Some(())
            }))
        })
        .collect();
    for t in tasks {
        let _ = t.join();
    }
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

#[cfg(test)]
mod tests {
    use super::cover_resize;

    fn solid(w: u32, h: u32) -> image::RgbImage {
        image::RgbImage::from_pixel(w, h, image::Rgb([120, 120, 120]))
    }

    #[test]
    fn cover_resize_fills_exact_output() {
        for &(w, h) in &[(350u32, 104u32), (400, 50), (80, 80), (1, 1), (1, 200), (200, 1)] {
            let out = cover_resize(&solid(480, 270), w, h);
            assert_eq!(out.dimensions(), (w, h), "output must be exactly {w}x{h}");
        }
    }

    #[test]
    fn cover_resize_preserves_visual_aspect_16_9() {
        // Grid whose visual area (PAR 0.5) is itself 16:9 → no crop needed,
        // source maps 1:1 and should be untouched in content size terms.
        // grid_w * 0.5 / grid_h = 16/9  →  grid_w / grid_h = 32/9 ≈ 3.556
        let (w, h) = (356u32, 100u32); // ≈ 3.56
        let src = image::RgbImage::from_pixel(480, 270, image::Rgb([200, 30, 30]));
        let out = cover_resize(&src, w, h);
        // Pixels survive the cover→resize (no black bars introduced).
        let p = out.get_pixel(w / 2, h / 2).0;
        assert_eq!(p, [200, 30, 30]);
    }

    #[test]
    fn cover_resize_wide_area_crops_source_sides() {
        // Visual area PAR 0.5 → 400*0.5/50 = 4.0, much wider than the 16:9 source,
        // so cover fits width and crops top/bottom (not sides). Paint the source in
        // three horizontal bands: red top, green middle, blue bottom. The vertical
        // crop must drop red and blue entirely, leaving a uniformly green output.
        let (w, h) = (400u32, 50u32);
        let src = image::RgbImage::from_fn(480, 270, |_, y| {
            if y < 67 { image::Rgb([255, 0, 0]) }
            else if y >= 202 { image::Rgb([0, 0, 255]) }
            else { image::Rgb([0, 255, 0]) }
        });
        let out = cover_resize(&src, w, h);
        assert_eq!(out.dimensions(), (w, h));
        for p in out.pixels() {
            assert_eq!(p.0, [0, 255, 0], "cropped output must be pure green (red/blue bands cropped)");
        }
    }
}
