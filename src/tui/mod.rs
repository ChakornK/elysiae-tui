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

    scan_game_state(&mut app);
    sync_component_state(&mut app);

    // Load pre-encoded backgrounds from the quadrant cache
    load_from_cache(&mut app, term_size);

    // Spawn background image-sync and update-check tasks
    let BackgroundTasks { bg_tx, mut bg_rx, update_tx, mut update_rx } =
        spawn_background_tasks(&mut app, &client, term_size);

    // Main event loop — TUI is interactive immediately
    let mut auto_start_done = false;
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
        while let Ok(updates) = update_rx.try_recv() {
            for (game, info) in &updates {
                if let Some(gs) = app.games.get_mut(game) {
                    gs.update_info = Some(info.clone());
                }
            }
            // Auto-update the first eligible game once per session; preinstalls are manual.
            if !auto_start_done && app.download.is_none() {
                for (game, info) in &updates {
                    if info.update_available && app.config.auto_update {
                        let gc = app.config.game_config(*game).clone();
                        if gc.install_path.is_some() {
                            actions::start_update(*game, &mut app, &client, &progress_tx);
                            auto_start_done = true;
                            break;
                        }
                    }
                }
            }
        }

        if let Some(key) = poll_key_event()? {
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
            if app.download.is_some()
                && matches!(
                    key.code,
                    crossterm::event::KeyCode::Char('p') | crossterm::event::KeyCode::Char('c')
                )
            {
                input::handle_key(&mut app, key.code, &client, &progress_tx, &mut terminal).await?;
                continue;
            }

            if input::dispatch_modal_key(&mut app, key.code, &client, &progress_tx) {
                continue;
            }

            input::handle_key(&mut app, key.code, &client, &progress_tx, &mut terminal).await?;
        }

        // Re-encode backgrounds on terminal resize
        if let Ok(new_size) = crossterm::terminal::size() && new_size != term_size {
            term_size = new_size;
            respawn_bg_encode(&mut app, new_size, &bg_tx);
        }

        drain_progress(&mut app, &mut progress_rx, &client, &update_tx);

        // Receive game launch log lines
        drain_launch_log(&mut app, &mut log_rx);

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

/// Reads each game's install tag and resume state into `app.games`.
///
/// When a game has resume state but no configured install path, the state's
/// `output_path` is adopted so a fresh download targeting the default path
/// never silently discards the resumable chunks.
fn scan_game_state(app: &mut App) {
    let data_dir = crate::config::app_data_dir();
    let mut config_dirty = false;
    for game in GameId::ALL {
        let gc = app.config.game_config(game).clone();
        let vo_lang = gc.primary_vo_lang().to_owned();
        let tag = gc
            .install_path
            .as_ref()
            .and_then(|p| irmin::game_installer::read_installed_tag(p));
        let has_resume = tag.is_none()
            && crate::state::resumable_state(&data_dir, game.as_str(), gc.install_path.as_deref(), &vo_lang)
                .map(|state| {
                    if gc.install_path.is_none() {
                        app.config.game_config(game).install_path =
                            Some(std::path::PathBuf::from(&state.output_path));
                        config_dirty = true;
                    }
                })
                .is_some();
        app.games.insert(
            game,
            crate::app::GameStatus { installed_tag: tag, update_info: None, has_resume },
        );
    }
    if config_dirty {
        let _ = app.config.save();
    }
}

/// Syncs the installed Phlogiston version from disk into config.
fn sync_component_state(app: &mut App) {
    use crate::components::{proton_available, read_component_tag};
    let data_dir = crate::config::app_data_dir();
    let proton_tag = read_component_tag(&data_dir, "proton")
        .or_else(|| if proton_available(&data_dir) { Some("installed".to_owned()) } else { None });
    let cv = &mut app.config.installed_components;
    if cv.proton != proton_tag {
        cv.proton = proton_tag;
        let _ = app.config.save();
    }
}

type BgMessage = (bool, HashMap<GameId, QuadrantImage>);

/// Bundles the send/receive ends of the startup background tasks.
///
/// The background senders are kept by the event loop so it can re-encode
/// backgrounds on resize and re-check updates after a download finishes.
struct BackgroundTasks {
    bg_tx: mpsc::Sender<BgMessage>,
    bg_rx: mpsc::Receiver<BgMessage>,
    update_tx: mpsc::Sender<HashMap<GameId, UpdateInfo>>,
    update_rx: mpsc::Receiver<HashMap<GameId, UpdateInfo>>,
}

