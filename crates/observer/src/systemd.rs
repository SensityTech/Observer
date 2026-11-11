//! Linux under systemd: the unit `observer service print-unit` writes, and the notifications a
//! `Type=notify` unit waits for. The protocol is one datagram per message, sent with the
//! standard library.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use observer_app::Supervision;
use observer_domain::server::{PlayerCount, ServerState, ServerStatus};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::shutdown::{EXTEND_BY, EXTEND_EVERY, StopTime};

/// The status line changes at most this often.
const STATUS_EVERY: Duration = Duration::from_secs(1);

/// `observer service print-unit`: the unit on stdout, what to do with it on stderr.
pub fn print_unit(config: &Path) -> anyhow::Result<()> {
    let binary = std::env::current_exe().context("cannot tell where this binary is")?;
    let config =
        std::path::absolute(config).with_context(|| format!("cannot resolve {}", config.display()))?;
    let folder = config.parent().unwrap_or(Path::new("/"));
    let root = rustix::process::getuid().is_root();
    let user = if root { "root".to_owned() } else { user_name()? };

    print!("{}", unit(&binary, &config, folder, &user)?);

    if root {
        eprintln!(
            "warning: as root, the unit runs Observer and FXServer as root; run this command as the account meant to own the server"
        );
    }
    if !config.exists() {
        eprintln!(
            "note: {} does not exist yet; Observer runs with its defaults until it does",
            config.display()
        );
    }
    eprint!(
        "{}",
        instructions(plain(binary.as_os_str())?, plain(config.as_os_str())?, &user)
    );
    Ok(())
}

/// What to do with the unit. The commands name this binary, which an extracted archive does not
/// put on `PATH`, and run as printed in a POSIX shell.
fn instructions(binary: &str, config: &str, user: &str) -> String {
    let observer = format!("{} --config {}", shell_quoted(binary), shell_quoted(config));
    format!(
        "
Install it with:
  {observer} service print-unit | sudo tee /etc/systemd/system/observer.service
  sudo systemctl daemon-reload && sudo systemctl enable --now observer
Then, as {user}, print the link that creates the owner:
  {observer} setup-link
Follow its logs with:
  journalctl -u observer -f
"
    )
}

