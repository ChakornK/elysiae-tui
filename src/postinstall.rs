use std::path::Path;

use irmin::SophonProgress;
use tokio::sync::mpsc;

/// Runs plugin and channel SDK installation after a successful game operation.
pub async fn run_post_install(
    client: &reqwest::Client,
    game_dir: &Path,
    game_id: &str,
    progress_tx: mpsc::Sender<SophonProgress>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tx1 = progress_tx.clone();
    let tx2 = progress_tx;

    irmin::game_installer::install_plugins(client, game_dir, game_id, move |p| {
        let _ = tx1.try_send(p);
    })
    .await?;

    irmin::game_installer::install_channel_sdks(client, game_dir, game_id, move |p| {
        let _ = tx2.try_send(p);
    })
    .await?;

    Ok(())
}
