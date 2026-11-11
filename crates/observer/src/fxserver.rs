//! `observer fxserver run`: the supervisor in the foreground, for driving a real server by hand.
//! It reads no database; the launch comes from the command line. The server tools do not run
//! before its start: neither their heal nor their fix, which only Observer itself runs.

use std::io::{self, BufRead, IsTerminal, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use clap::Args;
use jiff::tz::TimeZone;
use observer_domain::launch::{
    DEFAULT_HANG_TIMEOUT_SECONDS, ExtraArgument, LaunchSettings, MAX_HANG_TIMEOUT_SECONDS,
    MIN_HANG_TIMEOUT_SECONDS, OneSync,
};
use observer_fxserver::console::{ConsoleHub, ConsoleLine, ConsoleLog, LineKind, LogLimits, plain};
use observer_fxserver::launch::ServerLaunch;
use observer_fxserver::stdin::{StdinError, UserCommand};
use observer_fxserver::supervisor::{ControlError, Event, EventKind, Settings, State, Supervisor};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::shutdown::Signals;
use crate::{lock, logging};

/// Who the console shows for commands typed here.
const LOCAL: &str = "local";
/// Lines waiting for the terminal. Past this they are dropped, and counted.
const TERMINAL_LINES: usize = 4096;
/// How long the last lines get to reach the terminal once everything has stopped.
const TERMINAL_DRAIN: Duration = Duration::from_secs(2);
/// Said before the start: the server tools' work is Observer's own.
const NO_TOOLS: &str = "[observer] the server tools do not run here: no fix is applied or healed before \
    this start; `observer run` runs them, and `observer tools` runs them by hand";

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The FXServer build: the folder with FXServer.exe, or with alpine/ on Linux.
    #[arg(long)]
    build: PathBuf,
    /// The server data folder, FXServer's working directory.
    #[arg(long)]
    data: PathBuf,
    /// The cfg to execute, relative to the data folder.
    #[arg(long, default_value = "server.cfg")]
    cfg: String,
    /// OneSync, set after the cfg: `off`, `on` or `legacy`.
    #[arg(long, default_value = "on", value_parser = onesync)]
    onesync: OneSync,
    /// The port of FXServer's HTTP endpoint; by default the cfg's first `endpoint_add_tcp`.
    #[arg(long)]
    port: Option<u16>,
    /// An extra convar, passed as `+set NAME VALUE`. Repeat for more.
    #[arg(long = "set", value_name = "NAME=VALUE", value_parser = convar)]
    extra: Vec<(String, String)>,
    /// How long FXServer's main loop or HTTP endpoints may stay silent before it is restarted as
    /// hung, in whole minutes from 1 to 60. Raise it for a first start that builds caches.
    #[arg(
        long,
        value_name = "MINUTES",
        default_value_t = DEFAULT_HANG_TIMEOUT_SECONDS / 60,
        value_parser = hang_minutes,
        conflicts_with = "no_hang_detection"
    )]
    hang_timeout: u32,
    /// Never restart FXServer as hung, however long it stays silent. A crash still restarts it.
    #[arg(long)]
    no_hang_detection: bool,
}

fn convar(text: &str) -> Result<(String, String), String> {
    text.split_once('=')
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .ok_or_else(|| "expected NAME=VALUE".to_owned())
}

fn onesync(text: &str) -> Result<OneSync, String> {
    OneSync::from_name(text).ok_or_else(|| "expected off, on or legacy".to_owned())
}

/// Whole minutes only, within the range the launch settings take in seconds.
fn hang_minutes(text: &str) -> Result<u32, String> {
    let minutes = MIN_HANG_TIMEOUT_SECONDS / 60..=MAX_HANG_TIMEOUT_SECONDS / 60;
    text.parse()
        .ok()
        .filter(|value| minutes.contains(value))
        .ok_or_else(|| {
            format!(
                "expected whole minutes from {} to {}",
                minutes.start(),
                minutes.end()
            )
        })
}

/// The launch the command line describes: started here by hand, without txAdmin's events.
fn server_launch(args: RunArgs) -> anyhow::Result<ServerLaunch> {
    Ok(ServerLaunch {
        build: PathBuf::from(full_path(&args.build)?),
        settings: LaunchSettings {
            data_path: full_path(&args.data)?,
            cfg_file: args.cfg,
            onesync: args.onesync,
            auto_start: false,
            txadmin_events: false,
            extra_arguments: args
                .extra
                .into_iter()
                .map(|(name, value)| ExtraArgument { name, value })
                .collect(),
            hang_detection: !args.no_hang_detection,
            hang_timeout_seconds: args.hang_timeout * 60,
            stop_hold_seconds: observer_domain::launch::DEFAULT_STOP_HOLD_SECONDS,
        },
    })
}

/// The launch rules take full paths only; the command line also takes them relative to here.
fn full_path(path: &std::path::Path) -> anyhow::Result<String> {
    let full = std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))?;
    full.into_os_string()
        .into_string()
        .map_err(|path| anyhow::anyhow!("{} is not valid Unicode", path.display()))
}

