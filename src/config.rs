use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::game::GameId;

/// Top-level persisted configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub version: u32,
    pub selected_game: GameId,
    pub games: HashMap<GameId, GameConfig>,
    pub installed_components: ComponentVersions,
}

/// Per-game settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameConfig {
    pub vo_lang: String,
    pub install_path: Option<PathBuf>,
}

/// Tracks installed versions of runtime components.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentVersions {
    pub proton: Option<String>,
    pub jadeite: Option<String>,
}

const CURRENT_VERSION: u32 = 1;
const CONFIG_DIR_NAME: &str = "elysiae-cli";
const CONFIG_FILE_NAME: &str = "config.json";

impl Config {
    /// Loads config from XDG_CONFIG_HOME, or creates a default if missing/corrupt.
    pub fn load() -> Self {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|_| Self::default()),
            Err(_) => Self::default(),
        }
    }

    /// Persists config to disk. Creates parent directories if needed.
    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(ConfigError::Io)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(ConfigError::Serialize)?;
        fs::write(&path, json).map_err(ConfigError::Io)?;
        Ok(())
    }

    /// Returns the config file path under XDG_CONFIG_HOME.
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join(CONFIG_DIR_NAME)
            .join(CONFIG_FILE_NAME)
    }

    /// Returns the game config for the given id, inserting a default if absent.
    pub fn game_config(&mut self, game: GameId) -> &mut GameConfig {
        self.games.entry(game).or_insert_with(GameConfig::default)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            selected_game: GameId::Hk4e,
            games: HashMap::new(),
            installed_components: ComponentVersions {
                proton: None,
                jadeite: None,
            },
        }
    }
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            vo_lang: "en-us".to_owned(),
            install_path: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(std::io::Error),
    #[error("serialization error: {0}")]
    Serialize(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrip() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn config_with_games_roundtrip() {
        let mut config = Config::default();
        config.games.insert(
            GameId::Hk4e,
            GameConfig {
                vo_lang: "ja-jp".to_owned(),
                install_path: Some(PathBuf::from("/games/hk4e")),
            },
        );
        config.installed_components.proton = Some("9-27".to_owned());

        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn corrupt_json_falls_back_to_default() {
        let result: Result<Config, _> = serde_json::from_str("not json at all");
        assert!(result.is_err());
        // Config::load() would return default in this case
    }
}
