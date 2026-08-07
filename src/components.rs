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
    #[allow(dead_code)]
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
    #[error("{0}")]
    Other(String),
}

const AEDES_BASE: &str = "https://aedes.elysiae.app/components";

impl ComponentManager {
    pub fn new(client: reqwest::Client, data_dir: PathBuf) -> Self {
        Self { client, data_dir }
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
        let module = meta.first().ok_or_else(|| {
            ComponentError::Other(format!("no metadata available for {}", name))
        })?;

        let mut response = self.client.get(&module.download_url).send().await?;
        let total = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let dest_dir = self.data_dir.join(extract_dir);
        let archive_path = self.data_dir.join(format!("{}.archive", name));

        // Ensure parent dir exists but don't create dest_dir yet (extraction creates it)
        if let Some(parent) = archive_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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

        // Create dest dir for extraction
        std::fs::create_dir_all(&dest_dir)?;

        let extract_result = if name == "proton" {
            extract_tar_gz(&archive_path, &dest_dir)
        } else {
            extract_zip(&archive_path, &dest_dir)
        };

        // On extraction failure, clean up dest dir so future installs aren't blocked
        if let Err(e) = extract_result {
            let _ = std::fs::remove_dir_all(&dest_dir);
            let _ = std::fs::remove_file(&archive_path);
            return Err(e);
        }

        // Post-install for jadeite
        if name == "jadeite" {
            let script = dest_dir.join("block_analytics.sh");
            if script.exists() {
                let status = std::process::Command::new("sh")
                    .arg(&script)
                    .current_dir(&dest_dir)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()?;
                if !status.success() {
                    let _ = std::fs::remove_dir_all(&dest_dir);
                    return Err(ComponentError::Other(
                        "block_analytics.sh failed".to_owned(),
                    ));
                }
            }
        }

        // Create proton-data dir
        if name == "proton" {
            std::fs::create_dir_all(self.data_dir.join("proton-data"))?;
        }

        let _ = std::fs::remove_file(&archive_path);

        let tag = module.tag.clone();
        let _ = tx.try_send(ComponentProgress::Finished { tag: tag.clone() });
        Ok(tag)
    }
}

/// Checks whether Proton directories exist and are non-empty.
pub fn proton_available(data_dir: &std::path::Path) -> bool {
    let proton_dir = data_dir.join("proton");
    let proton_data = data_dir.join("proton-data");
    proton_dir.exists()
        && proton_data.exists()
        && std::fs::read_dir(&proton_dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
}

/// Checks whether Jadeite directory exists and is non-empty.
pub fn jadeite_available(data_dir: &std::path::Path) -> bool {
    let jadeite_dir = data_dir.join("jadeite");
    jadeite_dir.exists()
        && std::fs::read_dir(&jadeite_dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
}

fn extract_tar_gz(
    archive: &std::path::Path,
    dest: &std::path::Path,
) -> Result<(), ComponentError> {
    use std::process::{Command, Stdio};
    let status = Command::new("tar")
        .args([
            "xzf",
            archive.to_str().unwrap_or_default(),
            "-C",
            dest.to_str().unwrap_or_default(),
            "--strip-components=1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(ComponentError::Io)?;
    if !status.success() {
        return Err(ComponentError::Other(format!(
            "tar extraction failed with exit code {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

fn extract_zip(archive: &std::path::Path, dest: &std::path::Path) -> Result<(), ComponentError> {
    use std::process::{Command, Stdio};
    let status = Command::new("unzip")
        .args([
            "-o",
            archive.to_str().unwrap_or_default(),
            "-d",
            dest.to_str().unwrap_or_default(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(ComponentError::Io)?;
    if !status.success() {
        return Err(ComponentError::Other(format!(
            "unzip extraction failed with exit code {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn proton_available_returns_false_for_empty_dir() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("proton")).unwrap();
        fs::create_dir_all(tmp.path().join("proton-data")).unwrap();
        assert!(!proton_available(tmp.path()));
    }

    #[test]
    fn proton_available_returns_false_when_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(!proton_available(tmp.path()));
    }

    #[test]
    fn proton_available_returns_true_when_populated() {
        let tmp = TempDir::new().unwrap();
        let proton = tmp.path().join("proton");
        fs::create_dir_all(&proton).unwrap();
        fs::write(proton.join("proton"), "binary").unwrap();
        fs::create_dir_all(tmp.path().join("proton-data")).unwrap();
        assert!(proton_available(tmp.path()));
    }

    #[test]
    fn jadeite_available_returns_false_for_empty_dir() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("jadeite")).unwrap();
        assert!(!jadeite_available(tmp.path()));
    }

    #[test]
    fn jadeite_available_returns_false_when_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(!jadeite_available(tmp.path()));
    }

    #[test]
    fn jadeite_available_returns_true_when_populated() {
        let tmp = TempDir::new().unwrap();
        let jadeite = tmp.path().join("jadeite");
        fs::create_dir_all(&jadeite).unwrap();
        fs::write(jadeite.join("jadeite.exe"), "binary").unwrap();
        assert!(jadeite_available(tmp.path()));
    }

    #[test]
    fn extract_tar_gz_fails_on_invalid_archive() {
        let tmp = TempDir::new().unwrap();
        let archive = tmp.path().join("bad.tar.gz");
        fs::write(&archive, "not a real archive").unwrap();
        let dest = tmp.path().join("output");
        fs::create_dir_all(&dest).unwrap();
        let result = extract_tar_gz(&archive, &dest);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tar extraction failed"));
    }

    #[test]
    fn extract_zip_fails_on_invalid_archive() {
        let tmp = TempDir::new().unwrap();
        let archive = tmp.path().join("bad.zip");
        fs::write(&archive, "not a real archive").unwrap();
        let dest = tmp.path().join("output");
        fs::create_dir_all(&dest).unwrap();
        let result = extract_zip(&archive, &dest);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unzip extraction failed"));
    }

    #[test]
    fn extract_tar_gz_succeeds_on_valid_archive() {
        let tmp = TempDir::new().unwrap();
        let archive = tmp.path().join("test.tar.gz");
        let dest = tmp.path().join("output");
        fs::create_dir_all(&dest).unwrap();

        // Create a valid tar.gz with a single file
        let inner_dir = tmp.path().join("inner");
        fs::create_dir_all(&inner_dir).unwrap();
        fs::write(inner_dir.join("hello.txt"), "world").unwrap();
        let status = std::process::Command::new("tar")
            .args(["czf", archive.to_str().unwrap(), "-C", tmp.path().to_str().unwrap(), "inner"])
            .status()
            .unwrap();
        assert!(status.success());

        let result = extract_tar_gz(&archive, &dest);
        assert!(result.is_ok());
        assert!(dest.join("hello.txt").exists());
    }

    #[test]
    fn extract_zip_succeeds_on_valid_archive() {
        let tmp = TempDir::new().unwrap();
        let archive = tmp.path().join("test.zip");
        let dest = tmp.path().join("output");
        fs::create_dir_all(&dest).unwrap();

        // Create a valid zip with a single file
        let src_file = tmp.path().join("hello.txt");
        fs::write(&src_file, "world").unwrap();
        let status = std::process::Command::new("zip")
            .args(["-j", archive.to_str().unwrap(), src_file.to_str().unwrap()])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());

        let result = extract_zip(&archive, &dest);
        assert!(result.is_ok());
        assert!(dest.join("hello.txt").exists());
    }
}
