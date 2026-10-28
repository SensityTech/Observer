//! The end of the Windows session. A process that loads `user32.dll` no longer gets the console's
//! logoff and shutdown events; Windows asks its top-level windows instead. This hidden window
//! answers, so that signing out or shutting down stops FXServer as closing the console does.

use std::io;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::{ptr, thread};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, TranslateMessage,
    WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW,
};

use super::console::{ConsoleEvent, deliver};

/// What the shutdown screen shows while Observer holds Windows back.
const BLOCK_REASON: &str = "Stopping FXServer";

/// The session window, as an address: a window handle is a plain value any thread may use.
static WINDOW: OnceLock<Result<usize, i32>> = OnceLock::new();

/// Starts the window's thread, once per process.
pub(crate) fn listen() -> io::Result<()> {
    match WINDOW.get_or_init(start) {
        Ok(_) => Ok(()),
        Err(code) => Err(io::Error::from_raw_os_error(*code)),
    }
}

#[cfg(test)]
pub(crate) fn window() -> HWND {
    match WINDOW.get() {
        Some(Ok(address)) => *address as HWND,
        _ => ptr::null_mut(),
    }
}

fn start() -> Result<usize, i32> {
    let (created, window) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("observer-session".to_owned())
        .spawn(move || {
            let hwnd = create();
            let failed = hwnd.is_null();
            let _ = created.send(if failed {
                Err(io::Error::last_os_error().raw_os_error().unwrap_or_default())
            } else {
                Ok(hwnd as usize)
            });
            if failed {
                return;
            }
            let mut message = MSG::default();
            // SAFETY: `message` is a valid MSG to write into; the loop ends with the process.
            while unsafe { GetMessageW(&raw mut message, ptr::null_mut(), 0, 0) } > 0 {
                // SAFETY: `message` was just filled by GetMessageW.
                unsafe {
                    TranslateMessage(&raw const message);
                    DispatchMessageW(&raw const message);
                }
            }
        })
        .map_err(|error| error.raw_os_error().unwrap_or_default())?;
    window.recv().unwrap_or(Err(0))
}

fn create() -> HWND {
    let class = wide("ObserverSession");
    // SAFETY: a null name asks for the module of the running executable.
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(on_message),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..WNDCLASSW::default()
    };
    // SAFETY: the class names a window procedure that lives as long as the process, and the
    // strings outlive both calls. A top-level window, not a message-only one: only top-level
    // windows hear the end of the session. It is never shown.
    unsafe {
        RegisterClassW(&raw const window_class);
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    }
}

unsafe extern "system" fn on_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_QUERYENDSESSION => {
            let reason = wide(BLOCK_REASON);
            // SAFETY: `hwnd` is this thread's window and `reason` outlives the call.
            unsafe { ShutdownBlockReasonCreate(hwnd, reason.as_ptr()) };
            // Observer never vetoes the end of a session: it only asks for the time to stop.
            TRUE as LRESULT
        }
        WM_ENDSESSION => {
            if wparam != 0 {
                // The process ends once this returns: hold it until the stop is done.
                deliver(ConsoleEvent::Close);
            }
            // SAFETY: `hwnd` is this thread's window.
            unsafe { ShutdownBlockReasonDestroy(hwnd) };
            0
        }
        // SAFETY: the arguments are the ones Windows passed in.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
