//! Every word the status window and the tray icon show. The panel says the same things in
//! `web/src/i18n/en.ts`; where both say one thing, they say it in the same words.

use std::fmt::Write as _;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use observer_domain::error::ErrorCode;
use observer_domain::schedule::NextRestart;
use observer_domain::server::{LaunchReadiness, PlayerCount, ServerState};

/// The most a tray tooltip holds, its closing NUL aside.
pub const TOOLTIP_CHARS: usize = 127;

pub const TITLE: &str = "Observer";
pub const UNKNOWN: &str = "n/a";
pub const PLAYERS: &str = "Players";
pub const BUILD: &str = "Build";
pub const NEXT_RESTART: &str = "Next restart";
pub const NONE: &str = "None";
pub const PANEL: &str = "Panel";
pub const OPEN_PANEL: &str = "Open the panel";
pub const SHOW_WINDOW: &str = "Show the window";
pub const OPEN_DATA_FOLDER: &str = "Open the data folder";
pub const QUIT: &str = "Quit Observer…";
pub const START: &str = "Start";
pub const STOP: &str = "Stop";
pub const RESTART: &str = "Restart";
pub const STOP_ASKING: &str = "Stop…";
pub const RESTART_ASKING: &str = "Restart…";
pub const CANCEL: &str = "Cancel";
pub const STOPPING_OBSERVER: &str = "Stopping FXServer, then Observer…";
/// The state while FXServer stops for a restart, to start again.
pub const RESTARTING: &str = "Restarting";
/// What a control says while its request is on its way.
pub const START_SENT: &str = "Starting…";
pub const STOP_SENT: &str = "Stopping…";
pub const RESTART_SENT: &str = "Restarting…";
/// The version's hover text when no commit is known, as the About page words it.
pub const DEVELOPMENT_BUILD: &str = "Development build";

pub const SETUP_TITLE: &str = "Create the owner account";
pub const OPEN_SETUP: &str = "Open the setup page";
pub const COPY_LINK: &str = "Copy the link";
pub const LINK_COPIED: &str = "Copied";
pub const LINK_EXPIRED: &str = "This link has expired.";
pub const NEW_LINK: &str = "New link";

pub const RESTART_TITLE: &str = "Restart FXServer?";
pub const STOP_TITLE: &str = "Stop FXServer?";
pub const KEEP_STOPPED_TITLE: &str = "Keep FXServer stopped?";
pub const QUIT_TITLE: &str = "Quit Observer?";
pub const NOTICE_FIRST: &str = "Resources receive observer:shuttingDown first, then FXServer quits; whatever does not end in 15 seconds is killed.";
pub const KEEP_STOPPED: &str = "FXServer is not running. Stopping it cancels any automatic restart and leaves it stopped until someone starts it.";
pub const CONFIRM_RESTART: &str = "Restart";
pub const CONFIRM_STOP: &str = "Stop server";
pub const CONFIRM_KEEP_STOPPED: &str = "Keep it stopped";
pub const CONFIRM_QUIT_RUNNING: &str = "Quit and stop FXServer";
pub const CONFIRM_QUIT: &str = "Quit Observer";

pub const NOT_CONFIGURED: &str = "FXServer has no launch settings yet.";
pub const INVALID: &str = "The launch settings need fixing on the Server page.";
pub const NO_BUILD: &str = "No build is selected: install or import one on the Artifacts page.";
pub const RUNNING: &str = "FXServer is already running.";
pub const NOT_RUNNING: &str = "FXServer is not running.";
pub const SERVER_STOPPING: &str = "FXServer is stopping.";
pub const OBSERVER_STOPPING: &str = "Observer is stopping.";

pub const LAUNCH_NOT_CONFIGURED: &str =
    "FXServer has no launch settings yet. The owner saves them on the Server page.";
pub const LAUNCH_FAILED: &str = "FXServer could not start. The console says why.";
pub const NO_BUILD_SELECTED: &str = "No build is selected yet. Install or import one on the Artifacts page.";
pub const STOPPING_REFUSAL: &str = "FXServer is stopping. Start it again once it has stopped.";
pub const SHUTTING_DOWN: &str = "Observer is stopping, and FXServer with it.";
pub const INTERNAL: &str = "Observer hit an internal error. Its log has the details.";
pub const UNKNOWN_FAILURE: &str = "Something went wrong. Try again.";

