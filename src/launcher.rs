use std::path::{Path, PathBuf};

use tokio::sync::mpsc::Sender;

use crate::game::GameId;

/// Launches games through Proton, with optional Jadeite injection for hkrpg.
pub struct Launcher {
    data_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("proton not installed at {0}")]
    ProtonMissing(PathBuf),
    #[error("jadeite not installed at {0}")]
    JadeiteMissing(PathBuf),
    #[error("game executable not found at {0}")]
    GameExeMissing(PathBuf),
    #[error("failed to spawn process: {0}")]
    Spawn(std::io::Error),
}

impl Launcher {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Launches the game via Proton in the background.
    /// Streams stdout/stderr lines to `log_tx` for TUI display.
    pub fn launch(
        &self,
        game: GameId,
        game_dir: &Path,
        log_tx: Sender<String>,
    ) -> Result<(), LaunchError> {
        let proton_bin = self.data_dir.join("proton").join("proton");
        if !proton_bin.exists() {
            return Err(LaunchError::ProtonMissing(proton_bin));
        }

        let exe_path = game_dir.join(game.exe_name());
        if !exe_path.exists() {
            return Err(LaunchError::GameExeMissing(exe_path));
        }

        let compat_data = self.data_dir.join("proton-data");

        let command_str = if game.needs_jadeite() {
            let jadeite_exe = self.data_dir.join("jadeite").join("jadeite.exe");
            if !jadeite_exe.exists() {
                return Err(LaunchError::JadeiteMissing(jadeite_exe));
            }
            format!(
                "{} run {} {}",
                proton_bin.display(),
                jadeite_exe.display(),
                exe_path.display()
            )
        } else {
            format!("{} run {}", proton_bin.display(), exe_path.display())
        };

        let compat_str = compat_data.to_string_lossy().to_string();

        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::process::Command;

            let mut child = match Command::new("sh")
                .arg("-c")
                .arg(&command_str)
                .env("STEAM_COMPAT_DATA_PATH", &compat_str)
                .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", "")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => return,
            };

            let mut handles = Vec::new();

            if let Some(stdout) = child.stdout.take() {
                let tx = log_tx.clone();
                handles.push(tokio::spawn(async move {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if tx.send(line).await.is_err() {
                            break;
                        }
                    }
                }));
            }

            if let Some(stderr) = child.stderr.take() {
                let tx = log_tx.clone();
                handles.push(tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if tx.send(line).await.is_err() {
                            break;
                        }
                    }
                }));
            }

            // Wait for readers to finish
            for h in handles {
                let _ = h.await;
            }
            let _ = child.wait().await;
        });

        Ok(())
    }

    pub fn proton_available(&self) -> bool {
        self.data_dir.join("proton").join("proton").exists()
    }

    pub fn jadeite_available(&self) -> bool {
        self.data_dir.join("jadeite").join("jadeite.exe").exists()
    }
}
