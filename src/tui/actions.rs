use std::io;
use std::path::PathBuf;

use irmin::{DownloadHandle, SophonProgress};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::Sender;

use crate::app::App;
use crate::components::{ComponentManager, ComponentProgress};
use crate::game::GameId;
use crate::operations::Operations;

/// Spawns a fresh download task for the active game.
pub fn start_download(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    let path = match gc.install_path {
        Some(p) => p,
        None => {
            // Auto-assign a default install path
            let default = default_install_path(game);
            app.config.game_config(game).install_path = Some(default.clone());
            let _ = app.config.save();
            default
        }
    };
    let handle = DownloadHandle::new();
    app.start_download(game, handle.clone());
    spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Download);
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

/// Applies a previously downloaded preinstall patch.
pub fn apply_preinstall(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    if let Some(ref path) = gc.install_path {
        let info = app.games.get(&game)
            .and_then(|gs| gs.update_info.as_ref())
            .and_then(|i| i.preinstall_tag.clone());
        let Some(preinstall_tag) = info else { return };
        let handle = DownloadHandle::new();
        app.start_download(game, handle.clone());
        let client = client.clone();
        let tx = progress_tx.clone();
        let path_str = path.to_string_lossy().to_string();
        tokio::spawn(async move {
            if let Err(e) = irmin::game_installer::validate_asset_name(&preinstall_tag) {
                let _ = tx.send(SophonProgress::Error { message: format!("invalid preinstall tag: {e}") }).await;
                return;
            }
            let ops = Operations::new(client.clone());
            let result = ops.apply_preinstall(&preinstall_tag, &path_str, &handle, tx.clone()).await;
            if let Err(e) = result {
                let msg = e.to_string();
                if !msg.to_lowercase().contains("cancel") {
                    let _ = tx.send(SophonProgress::Error { message: msg }).await;
                }
            } else {
                if let Err(e) = crate::postinstall::run_post_install(ops.client(), std::path::Path::new(&path_str), game.as_str(), tx.clone()).await {
                    tracing::warn!("post-install failed: {e}");
                }
            }
            let _ = tx.send(SophonProgress::Finished).await;
        });
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

/// Resumes an interrupted download using irmin's saved state.
pub fn resume_download(app: &mut App, client: &reqwest::Client, progress_tx: &Sender<SophonProgress>) {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    if let Some(ref path) = gc.install_path {
        let handle = DownloadHandle::new();
        app.start_download(game, handle.clone());
        // Resume uses the same download function; irmin detects the state file automatically
        spawn_operation(client, game, gc.vo_lang.clone(), path.to_string_lossy().to_string(), handle, progress_tx.clone(), Op::Download);
    }
}

/// Leaves the TUI, launches the game, then re-enters the TUI.
pub fn launch_game(
    app: &mut App,
    _terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    log_tx: &tokio::sync::mpsc::Sender<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let game = app.active_game;
    let gc = app.config.game_config(game).clone();
    let Some(ref path) = gc.install_path else {
        app.error_message = Some("No install path configured for this game.".to_owned());
        return Ok(());
    };

    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
        .join("elysiae-tui");
    let launcher = crate::launcher::Launcher::new(data_dir);

    app.launch_log.clear();
    app.launch_log_scroll = 0;
    app.game_running = true;
    app.launch_log_game = Some(game);

    if let Err(e) = launcher.launch(game, path, log_tx.clone()) {
        app.game_running = false;
        app.error_message = Some(format!("Launch failed: {e}"));
    }

    Ok(())
}

/// Installs missing components then launches the game.
/// If components need installing, shows progress overlay first.
pub fn prepare_and_launch(
    app: &mut App,
    client: &reqwest::Client,
    progress_tx: &Sender<SophonProgress>,
) {
    let game = app.active_game;
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
        .join("elysiae-tui");

    use crate::components::{proton_available, jadeite_available};
    let needs_proton = !proton_available(&data_dir);
    let needs_jadeite = game.needs_jadeite() && !jadeite_available(&data_dir);

    if !needs_proton && !needs_jadeite {
        // Components ready — mark as ready to launch (handled by caller)
        app.ready_to_launch = true;
        return;
    }

    // Components missing — install them with progress, then signal ready to launch
    let handle = DownloadHandle::new();
    app.start_download(game, handle.clone());
    if let Some(ref mut dl) = app.download {
        dl.launch_on_complete = true;
    }
    let tx = progress_tx.clone();
    let client = client.clone();

    tokio::spawn(async move {
        if let Err(msg) = ensure_components(&client, &data_dir, game, &tx, &handle).await {
            if msg != "Cancelled" {
                let _ = tx.send(SophonProgress::Error { message: msg }).await;
            }
            return;
        }
        let _ = tx.send(SophonProgress::Finished).await;
    });
}

enum Op {
    Download,
    Update,
    Preinstall,
    Verify,
}

fn spawn_operation(
    client: &reqwest::Client,
    game: GameId,
    vo_lang: String,
    path: String,
    handle: DownloadHandle,
    tx: Sender<SophonProgress>,
    op: Op,
) {
    let client = client.clone();
    tokio::spawn(async move {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
            .join("elysiae-tui");

        // Auto-install components before any download operation
        if matches!(op, Op::Download | Op::Update | Op::Preinstall)
            && let Err(msg) = ensure_components(&client, &data_dir, game, &tx, &handle).await
        {
            let _ = tx.send(SophonProgress::Error { message: msg }).await;
            return;
        }

        let ops = Operations::new(client.clone());
        let result = match op {
            Op::Download => ops.download(game, &vo_lang, &path, &handle, tx.clone()).await,
            Op::Update => ops.update(game, &vo_lang, &path, &handle, tx.clone()).await,
            Op::Preinstall => ops.preinstall(game, &vo_lang, &path, &handle, tx.clone()).await,
            Op::Verify => ops.verify(game, &vo_lang, &path, tx.clone()).await,
        };
        if let Err(e) = result {
            let msg = e.to_string();
            if !msg.to_lowercase().contains("cancel") {
                let _ = tx.send(SophonProgress::Error { message: msg }).await;
            }
        } else if matches!(op, Op::Download | Op::Update)
            && let Err(e) = crate::postinstall::run_post_install(&client, std::path::Path::new(&path), game.as_str(), tx.clone()).await
        {
            tracing::warn!("post-install failed: {e}");
        }
        let _ = tx.send(SophonProgress::Finished).await;
    });
}

/// Installs Proton (and Jadeite for HKRPG) if not already present or outdated.
/// Persists installed component versions to config.
async fn ensure_components(
    client: &reqwest::Client,
    data_dir: &std::path::Path,
    game: GameId,
    tx: &Sender<SophonProgress>,
    handle: &DownloadHandle,
) -> Result<(), String> {
    use crate::components::{component_needs_update, jadeite_available, proton_available};
    use crate::config::Config;

    let proton_missing = !proton_available(data_dir);
    let proton_outdated = if !proton_missing {
        component_needs_update(client, data_dir, "proton").await
    } else {
        false
    };

    if proton_missing || proton_outdated {
        let label = if proton_outdated {
            "Updating Proton"
        } else {
            "Installing Proton"
        };
        // Remove stale/wrong-arch install before downloading fresh
        let proton_dir = data_dir.join("proton");
        let _ = crate::atomic::safe_remove_dir_all(&proton_dir);
        let _ = crate::atomic::safe_remove_dir_all(&data_dir.join("proton-data"));
        let tag = install_component_with_progress(client, data_dir, "proton", label, tx, handle)
            .await?;
        let mut config = Config::load();
        config.installed_components.proton = Some(tag);
        let _ = config.save();
    }

    if handle.is_cancelled() {
        return Err("Cancelled".to_owned());
    }

    if game.needs_jadeite() {
        let jadeite_missing = !jadeite_available(data_dir);
        let jadeite_outdated = if !jadeite_missing {
            component_needs_update(client, data_dir, "jadeite").await
        } else {
            false
        };

        if jadeite_missing || jadeite_outdated {
            let label = if jadeite_outdated { "Updating Jadeite" } else { "Installing Jadeite" };
            if jadeite_outdated {
                let _ = crate::atomic::safe_remove_dir_all(&data_dir.join("jadeite"));
            }
            let tag = install_component_with_progress(client, data_dir, "jadeite", label, tx, handle)
                .await?;
            let mut config = Config::load();
            config.installed_components.jadeite = Some(tag);
            let _ = config.save();
        }
    }

    Ok(())
}

/// Installs a single component, bridging ComponentProgress to SophonProgress.
/// Checks handle cancellation and aborts the install task if cancelled.
/// Returns the installed version tag on success.
async fn install_component_with_progress(
    client: &reqwest::Client,
    data_dir: &std::path::Path,
    component: &str,
    status_msg: &str,
    tx: &Sender<SophonProgress>,
    handle: &DownloadHandle,
) -> Result<String, String> {
    let _ = tx
        .send(SophonProgress::Warning {
            message: status_msg.to_owned(),
        })
        .await;

    let mgr = ComponentManager::new(client.clone(), data_dir.to_path_buf());
    let (comp_tx, mut comp_rx) = tokio::sync::mpsc::channel::<ComponentProgress>(64);

    let comp_name = component.to_owned();
    let install_task = tokio::spawn(async move {
        if comp_name == "proton" {
            mgr.install_proton(comp_tx).await
        } else {
            mgr.install_jadeite(comp_tx).await
        }
    });

    let mut last_bytes: u64 = 0;
    let mut last_time = std::time::Instant::now();
    let mut speed_bps: f64 = 0.0;
    let status = status_msg.to_owned();
    let comp_for_cleanup = component.to_owned();

    loop {
        tokio::select! {
            msg = comp_rx.recv() => {
                let Some(prog) = msg else { break };
                match prog {
                    ComponentProgress::Downloading { downloaded_bytes, total_bytes } => {
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(last_time).as_secs_f64();
                        if elapsed > 0.3 {
                            let bytes_delta = downloaded_bytes.saturating_sub(last_bytes);
                            speed_bps = bytes_delta as f64 / elapsed;
                            last_bytes = downloaded_bytes;
                            last_time = now;
                        }
                        let eta = if speed_bps > 0.0 && total_bytes > downloaded_bytes {
                            (total_bytes - downloaded_bytes) as f64 / speed_bps
                        } else {
                            0.0
                        };
                        let _ = tx.send(SophonProgress::Downloading {
                            downloaded_bytes, total_bytes, speed_bps, eta_seconds: eta,
                        }).await;
                    }
                    ComponentProgress::Extracting => {
                        let _ = tx.send(SophonProgress::InstallingPlugins {
                            current_plugin: "Extracting...".to_owned(),
                            total_plugins: 1,
                        }).await;
                    }
                    ComponentProgress::Finished { .. } => {}
                    ComponentProgress::Error { message } => {
                        return Err(format!("{}: {}", status, message));
                    }
                }
            }
            _ = handle.cancelled_future() => {
                install_task.abort();
                let archive = data_dir.join(format!("{}.archive", comp_for_cleanup));
                let _ = std::fs::remove_file(&archive);
                return Err("Cancelled".to_owned());
            }
        }
    }

    match install_task.await {
        Ok(Ok(tag)) => Ok(tag),
        Ok(Err(e)) => Err(format!("{}: {}", status_msg, e)),
        Err(e) if e.is_cancelled() => {
            let archive = data_dir.join(format!("{}.archive", component));
            let _ = std::fs::remove_file(&archive);
            Err("Cancelled".to_owned())
        }
        Err(e) => Err(format!("{}: task panicked: {}", status_msg, e)),
    }
}

fn default_install_path(game: GameId) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
        .join("elysiae-tui")
        .join("games")
        .join(game.as_str())
}
