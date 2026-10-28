use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    SetConsoleCtrlHandler,
};
use windows_sys::core::BOOL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleEvent {
    /// Ctrl+C or Ctrl+Break: stop as usual.
    Interrupt,
    /// The console window is closing, or the session is ending. Windows ends the process about
    /// five seconds later whatever happens, so only the shortened stop fits.
    Close,
}

/// The console events Observer must turn into a stop. On a close event Windows ends the process
/// as soon as the handler returns, so the handler holds it until [`ConsoleEvents::release`].
pub struct ConsoleEvents {
    events: mpsc::UnboundedReceiver<ConsoleEvent>,
}

static REGISTERED: AtomicBool = AtomicBool::new(false);
static LISTENER: Mutex<Option<mpsc::UnboundedSender<ConsoleEvent>>> = Mutex::new(None);
static RELEASED: Mutex<bool> = Mutex::new(false);
static RELEASE: Condvar = Condvar::new();

impl ConsoleEvents {
    /// Takes over Ctrl+C, Ctrl+Break and the close, logoff and shutdown events. A later call
    /// replaces the earlier listener.
    pub fn listen() -> io::Result<Self> {
        if !REGISTERED.swap(true, Ordering::SeqCst) {
            // SAFETY: the handler is a plain function that lives as long as the process.
            if unsafe { SetConsoleCtrlHandler(Some(on_console_event), TRUE) } == 0 {
                REGISTERED.store(false, Ordering::SeqCst);
                return Err(io::Error::last_os_error());
            }
        }
        let (sender, events) = mpsc::unbounded_channel();
        *lock(&RELEASED) = false;
        *lock(&LISTENER) = Some(sender);
        Ok(Self { events })
    }

    pub async fn next(&mut self) -> Option<ConsoleEvent> {
        self.events.recv().await
    }

    /// Lets Windows end the process after a close event. Call it once the stop is done.
    pub fn release(&self) {
        *lock(&RELEASED) = true;
        RELEASE.notify_all();
    }
}

impl Drop for ConsoleEvents {
    fn drop(&mut self) {
        lock(&LISTENER).take();
        self.release();
    }
}

unsafe extern "system" fn on_console_event(kind: u32) -> BOOL {
    if handle(kind) { TRUE } else { FALSE }
}

/// Windows runs the handler on a thread of its own, so it may block. Returns whether the event
/// was taken; if not, the next handler in the chain (in the end, the default one) gets it.
fn handle(kind: u32) -> bool {
    let event = match kind {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => ConsoleEvent::Interrupt,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => ConsoleEvent::Close,
        _ => return false,
    };
    let delivered = lock(&LISTENER)
        .as_ref()
        .is_some_and(|listener| listener.send(event).is_ok());
    if delivered && event == ConsoleEvent::Close {
        let mut released = lock(&RELEASED);
        while !*released {
            released = RELEASE.wait(released).unwrap_or_else(PoisonError::into_inner);
        }
    }
    delivered
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::*;

    // One test only: the handler state is process-wide, like the console it serves.
    #[tokio::test]
    async fn a_close_event_holds_the_process_until_the_stop_is_released() {
        let mut events = ConsoleEvents::listen().unwrap();

        assert!(thread::spawn(|| handle(CTRL_C_EVENT)).join().unwrap());
        assert_eq!(events.next().await, Some(ConsoleEvent::Interrupt));

        let closing = thread::spawn(|| handle(CTRL_CLOSE_EVENT));
        assert_eq!(events.next().await, Some(ConsoleEvent::Close));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !closing.is_finished(),
            "the handler returned before the stop was released"
        );
        events.release();
        assert!(closing.join().unwrap());

        drop(events);
        assert!(
            !handle(CTRL_C_EVENT),
            "an event nobody listens to must fall through"
        );
    }
}
