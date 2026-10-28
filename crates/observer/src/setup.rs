use std::env;
use std::net::{IpAddr, SocketAddr};

use anyhow::bail;
use observer_app::Accounts;
use observer_domain::audit::ActorKind;
use tracing::info;

use crate::config::Config;
use crate::user;

/// At start, while no account exists, prints the link that creates the owner. It goes to the
/// terminal only, never to the logs: whoever holds it can take the server. Under systemd nobody
/// reads the terminal, so the operator asks for a link with `observer setup-link` instead.
pub async fn offer_link(accounts: &Accounts, listen: SocketAddr) -> anyhow::Result<()> {
    if under_systemd() {
        if accounts.setup_required().await? {
            info!("no account exists yet; run `observer setup-link` to create the owner");
        }
        return Ok(());
    }
    if let Some(token) = accounts.create_setup_token(ActorKind::System).await? {
        print_link(listen, &token);
    }
    Ok(())
}

/// `observer setup-link`: a fresh link, which the running Observer accepts like its own.
pub async fn link(config: &Config) -> anyhow::Result<()> {
    let (db, accounts) = user::open(config).await?;
    let token = accounts.create_setup_token(ActorKind::Cli).await?;
    db.close().await?;
    match token {
        Some(token) => print_link(config.http.listen, &token),
        None => bail!("the owner already exists; `observer user reset-password <name>` lets them back in"),
    }
    Ok(())
}

fn print_link(listen: SocketAddr, token: &str) {
    let host = match listen.ip() {
        ip if ip.is_unspecified() => "localhost".to_owned(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    println!("Create the owner account within 30 minutes, from this link:");
    println!("http://{host}:{}/setup#{token}", listen.port());
}

fn under_systemd() -> bool {
    env::var_os("INVOCATION_ID").is_some() || env::var_os("NOTIFY_SOCKET").is_some()
}
