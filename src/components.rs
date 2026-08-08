use std::path::PathBuf;

use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::Sender;

/// Progress events for component downloads.
#[derive(Debug, Clone)]
pub enum ComponentProgress {
    Downloading { downloaded_bytes: u64, total_bytes: u64 },
    Extracting,
    #[allow(dead_code)]
    Finished { tag: String },
    #[allow(dead_code)]
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

/// Selects the appropriate module entry for the current architecture.
/// ARM builds contain "aarch64" in the URL; x86_64 builds do not.
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

/// Returns the correct download URL for the current architecture.
/// The Aedes API may serve architecture-specific URLs, or we override
/// to get the correct GE-Proton build for x86_64 vs aarch64.
fn resolve_download_url(module: &ModuleData, name: &str) -> String {
    let arch = std::env::consts::ARCH;
    // If the URL already matches our arch, use it directly
    if arch == "aarch64" && module.download_url.contains("aarch64") {
        return module.download_url.clone();
    }
    if arch == "x86_64" && !module.download_url.contains("aarch64") {
        return module.download_url.clone();
    }
    // Override: swap architecture in the GitHub release URL
    if name == "proton" && arch == "x86_64" && module.download_url.contains("aarch64") {
        return module.download_url.replace("-aarch64.tar.gz", ".tar.gz");
    }
    if name == "proton" && arch == "aarch64" && !module.download_url.contains("aarch64") {
        return module.download_url.replace(".tar.gz", "-aarch64.tar.gz");
    }
    module.download_url.clone()
}

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
        let meta: Vec<ModuleData> = self.client.get(&url).send().await?.error_for_status()?.json().await?;
        let module = meta.first().ok_or_else(|| {
            ComponentError::Other(format!("no metadata available for {}", name))
        })?;

        let download_url = resolve_download_url(module, name);
        let mut response = self.client.get(&download_url).send().await?.error_for_status()?;
        let total = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let dest_dir = self.data_dir.join(extract_dir);
        let archive_path = self.data_dir.join(format!("{}.archive", name));

        // Ensure parent dir exists but don't create dest_dir yet (extraction creates it)
        if let Some(parent) = archive_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::File::create(&archive_path).await?;

        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            let _ = tx.try_send(ComponentProgress::Downloading {
                downloaded_bytes: downloaded,
                total_bytes: total,
            });
        }
        drop(file);

        // Verify download completed fully — partial files cause corrupt extraction
        if total == 0 {
            tracing::warn!("server did not provide Content-Length, skipping size verification");
        }
        if total > 0 && downloaded != total {
            let _ = std::fs::remove_file(&archive_path);
            return Err(ComponentError::Other(format!(
                "{} download incomplete: got {} of {} bytes",
                name, downloaded, total
            )));
        }

        // Flush channel before sending extracting state
        let _ = tx.send(ComponentProgress::Extracting).await;

        // Create dest dir for extraction
        std::fs::create_dir_all(&dest_dir)?;

        // Run extraction on a blocking thread to avoid stalling the async runtime
        let archive_clone = archive_path.clone();
        let dest_clone = dest_dir.clone();
        let extract_name = name.to_owned();
        let extract_result = tokio::task::spawn_blocking(move || {
            if extract_name == "proton" {
                extract_tar_gz(&archive_clone, &dest_clone)
            } else {
                extract_zip(&archive_clone, &dest_clone)
            }
        })
        .await
        .map_err(|e| ComponentError::Other(format!("extraction task failed: {e}")))?;

        // On extraction failure, clean up dest dir so future installs aren't blocked
        if let Err(e) = extract_result {
            let _ = crate::atomic::safe_remove_dir_all(&dest_dir);
            let _ = std::fs::remove_file(&archive_path);
            return Err(e);
        }

        // Post-install for jadeite
        if name == "jadeite" {
            let script = dest_dir.join("block_analytics.sh");
            if script.exists() {
                let status = tokio::process::Command::new("sh")
                    .arg(&script)
                    .current_dir(&dest_dir)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await?;
                if !status.success() {
                    let _ = crate::atomic::safe_remove_dir_all(&dest_dir);
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
        // Persist tag so the main thread can update config after Finished
        let tag_path = self.data_dir.join(format!("{}.tag", name));
        let _ = std::fs::write(&tag_path, &tag);
        let _ = tx.try_send(ComponentProgress::Finished { tag: tag.clone() });
        Ok(tag)
    }
}

/// Reads a persisted component tag file (e.g. `proton.tag`).
pub fn read_component_tag(data_dir: &std::path::Path, name: &str) -> Option<String> {
    let path = data_dir.join(format!("{}.tag", name));
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

/// Checks if a component is outdated by comparing the local tag against the Aedes API.
/// Returns `true` if an update is available (remote tag differs from installed tag).
/// Returns `false` if up-to-date or if the check fails (network error, etc.).
pub async fn component_needs_update(
    client: &reqwest::Client,
    data_dir: &std::path::Path,
    name: &str,
) -> bool {
    let installed_tag = match read_component_tag(data_dir, name) {
        Some(tag) => tag,
        None => return false, // Not installed — handled by availability checks
    };
    let url = format!("{}/{}.json", AEDES_BASE, name);
    let meta: Vec<ModuleData> = match client.get(&url).send().await {
        Ok(resp) => match resp.json().await {
            Ok(m) => m,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    match meta.first() {
        Some(module) => module.tag != installed_tag,
        None => false,
    }
}

/// Checks whether Proton is installed and has correct architecture.
pub fn proton_available(data_dir: &std::path::Path) -> bool {
    let proton_dir = data_dir.join("proton");
    let proton_data = data_dir.join("proton-data");
    if !proton_dir.exists() || !proton_data.exists() {
        return false;
    }
    if !std::fs::read_dir(&proton_dir)
        .map(|mut d| d.next().is_some())
        .unwrap_or(false)
    {
        return false;
    }
    // Verify wine binary matches host architecture
    let wine_bin = proton_dir.join("files").join("bin").join("wine");
    if !wine_bin.exists() {
        return false;
    }
    is_correct_arch(&wine_bin)
}

/// Checks ELF header to verify binary matches the current architecture.
fn is_correct_arch(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut header = [0u8; 20];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    // ELF magic: 0x7f 'E' 'L' 'F'
    if &header[0..4] != b"\x7fELF" {
        return false;
    }
    // e_machine at offset 18 (little-endian u16)
    let machine = u16::from_le_bytes([header[18], header[19]]);
    match std::env::consts::ARCH {
        "x86_64" => machine == 62,   // EM_X86_64
        "aarch64" => machine == 183, // EM_AARCH64
        _ => true, // Unknown arch — assume OK
    }
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
        fs::create_dir_all(proton.join("files").join("bin")).unwrap();
        // Write a fake ELF with correct arch header
        let mut elf = vec![0x7f, b'E', b'L', b'F', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        // e_machine at offset 18: EM_X86_64=62 or EM_AARCH64=183
        let machine: u16 = if std::env::consts::ARCH == "aarch64" { 183 } else { 62 };
        elf.extend_from_slice(&machine.to_le_bytes());
        fs::write(proton.join("files").join("bin").join("wine"), &elf).unwrap();
        fs::write(proton.join("proton"), "script").unwrap();
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
