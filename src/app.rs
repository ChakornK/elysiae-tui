use std::collections::{HashMap, VecDeque};

use irmin::game_installer::UpdateInfo;
use irmin::{DownloadHandle, SophonProgress};

use crate::config::Config;
use crate::game::GameId;
use crate::quadrant::QuadrantImage;

/// Which screen the TUI is currently showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    GameList,
    Settings,
}

/// Installation status for a single game.
#[derive(Debug, Clone)]
pub struct GameStatus {
    pub installed_tag: Option<String>,
    pub update_info: Option<UpdateInfo>,
    pub has_resume: bool,
}

/// Tracks an in-flight download operation with per-phase progress.
pub struct ActiveDownload {
    pub game_id: GameId,
    pub handle: DownloadHandle,
    pub paused: bool,
    pub download_progress: Option<DownloadPhase>,
    pub assemble_progress: Option<FilePhase>,
    pub check_progress: Option<FilePhase>,
    pub status_label: Option<String>,
    /// When true, completion triggers a game launch instead of state update.
    pub launch_on_complete: bool,
    /// Overrides the progress overlay header (e.g. "Installing Proton").
    pub header_override: Option<String>,
}

/// Download byte progress.
pub struct DownloadPhase {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub eta_seconds: f64,
}

/// File-based phase progress (assembling, verifying, checking).
pub struct FilePhase {
    pub label: String,
    pub done: u64,
    pub total: u64,
}

/// Central application state for the TUI.
pub struct App {
    pub active_game: GameId,
    pub games: HashMap<GameId, GameStatus>,
    pub current_view: View,
    pub download: Option<ActiveDownload>,
    pub config: Config,
    pub should_quit: bool,
    pub game_list_index: usize,
    pub status_message: Option<String>,
    pub error_message: Option<String>,
    pub show_resume_prompt: bool,
    pub backgrounds: HashMap<GameId, QuadrantImage>,
    pub ready_to_launch: bool,
    /// Game launch log lines (ring buffer, max 1000)
    pub launch_log: VecDeque<String>,
    /// Scroll offset for launch log display
    pub launch_log_scroll: usize,
    /// Whether a game is currently running
    pub game_running: bool,
    /// Which game the log belongs to
    pub launch_log_game: Option<GameId>,
}

impl App {
    /// Creates a new App with the given config. Starts on the game list view.
    pub fn new(config: Config) -> Self {
        Self {
            active_game: config.selected_game,
            games: HashMap::new(),
            current_view: View::GameList,
            download: None,
            config,
            should_quit: false,
            game_list_index: 0,
            status_message: None,
            error_message: None,
            show_resume_prompt: false,
            backgrounds: HashMap::new(),
            ready_to_launch: false,
            launch_log: VecDeque::new(),
            launch_log_scroll: 0,
            game_running: false,
            launch_log_game: None,
        }
    }

    /// Returns the GameId currently highlighted in the list.
    pub fn selected_game(&self) -> GameId {
        GameId::ALL[self.game_list_index]
    }

    /// Moves selection forward in the game list, wrapping around.
    pub fn next_game(&mut self) {
        self.game_list_index = (self.game_list_index + 1) % GameId::ALL.len();
    }

    /// Moves selection backward in the game list, wrapping around.
    pub fn prev_game(&mut self) {
        self.game_list_index = self
            .game_list_index
            .checked_sub(1)
            .unwrap_or(GameId::ALL.len() - 1);
    }

    /// Begins tracking a new download. Stays on the current view.
    pub fn start_download(&mut self, game_id: GameId, handle: DownloadHandle) {
        self.download = Some(ActiveDownload {
            game_id,
            handle,
            paused: false,
            download_progress: None,
            assemble_progress: None,
            check_progress: None,
            status_label: Some("Fetching manifest...".to_owned()),
            launch_on_complete: false,
            header_override: None,
        });
    }

