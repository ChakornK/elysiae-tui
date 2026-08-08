use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::game::GameId;

// sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGames?launcher_id=VYTpXlbWo8
const GAMES_API_URL: &str = concat!(
    "\x68\x74\x74\x70\x73\x3a\x2f\x2f",
    "\x73\x67\x2d\x68\x79\x70\x2d\x61\x70\x69\x2e",
    "\x68\x6f\x79\x6f\x76\x65\x72\x73\x65\x2e\x63\x6f\x6d",
    "\x2f\x68\x79\x70\x2f\x68\x79\x70\x2d\x63\x6f\x6e\x6e\x65\x63\x74",
    "\x2f\x61\x70\x69\x2f\x67\x65\x74\x47\x61\x6d\x65\x73",
    "\x3f\x6c\x61\x75\x6e\x63\x68\x65\x72\x5f\x69\x64\x3d",
    "\x56\x59\x54\x70\x58\x6c\x62\x57\x6f\x38",
);

#[derive(Debug, Deserialize)]
struct ApiResponse {
    data: Option<ApiData>,
}

#[derive(Debug, Deserialize)]
struct ApiData {
    games: Vec<GameEntry>,
}

#[derive(Debug, Deserialize)]
struct GameEntry {
    id: String,
    display: DisplayInfo,
}

#[derive(Debug, Deserialize)]
struct DisplayInfo {
    background: BackgroundInfo,
}

#[derive(Debug, Deserialize)]
struct BackgroundInfo {
    url: String,
}

/// Manages downloading and caching background images for the TUI.
pub struct Backgrounds {
    cache_dir: PathBuf,
    paths: HashMap<GameId, PathBuf>,
}

impl Backgrounds {
    pub fn new(cache_dir: PathBuf) -> Self {
        let mut bg = Self {
            cache_dir,
            paths: HashMap::new(),
        };
        bg.load_cached();
        bg
    }

    /// Returns the cached background image path for a game, if available.
    pub fn get(&self, game: GameId) -> Option<&PathBuf> {
        self.paths.get(&game)
    }

    /// Populates paths from already-cached files on disk. No network.
    fn load_cached(&mut self) {
        for game in GameId::ALL {
            let local_path = self.cache_dir.join(game.as_str()).join("bg.webp");
            if local_path.exists() {
                self.paths.insert(game, local_path);
            }
        }
    }

    /// Fetches game list from the API and downloads missing background images.
    pub async fn sync(&mut self, client: &reqwest::Client) {
        let games = match fetch_games(client).await {
            Some(g) => g,
            None => return,
        };

        for entry in &games {
            let Some(game_id) = api_id_to_game(&entry.id) else { continue };
            let local_path = self.cache_dir.join(game_id.as_str()).join("bg.webp");

            if local_path.exists() {
                self.paths.insert(game_id, local_path);
                continue;
            }

            if let Some(parent) = local_path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            if let Ok(bytes) = download_file(client, &entry.display.background.url).await
                && fs::write(&local_path, &bytes).is_ok()
            {
                self.paths.insert(game_id, local_path);
            }
        }
    }
}

/// Maps the API game id to our internal GameId.
fn api_id_to_game(id: &str) -> Option<GameId> {
    match id {
        "5TIVvvcwtM" => Some(GameId::Bh3),
        "gopR6Cufr3" => Some(GameId::Hk4e),
        "4ziysqXOQ8" => Some(GameId::Hkrpg),
        "U5hbdsT9W7" => Some(GameId::Nap),
        _ => None,
    }
}

async fn fetch_games(client: &reqwest::Client) -> Option<Vec<GameEntry>> {
    let resp: ApiResponse = client
        .get(GAMES_API_URL)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    resp.data.map(|d| d.games)
}

/// Max background image size (20 MB) to prevent OOM on malformed responses.
const MAX_BG_SIZE: u64 = 20 * 1024 * 1024;

async fn download_file(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, reqwest::Error> {
    let resp = client.get(url).send().await?.error_for_status()?;
    if let Some(len) = resp.content_length() {
        if len > MAX_BG_SIZE {
            return Ok(Vec::new());
        }
    }
    let bytes = resp.bytes().await?;
    if bytes.len() as u64 > MAX_BG_SIZE {
        return Ok(Vec::new());
    }
    Ok(bytes.to_vec())
}
