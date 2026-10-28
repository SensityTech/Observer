use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use tracing::warn;

use super::{ConsoleLine, LineKind, plain};

/// Lines waiting for the writer. Past this, lines are dropped rather than slowing the console.
const QUEUE: usize = 8192;
/// Lines reach the file at most this late, even while they keep coming.
const FLUSH_EVERY: Duration = Duration::from_secs(1);
const BUFFER: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct LogLimits {
    /// A file reaching this size is continued in `console-<date>.<n>.log`.
    pub file_bytes: u64,
    pub keep_days: i32,
    /// The oldest files go first once all of them together pass this.
    pub total_bytes: u64,
}

impl Default for LogLimits {
    fn default() -> Self {
        Self {
            file_bytes: 256 * 1024 * 1024,
            keep_days: 14,
            total_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

enum Entry {
    Line(Arc<ConsoleLine>),
    Close,
}

#[derive(Clone)]
pub(super) struct LogQueue {
    entries: mpsc::SyncSender<Entry>,
    dropped: Arc<AtomicU64>,
}

impl LogQueue {
    /// Never waits: a full queue means a disk that cannot keep up, and the console comes first.
    pub(super) fn offer(&self, line: Arc<ConsoleLine>) {
        if let Err(mpsc::TrySendError::Full(_)) = self.entries.try_send(Entry::Line(line)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Writes every console line to `console-YYYY-MM-DD[.N].log` in its folder, one line each with a
/// local RFC 3339 timestamp, the generation and the kind, without colours. A new file starts at
/// local midnight and when one gets too big; old files go after `keep_days` or when the folder
/// gets too big.
pub struct ConsoleLog {
    queue: LogQueue,
    writer: thread::JoinHandle<()>,
}

impl ConsoleLog {
    pub fn start(dir: impl Into<PathBuf>, limits: LogLimits, zone: TimeZone) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let mut files = LogFiles {
            dir,
            limits,
            zone,
            current: None,
        };
        files.retain(None)?;
        let (entries, received) = mpsc::sync_channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let writer = thread::Builder::new().name("console-log".to_owned()).spawn({
            let dropped = Arc::clone(&dropped);
            move || files.run(&received, &dropped)
        })?;
        Ok(Self {
            queue: LogQueue { entries, dropped },
            writer,
        })
    }

    pub(super) fn queue(&self) -> LogQueue {
        self.queue.clone()
    }

    /// Writes what is queued, syncs the file and stops the writer.
    pub async fn close(self) {
        let Self { queue, writer } = self;
        let _ = tokio::task::spawn_blocking(move || {
            let _ = queue.entries.send(Entry::Close);
            let _ = writer.join();
        })
        .await;
    }
}

struct LogFiles {
    dir: PathBuf,
    limits: LogLimits,
    zone: TimeZone,
    current: Option<Current>,
}

struct Current {
    date: Date,
    index: u32,
    file: BufWriter<File>,
    size: u64,
}

impl LogFiles {
    fn run(&mut self, entries: &mpsc::Receiver<Entry>, dropped: &AtomicU64) {
        let mut failing = false;
        let mut flush_at = Instant::now() + FLUSH_EVERY;
        loop {
            let mut result = match entries.recv_timeout(flush_at.saturating_duration_since(Instant::now())) {
                Ok(Entry::Line(line)) => {
                    let lost = dropped.swap(0, Ordering::Relaxed);
                    if lost > 0 {
                        warn!(lines = lost, "the console log fell behind and dropped lines");
                    }
                    self.write(&line, lost)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
                Ok(Entry::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if Instant::now() >= flush_at {
                result = result.and_then(|()| self.flush());
                flush_at = Instant::now() + FLUSH_EVERY;
            }
            match result {
                Err(error) if !failing => {
                    warn!(%error, dir = %self.dir.display(), "cannot write the console log");
                    failing = true;
                }
                Ok(()) => failing = false,
                Err(_) => {}
            }
        }
        if let Err(error) = self.close_current() {
            warn!(%error, "cannot finish the console log file");
        }
    }

    fn write(&mut self, line: &ConsoleLine, lost: u64) -> io::Result<()> {
        let at = Timestamp::from_millisecond(line.ts_ms)
            .unwrap_or_else(|_| Timestamp::now())
            .to_zoned(self.zone.clone());
        let stamp = at.strftime("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string();
        let mut record = String::new();
        if lost > 0 {
            let _ = writeln!(
                record,
                "{stamp} #{} OBS [{lost} lines not written: the log could not keep up]",
                line.generation
            );
        }
        record.push_str(&format_line(&stamp, line));

        let date = at.date();
        let full = self.current.as_ref().is_some_and(|current| {
            current.size > 0 && current.size + record.len() as u64 > self.limits.file_bytes
        });
        if self.current.as_ref().is_none_or(|current| current.date != date) || full {
            self.open(date)?;
        }
        let Some(current) = &mut self.current else {
            return Ok(());
        };
        current.file.write_all(record.as_bytes())?;
        current.size += record.len() as u64;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.current {
            Some(current) => current.file.flush(),
            None => Ok(()),
        }
    }

    /// Moves on to the file for `date`: the next one after the current file of the same day, or
    /// the last one of that day if it still has room, as after a restart.
    fn open(&mut self, date: Date) -> io::Result<()> {
        let previous = self.current.as_ref().map(|current| (current.date, current.index));
        self.close_current()?;
        let index = match previous {
            Some((day, index)) if day == date => index + 1,
            _ => match self.files()?.into_iter().rfind(|file| file.date == date) {
                Some(last) if last.size < self.limits.file_bytes => last.index,
                Some(last) => last.index + 1,
                None => 0,
            },
        };
        let path = self.dir.join(file_name(date, index));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        self.current = Some(Current {
            date,
            index,
            file: BufWriter::with_capacity(BUFFER, file),
            size,
        });
        self.retain(Some(&path))
    }

    fn close_current(&mut self) -> io::Result<()> {
        if let Some(mut current) = self.current.take() {
            current.file.flush()?;
            current.file.get_ref().sync_data()?;
        }
        Ok(())
    }

    /// Deletes files older than `keep_days`, then the oldest ones while the folder is too big.
    /// `keep` is never deleted.
    fn retain(&self, keep: Option<&Path>) -> io::Result<()> {
        let today = Timestamp::now().to_zoned(self.zone.clone()).date();
        let oldest = today
            .checked_sub(self.limits.keep_days.days())
            .unwrap_or(Date::MIN);
        let mut files = self.files()?;
        let mut total: u64 = files.iter().map(|file| file.size).sum();
        files.retain(|file| Some(file.path.as_path()) != keep);
        for file in files {
            if file.date < oldest || total > self.limits.total_bytes {
                fs::remove_file(&file.path)?;
                total -= file.size;
            }
        }
        Ok(())
    }

    /// The log files in the folder, oldest first.
    fn files(&self) -> io::Result<Vec<LogFile>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let Some((date, index)) = entry.file_name().to_str().and_then(parse_name) else {
                continue;
            };
            files.push(LogFile {
                date,
                index,
                size: entry.metadata()?.len(),
                path: entry.path(),
            });
        }
        files.sort_by_key(|file| (file.date, file.index));
        Ok(files)
    }
}

struct LogFile {
    date: Date,
    index: u32,
    size: u64,
    path: PathBuf,
}

fn format_line(stamp: &str, line: &ConsoleLine) -> String {
    let kind = match line.kind {
        LineKind::Stdout => "OUT",
        LineKind::Stderr => "ERR",
        LineKind::Command => "CMD",
        LineKind::Observer => "OBS",
    };
    let actor = line
        .actor
        .as_ref()
        .map(|actor| format!("{actor} | "))
        .unwrap_or_default();
    let cut = if line.flags.truncated { " [truncated]" } else { "" };
    format!(
        "{stamp} #{} {kind} {actor}{}{cut}\n",
        line.generation,
        plain(&line.text)
    )
}

fn file_name(date: Date, index: u32) -> String {
    if index == 0 {
        format!("console-{date}.log")
    } else {
        format!("console-{date}.{index}.log")
    }
}

fn parse_name(name: &str) -> Option<(Date, u32)> {
    let stem = name.strip_prefix("console-")?.strip_suffix(".log")?;
    let (date, index) = match stem.split_once('.') {
        Some((date, index)) => (date, index.parse().ok()?),
        None => (stem, 0),
    };
    Some((date.parse().ok()?, index))
}

#[cfg(test)]
mod tests {
    use super::super::LineFlags;
    use super::*;

    fn line(ts: &str, generation: u32, kind: LineKind, actor: Option<&str>, text: &str) -> Arc<ConsoleLine> {
        Arc::new(ConsoleLine {
            seq: 0,
            ts_ms: ts.parse::<Timestamp>().unwrap().as_millisecond(),
            generation,
            kind,
            actor: actor.map(str::to_owned),
            text: text.to_owned(),
            flags: LineFlags::default(),
        })
    }

    fn stdout(ts: &str, text: &str) -> Arc<ConsoleLine> {
        line(ts, 1, LineKind::Stdout, None, text)
    }

    async fn write_all(dir: &Path, limits: LogLimits, lines: Vec<Arc<ConsoleLine>>) {
        let log = ConsoleLog::start(dir, limits, TimeZone::fixed(jiff::tz::offset(1))).unwrap();
        for line in lines {
            log.queue().offer(line);
        }
        log.close().await;
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn today() -> Date {
        Timestamp::now()
            .to_zoned(TimeZone::fixed(jiff::tz::offset(1)))
            .date()
    }

    #[tokio::test]
    async fn each_line_carries_its_local_time_generation_kind_and_actor_without_colours() {
        let dir = tempfile::tempdir().unwrap();
        let day = today();
        let at = |time: &str| format!("{day}T{time}+01:00");

        write_all(
            dir.path(),
            LogLimits::default(),
            vec![
                line(
                    &at("04:00:00.123"),
                    12,
                    LineKind::Stdout,
                    None,
                    "\x1b[38;5;161m[    c-scripting-core] \x1b[0mCreating script environments for observer",
                ),
                line(
                    &at("04:00:00.456"),
                    12,
                    LineKind::Stderr,
                    None,
                    "SCRIPT ERROR: @foo/server.lua:12",
                ),
                line(
                    &at("04:00:01.000"),
                    12,
                    LineKind::Command,
                    Some("alice"),
                    "refresh",
                ),
                line(
                    &at("04:00:02.000"),
                    13,
                    LineKind::Observer,
                    Some("supervisor"),
                    "FXServer started",
                ),
            ],
        )
        .await;

        assert_eq!(
            fs::read_to_string(dir.path().join(format!("console-{day}.log"))).unwrap(),
            format!(
                "{day}T04:00:00.123+01:00 #12 OUT [    c-scripting-core] Creating script environments for observer\n\
                 {day}T04:00:00.456+01:00 #12 ERR SCRIPT ERROR: @foo/server.lua:12\n\
                 {day}T04:00:01.000+01:00 #12 CMD alice | refresh\n\
                 {day}T04:00:02.000+01:00 #13 OBS supervisor | FXServer started\n"
            )
        );
    }

    #[tokio::test]
    async fn a_new_file_starts_at_local_midnight() {
        let dir = tempfile::tempdir().unwrap();
        let day = today();
        let yesterday = day.yesterday().unwrap();

        write_all(
            dir.path(),
            LogLimits::default(),
            vec![
                stdout(&format!("{yesterday}T23:59:59.999+01:00"), "late"),
                // 23:00 UTC is already midnight at +01:00.
                stdout(&format!("{yesterday}T23:00:00Z"), "early"),
            ],
        )
        .await;

        assert_eq!(
            names(dir.path()),
            [format!("console-{yesterday}.log"), format!("console-{day}.log")]
        );
    }

    #[tokio::test]
    async fn a_full_file_continues_in_a_numbered_one_and_a_restart_appends_to_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let day = today();
        let limits = LogLimits {
            file_bytes: 100,
            ..LogLimits::default()
        };
        let lines = |count| {
            (0..count)
                .map(|index| stdout(&format!("{day}T10:00:00+01:00"), &format!("line {index:02}")))
                .collect()
        };

        write_all(dir.path(), limits, lines(5)).await;
        assert_eq!(
            names(dir.path()),
            [
                format!("console-{day}.1.log"),
                format!("console-{day}.2.log"),
                format!("console-{day}.log")
            ]
        );
        let last = dir.path().join(format!("console-{day}.2.log"));
        let before = fs::metadata(&last).unwrap().len();

        write_all(dir.path(), limits, lines(1)).await;

        assert!(fs::metadata(&last).unwrap().len() > before);
        assert_eq!(names(dir.path()).len(), 3);
    }

    #[tokio::test]
    async fn files_older_than_the_retention_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let day = today();
        let old = day.checked_sub(15.days()).unwrap();
        let kept = day.checked_sub(14.days()).unwrap();
        for name in [
            format!("console-{old}.log"),
            format!("console-{old}.1.log"),
            format!("console-{kept}.log"),
            "notes.txt".to_owned(),
        ] {
            fs::write(dir.path().join(name), "x\n").unwrap();
        }

        write_all(dir.path(), LogLimits::default(), Vec::new()).await;

        assert_eq!(
            names(dir.path()),
            [format!("console-{kept}.log"), "notes.txt".to_owned()]
        );
    }

    #[tokio::test]
    async fn the_oldest_files_go_first_when_the_folder_is_too_big() {
        let dir = tempfile::tempdir().unwrap();
        let day = today();
        for back in 1..=3 {
            let date = day.checked_sub(back.days()).unwrap();
            fs::write(dir.path().join(format!("console-{date}.log")), "0123456789").unwrap();
        }
        let limits = LogLimits {
            total_bytes: 25,
            ..LogLimits::default()
        };

        write_all(
            dir.path(),
            limits,
            vec![stdout(&format!("{day}T10:00:00+01:00"), "now")],
        )
        .await;

        let two_days_ago = day.checked_sub(2.days()).unwrap();
        let yesterday = day.yesterday().unwrap();
        assert_eq!(
            names(dir.path()),
            [
                format!("console-{two_days_ago}.log"),
                format!("console-{yesterday}.log"),
                format!("console-{day}.log")
            ]
        );
    }

    #[tokio::test]
    async fn lines_the_queue_could_not_hold_are_counted_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let day = today();
        let log = ConsoleLog::start(
            dir.path(),
            LogLimits::default(),
            TimeZone::fixed(jiff::tz::offset(1)),
        )
        .unwrap();
        log.queue.dropped.store(3, Ordering::Relaxed);

        log.queue()
            .offer(stdout(&format!("{day}T10:00:00+01:00"), "after the drop"));
        log.close().await;

        let text = fs::read_to_string(dir.path().join(format!("console-{day}.log"))).unwrap();
        assert_eq!(
            text,
            format!(
                "{day}T10:00:00.000+01:00 #1 OBS [3 lines not written: the log could not keep up]\n\
                 {day}T10:00:00.000+01:00 #1 OUT after the drop\n"
            )
        );
    }

    #[tokio::test]
    async fn lines_that_never_stop_coming_still_reach_the_file_every_second() {
        let dir = tempfile::tempdir().unwrap();
        let day = today();
        let log = ConsoleLog::start(
            dir.path(),
            LogLimits::default(),
            TimeZone::fixed(jiff::tz::offset(1)),
        )
        .unwrap();
        let file = dir.path().join(format!("console-{day}.log"));

        let started = Instant::now();
        while started.elapsed() < FLUSH_EVERY * 2 {
            log.queue()
                .offer(stdout(&format!("{day}T10:00:00+01:00"), "a player said hello"));
            tokio::time::sleep(FLUSH_EVERY / 5).await;
        }

        let on_disk = fs::metadata(&file).map_or(0, |metadata| metadata.len());
        log.close().await;
        assert!(on_disk > 0, "nothing on disk after {:?}", started.elapsed());
    }

    #[test]
    fn a_full_queue_drops_and_counts_instead_of_waiting() {
        let (entries, _received) = mpsc::sync_channel(2);
        let queue = LogQueue {
            entries,
            dropped: Arc::new(AtomicU64::new(0)),
        };

        for _ in 0..5 {
            queue.offer(stdout("2026-10-28T10:00:00Z", "x"));
        }

        assert_eq!(queue.dropped.load(Ordering::Relaxed), 3);
    }
}
