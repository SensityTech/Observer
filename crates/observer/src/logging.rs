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
    let (terminal, terminal_guard) = tracing_appender::non_blocking(std::io::stderr());

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(terminal))
        .with(fmt::layer().with_ansi(false).with_writer(file))
        .init();
    Ok([terminal_guard, file_guard])
}