/// Spawns the startup background tasks and returns their channel ends.
///
/// The background task syncs remote background images and encodes any that are
/// missing from the quadrant cache. The update task checks every installed game
/// for updates.
fn spawn_background_tasks(
    app: &mut App,
    client: &reqwest::Client,
    term_size: (u16, u16),
) -> BackgroundTasks {
    let (bg_tx, bg_rx) = mpsc::channel::<BgMessage>(4);
    let (update_tx, update_rx) = mpsc::channel::<HashMap<GameId, UpdateInfo>>(4);

    let cache_dir = bg_cache_dir();
    let qcache_dir = quadrant_cache_dir();
    // Pair: (fade_on_arrival, images). Startup sends `true` to fade in from
    // dark; a resize re-encode sends `false` so the image just reflows in place.
    let startup_tx = bg_tx.clone();
    let startup_client = client.clone();
    tokio::spawn(async move {
        let mut backgrounds = Backgrounds::new(cache_dir);
        backgrounds.sync(&startup_client).await;
        let encoded = tokio::task::spawn_blocking(move || {
            // Generate 480×270 raw-RGB previews so the encode step only reads
            // a small file instead of decoding the full ~4 MB webp.
            ensure_previews(&backgrounds);
            encode_missing(&backgrounds, term_size, &qcache_dir)
        })
        .await
        .unwrap_or_default();
        let _ = startup_tx.send((true, encoded)).await;
    });

    // Only installed games with a configured path are update-checked.
    let update_configs: Vec<_> = GameId::ALL
        .iter()
        .filter_map(|&game| {
            let gc = app.config.game_config(game).clone();
            let path = gc.install_path.as_ref()?.to_string_lossy().to_string();
            let installed = app
                .games
                .get(&game)
                .and_then(|gs| gs.installed_tag.as_ref())
                .is_some();
            installed.then_some((game, gc.primary_vo_lang().to_owned(), path))
        })
        .collect();
    let update_client = client.clone();
    let update_tx_send = update_tx.clone();
    tokio::spawn(async move {
        let ops = Operations::new(update_client, crate::config::app_data_dir());
        let mut results = HashMap::new();
        for (game, vo_lang, path) in update_configs {
            if let Ok(info) = ops.check_update(game, &vo_lang, &path).await {
                results.insert(game, info);
            }
        }
        let _ = update_tx_send.send(results).await;
    });

    BackgroundTasks { bg_tx, bg_rx, update_tx, update_rx }
}

/// Polls the terminal for a single key event, returning it if present.
fn poll_key_event() -> io::Result<Option<crossterm::event::KeyEvent>> {
    if event::poll(Duration::from_millis(33))? {
        match event::read()? {
            Event::Key(key) => Ok(Some(key)),
            _ => Ok(None),
        }
    } else {
        Ok(None)
    }
}

/// Clears cached backgrounds and spawns a background re-encode for the new
/// terminal size. The result arrives through `bg_tx` with `fade = false` so
/// the image reflows in place instead of crossfading.
fn respawn_bg_encode(app: &mut App, term_size: (u16, u16), bg_tx: &mpsc::Sender<BgMessage>) {
    app.backgrounds.clear();
    app.bg_transition = None;
    load_from_cache(app, term_size);

    let cache_dir = bg_cache_dir();
    let qcache_dir = quadrant_cache_dir();
    let resize_tx = bg_tx.clone();
    let (cols, rows) = term_size;
    tokio::spawn(async move {
        let backgrounds = Backgrounds::new(cache_dir);
        let encoded = tokio::task::spawn_blocking(move || {
            ensure_previews(&backgrounds);
            encode_missing(&backgrounds, (cols, rows), &qcache_dir)
        })
        .await
        .unwrap_or_default();
        let _ = resize_tx.send((false, encoded)).await;
    });
}

/// Drains download progress events into the app. A finished op may have
/// changed on-disk state (preinstall marker written, update applied), so a
/// fresh update check is spawned for that game.
fn drain_progress(
    app: &mut App,
    progress_rx: &mut mpsc::Receiver<SophonProgress>,
    client: &reqwest::Client,
    update_tx: &mpsc::Sender<HashMap<GameId, UpdateInfo>>,
) {
    while let Ok(progress) = progress_rx.try_recv() {
        let refresh_game = if matches!(progress, SophonProgress::Finished)
            && let Some(ref dl) = app.download
        {
            Some(dl.game_id)
        } else {
            None
        };
        app.update_progress(progress);
        if let Some(game) = refresh_game {
            let gc = app.config.game_config(game).clone();
            if let Some(path) = gc.install_path.as_ref() {
                let update_tx = update_tx.clone();
                let update_client = client.clone();
                let vo_lang = gc.primary_vo_lang().to_owned();
                let path = path.to_string_lossy().to_string();
                tokio::spawn(async move {
                    let ops = Operations::new(update_client, crate::config::app_data_dir());
                    if let Ok(info) = ops.check_update(game, &vo_lang, &path).await {
                        let mut results = HashMap::new();
                        results.insert(game, info);
                        let _ = update_tx.send(results).await;
                    }
                });
            }
        }
    }
}

/// Drains game launch log lines into the ring buffer, auto-scrolling to the
/// bottom. The `\x00__PROCESS_EXIT__` sentinel clears the running state.
fn drain_launch_log(app: &mut App, log_rx: &mut mpsc::Receiver<String>) {
    while let Ok(line) = log_rx.try_recv() {
        if line == "\x00__PROCESS_EXIT__" {
            app.game_running = false;
            continue;
        }
        app.launch_log.push_back(line);
        if app.launch_log.len() > crate::app::MAX_LOG_LINES {
            app.launch_log.pop_front();
        }
        if app.launch_log.len() > crate::app::VISIBLE_LOG_LINES {
            app.launch_log_scroll = app.launch_log.len() - crate::app::VISIBLE_LOG_LINES;
        }
    }
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
/// resize to the grid. The image keeps its proportions and fills the terminal.
/// PAR is fixed at 0.5 (a typical monospace cell is ~8×16px); adjust it if a
/// specific font measures off and the background looks skewed.
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
    crate::config::app_cache_dir().join("backgrounds")
}

fn quadrant_cache_dir() -> std::path::PathBuf {
    crate::config::app_cache_dir().join("quadrant")
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