/// The panel's word for each state.
pub fn state(state: ServerState) -> &'static str {
    match state {
        ServerState::Stopped => "Stopped",
        ServerState::Starting => "Starting",
        ServerState::Online => "Online",
        ServerState::Degraded => "Degraded",
        ServerState::Hung => "Not responding",
        ServerState::Stopping => "Stopping",
        ServerState::Crashed => "Crashed",
        ServerState::CrashLoop => "Crash loop",
    }
}

/// What the owner still has to do before FXServer can start, if anything.
pub fn launch_note(launch: LaunchReadiness) -> Option<&'static str> {
    match launch {
        LaunchReadiness::Ready => None,
        LaunchReadiness::NotConfigured => Some(LAUNCH_NOT_CONFIGURED),
        LaunchReadiness::Invalid => Some(INVALID),
        LaunchReadiness::NoBuild => Some(NO_BUILD_SELECTED),
    }
}

/// `12 / 64`, `12` without the maximum, `n/a` while FXServer has not told.
pub fn players(players: Option<PlayerCount>) -> String {
    match players {
        Some(PlayerCount {
            online,
            max: Some(max),
        }) => format!("{} / {}", grouped(online), grouped(max)),
        Some(PlayerCount { online, max: None }) => grouped(online),
        None => UNKNOWN.to_owned(),
    }
}

pub fn build(build: Option<u32>) -> String {
    build.map_or_else(|| UNKNOWN.to_owned(), |build| build.to_string())
}

/// `up 2h 14m`.
pub fn uptime(started_at: Timestamp, now: Timestamp) -> String {
    format!("up {}", duration(now.duration_since(started_at).as_secs()))
}

/// The panel's durations: `12 s`, `15 min`, `2h 14m`, `3h`, `3d 4h`, never below zero.
pub fn duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        return format!("{seconds} s");
    }
    if seconds < 3600 {
        return format!("{} min", (seconds + 59) / 60);
    }
    let minutes = seconds / 60;
    if minutes < 24 * 60 {
        let hours = minutes / 60;
        return match minutes % 60 {
            0 => format!("{hours}h"),
            rest => format!("{hours}h {rest}m"),
        };
    }
    let hours = minutes / 60;
    match hours % 24 {
        0 => format!("{}d", hours / 24),
        rest => format!("{}d {rest}h", hours / 24),
    }
}

/// `04:00 (in 3h 12m)` in the schedule's zone, `29 Oct 04:00 (in 1d 2h)` on another day there,
/// `None` when nothing is ahead.
pub fn next_restart(next: Option<&NextRestart>, zone: &TimeZone, now: Timestamp) -> String {
    let Some(next) = next else {
        return NONE.to_owned();
    };
    let at = next.at.to_zoned(zone.clone());
    let today = now.to_zoned(zone.clone()).date() == at.date();
    let time = if today {
        at.strftime("%H:%M").to_string()
    } else {
        at.strftime("%-d %b %H:%M").to_string()
    };
    let left = duration(now.duration_until(next.at).as_secs());
    format!("{time} (in {left})")
}

/// `Works once, until 14:32.`, in the host's time.
pub fn link_expiry(expires_at: Timestamp, zone: &TimeZone) -> String {
    format!(
        "Works once, until {}.",
        expires_at.to_zoned(zone.clone()).strftime("%H:%M")
    )
}

/// `12 players are connected. They will be disconnected.`
pub fn connected(players: u64) -> String {
    match players {
        0 => "No player is connected.".to_owned(),
        1 => "1 player is connected. They will be disconnected.".to_owned(),
        players => format!(
            "{} players are connected. They will be disconnected.",
            grouped(players)
        ),
    }
}

/// What quitting does to the server, while it runs with `players`, or while it does not.
pub fn quit_body(players: Option<u64>) -> String {
    match players {
        Some(players) => format!("FXServer stops with it. {}", connected(players)),
        None => NOT_RUNNING.to_owned(),
    }
}

/// `Start failed. FXServer could not start. The console says why.`
pub fn failed(action: &str, code: Option<ErrorCode>) -> String {
    format!("{action} failed. {}", refusal(code))
}

