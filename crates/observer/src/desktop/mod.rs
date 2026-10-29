//! The status window and the tray icon, on Windows: the server's state, its controls and the
//! panel's address for whoever uses the Windows session Observer runs in. They act as `local`
//! and sign nobody in: that person already holds the data folder, and with it every account.
//!
//! The window runs eframe on a thread of its own and the tray a Win32 message loop on another.
//! Neither ever waits on the runtime: they read the status as it stands and hand actions over,
//! whose outcome comes back with a repaint.

mod palette;
mod text;
mod tray;
mod window;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use jiff::Timestamp;
use observer_app::{Accounts, Schedule, SetupToken, Supervision};
use observer_domain::about::Build;
use observer_domain::audit::ActorKind;
use observer_domain::schedule::ScheduleStatus;
use observer_domain::server::{LaunchReadiness, ServerState, ServerStatus};
use observer_platform::{MessageLoop, Waker};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle as TaskHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::setup;
use crate::shutdown::StopRequests;

/// How long Observer waits for the window and the tray to go as it ends.
const CLOSE_WITHIN: Duration = Duration::from_secs(2);
/// How often the window asks whether the owner was created meanwhile, while it shows the link.
const SETUP_CHECK_EVERY: Duration = Duration::from_secs(5);

pub struct DesktopSetup {
    pub runtime: Handle,
    pub supervision: Supervision,
    pub schedule: Schedule,
    pub accounts: Accounts,
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub setup: Option<SetupToken>,
    pub stop: StopRequests,
    pub build: Build,
}

pub struct Desktop {
    shared: Arc<Shared>,
    window: Option<JoinHandle<()>>,
    tray: Option<MessageLoop>,
    feeder: TaskHandle<()>,
    stop: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Start,
    Stop,
    Restart,
}

/// What the window asks before it acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirm {
    Restart,
    Stop,
    /// A stop of a server that crashed: nothing runs, but a restart may be on its way.
    KeepStopped,
    Quit,
}

enum Request {
    Show,
    Exit,
}

/// Why each control cannot act now, or `None` when it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Controls {
    pub start: Option<&'static str>,
    pub stop: Option<&'static str>,
    pub restart: Option<&'static str>,
}

/// The panel's rules for its controls (`blocked` in `web/src/server/state.ts`).
pub(crate) fn controls(status: &ServerStatus) -> Controls {
    if status.state == ServerState::Stopping {
        let stopping = Some(text::SERVER_STOPPING);
        return Controls {
            start: stopping,
            stop: stopping,
            restart: stopping,
        };
    }
    let running = is_running(status.state);
    let launch = match status.launch {
        LaunchReadiness::Ready => None,
        LaunchReadiness::NotConfigured => Some(text::NOT_CONFIGURED),
        LaunchReadiness::Invalid => Some(text::INVALID),
    };
    Controls {
        start: if running { Some(text::RUNNING) } else { launch },
        stop: (status.state == ServerState::Stopped).then_some(text::NOT_RUNNING),
        restart: if running { launch } else { Some(text::NOT_RUNNING) },
    }
}

/// A process runs: it has players, and its uptime counts.
pub(crate) fn is_running(state: ServerState) -> bool {
    matches!(
        state,
        ServerState::Starting | ServerState::Online | ServerState::Degraded | ServerState::Hung
    )
}

/// What the desktop threads and the runtime share.
pub(crate) struct Shared {
    runtime: Handle,
    supervision: Supervision,
    accounts: Accounts,
    status: watch::Receiver<ServerStatus>,
    schedule: watch::Receiver<ScheduleStatus>,
    setup: watch::Sender<Option<SetupToken>>,
    panel: String,
    listen: SocketAddr,
    data_dir: PathBuf,
    stop: StopRequests,
    build: Build,
    state: Mutex<State>,
    requests: mpsc::Sender<Request>,
    tray: OnceLock<Waker>,
}

#[derive(Default)]
struct State {
    stopping: bool,
    /// Observer is ending: the window closes, and does not open again.
    exiting: bool,
    /// The open window's context, through which anyone can repaint or focus it.
    window: Option<egui::Context>,
    /// The confirmation the window shows next.
    confirm: Option<Confirm>,
    in_flight: Option<Action>,
    outcome: Option<Outcome>,
}

/// A refusal, shown until the state of the server next changes.
#[derive(Debug, Clone)]
struct Outcome {
    text: String,
    since: Timestamp,
}

