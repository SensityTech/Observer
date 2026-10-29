//! What asks Observer to stop: Ctrl+C everywhere; closing the console, logging off, shutting
//! down and Quit in the status window on Windows; SIGTERM (service managers) and SIGHUP (a closed
//! SSH session) on Unix.

use std::io;

use observer_domain::audit::StopSource;

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