/// A refusal, from its code, in the panel's words (`errors` in `en.ts`). `None` is a failure of
/// Observer itself; a code the window does not expect gets the panel's fallback, never its name.
pub fn refusal(code: Option<ErrorCode>) -> &'static str {
    match code {
        Some(ErrorCode::LaunchNotConfigured) => LAUNCH_NOT_CONFIGURED,
        Some(ErrorCode::InvalidLaunchSettings) => INVALID,
        Some(ErrorCode::LaunchFailed) => LAUNCH_FAILED,
        Some(ErrorCode::NoBuildSelected) => NO_BUILD_SELECTED,
        Some(ErrorCode::ServerRunning) => RUNNING,
        Some(ErrorCode::ServerNotRunning) => NOT_RUNNING,
        Some(ErrorCode::ServerStopping) => STOPPING_REFUSAL,
        Some(ErrorCode::ShuttingDown) => SHUTTING_DOWN,
        None | Some(ErrorCode::Internal) => INTERNAL,
        Some(_) => UNKNOWN_FAILURE,
    }
}

/// `Start is not available. FXServer has no launch settings yet.`, when a control that is off is
/// pressed: the panel's `controls.unavailable`.
pub fn unavailable(control: &str, reason: &str) -> String {
    format!("{} is not available. {reason}", control.trim_end_matches('…'))
}

/// `Observer · Online · 12 / 64 players · port 40125`, cut to what a tooltip holds.
pub fn tooltip(state: Option<ServerState>, players: Option<PlayerCount>, port: u16) -> String {
    let text = match state {
        None => "Observer · Stopping".to_owned(),
        Some(server) => {
            let mut text = format!("Observer · {}", self::state(server));
            if let Some(players) = players {
                let _ = write!(text, " · {} players", self::players(Some(players)));
            }
            let _ = write!(text, " · port {port}");
            text
        }
    };
    text.chars().take(TOOLTIP_CHARS).collect()
}

