//! What asks Observer to stop: Ctrl+C everywhere; closing the console, logging off, shutting
//! down and Quit in the status window on Windows; SIGTERM (service managers) and SIGHUP (a closed
//! SSH session) on Unix.

use std::io;
#[cfg(any(target_os = "linux", test))]
use std::time::{Duration, Instant};

use observer_domain::audit::StopSource;

/// How long Observer asks its service manager for at a time while it stops, and how often.
#[cfg(any(target_os = "linux", test))]
pub const EXTEND_BY: Duration = Duration::from_secs(20);
#[cfg(target_os = "linux")]
pub const EXTEND_EVERY: Duration = Duration::from_secs(5);
/// What a stop still takes once no resource holds it: `quit`, the kill of what is left, then
/// Observer's own closing.
#[cfg(any(target_os = "linux", test))]
const AFTER_HOLD: Duration = Duration::from_secs(40);

/// Whether Observer asks its service manager for more time to stop. A stop no resource holds fits
/// the unit's own limit, as it always did, and asks for nothing. While resources hold the stop
/// the wait is theirs, within the owner's limit: Observer asks for as long as it lasts, then for
/// what the stop takes after it, and no longer, so that a stop that hangs is still ended.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Default)]
pub struct StopTime {
    until: Option<Instant>,
}

#[cfg(any(target_os = "linux", test))]
impl StopTime {
    pub fn asks(&mut self, held: bool, now: Instant) -> bool {
        if held {
            self.until = Some(now + AFTER_HOLD);
        }
        self.until.is_some_and(|until| now < until)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopRequest {
    /// Windows is closing the console or ending the session, and ends the process about five
    /// seconds later.
    pub shortened: bool,
    pub from: StopSource,
}

pub struct Signals {
    #[cfg(windows)]
    console: observer_platform::ConsoleEvents,
    #[cfg(windows)]
    window: tokio::sync::mpsc::UnboundedReceiver<()>,
    #[cfg(windows)]
    requests: StopRequests,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hangup: tokio::signal::unix::Signal,
}

/// Asks for the graceful stop, as Quit in the status window does.
#[cfg(windows)]
#[derive(Clone)]
pub struct StopRequests(tokio::sync::mpsc::UnboundedSender<()>);

#[cfg(windows)]
impl StopRequests {
    pub fn request(&self) {
        let _ = self.0.send(());
    }
}

impl Signals {
    /// Listens from now on: a request that comes before the first [`Signals::next`] waits for it,
    /// rather than end the process at once.
    pub fn listen() -> io::Result<Self> {
        #[cfg(windows)]
        {
            let (requests, window) = tokio::sync::mpsc::unbounded_channel();
            Ok(Self {
                console: observer_platform::ConsoleEvents::listen()?,
                window,
                requests: StopRequests(requests),
            })
        }
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};

            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
                hangup: signal(SignalKind::hangup())?,
            })
        }
    }

    #[cfg(windows)]
    pub fn requests(&self) -> StopRequests {
        self.requests.clone()
    }

    pub async fn next(&mut self) -> StopRequest {
        #[cfg(windows)]
        {
            use observer_platform::ConsoleEvent;

            let from_console = |shortened| StopRequest {
                shortened,
                from: StopSource::Console,
            };
            tokio::select! {
                event = self.console.next() => match event {
                    Some(ConsoleEvent::Interrupt) => from_console(false),
                    Some(ConsoleEvent::Close) => from_console(true),
                    None => std::future::pending().await,
                },
                Some(()) = self.window.recv() => StopRequest {
                    shortened: false,
                    from: StopSource::Window,
                },
            }
        }
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.interrupt.recv() => {}
                _ = self.terminate.recv() => {}
                _ = self.hangup.recv() => {}
            }
            StopRequest {
                shortened: false,
                from: StopSource::Signal,
            }
        }
    }

    /// After a console close, Windows ends the process as soon as it is let go; call this once
    /// everything is stopped.
    #[cfg_attr(
        unix,
        expect(clippy::unused_self, reason = "only a Windows console holds the process")
    )]
    pub fn release(&self) {
        #[cfg(windows)]
        self.console.release();
    }
}

#[cfg(test)]
mod stop_time {
    use super::*;

    #[test]
    fn a_stop_no_resource_holds_asks_the_service_manager_for_nothing() {
        let mut time = StopTime::default();
        let start = Instant::now();

        for seconds in [0, 5, 30, 300] {
            assert!(
                !time.asks(false, start + Duration::from_secs(seconds)),
                "{seconds} s"
            );
        }
    }

    #[test]
    fn a_held_stop_asks_while_it_is_held_then_for_what_is_left_and_no_longer() {
        let mut time = StopTime::default();
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);

        assert!(!time.asks(false, at(0)));
        for seconds in [5, 60, 590] {
            assert!(time.asks(true, at(seconds)), "{seconds} s");
        }
        assert!(time.asks(false, at(595)));
        assert!(time.asks(false, at(629)));
        assert!(!time.asks(false, at(630)));
        assert!(EXTEND_BY < AFTER_HOLD);
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_request_from_the_window_is_a_graceful_stop_from_the_window() {
        let mut signals = Signals::listen().unwrap();

        signals.requests().request();

        assert_eq!(
            signals.next().await,
            StopRequest {
                shortened: false,
                from: StopSource::Window,
            }
        );
    }
}
