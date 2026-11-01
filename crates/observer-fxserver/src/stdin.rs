//! FXServer's stdin runs anything with every right of the server console. Observer writes there
//! only lines it can check against an allow-list, and reads every operator command the way
//! FXServer will before letting it through.

use std::fmt::{self, Write as _};
use std::io;
use std::sync::Arc;
use std::time::Duration;

use observer_platform::Stdin;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::bridge::{BridgeEvent, StopReason};
use crate::command;
use crate::console::{ConsoleHub, LineFlags, LineKind, mask_command};

pub const MAX_COMMAND: usize = 1024;
const BRIDGE_COMMAND: &str = "observer_cmd";
/// The user name FXServer's `/perf/` endpoint expects from Observer.
pub const PERF_USER: &str = "observer";
const TOKEN_BYTES: usize = 16;
/// Longer than any event of the contract, short enough to refuse anything else.
const MAX_BRIDGE_HEX: usize = 4096;
/// The JSON of one event, which travels as hex at twice its length.
pub const MAX_BRIDGE_JSON: usize = MAX_BRIDGE_HEX / 2;
/// A server whose main loop is stuck stops reading; a write is reported pending after this.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const QUEUED_WRITES: usize = 32;
const QUIT_REASONS: [StopReason; 5] = [
    StopReason::Stop,
    StopReason::Restart,
    StopReason::Scheduled,
    StopReason::Shutdown,
    StopReason::CrashLoop,
];

/// A line Observer writes on its own behalf. No variant carries text a user typed: the reason of
/// `quit` comes from a closed list, and bridge events travel hex-encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemLine {
    Quit(StopReason),
    Bridge(BridgeEvent),
    SetPerfUser,
    SetPerfPassword(String),
    SetPlayersToken(String),
}

impl SystemLine {
    pub fn render(&self) -> String {
        match self {
            Self::Quit(reason) => format!("quit \"{}\"", reason.as_str()),
            Self::Bridge(event) => format!("observer_cmd {}", event.encode()),
            Self::SetPerfUser => format!("set sv_prometheusBasicAuthUser {PERF_USER}"),
            Self::SetPerfPassword(password) => format!("set sv_prometheusBasicAuthPassword {password}"),
            Self::SetPlayersToken(token) => format!("set sv_playersToken {token}"),
        }
    }

    /// What the console shows of the line; the credentials are not shown at all.
    fn echo(&self) -> Option<String> {
        match self {
            Self::Quit(_) => Some(self.render()),
            Self::Bridge(event) => Some(format!("observer_cmd {}", event.name())),
            Self::SetPerfUser | Self::SetPerfPassword(_) | Self::SetPlayersToken(_) => None,
        }
    }
}

/// Whether a line Observer is about to write is one of the few it may write. Checked on the
/// rendered text, so that a bug building a line cannot turn into a command.
pub fn allowed(line: &str) -> bool {
    if let Some(reason) = line
        .strip_prefix("quit \"")
        .and_then(|rest| rest.strip_suffix('"'))
    {
        return QUIT_REASONS.iter().any(|known| known.as_str() == reason);
    }
    if let Some(hex) = line.strip_prefix("observer_cmd ") {
        return !hex.is_empty()
            && hex.len() <= MAX_BRIDGE_HEX
            && hex.len() % 2 == 0
            && hex.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    }
    if let Some(password) = line.strip_prefix("set sv_prometheusBasicAuthPassword ") {
        return is_token(password);
    }
    if let Some(token) = line.strip_prefix("set sv_playersToken ") {
        return is_token(token);
    }
    line == format!("set sv_prometheusBasicAuthUser {PERF_USER}")
}

fn is_token(text: &str) -> bool {
    text.len() == TOKEN_BYTES * 2 && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The secrets of one launch, set through stdin so that they never show in the command line:
/// the `/perf/` password and the `/players.json` token.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub perf_password: String,
    pub players_token: String,
}

impl Credentials {
    pub fn generate() -> io::Result<Self> {
        Ok(Self {
            perf_password: token()?,
            players_token: token()?,
        })
    }

    pub fn lines(&self) -> [SystemLine; 3] {
        [
            SystemLine::SetPerfUser,
            SystemLine::SetPerfPassword(self.perf_password.clone()),
            SystemLine::SetPlayersToken(self.players_token.clone()),
        ]
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials { .. }")
    }
}

