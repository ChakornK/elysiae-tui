use tokio::sync::watch;

/// Spawns a background task that sets the channel to `true` on SIGINT or SIGTERM.
/// A second signal forces immediate process exit.
pub fn spawn_signal_handler() -> watch::Receiver<bool> {
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm = signal(SignalKind::terminate()).ok();
        let mut sigint = signal(SignalKind::interrupt()).ok();

        // Wait for first signal
        tokio::select! {
            _ = async { sigint.as_mut().unwrap().recv().await }, if sigint.is_some() => {}
            _ = async { sigterm.as_mut().unwrap().recv().await }, if sigterm.is_some() => {}
        }
        let _ = tx.send(true);

        // Second signal: restore terminal then force exit
        tokio::select! {
            _ = async { sigint.as_mut().unwrap().recv().await }, if sigint.is_some() => {}
            _ = async { sigterm.as_mut().unwrap().recv().await }, if sigterm.is_some() => {}
        }
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen
        );
        let _ = crossterm::terminal::disable_raw_mode();
        std::process::exit(130);
    });
    rx
}
