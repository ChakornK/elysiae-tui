use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::atomic;

/// Type alias for the state-saver callback.
pub type StateSaverFn = Arc<dyn Fn(&HashMap<String, u64>) + Send + Sync>;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum DownloadType {
    Fresh,
    Update,
    Preinstall,
}

/// Persisted state for resuming interrupted downloads.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DownloadState {
    pub game_id: String,
    pub download_type: DownloadType,
    pub manifest_hash: String,
    pub downloaded_chunks: HashMap<String, u64>,
    pub output_path: PathBuf,
    pub vo_lang: String,
}

impl DownloadState {
    pub fn state_path(data_dir: &Path, game_id: &str) -> PathBuf {
        data_dir.join(format!(".sophon_state_{game_id}.json"))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        atomic::atomic_write(path, &json)
    }

    pub fn load(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub fn remove(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

/// Creates a closure that persists chunk progress to disk on each callback.
/// Logs a single warning on save failure to avoid log spam.
pub fn make_state_saver(initial: DownloadState, path: PathBuf) -> StateSaverFn {
    let state = Arc::new(Mutex::new(initial));
    let path = Arc::new(path);
    let save_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    Arc::new(move |chunks: &HashMap<String, u64>| {
        let mut s = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::error!("state mutex poisoned, recovering");
                poisoned.into_inner()
            }
        };
        s.downloaded_chunks = chunks.clone();
        if let Err(e) = s.save(&path) {
            if !save_failed.swap(true, std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!("failed to persist download state: {e}");
            }
        } else {
            save_failed.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    })
}
