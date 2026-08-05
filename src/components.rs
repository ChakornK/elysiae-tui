use std::path::PathBuf;

use serde::Deserialize;
use tokio::sync::mpsc::Sender;

/// Progress events for component downloads.
#[derive(Debug, Clone)]
pub enum ComponentProgress {
    Downloading { downloaded_bytes: u64, total_bytes: u64 },
    Extracting,
    Finished { tag: String },
    Error { message: String },
}

/// Metadata returned by the Aedes component API.
#[derive(Debug, Deserialize)]
struct ModuleData {
    download_url: String,
    hash: String,
    tag: String,
}

/// Manages Proton and Jadeite downloads from the Aedes CDN.
pub struct ComponentManager {
    client: reqwest::Client,
    data_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

const AEDES_BASE: &str = "https://aedes.elysiae.app";

impl ComponentManager {
    pub fn new(client: reqwest::Client, data_dir: PathBuf) -> Self {
        Self { client, data_dir }
    }

    /// Fetches the latest tag for a component. Returns None if already up-to-date.
    pub async fn check_update(
        &self,
        component: &str,
        installed_tag: &Option<String>,
    ) -> Result<Option<String>, ComponentError> {
        let url = format!("{}/{}.json", AEDES_BASE, component);
        let meta: Vec<ModuleData> = self.client.get(&url).send().await?.json().await?;
        let remote_tag = &meta[0].tag;
        if installed_tag.as_deref() == Some(remote_tag) {
            return Ok(None);
        }
        Ok(Some(remote_tag.clone()))
    }

    /// Downloads and installs Proton. Returns the installed tag.
    pub async fn install_proton(
        &self,
        tx: Sender<ComponentProgress>,
    ) -> Result<String, ComponentError> {
        self.install_component("proton", "proton", tx).await
    }

    /// Downloads and installs Jadeite. Returns the installed tag.
    pub async fn install_jadeite(
        &self,
        tx: Sender<ComponentProgress>,
    ) -> Result<String, ComponentError> {
        self.install_component("jadeite", "jadeite", tx).await
    }

    async fn install_component(
        &self,
        name: &str,
        extract_dir: &str,
        tx: Sender<ComponentProgress>,
    ) -> Result<String, ComponentError> {
        let url = format!("{}/{}.json", AEDES_BASE, name);
        let meta: Vec<ModuleData> = self.client.get(&url).send().await?.json().await?;
        let module = &meta[0];

        // Download the archive
        let mut response = self.client.get(&module.download_url).send().await?;
        let total = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let dest_dir = self.data_dir.join(extract_dir);
        std::fs::create_dir_all(&dest_dir)?;

        let archive_path = self.data_dir.join(format!("{}.archive", name));
        let mut file = std::fs::File::create(&archive_path)?;

        use std::io::Write;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            let _ = tx.try_send(ComponentProgress::Downloading {
                downloaded_bytes: downloaded,
                total_bytes: total,
            });
        }
        drop(file);

        let _ = tx.try_send(ComponentProgress::Extracting);

        // Extract based on file type
        if name == "proton" {
            extract_tar_gz(&archive_path, &dest_dir)?;
        } else {
            extract_zip(&archive_path, &dest_dir)?;
        }

        // Run post-install for jadeite
        if name == "jadeite" {
            let script = dest_dir.join("block_analytics.sh");
            if script.exists() {
                std::process::Command::new("sh")
                    .arg(&script)
                    .current_dir(&dest_dir)
                    .status()?;
            }
        }

        // Create proton-data dir for proton
        if name == "proton" {
            std::fs::create_dir_all(self.data_dir.join("proton-data"))?;
        }

        // Cleanup archive
        let _ = std::fs::remove_file(&archive_path);

        let tag = module.tag.clone();
        let _ = tx.try_send(ComponentProgress::Finished { tag: tag.clone() });
        Ok(tag)
    }
}

fn extract_tar_gz(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), ComponentError> {
    use std::process::Command;
    Command::new("tar")
        .args(["xzf", archive.to_str().unwrap_or_default(), "-C", dest.to_str().unwrap_or_default(), "--strip-components=1"])
        .status()
        .map_err(ComponentError::Io)?;
    Ok(())
}

fn extract_zip(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), ComponentError> {
    use std::process::Command;
    Command::new("unzip")
        .args(["-o", archive.to_str().unwrap_or_default(), "-d", dest.to_str().unwrap_or_default()])
        .status()
        .map_err(ComponentError::Io)?;
    Ok(())
}
