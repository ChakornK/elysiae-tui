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
    #[allow(dead_code)]
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

        let mut args: Vec<PathBuf> = vec!["run".into()];
        if game.needs_jadeite() {
            let jadeite_exe = self.data_dir.join("jadeite").join("jadeite.exe");
            if !jadeite_exe.exists() {
                return Err(LaunchError::JadeiteMissing(jadeite_exe));
            }
            args.push(jadeite_exe);
        }
        args.push(exe_path);

        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::process::Command;

            let mut child = match Command::new(&proton_bin)
                .args(&args)
                .env("STEAM_COMPAT_DATA_PATH", &compat_data)
                .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", "")
                .env("__NV_DISABLE_EXPLICIT_SYNC", "1")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = log_tx.send(format!("failed to spawn process: {e}")).await;
                    let _ = log_tx.send("\x00__PROCESS_EXIT__".to_owned()).await;
                    return;
                }
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
            // Signal process exit
            let _ = log_tx.send("\x00__PROCESS_EXIT__".to_owned()).await;
        });

        Ok(())
    }
}
