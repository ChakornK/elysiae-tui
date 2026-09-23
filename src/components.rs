use std::path::{Path, PathBuf};

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

#[derive(Debug)]
struct ModuleData {
    download_url: String,
    checksum: String,
    tag: String,
}

/// Metadata returned by the Aedes v3 component API.
#[derive(Debug, Deserialize)]
struct AedesComponentData {
    tag: String,
    download: AedesDownloads,
}

#[derive(Debug, Deserialize)]
struct AedesDownloads {
    amd64: AedesDownload,
    aarch64: AedesDownload,
}

#[derive(Debug, Deserialize)]
struct AedesDownload {
    url: String,
    checksum: String,
}

/// Manages Phlogiston runtime installations.
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

const AEDES_BASE: &str = "https://aedes.elysiae.app";

fn phlogiston_metadata_url(arch: &str) -> Result<String, ComponentError> {
    let arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "aarch64",
        _ => {
            return Err(ComponentError::Other(format!(
                "unsupported component architecture: {arch}"
            )));
        }
    };
    Ok(format!(
        "{AEDES_BASE}/getComponentInfo?component=phlogiston&arch={arch}&latest=true"
    ))
}

fn select_phlogiston_download(
    data: AedesComponentData,
    arch: &str,
) -> Result<ModuleData, ComponentError> {
    let download = match arch {
        "x86_64" => data.download.amd64,
        "aarch64" => data.download.aarch64,
        _ => {
            return Err(ComponentError::Other(format!(
                "unsupported component architecture: {arch}"
            )));
        }
    };

    Ok(ModuleData {
        download_url: download.url,
        checksum: download.checksum,
        tag: data.tag,
    })
}

fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn fetch_module_data(client: &reqwest::Client) -> Result<ModuleData, ComponentError> {
    let url = phlogiston_metadata_url(std::env::consts::ARCH)?;
    let data = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    select_phlogiston_download(data, std::env::consts::ARCH)
}

impl ComponentManager {
    pub fn new(client: reqwest::Client, data_dir: PathBuf) -> Self {
        Self { client, data_dir }
    }

    /// Downloads and installs the Proton-compatible Phlogiston runtime.
    pub async fn install_proton(
        &self,
        tx: Sender<ComponentProgress>,
    ) -> Result<String, ComponentError> {
        self.install_component(tx).await
    }

    async fn install_component(
        &self,
        tx: Sender<ComponentProgress>,
    ) -> Result<String, ComponentError> {
        let module = fetch_module_data(&self.client).await?;
        let download_url = module.download_url.as_str();

        // Pre-flight: verify extraction tool exists before downloading
        if std::process::Command::new("which")
            .arg("tar")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| !s.success())
            .unwrap_or(true)
        {
            return Err(ComponentError::Other(
                "'tar' is not installed — required to extract Phlogiston".to_owned(),
            ));
        }

        let mut response = self
            .client
            .get(download_url)
            .send()
            .await?
            .error_for_status()?;
        let total = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let dest_dir = self.data_dir.join("proton");
        let archive_path = self.data_dir.join("proton.archive");

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
                "Phlogiston download incomplete: got {} of {} bytes",
                downloaded, total
            )));
        }

        if !module.checksum.is_empty() {
            let expected_checksum = module.checksum.clone();
            let archive_for_hash = archive_path.clone();
            let actual_checksum =
                tokio::task::spawn_blocking(move || sha256_file(&archive_for_hash))
                    .await
                    .map_err(|e| ComponentError::Other(format!("checksum task failed: {e}")))?
                    .map_err(ComponentError::Io)?;

            if actual_checksum != expected_checksum {
                let _ = std::fs::remove_file(&archive_path);
                return Err(ComponentError::Other(format!(
                    "Phlogiston checksum mismatch: expected {}, got {}",
                    expected_checksum, actual_checksum
                )));
            }
        }

        // Flush channel before sending extracting state
        let _ = tx.send(ComponentProgress::Extracting).await;

        // Create dest dir for extraction
        std::fs::create_dir_all(&dest_dir)?;

        // Run extraction on a blocking thread to avoid stalling the async runtime
        let archive_clone = archive_path.clone();
        let dest_clone = dest_dir.clone();
        let extract_result =
            tokio::task::spawn_blocking(move || extract_tar_gz(&archive_clone, &dest_clone))
                .await
                .map_err(|e| ComponentError::Other(format!("extraction task failed: {e}")))?;

        // On extraction failure, clean up dest dir so future installs aren't blocked
        if let Err(e) = extract_result {
            let _ = crate::atomic::safe_remove_dir_all(&dest_dir);
            let _ = std::fs::remove_file(&archive_path);
            return Err(e);
        }

        std::fs::create_dir_all(self.data_dir.join("proton-data"))?;

        let _ = std::fs::remove_file(&archive_path);

        let tag = module.tag.clone();
        // Persist tag so the main thread can update config after Finished
        let tag_path = self.data_dir.join("proton.tag");
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

/// Checks whether Phlogiston is outdated by comparing local and remote tags.
/// Returns `true` if an update is available (remote tag differs from installed tag).
/// Returns `false` if up-to-date or if the check fails (network error, etc.).
pub async fn proton_needs_update(
    client: &reqwest::Client,
    data_dir: &std::path::Path,
) -> bool {
    let installed_tag = match read_component_tag(data_dir, "proton") {
        Some(tag) => tag,
        None => return false, // Not installed — handled by availability checks
    };
    match fetch_module_data(client).await {
        Ok(module) => module.tag != installed_tag,
        Err(_) => false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn phlogiston_metadata() -> AedesComponentData {
        serde_json::from_str(
            r#"{
                "tag": "11-0",
                "download": {
                    "amd64": {
                        "url": "https://example.com/phlogiston-x86_64.tar.gz",
                        "checksum": "amd64-checksum"
                    },
                    "aarch64": {
                        "url": "https://example.com/phlogiston-aarch64.tar.gz",
                        "checksum": "aarch64-checksum"
                    }
                },
                "prerelease": false
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn builds_aedes_v3_phlogiston_metadata_url() {
        assert_eq!(
            phlogiston_metadata_url("x86_64").unwrap(),
            "https://aedes.elysiae.app/getComponentInfo?component=phlogiston&arch=amd64&latest=true"
        );
    }

    #[test]
    fn selects_amd64_phlogiston_download_for_x86_64() {
        let module = select_phlogiston_download(phlogiston_metadata(), "x86_64").unwrap();
        assert_eq!(
            module.download_url,
            "https://example.com/phlogiston-x86_64.tar.gz"
        );
        assert_eq!(module.checksum, "amd64-checksum");
        assert_eq!(module.tag, "11-0");
    }

    #[test]
    fn selects_aarch64_phlogiston_download_for_aarch64() {
        let module = select_phlogiston_download(phlogiston_metadata(), "aarch64").unwrap();
        assert_eq!(
            module.download_url,
            "https://example.com/phlogiston-aarch64.tar.gz"
        );
        assert_eq!(module.checksum, "aarch64-checksum");
        assert_eq!(module.tag, "11-0");
    }

    #[test]
    fn rejects_unsupported_component_architecture() {
        let error = select_phlogiston_download(phlogiston_metadata(), "riscv64").unwrap_err();
        assert_eq!(
            error.to_string(),
            "unsupported component architecture: riscv64"
        );
    }

    #[test]
    fn computes_sha256_checksum() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("archive");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

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
}
