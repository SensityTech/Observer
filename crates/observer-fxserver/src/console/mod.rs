//! The server console: FXServer's output cut into lines, cleaned, stripped of secrets, numbered
//! and kept in a bounded ring that clients can resume from, with a copy in daily log files.

mod assemble;
mod log;
mod mask;
mod sanitize;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use tokio::sync::broadcast;

pub use assemble::{Assembled, FLUSH_AFTER, LineAssembler, MAX_LINE};
pub use log::{ConsoleLog, LogFile, LogLimits};
pub use mask::{mask, mask_command, mask_command_line};
pub use sanitize::{plain, sanitize};

use log::LogQueue;

const RING_LINES: usize = 10_000;
const RING_BYTES: usize = 4 * 1024 * 1024;
/// What a line costs in the ring beyond its text.
const LINE_OVERHEAD: usize = 64;
/// Lines a live subscriber may fall behind before it is told it lagged.
const LIVE_CAPACITY: usize = 1024;
/// Numbers available to each millisecond Observer runs, far more than it ever prints.
const SEQS_PER_MS: u64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsoleLine {
    /// Numbers every line Observer saw since it started, across server restarts. Counted from the
    /// clock at start, so that a number from an earlier Observer is below this one's.
    pub seq: u64,
    pub ts_ms: i64,
    pub generation: u32,
    pub kind: LineKind,
    /// Who typed a command, or which part of Observer wrote a line.
    pub actor: Option<String>,
    /// Printable text and colour sequences only, secrets masked.
    pub text: String,
    pub flags: LineFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Stdout,
    Stderr,
    /// A command an operator typed, echoed once FXServer has it.
    Command,
    /// A note from Observer itself, such as a start or a stop.
    Observer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LineFlags {
    pub truncated: bool,
    pub no_eol: bool,
    pub masked: bool,
}

/// What [`ConsoleHub::subscribe_from`] hands a client: the lines it missed, then every new line.
pub struct Subscription {
    pub backlog: Vec<Arc<ConsoleLine>>,
    /// Lines the client asked for are gone from the ring.
    pub gap: bool,
    /// The number the client resumes from is not one this hub gave: Observer restarted since.
    pub restarted: bool,
    /// The number of the newest line pushed, in the backlog or not: before the first line, the
    /// one just below it.
    pub last: u64,
    pub live: broadcast::Receiver<Arc<ConsoleLine>>,
}

/// The console's single point of entry: every line gets its number here, goes into the ring and
/// out to subscribers in one step.
pub struct ConsoleHub {
    ring: Mutex<Ring>,
    live: broadcast::Sender<Arc<ConsoleLine>>,
    log: Option<LogQueue>,
}

struct Ring {
    lines: VecDeque<Arc<ConsoleLine>>,
    bytes: usize,
    /// The number of this hub's first line, pushed or to come.
    first_seq: u64,
    next_seq: u64,
    max_lines: usize,
    max_bytes: usize,
}

impl ConsoleHub {
    pub fn new(log: Option<&ConsoleLog>) -> Self {
        let now = u64::try_from(jiff::Timestamp::now().as_millisecond()).unwrap_or(0);
        Self::with_bounds(log, RING_LINES, RING_BYTES, now * SEQS_PER_MS + 1)
    }

    fn with_bounds(log: Option<&ConsoleLog>, max_lines: usize, max_bytes: usize, first_seq: u64) -> Self {
        Self {
            ring: Mutex::new(Ring {
                lines: VecDeque::new(),
                bytes: 0,
                first_seq,
                next_seq: first_seq,
                max_lines,
                max_bytes,
            }),
            live: broadcast::channel(LIVE_CAPACITY).0,
            log: log.map(ConsoleLog::queue),
        }
    }

    /// Masks secrets in `text`, which must already be sanitised, numbers the line and publishes it.
    /// Observer's own notes are sanitised here: they quote names a resource chose, such as a
    /// program's file or a dump's, and a newline in one would start a forged line in the logs.
    pub fn push(
        &self,
        generation: u32,
        kind: LineKind,
        actor: Option<String>,
        text: String,
        mut flags: LineFlags,
    ) -> Arc<ConsoleLine> {
        let text = if kind == LineKind::Observer {
            sanitize(&text)
        } else {
            text
        };
        let text = match mask(&text) {
            Some(masked) => {
                flags.masked = true;
                masked
            }
            None => text,
        };
        let mut ring = self.lock();
        let line = Arc::new(ConsoleLine {
            seq: ring.next_seq,
            ts_ms: jiff::Timestamp::now().as_millisecond(),
            generation,
            kind,
            actor,
            text,
            flags,
        });
        ring.next_seq += 1;
        ring.bytes += cost(&line);
        ring.lines.push_back(Arc::clone(&line));
        while ring.lines.len() > ring.max_lines || ring.bytes > ring.max_bytes {
            let Some(oldest) = ring.lines.pop_front() else {
                break;
            };
            ring.bytes -= cost(&oldest);
        }
        // Under the ring's lock, so that subscribers see lines in ring order.
        let _ = self.live.send(Arc::clone(&line));
        drop(ring);
        if let Some(log) = &self.log {
            log.offer(Arc::clone(&line));
        }
        line
    }

    /// The lines after `since` still in the ring, and every line pushed from then on, with nothing
    /// missed or repeated in between. `None` asks for the whole ring. A gap is only ever lines of
    /// this hub's gone from the ring: a `since` it never gave comes from an Observer before it,
    /// and everything is new to that client.
    pub fn subscribe_from(&self, since: Option<u64>) -> Subscription {
        let ring = self.lock();
        let live = self.live.subscribe();
        let last = ring.next_seq - 1;
        let first = ring.lines.front().map_or(ring.next_seq, |line| line.seq);
        let ours = |since: u64| (ring.first_seq - 1..=last).contains(&since);
        let (after, gap, restarted) = match since {
            None => (0, false, false),
            Some(since) if ours(since) => (since, since + 1 < first, false),
            Some(_) => (0, false, true),
        };
        let backlog = ring
            .lines
            .iter()
            .filter(|line| line.seq > after)
            .cloned()
            .collect();
        Subscription {
            backlog,
            gap,
            restarted,
            last,
            live,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Ring> {
        self.ring.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn cost(line: &ConsoleLine) -> usize {
    LINE_OVERHEAD + line.text.len() + line.actor.as_ref().map_or(0, String::len)
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    /// A hub numbering from 1, for readable numbers.
    fn hub() -> ConsoleHub {
        ConsoleHub::with_bounds(None, RING_LINES, RING_BYTES, 1)
    }

    fn push(hub: &ConsoleHub, text: &str) -> u64 {
        hub.push(1, LineKind::Stdout, None, text.to_owned(), LineFlags::default())
            .seq
    }

    fn seqs(lines: &[Arc<ConsoleLine>]) -> Vec<u64> {
        lines.iter().map(|line| line.seq).collect()
    }

    #[test]
    fn lines_are_numbered_in_order_and_secrets_masked_before_they_are_kept() {
        let hub = hub();

        let first = hub.push(
            3,
            LineKind::Command,
            Some("alice".to_owned()),
            "set rcon_password x".to_owned(),
            LineFlags::default(),
        );
        let second = push(&hub, "plain");

        assert_eq!((first.seq, second), (1, 2));
        assert_eq!(first.text, "set rcon_password [REDACTED]");
        assert!(first.flags.masked);
        assert_eq!(first.generation, 3);
        assert_eq!(
            hub.subscribe_from(None).backlog[0].text,
            "set rcon_password [REDACTED]"
        );
    }

    #[tokio::test]
    async fn a_note_quoting_a_name_with_a_newline_stays_one_line_in_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = ConsoleLog::start(dir.path(), LogLimits::default(), jiff::tz::TimeZone::UTC).unwrap();
        let hub = ConsoleHub::new(Some(&log));
        let name = "x\n2026-10-29T10:00:00.000+00:00 #1 CMD ada | quit";

        let note = hub.push(
            1,
            LineKind::Observer,
            Some("observer".to_owned()),
            format!("security alert: FXServer's tree started {name} (pid 4120, parent FXServer 7)"),
            LineFlags::default(),
        );
        log.close().await;

        assert!(!note.text.contains('\n'), "{:?}", note.text);
        let file = std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let written = std::fs::read_to_string(file).unwrap();
        assert_eq!(written.lines().count(), 1, "{written}");
    }

    #[test]
    fn the_ring_keeps_at_most_its_line_count() {
        let hub = ConsoleHub::with_bounds(None, 3, usize::MAX, 1);
        for index in 0..5 {
            push(&hub, &format!("line {index}"));
        }

        assert_eq!(seqs(&hub.subscribe_from(None).backlog), [3, 4, 5]);
    }

    #[test]
    fn the_ring_keeps_at_most_its_byte_count() {
        let hub = ConsoleHub::with_bounds(None, usize::MAX, 3 * (LINE_OVERHEAD + 10), 1);
        for _ in 0..5 {
            push(&hub, "0123456789");
        }

        assert_eq!(seqs(&hub.subscribe_from(None).backlog), [3, 4, 5]);
    }

    #[test]
    fn a_client_resuming_gets_only_what_it_missed() {
        let hub = hub();
        for index in 0..4 {
            push(&hub, &format!("line {index}"));
        }

        let subscription = hub.subscribe_from(Some(2));

        assert_eq!(seqs(&subscription.backlog), [3, 4]);
        assert!(!subscription.gap);
        assert_eq!(hub.subscribe_from(Some(4)).backlog, []);
    }

    #[test]
    fn a_client_whose_lines_were_evicted_is_told_of_the_gap() {
        let hub = ConsoleHub::with_bounds(None, 2, usize::MAX, 1);
        for index in 0..5 {
            push(&hub, &format!("line {index}"));
        }

        let behind = hub.subscribe_from(Some(1));
        let just_in_time = hub.subscribe_from(Some(3));

        assert_eq!((seqs(&behind.backlog), behind.gap), (vec![4, 5], true));
        assert_eq!(
            (seqs(&just_in_time.backlog), just_in_time.gap),
            (vec![4, 5], false)
        );
        assert!(!behind.restarted && !just_in_time.restarted);
    }

    #[test]
    fn a_number_this_hub_never_gave_means_observer_restarted_and_none_of_its_lines_are_missing() {
        let hub = ConsoleHub::with_bounds(None, 2, usize::MAX, 10);
        for index in 0..3 {
            push(&hub, &format!("line {index}"));
        }

        for since in [4, 99] {
            let resumed = hub.subscribe_from(Some(since));
            assert_eq!(
                (seqs(&resumed.backlog), resumed.gap, resumed.restarted),
                (vec![11, 12], false, true),
                "{since}"
            );
        }
    }

    #[test]
    fn an_empty_console_gives_the_number_to_resume_from_so_that_nothing_reads_as_missing() {
        let hub = ConsoleHub::with_bounds(None, RING_LINES, RING_BYTES, 10);

        let first = hub.subscribe_from(None);
        let again = hub.subscribe_from(Some(first.last));

        assert_eq!((first.backlog.len(), first.last), (0, 9));
        assert!(!again.gap && !again.restarted);
        push(&hub, "first line");
        assert_eq!(seqs(&hub.subscribe_from(Some(first.last)).backlog), [10]);
    }

    #[test]
    fn a_client_resuming_with_a_number_from_an_earlier_observer_is_told_it_restarted() {
        let earlier = ConsoleHub::new(None);
        let mut last_seen = 0;
        for index in 0..5 {
            last_seen = push(&earlier, &format!("earlier {index}"));
        }
        drop(earlier);
        // Even an Observer that starts at once and prints more than its predecessor did.
        thread::sleep(std::time::Duration::from_millis(2));
        let hub = ConsoleHub::new(None);
        for index in 0..8 {
            push(&hub, &format!("line {index}"));
        }

        let resumed = hub.subscribe_from(Some(last_seen));

        assert!(resumed.restarted && !resumed.gap);
        assert_eq!(resumed.backlog.len(), 8);
        assert_eq!(resumed.backlog[0].text, "line 0");
    }

    #[tokio::test]
    async fn a_subscription_taken_while_lines_pour_in_misses_and_repeats_nothing() {
        let hub = Arc::new(hub());
        let writer = thread::spawn({
            let hub = Arc::clone(&hub);
            move || {
                for index in 0..1000 {
                    push(&hub, &format!("line {index}"));
                }
            }
        });
        while hub.subscribe_from(None).backlog.len() < 200 {
            tokio::task::yield_now().await;
        }

        let Subscription {
            backlog, mut live, ..
        } = hub.subscribe_from(Some(100));
        writer.join().unwrap();

        let mut seen = seqs(&backlog);
        while let Ok(line) = live.try_recv() {
            seen.push(line.seq);
        }
        assert_eq!(seen, (101..=1000).collect::<Vec<_>>());
    }
}
