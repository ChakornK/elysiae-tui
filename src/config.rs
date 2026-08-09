use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::game::GameId;

/// Expands a home-relative subpath using the actual HOME directory.
/// Falls back to /tmp/elysiae-tui if HOME is unset.
pub(crate) fn fallback_home_join(subpath: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp/elysiae-tui"))
        .join(subpath)
}

/// Returns the application data directory (`{XDG_DATA_HOME}/elysiae-tui`).
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| fallback_home_join(".local/share"))
        .join("elysiae-tui")
}

pub const VALID_LANGS: &[&str] = &["en-us", "ja-jp", "zh-cn", "zh-tw", "ko-kr"];

pub fn validate_vo_lang(lang: &str) -> Result<(), ConfigError> {
    if VALID_LANGS.contains(&lang) {
        Ok(())
    } else {
        Err(ConfigError::InvalidLang(lang.to_string()))
    }
}

/// Top-level persisted configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub version: u32,
    pub selected_game: GameId,
    pub games: HashMap<GameId, GameConfig>,
    pub installed_components: ComponentVersions,
    /// Automatically download game updates on startup.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Automatically download preinstall patches on startup.
    #[serde(default = "default_true")]
    pub auto_preload: bool,
}

fn default_true() -> bool {
    true
}

/// Per-game settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GameConfig {
    pub vo_langs: Vec<String>,
    pub install_path: Option<PathBuf>,
}

/// Custom deserialization: accepts both old `vo_lang: String` and new `vo_langs: Vec<String>`.
impl<'de> Deserialize<'de> for GameConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            vo_lang: Option<String>,
            #[serde(default)]
            vo_langs: Option<Vec<String>>,
            install_path: Option<PathBuf>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let vo_langs = match raw.vo_langs {
            Some(langs) if !langs.is_empty() => langs,
            _ => vec![raw.vo_lang.unwrap_or_else(|| "en-us".to_owned())],
        };
        Ok(GameConfig {
            vo_langs,
            install_path: raw.install_path,
        })
    }
}

impl GameConfig {
    /// Returns the primary VO language (first in list, fallback "en-us").
    pub fn primary_vo_lang(&self) -> &str {
        self.vo_langs.first().map(|s| s.as_str()).unwrap_or("en-us")
    }
}

/// Returns a human-readable name for a VO language code.
pub fn lang_display_name(code: &str) -> &str {
    match code {
        "en-us" => "English",
        "ja-jp" => "Japanese",
        "zh-cn" => "Chinese (Simplified)",
        "zh-tw" => "Chinese (Traditional)",
        "ko-kr" => "Korean",
        _ => code,
    }
}

/// Tracks installed versions of runtime components.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentVersions {
    pub proton: Option<String>,
    pub jadeite: Option<String>,
}

const CURRENT_VERSION: u32 = 1;
const CONFIG_DIR_NAME: &str = "elysiae-tui";
const CONFIG_FILE_NAME: &str = "config.json";

impl Config {
    /// Loads config from XDG_CONFIG_HOME, or creates a default if missing/corrupt.
    pub fn load() -> Self {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|_| {
                if let Ok(preserved) = crate::atomic::preserve_corrupt(&path) {
                    tracing::warn!("corrupt config preserved as {}", preserved.display());
                }
                Self::default()
            }),
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
        crate::atomic::atomic_write(&path, json.as_bytes()).map_err(ConfigError::Io)?;
        Ok(())
    }

    /// Returns the config file path under XDG_CONFIG_HOME.
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| fallback_home_join(".config"))
            .join(CONFIG_DIR_NAME)
            .join(CONFIG_FILE_NAME)
    }

    /// Returns the game config for the given id, inserting a default if absent.
    pub fn game_config(&mut self, game: GameId) -> &mut GameConfig {
        self.games.entry(game).or_default()
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
            auto_update: true,
            auto_preload: true,
        }
    }
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            vo_langs: vec!["en-us".to_owned()],
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
    #[error("invalid voice-over language: {0}")]
    InvalidLang(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn arb_game_id() -> impl Strategy<Value = GameId> {
        prop_oneof![
            Just(GameId::Bh3),
            Just(GameId::Hk4e),
            Just(GameId::Hkrpg),
            Just(GameId::Nap),
        ]
    }

    fn arb_vo_lang() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("en-us".to_owned()),
            Just("zh-cn".to_owned()),
            Just("zh-tw".to_owned()),
            Just("ko-kr".to_owned()),
            Just("ja-jp".to_owned()),
        ]
    }

    fn arb_game_config() -> impl Strategy<Value = GameConfig> {
        (arb_vo_lang(), proptest::option::of("[a-z/]{1,30}")).prop_map(|(vo_lang, path)| {
            GameConfig {
                vo_langs: vec![vo_lang],
                install_path: path.map(PathBuf::from),
            }
        })
    }

    fn arb_config() -> impl Strategy<Value = Config> {
        (
            arb_game_id(),
            proptest::collection::hash_map(arb_game_id(), arb_game_config(), 0..=4),
            proptest::option::of("[a-z0-9-]{1,10}"),
            proptest::option::of("[a-z0-9-]{1,10}"),
        )
            .prop_map(|(selected, games, proton, jadeite)| Config {
                version: CURRENT_VERSION,
                selected_game: selected,
                games,
                installed_components: ComponentVersions { proton, jadeite },
                auto_update: true,
                auto_preload: true,
            })
    }

    proptest! {
        #[test]
        fn config_serde_roundtrip(config in arb_config()) {
            let json = serde_json::to_string(&config).unwrap();
            let parsed: Config = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(config, parsed);
        }
    }

    #[test]
    fn default_config_roundtrip() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn corrupt_json_falls_back_to_default() {
        let result: Result<Config, _> = serde_json::from_str("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn old_format_vo_lang_migrates() {
        let json = r#"{"vo_lang": "ja-jp", "install_path": "/games/hk4e"}"#;
        let gc: GameConfig = serde_json::from_str(json).unwrap();
        assert_eq!(gc.vo_langs, vec!["ja-jp"]);
        assert_eq!(gc.install_path, Some(PathBuf::from("/games/hk4e")));
    }

    #[test]
    fn missing_vo_fields_defaults_to_english() {
        let json = r#"{"install_path": null}"#;
        let gc: GameConfig = serde_json::from_str(json).unwrap();
        assert_eq!(gc.vo_langs, vec!["en-us"]);
    }

    #[test]
    fn new_format_vo_langs_preferred_over_vo_lang() {
        let json = r#"{"vo_lang": "ja-jp", "vo_langs": ["zh-cn", "ko-kr"]}"#;
        let gc: GameConfig = serde_json::from_str(json).unwrap();
        assert_eq!(gc.vo_langs, vec!["zh-cn", "ko-kr"]);
    }
}
