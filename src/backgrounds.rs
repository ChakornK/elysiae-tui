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
    /// Remote filename currently cached per game, for change detection.
    current: HashMap<GameId, String>,
}

impl Backgrounds {
    pub fn new(cache_dir: PathBuf) -> Self {
        let mut bg = Self {
            cache_dir,
            paths: HashMap::new(),
            current: HashMap::new(),
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
            let dir = self.cache_dir.join(game.as_str());
            let local_path = dir.join("bg.webp");
            if local_path.exists() {
                if let Ok(name) = fs::read_to_string(dir.join("bg.src"))
                    && !name.trim().is_empty()
                {
                    self.current.insert(game, name.trim().to_owned());
                }
                self.paths.insert(game, local_path);
            }
        }
    }

    /// Fetches the game list, downloads any background whose remote filename
    /// differs from the one currently cached, and returns the changed games.
    pub async fn sync(&mut self, client: &reqwest::Client) -> Vec<GameId> {
        let Some(games) = fetch_games(client).await else {
            return Vec::new();
        };

        let mut changed = Vec::new();
        for entry in &games {
            let Some(game_id) = api_id_to_game(&entry.id) else { continue };
            let Some(remote_name) = filename_of(&entry.display.background.url) else { continue };

            let dir = self.cache_dir.join(game_id.as_str());
            let local_path = dir.join("bg.webp");

            // Already have this exact remote background — nothing to do.
            if self.current.get(&game_id).is_some_and(|c| c == &remote_name)
                && local_path.exists()
            {
                self.paths.insert(game_id, local_path);
                continue;
            }

            if let Some(parent) = local_path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            if let Some(bytes) = download_file(client, &entry.display.background.url).await
                && !bytes.is_empty()
                && fs::write(&local_path, &bytes).is_ok()
            {
                self.paths.insert(game_id, local_path);
                self.current.insert(game_id, remote_name.clone());
                let _ = fs::write(dir.join("bg.src"), &remote_name);
                changed.push(game_id);
            }
        }
        changed
    }
}

/// Extracts the filename from a remote image URL, e.g. `.../44c0....webp` -> `44c0....webp`.
fn filename_of(url: &str) -> Option<String> {
    let path = url.split('?').next()?;
    path.rsplit('/').next().filter(|n| !n.is_empty()).map(str::to_owned)
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
const MAX_BG_SIZE: usize = 20 * 1024 * 1024;

/// Downloads a file with a streaming size cap. Returns None if oversized.
async fn download_file(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let mut resp = client.get(url).send().await.ok()?.error_for_status().ok()?;
    if resp.content_length().is_some_and(|len| len > MAX_BG_SIZE as u64) {
        return None;
    }
    let mut buf = Vec::with_capacity(
        resp.content_length().unwrap_or(8192).min(MAX_BG_SIZE as u64) as usize
    );
    while let Some(chunk) = resp.chunk().await.ok()? {
        if buf.len() + chunk.len() > MAX_BG_SIZE {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    Some(buf)
}
