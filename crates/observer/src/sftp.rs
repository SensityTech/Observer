//! Keeps SFTP's port as its switch and its address say: open while the feature runs, on the
//! address in force, closed the rest of the time. Neither needs Observer restarted.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use observer_app::{Features, Files, Switched};
use observer_domain::features::Feature;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// The port while it is open.
pub(crate) struct Serving {
    at: SocketAddr,
    /// The fingerprint of the host key clients are shown.
    fingerprint: String,
    switched: Switched,
}

/// What serving SFTP takes.
pub(crate) struct Port {
    pub(crate) features: Features,
    pub(crate) files: Files,
    /// Observer's own folders and files, which no session reaches.
    pub(crate) refused: Vec<PathBuf>,
}

impl Port {
    /// Opens the port on `at` and serves SFTP there, until the feature is turned off or what is
    /// returned is closed.
    pub(crate) async fn open(&self, at: SocketAddr) -> anyhow::Result<Serving> {
        let host_key = observer_sftp::host_key(&self.files)
            .await
            .context("cannot read or create the SFTP host key")?;
        let listener = TcpListener::bind(at).await?;
        let fingerprint = host_key.fingerprint(observer_sftp::FINGERPRINT).to_string();
        info!(listen = %at, %fingerprint, "SFTP listening");
        let closing = CancellationToken::new();
        let task = tokio::spawn(observer_sftp::serve(
            listener,
            self.files.clone(),
            host_key,
            self.refused.clone(),
            closing.clone(),
        ));
        let switched = self.features.switched(Feature::Sftp, move || async move {
            closing.cancel();
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => error!(%error, "the SFTP server stopped with an error"),
                Err(error) => error!(%error, "the SFTP server task failed"),
            }
        });
        Ok(Serving {
            at,
            fingerprint,
            switched,
        })
    }
}

/// Opens, moves and closes the port until `stop`. `first` is the port `observer.toml` names,
/// which Observer's start opened already.
pub(crate) async fn keep(port: Port, first: Option<Serving>, stop: CancellationToken) {
    let mut running = port.features.running();
    let mut listen = port.features.sftp_listen();
    let mut serving = first;
    if let Some(serving) = &serving {
        port.features.sftp_opened(serving.at, serving.fingerprint.clone());
    }
    loop {
        let on = running.borrow_and_update().contains(&Feature::Sftp);
        let at = *listen.borrow_and_update();
        let wanted = if on { at } else { None };
        if serving.as_ref().map(|serving| serving.at) != wanted {
            if let Some(serving) = serving.take() {
                serving.switched.close().await;
                port.features.sftp_closed();
            }
            if let Some(at) = wanted {
                match port.open(at).await {
                    Ok(opened) => {
                        port.features.sftp_opened(at, opened.fingerprint.clone());
                        serving = Some(opened);
                    }
                    Err(error) => {
                        let error = format!("{error:#}");
                        warn!(listen = %at, %error, "SFTP's port did not open");
                        port.features.sftp_refused(at, error);
                    }
                }
            }
        }
        tokio::select! {
            () = stop.cancelled() => break,
            changed = running.changed() => if changed.is_err() { break },
            changed = listen.changed() => if changed.is_err() { break },
        }
    }
    if let Some(serving) = serving {
        serving.switched.close().await;
    }
}