    /// Updates download progress. Routes to the appropriate phase bar.
    pub fn update_progress(&mut self, progress: SophonProgress) {
        match &progress {
            SophonProgress::Finished => {
                // Persist component versions if newly installed
                self.sync_component_versions();

                if let Some(ref dl) = self.download {
                    if dl.launch_on_complete {
                        self.ready_to_launch = true;
                        self.download = None;
                        return;
                    }
                    let game = dl.game_id;
                    if let Some(gs) = self.games.get_mut(&game) {
                        gs.has_resume = false;
                        let gc = self.config.games.get(&game);
                        if let Some(path) = gc.and_then(|c| c.install_path.as_ref()) {
                            gs.installed_tag = irmin::game_installer::read_installed_tag(path);
                        }
                    }
                }
                self.download = None;
                return;
            }
            SophonProgress::Error { message } => {
                self.error_message = Some(message.clone());
                self.download = None;
                return;
            }
            _ => {}
        }
        let Some(dl) = &mut self.download else { return };
        match progress {
            SophonProgress::Downloading {
                downloaded_bytes,
                total_bytes,
                speed_bps,
                eta_seconds,
            } => {
                // Discard stale events (progress going backwards = from a cancelled task)
                if let Some(ref existing) = dl.download_progress
                    && downloaded_bytes < existing.downloaded_bytes
                    && total_bytes == existing.total_bytes
                {
                    return;
                }
                dl.status_label = None;
                dl.download_progress = Some(DownloadPhase {
                    downloaded_bytes,
                    total_bytes,
                    speed_bps,
                    eta_seconds,
                });
            }
            SophonProgress::Paused {
                downloaded_bytes,
                total_bytes,
            } => {
                dl.download_progress = Some(DownloadPhase {
                    downloaded_bytes,
                    total_bytes,
                    speed_bps: 0.0,
                    eta_seconds: 0.0,
                });
            }
            SophonProgress::Assembling {
                assembled_files,
                total_files,
            } => {
                dl.status_label = None;
                dl.assemble_progress = Some(FilePhase {
                    label: "Assembled".to_owned(),
                    done: assembled_files,
                    total: total_files,
                });
            }
            SophonProgress::Verifying {
                scanned_files,
                total_files,
                ..
            } => {
                dl.status_label = None;
                dl.check_progress = Some(FilePhase {
                    label: "Verified".to_owned(),
                    done: scanned_files,
                    total: total_files,
                });
            }
            SophonProgress::CheckingFiles {
                checked_files,
                total_files,
            } => {
                dl.status_label = None;
                dl.check_progress = Some(FilePhase {
                    label: "Checked".to_owned(),
                    done: checked_files,
                    total: total_files,
                });
            }
            SophonProgress::CalculatingDownloads {
                checked_files,
                total_files,
            } => {
                dl.status_label = None;
                dl.check_progress = Some(FilePhase {
                    label: "Checked".to_owned(),
                    done: checked_files,
                    total: total_files,
                });
            }
            SophonProgress::ApplyingPreinstall {
                applied_files,
                total_files,
            } => {
                dl.status_label = None;
                dl.assemble_progress = Some(FilePhase {
                    label: "Applied".to_owned(),
                    done: applied_files,
                    total: total_files,
                });
            }
            SophonProgress::FetchingManifest => {
                dl.header_override = None;
                dl.status_label = Some("Fetching manifest...".to_owned());
            }
            SophonProgress::InstallingPlugins { current_plugin, .. } => {
                dl.status_label = Some(current_plugin);
                dl.download_progress = None;
            }
            SophonProgress::DownloadingPlugin {
                name,
                downloaded_bytes,
                total_bytes,
            } => {
                dl.status_label = Some(format!("Plugin: {}", name));
                dl.download_progress = Some(DownloadPhase {
                    downloaded_bytes,
                    total_bytes,
                    speed_bps: 0.0,
                    eta_seconds: 0.0,
                });
            }
            SophonProgress::Warning { message } => {
                dl.header_override = Some(message);
            }
            _ => {}
        }
    }

    /// Cancels the active download. Cleans up partial component files.
    pub fn finish_download(&mut self) {
        if let Some(ref dl) = self.download {
            dl.handle.cancel();
            if let Some(gs) = self.games.get_mut(&dl.game_id) {
                gs.has_resume = true;
            }
        }
        self.download = None;
        self.ready_to_launch = false;

        // Remove partial component archives so next install starts clean
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
            .join("elysiae-tui");
        let _ = std::fs::remove_file(data_dir.join("proton.archive"));
        let _ = std::fs::remove_file(data_dir.join("jadeite.archive"));
    }

    /// Dismisses the current error message.
    pub fn dismiss_error(&mut self) {
        self.error_message = None;
    }

    /// Dismisses the current status message.
    pub fn dismiss_status(&mut self) {
        self.status_message = None;
    }

    /// Reads component tag files and saves to config if changed.
    fn sync_component_versions(&mut self) {
        use crate::components::read_component_tag;
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| crate::config::fallback_home_join(".local/share"))
            .join("elysiae-tui");
        let proton = read_component_tag(&data_dir, "proton");
        let jadeite = read_component_tag(&data_dir, "jadeite");
        let cv = &mut self.config.installed_components;
        let changed = cv.proton != proton || cv.jadeite != jadeite;
        if changed {
            cv.proton = proton;
            cv.jadeite = jadeite;
            let _ = self.config.save();
        }
    }
}
