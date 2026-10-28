/// Resolves when the operator asks Observer to stop: Ctrl+C everywhere, plus SIGTERM (service
/// managers) and SIGHUP (a closed SSH session) on Unix.
pub async fn requested() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let (Ok(mut terminate), Ok(mut hangup)) =
            (signal(SignalKind::terminate()), signal(SignalKind::hangup()))
        else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