impl Desktop {
    pub fn start(setup: DesktopSetup) -> Self {
        let (requests, received) = mpsc::channel();
        let data_dir = std::path::absolute(&setup.data_dir).unwrap_or(setup.data_dir);
        let shared = Arc::new(Shared {
            status: setup.supervision.status(),
            schedule: setup.schedule.status(),
            setup: watch::Sender::new(setup.setup),
            panel: setup::panel_url(setup.listen),
            runtime: setup.runtime,
            supervision: setup.supervision,
            accounts: setup.accounts,
            listen: setup.listen,
            data_dir,
            stop: setup.stop,
            build: setup.build,
            state: Mutex::default(),
            requests,
            tray: OnceLock::new(),
        });
        // The window opens with Observer.
        let _ = shared.requests.send(Request::Show);
        let window = window::spawn(Arc::clone(&shared), received)
            .inspect_err(|error| warn!(%error, "the status window could not start"))
            .ok();
        let tray = tray::spawn(Arc::clone(&shared))
            .inspect_err(|error| warn!(%error, "the tray icon could not start"))
            .ok();
        if let Some(tray) = &tray {
            let _ = shared.tray.set(tray.waker());
        }
        let stop = CancellationToken::new();
        let feeder = shared.runtime.spawn(feed(Arc::clone(&shared), stop.clone()));
        Self {
            shared,
            window,
            tray,
            feeder,
            stop,
        }
    }

    /// Observer has begun to stop: every control goes off.
    pub fn stopping(&self) {
        self.shared.lock().stopping = true;
        self.shared.refresh();
    }

