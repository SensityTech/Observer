use std::fs::File;
use std::io::Write;
use std::mem::ManuallyDrop;
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};

enum Message {
    Trace(String),
    Flush(mpsc::SyncSender<()>),
}

/// Structured traces on fd 3, written by a thread of their own as FXServer does, so that a reader
/// that stops reading never blocks the main loop.
pub struct Traces {
    queue: Option<mpsc::Sender<Message>>,
}

impl Traces {
    /// Opens fd 3 if the parent passed one. Like FXServer, probes it with `{"test":"test"}` and
    /// stays silent for good if that fails.
    pub fn open() -> Self {
        let Some(mut pipe) = descriptor_3() else {
            return Self { queue: None };
        };
        if pipe.write_all(br#"{"test":"test"}"#).is_err() {
            return Self { queue: None };
        }
        let (queue, messages) = mpsc::channel();
        thread::spawn(move || {
            for message in messages {
                match message {
                    Message::Trace(trace) => {
                        let _ = pipe.write_all(trace.as_bytes());
                    }
                    Message::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Self { queue: Some(queue) }
    }

    pub fn native(&self, func: &str, data: &Value) {
        let trace = json!({
            "channel": "citizen-server-impl",
            "data": data,
            "file": "fake-fxserver",
            "func": func,
            "line": 0,
        });
        if let Some(queue) = &self.queue {
            let _ = queue.send(Message::Trace(trace.to_string()));
        }
    }

    /// What `PrintStructuredTrace` produces: FXServer names the resource itself.
    pub fn script(&self, resource: &str, payload: &Value) {
        self.native(
            "operator ()",
            &json!({ "type": "script_structured_trace", "resource": resource, "payload": payload }),
        );
    }

    /// Waits until every queued trace is written, before an exit that would lose them.
    pub fn flush(&self) {
        if let Some(queue) = &self.queue {
            let (done, written) = mpsc::sync_channel(1);
            if queue.send(Message::Flush(done)).is_ok() {
                let _ = written.recv();
            }
        }
    }
}

#[cfg(unix)]
fn descriptor_3() -> Option<ManuallyDrop<File>> {
    use std::os::fd::FromRawFd;

    if !std::path::Path::new("/proc/self/fd/3").exists() {
        return None;
    }
    // SAFETY: fd 3 is open, and this process opens nothing before this runs, so it is the
    // descriptor the parent passed. ManuallyDrop leaves it open for the life of the process.
    Some(ManuallyDrop::new(unsafe { File::from_raw_fd(3) }))
}

#[cfg(windows)]
fn descriptor_3() -> Option<ManuallyDrop<File>> {
    use std::os::windows::io::{FromRawHandle, RawHandle};

    type InvalidParameterHandler = unsafe extern "C" fn(*const u16, *const u16, *const u16, u32, usize);

    unsafe extern "C" {
        fn _get_osfhandle(fd: i32) -> isize;
        fn _set_invalid_parameter_handler(
            handler: Option<InvalidParameterHandler>,
        ) -> Option<InvalidParameterHandler>;
    }

    unsafe extern "C" fn ignore(_: *const u16, _: *const u16, _: *const u16, _: u32, _: usize) {}

    // SAFETY: with a handler that returns, the CRT answers -1 for a descriptor it does not know
    // instead of ending the process.
    unsafe { _set_invalid_parameter_handler(Some(ignore)) };
    // SAFETY: a lookup in the C runtime's descriptor table, which it rebuilt at startup from the
    // block the parent put in `lpReserved2`.
    let handle = unsafe { _get_osfhandle(3) };
    if handle == -1 || handle == -2 {
        return None;
    }
    // SAFETY: the handle is open and owned by the C runtime for the life of the process;
    // ManuallyDrop leaves it there.
    Some(ManuallyDrop::new(unsafe {
        File::from_raw_handle(handle as RawHandle)
    }))
}