/// `2,048`: the panel's English grouping.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use observer_domain::schedule::RestartSource;

    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn next(at: Timestamp) -> NextRestart {
        NextRestart {
            at,
            source: RestartSource::Schedule,
            by: None,
            delayed_from: None,
        }
    }

    #[test]
    fn states_read_as_the_panel_words_them() {
        let words: Vec<_> = [
            ServerState::Stopped,
            ServerState::Starting,
            ServerState::Online,
            ServerState::Degraded,
            ServerState::Hung,
            ServerState::Stopping,
            ServerState::Crashed,
            ServerState::CrashLoop,
        ]
        .into_iter()
        .map(state)
        .collect();

        assert_eq!(
            words,
            [
                "Stopped",
                "Starting",
                "Online",
                "Degraded",
                "Not responding",
                "Stopping",
                "Crashed",
                "Crash loop"
            ]
        );
    }

    #[test]
    fn facts_say_n_a_while_fxserver_has_not_told() {
        let count = |online, max| Some(PlayerCount { online, max });

        assert_eq!(players(count(12, Some(64))), "12 / 64");
        assert_eq!(players(count(2048, Some(2048))), "2,048 / 2,048");
        assert_eq!(players(count(3, None)), "3");
        assert_eq!(players(None), "n/a");
        assert_eq!(build(Some(37872)), "37872");
        assert_eq!(build(None), "n/a");
    }

    #[test]
    fn durations_read_as_the_panel_reads_them() {
        let cases = [
            (-5, "0 s"),
            (12, "12 s"),
            (61, "2 min"),
            (900, "15 min"),
            (3 * 3600, "3h"),
            (2 * 3600 + 14 * 60 + 30, "2h 14m"),
            (3 * 86_400 + 4 * 3600 + 59, "3d 4h"),
            (2 * 86_400, "2d"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(duration(seconds), expected, "{seconds} s");
        }
        assert_eq!(
            uptime(at("2026-10-29T08:00:00Z"), at("2026-10-29T10:14:00Z")),
            "up 2h 14m"
        );
    }

    #[test]
    fn the_next_restart_reads_in_the_schedules_zone() {
        let paris = TimeZone::get("Europe/Paris").unwrap();
        // 00:48 in Paris.
        let now = at("2026-10-28T23:48:00Z");

        assert_eq!(
            next_restart(Some(&next(at("2026-10-29T03:00:00Z"))), &paris, now),
            "04:00 (in 3h 12m)"
        );
        assert_eq!(
            next_restart(Some(&next(at("2026-10-30T03:00:00Z"))), &paris, now),
            "30 Oct 04:00 (in 1d 3h)"
        );
        assert_eq!(next_restart(None, &paris, now), "None");
    }

    #[test]
    fn quitting_says_what_happens_to_the_server_and_its_players() {
        assert_eq!(
            quit_body(Some(12)),
            "FXServer stops with it. 12 players are connected. They will be disconnected."
        );
        assert_eq!(
            quit_body(Some(1)),
            "FXServer stops with it. 1 player is connected. They will be disconnected."
        );
        assert_eq!(
            quit_body(Some(0)),
            "FXServer stops with it. No player is connected."
        );
        assert_eq!(quit_body(None), "FXServer is not running.");
    }

    #[test]
    fn every_refusal_has_its_sentence() {
        let cases = [
            (
                ErrorCode::LaunchNotConfigured,
                "FXServer has no launch settings yet. The owner saves them on the Server page.",
            ),
            (
                ErrorCode::InvalidLaunchSettings,
                "The launch settings need fixing on the Server page.",
            ),
            (
                ErrorCode::LaunchFailed,
                "FXServer could not start. The console says why.",
            ),
            (
                ErrorCode::NoBuildSelected,
                "No build is selected yet. Install or import one on the Artifacts page.",
            ),
            (ErrorCode::ServerRunning, "FXServer is already running."),
            (ErrorCode::ServerNotRunning, "FXServer is not running."),
            (
                ErrorCode::ServerStopping,
                "FXServer is stopping. Start it again once it has stopped.",
            ),
            (
                ErrorCode::ShuttingDown,
                "Observer is stopping, and FXServer with it.",
            ),
            (
                ErrorCode::Internal,
                "Observer hit an internal error. Its log has the details.",
            ),
            (ErrorCode::NotFound, "Something went wrong. Try again."),
        ];
        for (code, sentence) in cases {
            assert_eq!(refusal(Some(code)), sentence, "{code:?}");
        }
        assert_eq!(
            refusal(None),
            "Observer hit an internal error. Its log has the details."
        );
        assert_eq!(
            failed(START, Some(ErrorCode::LaunchFailed)),
            "Start failed. FXServer could not start. The console says why."
        );
    }

    #[test]
    fn a_control_that_is_off_says_why_when_pressed() {
        assert_eq!(
            unavailable(START, NOT_CONFIGURED),
            "Start is not available. FXServer has no launch settings yet."
        );
        assert_eq!(
            unavailable(QUIT, OBSERVER_STOPPING),
            "Quit Observer is not available. Observer is stopping."
        );
    }

    #[test]
    fn the_tooltip_fits_with_the_longest_values() {
        let most = Some(PlayerCount {
            online: 2048,
            max: Some(2048),
        });

        let longest = tooltip(Some(ServerState::Hung), most, 65535);

        assert_eq!(
            longest,
            "Observer · Not responding · 2,048 / 2,048 players · port 65535"
        );
        assert!(longest.chars().count() <= TOOLTIP_CHARS);
        assert_eq!(
            tooltip(Some(ServerState::Stopped), None, 40125),
            "Observer · Stopped · port 40125"
        );
        assert_eq!(tooltip(None, most, 40125), "Observer · Stopping");
    }

    #[test]
    fn the_setup_link_says_until_when_it_works() {
        let paris = TimeZone::get("Europe/Paris").unwrap();

        assert_eq!(
            link_expiry(at("2026-10-29T13:32:00Z"), &paris),
            "Works once, until 14:32."
        );
    }

    /// Changing one of these on either side is a decision: the window and the panel say them alike.
    #[test]
    fn the_sentences_the_panel_also_says_are_the_panels() {
        let catalog = include_str!("../../../../web/src/i18n/en.ts");
        let shared = [
            NOT_CONFIGURED,
            INVALID,
            NO_BUILD,
            RUNNING,
            NOT_RUNNING,
            SERVER_STOPPING,
            LAUNCH_NOT_CONFIGURED,
            LAUNCH_FAILED,
            NO_BUILD_SELECTED,
            STOPPING_REFUSAL,
            SHUTTING_DOWN,
            INTERNAL,
            UNKNOWN_FAILURE,
            DEVELOPMENT_BUILD,
            RESTARTING,
            RESTART_TITLE,
            STOP_TITLE,
            KEEP_STOPPED_TITLE,
            NOTICE_FIRST,
            KEEP_STOPPED,
            CONFIRM_STOP,
            CONFIRM_KEEP_STOPPED,
            NEXT_RESTART,
            "No player is connected.",
            "connected. They will be disconnected.",
            "Not responding",
            "Crash loop",
            "someone at the host",
            "is not available.",
        ];
        for sentence in shared {
            assert!(
                catalog.contains(&format!("'{sentence}'")) || catalog.contains(sentence),
                "en.ts no longer says {sentence:?}"
            );
        }
    }
}