    /// Closes the window and takes the tray icon out of the notification area. Threads that do
    /// not end in time end with the process.
    pub async fn close(self) {
        self.stop.cancel();
        let _ = self.feeder.await;
        let _ = self.shared.requests.send(Request::Exit);
        {
            let mut state = self.shared.lock();
            state.exiting = true;
            if let Some(window) = &state.window {
                window.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if let Some(tray) = &self.tray {
            tray.waker().quit();
        }
        let threads = tokio::task::spawn_blocking(move || {
            if let Some(window) = self.window {
                let _ = window.join();
            }
            if let Some(tray) = self.tray {
                tray.join();
            }
        });
        if tokio::time::timeout(CLOSE_WITHIN, threads).await.is_err() {
            warn!("the status window or the tray icon did not close in time");
        }
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Repaints the open window and refreshes the tray.
    fn refresh(&self) {
        if let Some(window) = &self.lock().window {
            window.request_repaint();
        }
        if let Some(tray) = self.tray.get() {
            tray.wake();
        }
    }

    /// Opens the window, or brings it forward, with `confirm` asked first.
    fn show(&self, confirm: Option<Confirm>) {
        let mut state = self.lock();
        if confirm.is_some() {
            state.confirm = confirm;
        }
        match &state.window {
            Some(window) => {
                window.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                window.send_viewport_cmd(egui::ViewportCommand::Focus);
                window.request_repaint();
            }
            None => {
                let _ = self.requests.send(Request::Show);
            }
        }
    }

    fn stopping(&self) -> bool {
        self.lock().stopping
    }

    /// Starts, stops or restarts FXServer as `local`. A refusal shows on the window, which it
    /// opens when `show_refusal` (a start from the tray, which shows nothing else).
    fn act(self: &Arc<Self>, action: Action, show_refusal: bool) {
        {
            let mut state = self.lock();
            if state.stopping || state.in_flight.is_some() {
                return;
            }
            state.in_flight = Some(action);
            state.outcome = None;
        }
        self.refresh();
        let shared = Arc::clone(self);
        self.runtime.spawn(async move {
            let supervision = &shared.supervision;
            let result = match action {
                Action::Start => supervision.start_local().await,
                Action::Stop => supervision.stop_local().await,
                Action::Restart => supervision.restart_local().await,
            };
            let refused = result.err().map(|error| {
                // The window shows a generic sentence for these: the log keeps what happened.
                if matches!(
                    text::refusal(error.code()),
                    text::INTERNAL | text::UNKNOWN_FAILURE
                ) {
                    warn!(%error, ?action, "the status window's request failed");
                }
                Outcome {
                    text: text::failed(word(action), error.code()),
                    since: shared.status.borrow().since,
                }
            });
            let show = show_refusal && refused.is_some();
            {
                let mut state = shared.lock();
                state.in_flight = None;
                state.outcome = refused;
            }
            if show {
                shared.show(None);
            }
            shared.refresh();
        });
    }

    /// A new setup link, once the shown one has expired, created as `local`.
    fn renew_link(self: &Arc<Self>) {
        let shared = Arc::clone(self);
        self.runtime.spawn(async move {
            match shared.accounts.create_setup_token(ActorKind::Local).await {
                Ok(token) => {
                    shared.setup.send_replace(token);
                }
                Err(error) => warn!(%error, "cannot create a setup link"),
            }
            shared.refresh();
        });
    }

    /// The refusal to show, unless the server changed state since.
    fn outcome(&self, status: &ServerStatus) -> Option<String> {
        let mut state = self.lock();
        if state
            .outcome
            .as_ref()
            .is_some_and(|outcome| outcome.since != status.since)
        {
            state.outcome = None;
        }
        state.outcome.as_ref().map(|outcome| outcome.text.clone())
    }

    fn setup_url(&self, token: &SetupToken) -> String {
        setup::setup_url(self.listen, &token.token)
    }

    fn open_panel(&self) {
        open(&self.panel);
    }

    fn open_data_folder(&self) {
        open(&self.data_dir.display().to_string());
    }
}

fn word(action: Action) -> &'static str {
    match action {
        Action::Start => text::START,
        Action::Stop => text::STOP,
        Action::Restart => text::RESTART,
    }
}

/// Opens a link in the default browser, or a folder in Explorer. Explorer exits with 1 even when
/// it succeeds, so its exit code says nothing.
fn open(target: &str) {
    if let Err(error) = Command::new("explorer.exe").arg(target).spawn() {
        warn!(%error, "cannot start Explorer");
    }
}

/// Wakes the window and the tray whenever what they show changes, and drops the setup link once
/// the owner exists.
async fn feed(shared: Arc<Shared>, stop: CancellationToken) {
    let mut status = shared.supervision.status();
    let mut schedule = shared.schedule.clone();
    let mut setup_check = tokio::time::interval(SETUP_CHECK_EVERY);
    loop {
        let link_shown = shared.setup.borrow().is_some();
        tokio::select! {
            () = stop.cancelled() => return,
            Ok(()) = status.changed() => {}
            Ok(()) = schedule.changed() => {}
            _ = setup_check.tick(), if link_shown => {
                match shared.accounts.setup_required().await {
                    Ok(false) => {
                        shared.setup.send_replace(None);
                    }
                    Ok(true) => continue,
                    Err(error) => warn!(%error, "cannot tell whether the owner exists"),
                }
            }
        }
        shared.refresh();
    }
}

#[cfg(test)]
mod tests {
    use observer_domain::server::{LaunchReadiness as L, ServerState as S};

    use super::*;

    fn status(state: S, launch: L) -> ServerStatus {
        ServerStatus {
            state,
            cause: None,
            since: Timestamp::UNIX_EPOCH,
            started_at: None,
            generation: 0,
            problems: Vec::new(),
            build: None,
            players: None,
            restart_at: None,
            launch,
        }
    }

    /// The same cases as `blocked` in the panel's `server/state.test.ts`.
    #[test]
    fn controls_follow_the_panels_rules() {
        let cases = [
            (
                S::Stopped,
                L::Ready,
                None,
                Some(text::NOT_RUNNING),
                Some(text::NOT_RUNNING),
            ),
            (S::Online, L::Ready, Some(text::RUNNING), None, None),
            (S::Starting, L::Ready, Some(text::RUNNING), None, None),
            (S::Degraded, L::Ready, Some(text::RUNNING), None, None),
            (S::Hung, L::Ready, Some(text::RUNNING), None, None),
            (
                S::Stopping,
                L::Ready,
                Some(text::SERVER_STOPPING),
                Some(text::SERVER_STOPPING),
                Some(text::SERVER_STOPPING),
            ),
            (S::Crashed, L::Ready, None, None, Some(text::NOT_RUNNING)),
            (S::CrashLoop, L::Ready, None, None, Some(text::NOT_RUNNING)),
            (
                S::Stopped,
                L::NotConfigured,
                Some(text::NOT_CONFIGURED),
                Some(text::NOT_RUNNING),
                Some(text::NOT_RUNNING),
            ),
            (
                S::Online,
                L::NotConfigured,
                Some(text::RUNNING),
                None,
                Some(text::NOT_CONFIGURED),
            ),
            (
                S::Crashed,
                L::Invalid,
                Some(text::INVALID),
                None,
                Some(text::NOT_RUNNING),
            ),
            (
                S::Online,
                L::Invalid,
                Some(text::RUNNING),
                None,
                Some(text::INVALID),
            ),
        ];
        for (state, launch, start, stop, restart) in cases {
            assert_eq!(
                controls(&status(state, launch)),
                Controls { start, stop, restart },
                "{state:?} with {launch:?}"
            );
        }
    }
}
