use std::io;
use std::ptr;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer,
    TranslateMessage, WM_APP, WM_QUIT, WM_TIMER, WM_USER,
};

const WM_WAKE: u32 = WM_APP;

/// A thread that pumps Win32 messages, for what must live where its messages are dispatched (a
/// tray icon and its menu). It calls back on a steady tick and whenever it is woken.
pub struct MessageLoop {
    waker: Waker,
    thread: JoinHandle<()>,
}

/// Wakes or ends a [`MessageLoop`] from any thread.
#[derive(Debug, Clone, Copy)]
pub struct Waker {
    thread_id: u32,
}

impl MessageLoop {
    /// Starts the loop. `start` builds the state on the loop thread; `on_wake` runs there on each
    /// tick and each [`Waker::wake`]. The state is dropped on the loop thread when it ends.
    pub fn spawn<S, F, W>(name: &str, tick: Duration, start: F, mut on_wake: W) -> io::Result<Self>
    where
        S: 'static,
        F: FnOnce(Waker) -> S + Send + 'static,
        W: FnMut(&mut S) + Send + 'static,
    {
        let (ready, thread_id) = mpsc::sync_channel(1);
        let tick = u32::try_from(tick.as_millis()).unwrap_or(u32::MAX).max(1);
        let thread = thread::Builder::new().name(name.to_owned()).spawn(move || {
            let mut message = MSG::default();
            // SAFETY: `message` is a valid MSG to write into. Peeking creates the thread's queue,
            // so that messages posted from now on are kept.
            unsafe { PeekMessageW(&raw mut message, ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE) };
            // SAFETY: no precondition.
            let thread_id = unsafe { GetCurrentThreadId() };
            let waker = Waker { thread_id };
            let _ = ready.send(thread_id);
            let mut state = start(waker);
            // SAFETY: a thread timer, with no window and no callback: it posts WM_TIMER here.
            let timer = unsafe { SetTimer(ptr::null_mut(), 0, tick, None) };
            loop {
                // SAFETY: `message` is a valid MSG to write into.
                let got = unsafe { GetMessageW(&raw mut message, ptr::null_mut(), 0, 0) };
                if got == 0 || got == -1 {
                    break;
                }
                if message.hwnd.is_null() && matches!(message.message, WM_TIMER | WM_WAKE) {
                    on_wake(&mut state);
                    continue;
                }
                // SAFETY: `message` was just filled by GetMessageW.
                unsafe {
                    TranslateMessage(&raw const message);
                    DispatchMessageW(&raw const message);
                }
            }
            if timer != 0 {
                // SAFETY: `timer` is the thread timer set above.
                unsafe { KillTimer(ptr::null_mut(), timer) };
            }
            drop(state);
        })?;
        let thread_id = thread_id
            .recv()
            .map_err(|_| io::Error::other("the message loop ended as it started"))?;
        Ok(Self {
            waker: Waker { thread_id },
            thread,
        })
    }

    pub fn waker(&self) -> Waker {
        self.waker
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Waits for the loop to end, after [`Waker::quit`].
    pub fn join(self) {
        let _ = self.thread.join();
    }
}

impl Waker {
    pub fn wake(self) {
        self.post(WM_WAKE);
    }

    /// Ends the loop once it has handled what is already queued.
    pub fn quit(self) {
        self.post(WM_QUIT);
    }

    fn post(self, message: u32) {
        // SAFETY: posting to a thread that is gone or has no queue fails, and nothing else.
        unsafe { PostThreadMessageW(self.thread_id, message, 0, 0) };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::ThreadId;
    use std::time::Instant;

    use super::*;

    /// The thread the state was built on, then the one it was dropped on.
    type Threads = Arc<Mutex<Option<(ThreadId, ThreadId)>>>;

    struct Counted {
        wakes: Arc<AtomicUsize>,
        woken: mpsc::Sender<Instant>,
        built_on: ThreadId,
        dropped_on: Threads,
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            *self.dropped_on.lock().unwrap() = Some((self.built_on, thread::current().id()));
        }
    }

    struct Counting {
        message_loop: MessageLoop,
        wakes: Arc<AtomicUsize>,
        woke: mpsc::Receiver<Instant>,
        dropped_on: Threads,
    }

    fn counting(tick: Duration) -> Counting {
        let wakes = Arc::new(AtomicUsize::new(0));
        let dropped_on = Threads::default();
        let (woken, woke) = mpsc::channel();
        let message_loop = MessageLoop::spawn(
            "test-loop",
            tick,
            {
                let wakes = Arc::clone(&wakes);
                let dropped_on = Arc::clone(&dropped_on);
                move |_| Counted {
                    wakes,
                    woken,
                    built_on: thread::current().id(),
                    dropped_on,
                }
            },
            |state: &mut Counted| {
                state.wakes.fetch_add(1, Ordering::SeqCst);
                let _ = state.woken.send(Instant::now());
            },
        )
        .unwrap();
        Counting {
            message_loop,
            wakes,
            woke,
            dropped_on,
        }
    }

    #[test]
    fn a_message_loop_ticks_wakes_and_quits() {
        let ticking = counting(Duration::from_millis(100));
        thread::sleep(Duration::from_millis(300));
        let ticks = ticking.wakes.load(Ordering::SeqCst);
        assert!(ticks >= 2, "{ticks} ticks");
        ticking.message_loop.waker().quit();
        ticking.message_loop.join();
        let (built_on, dropped) = ticking.dropped_on.lock().unwrap().unwrap();
        assert_eq!(built_on, dropped, "the state must drop where it was built");
        assert_ne!(dropped, thread::current().id());

        let waiting = counting(Duration::from_hours(1));
        let asked = Instant::now();
        waiting.message_loop.waker().wake();
        let at = waiting
            .woke
            .recv_timeout(Duration::from_millis(50))
            .expect("a wake runs on_wake at once");
        assert!(at.duration_since(asked) < Duration::from_millis(50));
        waiting.message_loop.waker().quit();
        waiting.message_loop.join();
    }
}
