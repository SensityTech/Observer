//! What asks Observer to stop: Ctrl+C everywhere; closing the console, logging off and shutting
//! down on Windows; SIGTERM (service managers) and SIGHUP (a closed SSH session) on Unix.

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    Graceful,
    /// Windows is closing the console and ends the process about five seconds later.
    Shortened,
}

pub struct Signals {
    #[cfg(windows)]
    console: observer_platform::ConsoleEvents,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hangup: tokio::signal::unix::Signal,
}

impl Signals {
    /// Listens from now on: a request that comes before the first [`Signals::next`] waits for it,
    /// rather than end the process at once.
    pub fn listen() -> io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                console: observer_platform::ConsoleEvents::listen()?,
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

    pub async fn next(&mut self) -> Shutdown {
        #[cfg(windows)]
        {
            use observer_platform::ConsoleEvent;

            match self.console.next().await {
                Some(ConsoleEvent::Interrupt) => Shutdown::Graceful,
                Some(ConsoleEvent::Close) => Shutdown::Shortened,
                None => std::future::pending().await,
            }
        }
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.interrupt.recv() => {}
                _ = self.terminate.recv() => {}
                _ = self.hangup.recv() => {}
            }
            Shutdown::Graceful
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
