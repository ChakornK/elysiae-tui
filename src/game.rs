use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Identifies one of the four supported game titles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameId {
    Bh3,
    Hk4e,
    Hkrpg,
    Nap,
}

/// Hex-encoded display names decoded at runtime.
const DISPLAY_NAMES: [&str; 4] = [
    "486f6e6b616920496d7061637420337264", // index 0
    "47656e7368696e20496d70616374",       // index 1
    "486f6e6b61693a2053746172205261696c", // index 2
    "5a656e6c657373205a6f6e65205a65726f", // index 3
];

fn decode_hex(s: &str) -> String {
    let bytes: Vec<u8> = (0..s.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes).unwrap_or_default()
}

impl GameId {
    pub const ALL: [GameId; 4] = [GameId::Bh3, GameId::Hk4e, GameId::Hkrpg, GameId::Nap];

    /// Returns the string identifier used by irmin and the Sophon API.
    pub fn as_str(self) -> &'static str {
        match self {
            GameId::Bh3 => "bh3",
            GameId::Hk4e => "hk4e",
            GameId::Hkrpg => "hkrpg",
            GameId::Nap => "nap",
        }
    }

    /// Proper game name decoded from hex at runtime.
    pub fn display_name(self) -> String {
        decode_hex(DISPLAY_NAMES[self as usize])
    }

    /// Windows executable name for launching via Proton.
    pub fn exe_name(self) -> &'static str {
        match self {
            GameId::Bh3 => "BH3.exe",
            GameId::Hk4e => "GenshinImpact.exe",
            GameId::Hkrpg => "StarRail.exe",
            GameId::Nap => "ZenlessZoneZero.exe",
        }
    }

    /// Whether this game requires Jadeite for launching.
    pub fn needs_jadeite(self) -> bool {
        matches!(self, GameId::Hkrpg)
    }
}

impl fmt::Display for GameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GameId {
    type Err = ParseGameIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bh3" => Ok(GameId::Bh3),
            "hk4e" => Ok(GameId::Hk4e),
            "hkrpg" => Ok(GameId::Hkrpg),
            "nap" => Ok(GameId::Nap),
            _ => Err(ParseGameIdError(s.to_owned())),
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown game id: {0}")]
pub struct ParseGameIdError(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_variants() {
        for game in GameId::ALL {
            let parsed: GameId = game.as_str().parse().unwrap();
            assert_eq!(parsed, game);
        }
    }

    #[test]
    fn serde_roundtrip() {
        for game in GameId::ALL {
            let json = serde_json::to_string(&game).unwrap();
            let parsed: GameId = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, game);
        }
    }

    #[test]
    fn invalid_parse() {
        assert!("invalid".parse::<GameId>().is_err());
    }
}
