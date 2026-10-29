use std::io::{self, Write};
use std::path::Path;

use anyhow::Context;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

const KEPT_LOG_FILES: usize = 14;

/// Logs to the terminal and to a daily file under `logs`. The returned guards flush both on drop,
/// so they must live until the end of `main`.
pub fn init(level: &str, logs: &Path) -> anyhow::Result<[WorkerGuard; 2]> {
    let filter = match EnvFilter::try_from_env("OBSERVER_LOG") {
        Ok(filter) => filter,
        Err(_) => EnvFilter::try_new(level).with_context(|| format!("invalid log level `{level}`"))?,
    };

    // The appender prunes old files before it creates its first one, and complains on stderr when
    // the directory does not exist yet.
    std::fs::create_dir_all(logs).with_context(|| format!("cannot create {}", logs.display()))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("observer")
        .filename_suffix("log")
        .max_log_files(KEPT_LOG_FILES)
        .build(logs)
        .with_context(|| format!("cannot write logs to {}", logs.display()))?;
    let (file, file_guard) = tracing_appender::non_blocking(appender);
    // A terminal that stops reading (a text selection in a console window, a stalled pipe) would
    // otherwise hold up whichever task logs next, the supervisor's included. Lines it cannot take
    // in time are dropped.
    let (terminal, terminal_guard) = tracing_appender::non_blocking(io::stderr());

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(move || OneLine(terminal.clone())))
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(move || OneLine(file.clone())),
        )
        .init();
    Ok([terminal_guard, file_guard])
}

/// Writes each record on a line of its own: a line break inside one, which a name a resource chose
/// can hold (a program's file, a dump's), is written escaped instead of starting a forged record.
/// The formatter hands over one whole record per write.
struct OneLine<W>(W);

impl<W: Write> Write for OneLine<W> {
    fn write(&mut self, record: &[u8]) -> io::Result<usize> {
        let (body, end) = match record.strip_suffix(b"\n") {
            Some(body) => (body, &b"\n"[..]),
            None => (record, &b""[..]),
        };
        if !body.iter().any(|&byte| byte == b'\n' || byte == b'\r') {
            self.0.write_all(record)?;
            return Ok(record.len());
        }
        let mut escaped = Vec::with_capacity(record.len() + 16);
        for &byte in body {
            match byte {
                b'\n' => escaped.extend_from_slice(b"\\n"),
                b'\r' => escaped.extend_from_slice(b"\\r"),
                byte => escaped.push(byte),
            }
        }
        escaped.extend_from_slice(end);
        self.0.write_all(&escaped)?;
        Ok(record.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use super::*;

    #[derive(Clone, Default)]
    struct Written(Arc<Mutex<Vec<u8>>>);

    impl Write for Written {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_name_holding_a_line_break_stays_on_the_line_of_its_record() {
        let written = Written::default();
        let writer = written.clone();
        let subscriber = tracing_subscriber::registry().with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(move || OneLine(writer.clone())),
        );
        let name = "x\n2026-10-29T10:00:00.000000Z  INFO observer: the owner signed in\r";

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(%name, pid = 4120, "FXServer's tree started a shell");
            tracing::info!("the next record");
        });

        let log = String::from_utf8(written.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2, "{log}");
        assert!(
            lines[0].contains(
                r"name=x\n2026-10-29T10:00:00.000000Z  INFO observer: the owner signed in\r pid=4120"
            ),
            "{log}"
        );
        assert!(lines[1].ends_with("the next record"), "{log}");
    }
}