fn token() -> io::Result<String> {
    let mut bytes = [0; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    let mut token = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        let _ = write!(token, "{byte:02x}");
    }
    Ok(token)
}

/// A console line an operator typed, accepted for FXServer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCommand {
    line: String,
    server_control: bool,
    quits: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandRefusal {
    #[error("the command is empty")]
    Empty,
    #[error("the command is longer than {MAX_COMMAND} bytes")]
    TooLong,
    #[error("the command contains a control character")]
    ControlCharacter,
    #[error("`observer_cmd` is reserved to Observer")]
    Reserved,
}

impl UserCommand {
    /// Reads the line as FXServer will, command by command, since one line can hold several.
    pub fn parse(line: &str) -> Result<Self, CommandRefusal> {
        let line = line.trim_matches(' ');
        if line.len() > MAX_COMMAND {
            return Err(CommandRefusal::TooLong);
        }
        if line.chars().any(char::is_control) {
            return Err(CommandRefusal::ControlCharacter);
        }
        // A convar's value runs as a command through `vstr`, and quotes and `\` only disguise a
        // word, so the name is refused anywhere in the line and not only as a command.
        let bare: String = line.chars().filter(|c| !matches!(c, '"' | '\\')).collect();
        if bare.to_ascii_lowercase().contains(BRIDGE_COMMAND) {
            return Err(CommandRefusal::Reserved);
        }
        let commands = command::split(line);
        if commands.is_empty() {
            return Err(CommandRefusal::Empty);
        }
        let mut server_control = false;
        let mut quits = false;
        for words in commands.into_iter().map(command::words) {
            let name = words[0].to_ascii_lowercase();
            let on_bridge = words
                .get(1)
                .is_some_and(|resource| resource.eq_ignore_ascii_case(crate::bridge::NAME));
            match name.as_str() {
                "quit" => {
                    quits = true;
                    server_control = true;
                }
                "stop" | "start" | "restart" | "ensure" if on_bridge => server_control = true,
                // `_crash` ends the server with a dump; the others run commands Observer cannot
                // see, a file's or a convar's value.
                "_crash" | "exec" | "vstr" | "+vstr" | "-vstr" => server_control = true,
                _ => {}
            }
        }
        Ok(Self {
            line: line.to_owned(),
            server_control,
            quits,
        })
    }

    /// The command stops the server or its bridge, or runs commands Observer cannot read, so it
    /// takes the right to control the server and not only to use the console.
    pub fn needs_server_control(&self) -> bool {
        self.server_control
    }

    /// The command quits the server, which then ends as a stop rather than a crash.
    pub fn quits(&self) -> bool {
        self.quits
    }

