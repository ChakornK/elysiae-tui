use tokio::sync::watch;

/// Spawns a background task that sets the channel to `true` on SIGINT (Ctrl+C).
pub fn spawn_signal_handler() -> watch::Receiver<bool> {
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = tx.send(true);
    });
    rx
}