pub async fn run(config: Config, args: RunArgs) -> anyhow::Result<()> {
    let paths = config.paths();
    let _lock = lock::acquire(&paths.root)?;
    let log_guard = logging::init(&config.log.level, &paths.logs)?;
    let mut signals = Signals::listen().context("cannot listen for stop requests")?;

    let console_log = ConsoleLog::start(
        paths.logs.join("console"),
        LogLimits::default(),
        TimeZone::system(),
    )
    .context("cannot open the console log")?;
    let console = Arc::new(ConsoleHub::new(Some(&console_log)));
    let probe = args
        .port
        .map(|port| SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    let mut settings = Settings::new(Some(server_launch(args)?), &paths.root);
    settings.probe = probe;
    let (supervisor, mut task) = Supervisor::spawn(settings, Arc::clone(&console));

    let (terminal, terminal_drained) =
        Terminal::start(io::stdout()).context("cannot print to the terminal")?;
    terminal.print(NO_TOOLS.to_owned());
    let stop = CancellationToken::new();
    let printer = tokio::spawn(print_console(
        Arc::clone(&console),
        terminal.clone(),
        stop.clone(),
    ));
    let reporter = tokio::spawn(print_events(supervisor.events(), terminal.clone(), stop.clone()));
    let mut typed = read_typed_commands();

    let started = supervisor.start(LOCAL).await;
    let mut status = supervisor.status();
    // Without the panel nothing can start the server again once it is down for good.
    let mut shortened = false;
    while started.is_ok() {
        tokio::select! {
            request = signals.next() => {
                shortened = request.shortened;
                info!(shortened, "stopping the server");
                break;
            }
            Some(line) = typed.recv() => submit(&supervisor, &line).await,
            changed = status.changed() => {
                let state = status.borrow_and_update().state;
                if changed.is_err() || matches!(state, State::Stopped | State::CrashLoop) {
                    break;
                }
            }
        }
    }
    let last = status.borrow().state;
    supervisor.shutdown(shortened).await;
    loop {
        tokio::select! {
            ended = &mut task => {
                ended.context("the supervisor failed")?;
                break;
            }
            // A console closed during a graceful stop leaves Windows' few seconds only.
            request = signals.next(), if !shortened => {
                if request.shortened {
                    shortened = true;
                    info!("the console is closing; shortening the stop");
                    supervisor.shutdown(true).await;
                }
            }
        }
    }

    stop.cancel();
    let _ = printer.await;
    let _ = reporter.await;
    drop(terminal);
    let _ = tokio::time::timeout(TERMINAL_DRAIN, terminal_drained).await;
    console_log.close().await;
    info!("Observer stopped");
    drop(log_guard);
    signals.release();
    started?;
    if last == State::CrashLoop {
        anyhow::bail!("the server kept failing; automatic restarts stopped");
    }
    Ok(())
}

async fn submit(supervisor: &Supervisor, line: &str) {
    if line.trim().is_empty() {
        return;
    }
    let result = match UserCommand::parse(line) {
        Ok(command) => supervisor.command(command, LOCAL).await,
        Err(refusal) => Err(refusal.into()),
    };
    match result {
        Ok(()) => {}
        Err(error @ ControlError::Stdin(StdinError::Pending)) => warn!(%error, "command not read yet"),
        Err(error) => warn!(%error, "command not sent"),
    }
}

/// Lines typed on Observer's own stdin. The reading thread cannot be interrupted portably, so it
/// is left to end with the process; it holds nothing but the channel.
fn read_typed_commands() -> mpsc::Receiver<String> {
    let (lines, typed) = mpsc::channel(16);
    let spawned = thread::Builder::new()
        .name("typed-commands".to_owned())
        .spawn(move || {
            for line in io::stdin().lock().lines() {
                let Ok(line) = line else {
                    break;
                };
                if lines.blocking_send(line).is_err() {
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        warn!(%error, "cannot read commands from the terminal");
    }
    typed
}

/// What the command prints goes through a thread of its own: a terminal that stops reading (a
/// text selection in a console window, a stalled pipe) must hold up none of the tasks that print.
#[derive(Clone)]
struct Terminal {
    lines: std_mpsc::SyncSender<String>,
    dropped: Arc<AtomicU64>,
}

impl Terminal {
    /// Also hands back what resolves once every line is written and every clone dropped. The
    /// thread is not joined: a terminal that never reads again must not hold up the exit.
    fn start(mut out: impl Write + Send + 'static) -> io::Result<(Self, oneshot::Receiver<()>)> {
        let (lines, queue) = std_mpsc::sync_channel::<String>(TERMINAL_LINES);
        let (drained, terminal_drained) = oneshot::channel();
        let dropped = Arc::new(AtomicU64::new(0));
        thread::Builder::new().name("terminal".to_owned()).spawn({
            let dropped = Arc::clone(&dropped);
            move || {
                for line in queue {
                    let missed = dropped.swap(0, Ordering::Relaxed);
                    if missed > 0 {
                        let _ = writeln!(
                            out,
                            "[observer] {missed} lines not shown: the terminal is too slow"
                        );
                    }
                    let _ = writeln!(out, "{line}");
                }
                let _ = drained.send(());
            }
        })?;
        Ok((Self { lines, dropped }, terminal_drained))
    }

    /// Never waits: a line the terminal has no room for is dropped.
    fn print(&self, line: String) {
        if self.lines.try_send(line).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn print_console(console: Arc<ConsoleHub>, terminal: Terminal, stop: CancellationToken) {
    let subscription = console.subscribe_from(None);
    let colours = io::stdout().is_terminal();
    for line in &subscription.backlog {
        terminal.print(format_line(line, colours));
    }
    let mut lines = subscription.live;
    loop {
        tokio::select! {
            () = stop.cancelled() => break,
            line = lines.recv() => match line {
                Ok(line) => terminal.print(format_line(&line, colours)),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    terminal.print(format!("[observer] {missed} console lines not shown: the terminal is too slow"));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    // Whatever came in while stopping.
    while let Ok(line) = lines.try_recv() {
        terminal.print(format_line(&line, colours));
    }
}

fn format_line(line: &ConsoleLine, colours: bool) -> String {
    let text = if colours {
        line.text.clone()
    } else {
        plain(&line.text)
    };
    let reset = if colours { "\x1b[0m" } else { "" };
    match (line.kind, &line.actor) {
        (LineKind::Command, Some(actor)) => format!("[{actor}] > {text}{reset}"),
        (LineKind::Observer, _) => format!("[observer] {text}{reset}"),
        _ => format!("{text}{reset}"),
    }
}

async fn print_events(mut events: broadcast::Receiver<Event>, terminal: Terminal, stop: CancellationToken) {
    loop {
        let event = tokio::select! {
            () = stop.cancelled() => break,
            event = events.recv() => match event {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
        };
        match &event.kind {
            EventKind::State { .. } => {}
            // Routine reports, every few seconds.
            kind @ (EventKind::Heap { .. } | EventKind::Resources { .. }) => {
                debug!(generation = event.generation, ?kind, "bridge report");
            }
            kind => {
                let kind = serde_json::to_string(kind).unwrap_or_default();
                terminal.print(format!("[observer] #{} {kind}", event.generation));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct Command {
        #[command(flatten)]
        args: RunArgs,
    }

    fn parsed(extra: &[&str]) -> Result<LaunchSettings, clap::Error> {
        let line = ["fxserver", "--build", "build", "--data", "server-data"];
        let command = Command::try_parse_from(line.iter().chain(extra))?;
        Ok(server_launch(command.args).unwrap().settings)
    }

    #[test]
    fn hang_detection_is_on_after_six_minutes_unless_the_command_line_says_otherwise() {
        let default = parsed(&[]).unwrap();
        assert!(default.hang_detection);
        assert_eq!(default.hang_timeout_seconds, DEFAULT_HANG_TIMEOUT_SECONDS);

        let longer = parsed(&["--hang-timeout", "20"]).unwrap();
        assert!(longer.hang_detection);
        assert_eq!(longer.hang_timeout_seconds, 1200);
        for minutes in ["1", "60"] {
            let edge = parsed(&["--hang-timeout", minutes]).unwrap();
            assert!(
                edge.issues(observer_domain::launch::Platform::host()).is_empty(),
                "{minutes}"
            );
        }

        let off = parsed(&["--no-hang-detection"]).unwrap();
        assert!(!off.hang_detection);
        assert_eq!(off.hang_timeout_seconds, DEFAULT_HANG_TIMEOUT_SECONDS);
    }

    #[test]
    fn a_hang_timeout_outside_one_to_sixty_whole_minutes_or_with_detection_off_is_refused() {
        for minutes in ["0", "61", "1.5", "six"] {
            let refused = parsed(&["--hang-timeout", minutes]).unwrap_err();
            assert_eq!(
                refused.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{minutes}"
            );
            assert!(
                refused
                    .to_string()
                    .contains("expected whole minutes from 1 to 60"),
                "{refused}"
            );
        }
        let both = parsed(&["--hang-timeout", "10", "--no-hang-detection"]).unwrap_err();
        assert_eq!(both.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    /// A terminal that takes nothing until it is released, like a console window with a text
    /// selection held. It says when a write reaches it.
    struct Held {
        reached: std_mpsc::Sender<()>,
        release: std_mpsc::Receiver<()>,
    }

    impl Write for Held {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let _ = self.reached.send(());
            let _ = self.release.recv();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn printing_never_waits_for_a_terminal_that_stopped_reading() {
        let (reached, writing) = std_mpsc::channel();
        let (release, held) = std_mpsc::channel();
        let (terminal, drained) = Terminal::start(Held {
            reached,
            release: held,
        })
        .unwrap();
        // The thread takes the count of dropped lines each time it reports them: once it is stuck
        // writing a line, it no longer can.
        terminal.print("first".to_owned());
        writing.recv().unwrap();
        let started = Instant::now();

        for index in 0..TERMINAL_LINES * 2 {
            terminal.print(format!("line {index}"));
        }

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(terminal.dropped.load(Ordering::Relaxed), TERMINAL_LINES as u64);
        drop(release);
        drop(terminal);
        drained.blocking_recv().unwrap();
    }
}