    pub fn as_str(&self) -> &str {
        &self.line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StdinError {
    #[error("the line is not one Observer may write")]
    NotAllowed,
    #[error("FXServer's stdin is closed")]
    Closed,
    /// Refused without being written: FXServer is not reading what it was given already.
    #[error("FXServer is not reading its stdin")]
    Busy,
    /// Handed to FXServer, which has not read it yet: it runs, and is echoed, once it does.
    #[error("FXServer has not read the line yet; it runs when the server reads its stdin again")]
    Pending,
}

pub(crate) enum Write {
    System(SystemLine),
    User { command: UserCommand, actor: String },
}

type Request = (Write, oneshot::Sender<Result<(), StdinError>>);

/// The one task that writes to a server's stdin. Each accepted line is echoed into the console.
#[derive(Clone)]
pub(crate) struct StdinWriter {
    requests: mpsc::Sender<Request>,
}

impl StdinWriter {
    pub(crate) fn start(stdin: Stdin, generation: u32, console: Arc<ConsoleHub>) -> (Self, JoinHandle<()>) {
        let (requests, queue) = mpsc::channel(QUEUED_WRITES);
        let task = tokio::spawn(own(stdin, queue, generation, console));
        (Self { requests }, task)
    }

    /// Queues the line and hands back the outcome to wait on, without waiting for it.
    pub(crate) fn send(&self, write: Write) -> oneshot::Receiver<Result<(), StdinError>> {
        let (reply, done) = oneshot::channel();
        match self.requests.try_send((write, reply)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full((_, reply))) => {
                let _ = reply.send(Err(StdinError::Busy));
            }
            Err(mpsc::error::TrySendError::Closed((_, reply))) => {
                let _ = reply.send(Err(StdinError::Closed));
            }
        }
        done
    }
}

async fn own(stdin: Stdin, mut queue: mpsc::Receiver<Request>, generation: u32, console: Arc<ConsoleHub>) {
    while let Some((write, reply)) = queue.recv().await {
        let (line, echo) = match &write {
            Write::System(line) => {
                let text = line.render();
                if !allowed(&text) {
                    let _ = reply.send(Err(StdinError::NotAllowed));
                    continue;
                }
                let echo = line.echo().map(|echo| (LineKind::Observer, None, echo, false));
                (text, echo)
            }
            Write::User { command, actor } => {
                let shown = mask_command(command.as_str());
                let masked = shown.is_some();
                let shown = shown.unwrap_or_else(|| command.as_str().to_owned());
                let echo = (LineKind::Command, Some(actor.clone()), shown, masked);
                (command.as_str().to_owned(), Some(echo))
            }
        };
        let writing = stdin.write(format!("{line}\n").into_bytes());
        tokio::pin!(writing);
        let mut reply = Some(reply);
        let written = tokio::select! {
            written = &mut writing => written,
            () = tokio::time::sleep(WRITE_TIMEOUT) => {
                // The line is FXServer's now and runs once it reads again. Nothing more goes in
                // behind it meanwhile, so that no line runs after its sender was told it failed.
                if let Some(reply) = reply.take() {
                    let _ = reply.send(Err(StdinError::Pending));
                }
                loop {
                    tokio::select! {
                        written = &mut writing => break written,
                        Some((_, refused)) = queue.recv() => {
                            let _ = refused.send(Err(StdinError::Busy));
                        }
                    }
                }
            }
        };
        let result = written.map_err(|_| StdinError::Closed);
        if result.is_ok()
            && let Some((kind, actor, text, masked)) = echo
        {
            let flags = LineFlags {
                masked,
                ..LineFlags::default()
            };
            console.push(generation, kind, actor, text, flags);
        }
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use observer_domain::schedule::RestartSource;

    use super::*;

    const HOSTILE: &str = "\";\r\n\\";

    fn events_carrying(text: &str) -> Vec<BridgeEvent> {
        vec![
            BridgeEvent::RestartScheduled {
                at: 1,
                seconds_remaining: 60,
                reason: RestartSource::OneOff,
            },
            BridgeEvent::RestartSkipped {
                at: 1,
                by: text.to_owned(),
                reason: RestartSource::Schedule,
            },
            BridgeEvent::RestartRescheduled {
                at: 2,
                previous_at: 1,
                by: text.to_owned(),
                reason: RestartSource::OneOff,
            },
            BridgeEvent::ShuttingDown {
                delay_ms: 5000,
                reason: StopReason::Stop,
                by: text.to_owned(),
            },
        ]
    }

    fn refusal(line: &str) -> Option<CommandRefusal> {
        UserCommand::parse(line).err()
    }

    fn needs_control(line: &str) -> bool {
        UserCommand::parse(line).unwrap().needs_server_control()
    }

    #[test]
    fn every_line_observer_builds_passes_the_allow_list() {
        let credentials = Credentials::generate().unwrap();
        let mut lines: Vec<SystemLine> = QUIT_REASONS.into_iter().map(SystemLine::Quit).collect();
        lines.extend(credentials.lines());
        lines.extend(events_carrying("alice").into_iter().map(SystemLine::Bridge));

        for line in lines {
            assert!(allowed(&line.render()), "{:?} was refused", line.render());
        }
    }

    #[test]
    fn hostile_text_in_any_field_never_reaches_stdin_as_such() {
        for event in events_carrying(HOSTILE) {
            let line = SystemLine::Bridge(event).render();

            assert!(allowed(&line));
            assert!(!line.contains(['"', ';', '\r', '\n', '\\']), "{line:?}");
            assert_eq!(command::split(&line), [line.as_str()]);
        }
    }

    #[test]
    fn lines_outside_the_allow_list_are_refused() {
        for line in [
            "quit",
            "quit stop",
            "quit \"bye\"",
            "quit \"stop\"; quit",
            "quit \"stop\"\nrefresh",
            "observer_cmd",
            "observer_cmd ",
            "observer_cmd 4",
            "observer_cmd 4A",
            "observer_cmd 41;quit",
            "observer_cmd 41 42",
            "set sv_prometheusBasicAuthUser admin",
            "set sv_prometheusBasicAuthPassword short",
            "set sv_playersToken 0123456789abcdef0123456789abcdeg",
            "set sv_licenseKey cfxk_x",
            "refresh",
        ] {
            assert!(!allowed(line), "{line:?} was allowed");
        }
        assert!(!allowed(&format!("observer_cmd {}", "ab".repeat(MAX_BRIDGE_HEX))));
    }

    #[test]
    fn credentials_differ_on_every_launch_and_never_show_in_debug_output() {
        let first = Credentials::generate().unwrap();
        let second = Credentials::generate().unwrap();

        assert_ne!(first.perf_password, second.perf_password);
        assert_ne!(first.perf_password, first.players_token);
        assert!(is_token(&first.perf_password));
        assert_eq!(format!("{first:?}"), "Credentials { .. }");
        assert_eq!(SystemLine::SetPerfPassword(first.perf_password).echo(), None);
    }

    #[test]
    fn a_plain_command_goes_through_as_typed() {
        let command = UserCommand::parse("  say \"hello; world\" ").unwrap();

        assert_eq!(command.as_str(), "say \"hello; world\"");
        assert!(!command.needs_server_control());
        assert!(!command.quits());
    }

    #[test]
    fn quit_needs_server_control_however_it_is_hidden() {
        for line in [
            "quit",
            "QUIT \"bye\"",
            "say hi; quit",
            "say \"a \\\" ; quit",
            "\"quit\"",
            "/* comment */ quit",
            "\u{e9}quit",
        ] {
            let command = UserCommand::parse(line).unwrap();
            assert!(command.needs_server_control(), "{line:?}");
            assert!(command.quits(), "{line:?}");
        }
    }

    #[test]
    fn stopping_or_reloading_the_bridge_needs_server_control() {
        for line in [
            "stop observer",
            "restart Observer",
            "ensure observer",
            "start observer",
            "refresh; stop \"observer\"",
        ] {
            assert!(needs_control(line), "{line:?}");
        }
        for line in ["stop chat", "ensure observer_extras", "say quit", "status"] {
            assert!(!needs_control(line), "{line:?}");
        }
    }

    #[test]
    fn commands_that_run_hidden_commands_need_server_control() {
        for line in [
            "exec resources.cfg",
            "set x quit; vstr x",
            "+vstr x x",
            "say hi; -VSTR x x",
        ] {
            assert!(needs_control(line), "{line:?}");
        }
    }

    #[test]
    fn crashing_the_server_needs_server_control() {
        let command = UserCommand::parse("_crash").unwrap();

        assert!(command.needs_server_control());
        assert!(!command.quits());
    }

    #[test]
    fn observer_cmd_is_refused_however_it_is_hidden() {
        for line in [
            "observer_cmd 7b7d",
            "OBSERVER_CMD 7b7d",
            "say hi;observer_cmd 7b7d",
            "say \"x \\\" ; observer_cmd 7b7d",
            "\"observer_cmd\" 7b7d",
            "set x \"observer_cmd 7b7d\"; vstr x",
            "set x \"observer\\_cmd 7b7d\"",
            "set x \"\\\"observer\\\\_cmd\\\" 7b7d\"",
            "say observer_cmd",
        ] {
            assert_eq!(refusal(line), Some(CommandRefusal::Reserved), "{line:?}");
        }
    }

    #[test]
    fn control_characters_long_lines_and_blank_lines_are_refused() {
        for line in ["say a\rquit", "say a\nquit", "say \u{0}", "say \x1b[2J", "say\ta"] {
            assert_eq!(refusal(line), Some(CommandRefusal::ControlCharacter), "{line:?}");
        }
        assert_eq!(
            refusal(&format!("say {}", "x".repeat(MAX_COMMAND))),
            Some(CommandRefusal::TooLong)
        );
        assert!(UserCommand::parse(&format!("say {}", "x".repeat(MAX_COMMAND - 4))).is_ok());
        assert_eq!(refusal("   "), Some(CommandRefusal::Empty));
        assert_eq!(refusal(" ; // nothing"), Some(CommandRefusal::Empty));
    }

    #[test]
    fn hostile_text_typed_by_an_operator_cannot_end_its_line() {
        assert_eq!(
            refusal(&format!("say {HOSTILE}")),
            Some(CommandRefusal::ControlCharacter)
        );
    }
}
