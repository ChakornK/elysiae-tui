use std::collections::HashMap;

use irmin::game_installer::UpdateInfo;
use irmin::{DownloadHandle, SophonProgress};

use crate::config::Config;
use crate::game::GameId;

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

/// Tracks an in-flight download operation.
pub struct ActiveDownload {
    pub game_id: GameId,
    pub progress: SophonProgress,
    pub handle: DownloadHandle,
    pub paused: bool,
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

    /// Begins tracking a new download.
    pub fn start_download(&mut self, game_id: GameId, handle: DownloadHandle) {
        self.download = Some(ActiveDownload {
            game_id,
            progress: SophonProgress::FetchingManifest,
            handle,
            paused: false,
        });
        self.current_view = View::Downloading;
    }

    /// Updates download progress. Transitions to GameDetail on terminal states.
    pub fn update_progress(&mut self, progress: SophonProgress) {
        match &progress {
            SophonProgress::Finished => {
                self.status_message = Some("Operation completed.".to_owned());
                self.download = None;
                self.current_view = View::GameDetail;
                return;
            }
            SophonProgress::Error { message } => {
                self.error_message = Some(message.clone());
                self.download = None;
                self.current_view = View::GameDetail;
                return;
            }
            _ => {}
        }
        if let Some(dl) = &mut self.download {
            dl.progress = progress;
        }
    }

    /// Clears the active download and returns to game detail.
    pub fn finish_download(&mut self) {
        self.download = None;
        self.current_view = View::GameDetail;
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
