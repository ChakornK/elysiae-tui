use std::collections::HashMap;

use irmin::game_installer::UpdateInfo;
use irmin::{DownloadHandle, SophonProgress};

use crate::config::Config;
use crate::game::GameId;
use crate::quadrant::QuadrantImage;

/// Which screen the TUI is currently showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    GameList,
    GameDetail,
    Settings,
    Downloading,
}

/// Installation status for a single game.
#[derive(Debug, Clone)]
pub struct GameStatus {
    pub installed_tag: Option<String>,
    pub update_info: Option<UpdateInfo>,
}

/// Tracks an in-flight download operation with per-phase progress.
pub struct ActiveDownload {
    pub game_id: GameId,
    pub handle: DownloadHandle,
    pub paused: bool,
    /// Current download phase progress (bytes).
    pub download_progress: Option<DownloadPhase>,
    /// Current verify/assemble phase progress (files).
    pub verify_progress: Option<VerifyPhase>,
    /// Short status label for misc phases (fetching manifest, installing plugins, etc.)
    pub status_label: Option<String>,
}

/// Download byte progress.
pub struct DownloadPhase {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub eta_seconds: f64,
}

/// Verify/assemble file progress.
pub struct VerifyPhase {
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
    pub settings_index: usize,
    pub backgrounds: HashMap<GameId, QuadrantImage>,
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
            settings_index: 0,
            backgrounds: HashMap::new(),
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

    /// Enters the detail view for the currently selected game.
    pub fn enter_game_detail(&mut self) {
        self.active_game = self.selected_game();
        self.current_view = View::GameDetail;
    }

    /// Navigates back one view level. Downloading view is sticky.
    pub fn back(&mut self) {
        self.current_view = match self.current_view {
            View::GameDetail => View::GameList,
            View::Settings => View::GameDetail,
            // Downloading and GameList don't go back further
            other => other,
        };
    }

    /// Begins tracking a new download. Stays on the current view.
    pub fn start_download(&mut self, game_id: GameId, handle: DownloadHandle) {
        self.download = Some(ActiveDownload {
            game_id,
            handle,
            paused: false,
            download_progress: None,
            verify_progress: None,
            status_label: Some("Fetching manifest...".to_owned()),
        });
    }

    /// Updates download progress. Routes to the appropriate phase bar.
    pub fn update_progress(&mut self, progress: SophonProgress) {
        match &progress {
            SophonProgress::Finished => {
                self.status_message = Some("Operation completed.".to_owned());
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
                dl.status_label = Some("Paused".to_owned());
            }
            SophonProgress::Verifying {
                scanned_files,
                total_files,
                ..
            } => {
                dl.status_label = None;
                dl.verify_progress = Some(VerifyPhase {
                    label: "Verifying".to_owned(),
                    done: scanned_files,
                    total: total_files,
                });
            }
            SophonProgress::Assembling {
                assembled_files,
                total_files,
            } => {
                dl.status_label = None;
                dl.verify_progress = Some(VerifyPhase {
                    label: "Assembling".to_owned(),
                    done: assembled_files,
                    total: total_files,
                });
            }
            SophonProgress::CheckingFiles {
                checked_files,
                total_files,
            } => {
                dl.status_label = None;
                dl.verify_progress = Some(VerifyPhase {
                    label: "Checking".to_owned(),
                    done: checked_files,
                    total: total_files,
                });
            }
            SophonProgress::CalculatingDownloads {
                checked_files,
                total_files,
            } => {
                dl.status_label = Some(format!("Calculating {}/{}", checked_files, total_files));
            }
            SophonProgress::ApplyingPreinstall {
                applied_files,
                total_files,
            } => {
                dl.status_label = None;
                dl.verify_progress = Some(VerifyPhase {
                    label: "Applying".to_owned(),
                    done: applied_files,
                    total: total_files,
                });
            }
            SophonProgress::FetchingManifest => {
                dl.status_label = Some("Fetching manifest...".to_owned());
            }
            SophonProgress::InstallingPlugins { current_plugin, .. } => {
                dl.status_label = Some(format!("Plugin: {}", current_plugin));
            }
            SophonProgress::DownloadingPlugin {
                name,
                downloaded_bytes,
                total_bytes,
            } => {
                dl.status_label = None;
                dl.download_progress = Some(DownloadPhase {
                    downloaded_bytes,
                    total_bytes,
                    speed_bps: 0.0,
                    eta_seconds: 0.0,
                });
                dl.verify_progress = Some(VerifyPhase {
                    label: format!("Plugin: {}", name),
                    done: downloaded_bytes,
                    total: total_bytes,
                });
            }
            _ => {}
        }
    }

    /// Clears the active download.
    pub fn finish_download(&mut self) {
        self.download = None;
    }

    /// Dismisses the current error message.
    pub fn dismiss_error(&mut self) {
        self.error_message = None;
    }

    /// Dismisses the current status message.
    pub fn dismiss_status(&mut self) {
        self.status_message = None;
    }
}
