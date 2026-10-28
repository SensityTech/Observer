//! `observer fxserver run`: the supervisor in the foreground, for driving a real server by hand.
//! It reads no database; the launch comes from the command line.

use std::io::{self, BufRead, IsTerminal, Write as _};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use anyhow::Context;
use clap::Args;
use jiff::tz::TimeZone;
use observer_fxserver::console::{ConsoleHub, ConsoleLine, ConsoleLog, LineKind, LogLimits, plain};
use observer_fxserver::launch::LaunchSettings;
use observer_fxserver::stdin::UserCommand;
use observer_fxserver::supervisor::{Event, EventKind, Settings, State, Supervisor};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::shutdown::{Shutdown, Signals};
use crate::{lock, logging};

/// Who the console shows for commands typed here.
const LOCAL: &str = "local";

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
    cfg: PathBuf,
    /// The port of FXServer's HTTP endpoint; by default the cfg's first `endpoint_add_tcp`.
    #[arg(long)]
    port: Option<u16>,
    /// An extra convar, passed as `+set NAME VALUE`. Repeat for more.
    #[arg(long = "set", value_name = "NAME=VALUE", value_parser = convar)]
    extra: Vec<(String, String)>,
}

fn convar(text: &str) -> Result<(String, String), String> {
    text.split_once('=')
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .ok_or_else(|| "expected NAME=VALUE".to_owned())
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
    let mut settings = Settings::new(
        LaunchSettings {
            build: args.build,
            data: args.data,
            cfg: args.cfg,
            onesync: None,
            txadmin_events: false,
            extra: args.extra,
        },
        &paths.root,
    );
    settings.probe = args
        .port
        .map(|port| SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    let (supervisor, task) = Supervisor::spawn(settings, Arc::clone(&console));

    let stop = CancellationToken::new();
    let printer = tokio::spawn(print_console(Arc::clone(&console), stop.clone()));
    let reporter = tokio::spawn(print_events(supervisor.events(), stop.clone()));
    let mut typed = read_typed_commands();

    let started = supervisor.start(LOCAL).await;
    let mut status = supervisor.status();
    // Without the panel nothing can start the server again once it is down for good.
    let mut shortened = false;
    while started.is_ok() {
        tokio::select! {
            shutdown = signals.next() => {
                shortened = shutdown == Shutdown::Shortened;
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
    task.await.context("the supervisor failed")?;

    stop.cancel();
    let _ = printer.await;
    let _ = reporter.await;
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
    if let Err(error) = result {
        warn!(%error, "command not sent");
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

async fn print_console(console: Arc<ConsoleHub>, stop: CancellationToken) {
    let subscription = console.subscribe_from(None);
    let colours = io::stdout().is_terminal();
    for line in &subscription.backlog {
        print_line(line, colours);
    }
    let mut lines = subscription.live;
    loop {
        tokio::select! {
            () = stop.cancelled() => break,
            line = lines.recv() => match line {
                Ok(line) => print_line(&line, colours),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    println!("[observer] {missed} console lines not shown: the terminal is too slow");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    // Whatever came in while stopping.
    while let Ok(line) = lines.try_recv() {
        print_line(&line, colours);
    }
}

fn print_line(line: &ConsoleLine, colours: bool) {
    let text = if colours {
        line.text.clone()
    } else {
        plain(&line.text)
    };
    let reset = if colours { "\x1b[0m" } else { "" };
    let mut out = io::stdout().lock();
    let _ = match (line.kind, &line.actor) {
        (LineKind::Command, Some(actor)) => writeln!(out, "[{actor}] > {text}{reset}"),
        (LineKind::Observer, _) => writeln!(out, "[observer] {text}{reset}"),
        _ => writeln!(out, "{text}{reset}"),
    };
}

async fn print_events(mut events: broadcast::Receiver<Event>, stop: CancellationToken) {
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
                println!("[observer] #{} {kind}", event.generation);
            }
        }
    }
}