/// A word a POSIX shell reads back as `text`: bare when the shell would do nothing with it,
/// single-quoted otherwise, a quote inside written `'\''`.
fn shell_quoted(text: &str) -> String {
    let bare = !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/._-+,:@%=".contains(character));
    if bare {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

fn unit(binary: &Path, config: &Path, folder: &Path, user: &str) -> anyhow::Result<String> {
    let exec = [
        quoted(binary.as_os_str())?,
        "--config".to_owned(),
        quoted(config.as_os_str())?,
        "run".to_owned(),
    ]
    .join(" ");
    let folder = specifiers(folder.as_os_str())?;
    let restart = crate::update::RESTART_CODE;
    Ok(format!(
        "# Written by `observer service print-unit`.
[Unit]
Description=Observer, FXServer supervisor
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
NotifyAccess=main
User={user}
WorkingDirectory={folder}
ExecStart={exec}
Restart=on-failure
RestartSec=5
# Observer exits with {restart} to start again once it installed a release.
SuccessExitStatus={restart}
RestartForceExitStatus={restart}
# SIGTERM goes to Observer only: it stops FXServer gracefully, then the rest of the group.
KillMode=mixed
TimeoutStopSec=45
LimitNOFILE=65536
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=full

[Install]
WantedBy=multi-user.target
"
    ))
}

/// An argument of `ExecStart=`, double-quoted when systemd would read it otherwise. A single
/// quote anywhere in a word opens a quoted part for systemd; inside double quotes it is literal.
fn quoted(value: &OsStr) -> anyhow::Result<String> {
    let text = plain(value)?;
    if !text.contains([' ', '"', '\'', '\\', '%', '$']) {
        return Ok(text.to_owned());
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '%' => quoted.push_str("%%"),
            '$' => quoted.push_str("$$"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

/// A path setting, where only `%` means something to systemd.
fn specifiers(value: &OsStr) -> anyhow::Result<String> {
    Ok(plain(value)?.replace('%', "%%"))
}

fn plain(value: &OsStr) -> anyhow::Result<&str> {
    let Some(text) = value.to_str() else {
        bail!(
            "cannot write a unit for a path that is not valid UTF-8: {}",
            value.display()
        );
    };
    if text.chars().any(char::is_control) {
        bail!("cannot write a unit for a path holding a control character");
    }
    Ok(text)
}

/// The account running this command, from the password database, or else as the session names it.
fn user_name() -> anyhow::Result<String> {
    let uid = rustix::process::getuid().as_raw().to_string();
    let from_passwd = fs::read_to_string("/etc/passwd").ok().and_then(|passwd| {
        passwd.lines().find_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            (fields.nth(1)? == uid).then(|| name.to_owned())
        })
    });
    from_passwd
        .or_else(|| std::env::var("USER").ok())
        .context("cannot tell which account runs this command")
}

/// Where systemd listens for this service's notifications.
pub struct Notifier {
    socket: UnixDatagram,
    address: SocketAddr,
}

impl Notifier {
    pub fn from_env() -> Option<Self> {
        Self::from_value(std::env::var_os("NOTIFY_SOCKET").as_deref())
    }

    /// A path, or `@name` in the abstract namespace; `None` when unset or neither.
    fn from_value(value: Option<&OsStr>) -> Option<Self> {
        let value = value?.as_bytes();
        let address = match value.strip_prefix(b"@") {
            Some(name) => SocketAddr::from_abstract_name(name).ok()?,
            None if value.starts_with(b"/") => SocketAddr::from_pathname(OsStr::from_bytes(value)).ok()?,
            None => return None,
        };
        let socket = UnixDatagram::unbound().ok()?;
        Some(Self { socket, address })
    }

    /// A lost notification costs systemd's view of the service, never the service.
    pub fn notify(&self, message: &str) {
        if let Err(error) = self.socket.send_to_addr(message.as_bytes(), &self.address) {
            debug!(%error, "systemd did not take a notification");
        }
    }
}

/// `READY=1` once the panel answers, then the server's state as it changes, then `STOPPING=1`,
/// and more time to stop for as long as resources hold the stop. Ready means the panel answers,
/// not that FXServer is up.
pub struct Readiness {
    notifier: Option<Arc<Notifier>>,
    follower: Option<JoinHandle<()>>,
    status: watch::Receiver<ServerStatus>,
    stop: CancellationToken,
    closed: CancellationToken,
}

impl Readiness {
    pub fn start(supervision: &Supervision, port: u16) -> Self {
        let stop = CancellationToken::new();
        let closed = CancellationToken::new();
        let status = supervision.status();
        let Some(notifier) = Notifier::from_env().map(Arc::new) else {
            return Self {
                notifier: None,
                follower: None,
                status,
                stop,
                closed,
            };
        };
        let state = state(&status.borrow());
        notifier.notify(&format!("READY=1\nSTATUS=Panel on port {port}, FXServer {state}"));
        let follower = tokio::spawn(follow(Arc::clone(&notifier), status.clone(), stop.clone()));
        Self {
            notifier: Some(notifier),
            follower: Some(follower),
            status,
            stop,
            closed,
        }
    }

    pub fn stopping(&self) {
        self.stop.cancel();
        if let Some(notifier) = &self.notifier {
            notifier.notify("STOPPING=1\nSTATUS=Stopping FXServer");
            tokio::spawn(ask_for_time(
                Arc::clone(notifier),
                self.status.clone(),
                self.closed.clone(),
            ));
        }
    }

    pub async fn close(self) {
        self.stop.cancel();
        self.closed.cancel();
        if let Some(follower) = self.follower {
            let _ = follower.await;
        }
    }
}

/// Asks systemd for the time the stop takes while resources hold it, which the unit's
/// `TimeoutStopSec` does not count on: see [`StopTime`]. systemd takes the ask from 236 on.
async fn ask_for_time(
    notifier: Arc<Notifier>,
    status: watch::Receiver<ServerStatus>,
    closed: CancellationToken,
) {
    let mut time = StopTime::default();
    let mut told = String::new();
    loop {
        let holding = status
            .borrow()
            .stop
            .as_ref()
            .map(|stop| stop.holding.join(", "))
            .unwrap_or_default();
        if time.asks(!holding.is_empty(), std::time::Instant::now()) {
            notifier.notify(&format!("EXTEND_TIMEOUT_USEC={}", EXTEND_BY.as_micros()));
        }
        if holding != told {
            if !holding.is_empty() {
                notifier.notify(&format!("STATUS=Stopping FXServer, waiting for {holding}"));
            }
            told = holding;
        }
        tokio::select! {
            () = closed.cancelled() => return,
            () = tokio::time::sleep(EXTEND_EVERY) => {}
        }
    }
}

async fn follow(notifier: Arc<Notifier>, mut status: watch::Receiver<ServerStatus>, stop: CancellationToken) {
    let mut sent = String::new();
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            changed = status.changed() => if changed.is_err() {
                return;
            },
        }
        let line = status_line(&status.borrow_and_update());
        if line != sent {
            notifier.notify(&format!("STATUS={line}"));
            sent = line;
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(STATUS_EVERY) => {}
        }
    }
}

/// `FXServer online · 12 / 64 players`.
fn status_line(status: &ServerStatus) -> String {
    let mut line = format!("FXServer {}", state(status));
    match status.players {
        Some(PlayerCount {
            online,
            max: Some(max),
        }) => {
            let _ = write!(line, " · {online} / {max} players");
        }
        Some(PlayerCount { online, max: None }) => {
            let _ = write!(line, " · {online} players");
        }
        None => {}
    }
    line
}

fn state(status: &ServerStatus) -> &'static str {
    match status.state {
        ServerState::Stopped => "stopped",
        ServerState::Starting => "starting",
        ServerState::Online => "online",
        ServerState::Degraded => "degraded",
        ServerState::Hung => "not responding",
        ServerState::Stopping => "stopping",
        ServerState::Crashed => "crashed",
        ServerState::CrashLoop => "in a crash loop",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn unit_for(binary: &str, config: &str) -> anyhow::Result<String> {
        let config = PathBuf::from(config);
        unit(Path::new(binary), &config, config.parent().unwrap(), "ada")
    }

    #[test]
    fn the_unit_runs_this_binary_with_this_config_as_a_notify_service() {
        let unit = unit_for("/opt/observer/observer", "/srv/fivem/observer.toml").unwrap();

        for line in [
            "Type=notify",
            "KillMode=mixed",
            "User=ada",
            "WorkingDirectory=/srv/fivem",
            "ExecStart=/opt/observer/observer --config /srv/fivem/observer.toml run",
            "SuccessExitStatus=75",
            "RestartForceExitStatus=75",
        ] {
            assert!(
                unit.lines().any(|candidate| candidate == line),
                "no {line:?} in\n{unit}"
            );
        }
    }

    #[test]
    fn a_path_with_a_space_or_a_percent_is_quoted_for_systemd() {
        let unit = unit_for("/opt/observer/observer", "/srv/my server/100% live/observer.toml").unwrap();

        assert!(
            unit.contains(
                r#"ExecStart=/opt/observer/observer --config "/srv/my server/100%% live/observer.toml" run"#
            ),
            "{unit}"
        );
        assert!(
            unit.contains("WorkingDirectory=/srv/my server/100%% live\n"),
            "{unit}"
        );
        assert_eq!(quoted(OsStr::new(r#"a"b\c$d"#)).unwrap(), r#""a\"b\\c$$d""#);
    }

    #[test]
    fn a_path_with_a_single_quote_is_quoted_for_systemd() {
        let unit = unit_for("/opt/observer/observer", "/srv/bob's/observer.toml").unwrap();

        assert!(
            unit.contains(r#"ExecStart=/opt/observer/observer --config "/srv/bob's/observer.toml" run"#),
            "{unit}"
        );
        assert!(unit.contains("WorkingDirectory=/srv/bob's\n"), "{unit}");
    }

    #[test]
    fn the_install_commands_name_this_binary_and_quote_its_paths() {
        let printed = instructions(
            "/home/ada/observer-0.1.0/observer",
            "/srv/bob's server/observer.toml",
            "ada",
        );

        for line in [
            r"  /home/ada/observer-0.1.0/observer --config '/srv/bob'\''s server/observer.toml' service print-unit | sudo tee /etc/systemd/system/observer.service",
            r"  /home/ada/observer-0.1.0/observer --config '/srv/bob'\''s server/observer.toml' setup-link",
            "Then, as ada, print the link that creates the owner:",
        ] {
            assert!(
                printed.lines().any(|candidate| candidate == line),
                "no {line:?} in\n{printed}"
            );
        }
        assert_eq!(
            shell_quoted("/srv/fivem/observer.toml"),
            "/srv/fivem/observer.toml"
        );
        assert_eq!(shell_quoted("/srv/$HOME/a;b"), "'/srv/$HOME/a;b'");
    }

    #[test]
    fn a_path_holding_a_control_character_is_refused() {
        let error = unit_for(
            "/opt/observer/observer",
            "/srv/fivem\nExecStartPre=/bin/sh/observer.toml",
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "cannot write a unit for a path holding a control character"
        );
    }

    #[test]
    fn a_notification_reaches_the_socket_systemd_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notify");
        let systemd = UnixDatagram::bind(&path).unwrap();

        let notifier = Notifier::from_value(Some(path.as_os_str())).unwrap();
        notifier.notify("READY=1\nSTATUS=Panel on port 40125, FXServer stopped");

        let mut received = [0; 128];
        let length = systemd.recv(&mut received).unwrap();
        assert_eq!(
            &received[..length],
            b"READY=1\nSTATUS=Panel on port 40125, FXServer stopped"
        );
    }

    #[test]
    fn an_at_sign_names_an_abstract_socket_and_nothing_names_no_notifier() {
        let notifier =
            Notifier::from_value(Some(OsStr::new("@/org/freedesktop/systemd1/notify/42"))).unwrap();

        assert_eq!(
            notifier.address.as_abstract_name(),
            Some(&b"/org/freedesktop/systemd1/notify/42"[..])
        );
        assert!(Notifier::from_value(None).is_none());
        assert!(Notifier::from_value(Some(OsStr::new("relative/notify"))).is_none());
    }

    #[test]
    fn the_status_line_names_the_state_and_the_players() {
        let status = ServerStatus {
            state: ServerState::Online,
            cause: None,
            since: jiff::Timestamp::UNIX_EPOCH,
            started_at: None,
            generation: 1,
            problems: Vec::new(),
            build: None,
            players: Some(PlayerCount {
                online: 12,
                max: Some(64),
            }),
            restart_at: None,
            launch: observer_domain::server::LaunchReadiness::Ready,
            name: None,
            port: None,
            start: None,
            stop_holders: Vec::new(),
            stop: None,
        };

        assert_eq!(status_line(&status), "FXServer online · 12 / 64 players");
    }
}
