use std::env;
use std::net::{IpAddr, SocketAddr};

use anyhow::bail;
use observer_app::{Accounts, SetupToken};
use observer_domain::audit::ActorKind;
use tracing::info;

use crate::config::Config;
use crate::user;

/// At start, while no account exists, prints the link that creates the owner, and hands it back
/// for the status window. It goes to the terminal only, never to the logs: whoever holds it can
/// take the server. Under systemd nobody reads the terminal, so the operator asks for a link with
/// `observer setup-link` instead.
pub async fn offer_link(accounts: &Accounts, listen: SocketAddr) -> anyhow::Result<Option<SetupToken>> {
    if under_systemd() {
        if accounts.setup_required().await? {
            info!("no account exists yet; run `observer setup-link` to create the owner");
        }
        return Ok(None);
    }
    let token = accounts.create_setup_token(ActorKind::System).await?;
    if let Some(token) = &token {
        print_link(listen, &token.token);
    }
    Ok(token)
}

/// `observer setup-link`: a fresh link, which the running Observer accepts like its own.
pub async fn link(config: &Config) -> anyhow::Result<()> {
    let (db, accounts) = user::open(config).await?;
    let token = accounts.create_setup_token(ActorKind::Cli).await?;
    db.close().await?;
    match token {
        Some(token) => print_link(config.http.listen, &token.token),
        None => bail!("the owner already exists; `observer user reset-password <name>` lets them back in"),
    }
    Ok(())
}

fn print_link(listen: SocketAddr, token: &str) {
    println!("Create the owner account within 30 minutes, from this link:");
    println!("{}", setup_url(listen, token));
}

/// The panel's address on this host: `localhost` when Observer listens on every interface.
pub fn panel_url(listen: SocketAddr) -> String {
    let host = match listen.ip() {
        ip if ip.is_unspecified() => "localhost".to_owned(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    format!("http://{host}:{}", listen.port())
}

pub fn setup_url(listen: SocketAddr, token: &str) -> String {
    format!("{}/setup#{token}", panel_url(listen))
}

fn under_systemd() -> bool {
    env::var_os("INVOCATION_ID").is_some() || env::var_os("NOTIFY_SOCKET").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_localhost_for_every_interface_and_the_address_otherwise() {
        let link = |listen: &str| setup_url(listen.parse().unwrap(), "K3VQ");

        assert_eq!(link("0.0.0.0:40125"), "http://localhost:40125/setup#K3VQ");
        assert_eq!(link("[::]:40125"), "http://localhost:40125/setup#K3VQ");
        assert_eq!(link("127.0.0.1:41965"), "http://127.0.0.1:41965/setup#K3VQ");
        assert_eq!(link("[::1]:41965"), "http://[::1]:41965/setup#K3VQ");
    }
}
