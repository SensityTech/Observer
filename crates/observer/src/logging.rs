use std::path::Path;

use anyhow::Context;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

const KEPT_LOG_FILES: usize = 14;

/// Logs to the terminal and to a daily file under `logs`. The returned guard flushes the file on
/// drop, so it must live until the end of `main`.
pub fn init(level: &str, logs: &Path) -> anyhow::Result<WorkerGuard> {
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
    let (file, guard) = tracing_appender::non_blocking(appender);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_ansi(false).with_writer(file))
        .init();
    Ok(guard)
}
