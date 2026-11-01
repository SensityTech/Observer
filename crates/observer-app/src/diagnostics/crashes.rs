//! Crashes and hangs from the supervisor's events. A run's exit, dumps, hang snapshot and last
//! watchdog report are held until its state says what the end was: a crash or a hang gets a
//! record; any other end leaves its dumps as found ones.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use observer_domain::diagnostics::{AnalysisState, CrashFile, CrashFileKind, CrashKind};
use observer_domain::server::HangCause;
use observer_fxserver::console::{mask, plain, sanitize};
use observer_fxserver::health;
use observer_fxserver::supervisor::{Cause, Event, EventKind, State};

/// The most of a watchdog report a record keeps.
const MAX_STACK: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Exit {
    pub code: Option<u32>,
    pub signal: Option<i32>,
    pub description: String,
    pub expected: bool,
}

#[derive(Debug, Default)]
struct Pending {
    exit: Option<Exit>,
    dumps: Vec<PathBuf>,
    snapshot: Option<PathBuf>,
    bark: Option<String>,
    hang_cause: Option<HangCause>,
}

/// What the events of a run come to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Crash {
        kind: CrashKind,
        generation: u32,
        exit: Exit,
        hang_cause: Option<HangCause>,
        watchdog_stack: Option<String>,
        files: Vec<PathBuf>,
    },
    /// Dumps no crash or hang explains, one outcome per dump.
    Found(Vec<PathBuf>),
    /// Files the dump policy removed from Observer's crash folder.
    Removed(Vec<PathBuf>),
}

#[derive(Debug, Default)]
pub(super) struct Tracker {
    pending: HashMap<u32, Pending>,
}

impl Tracker {
    pub(super) fn event(&mut self, event: &Event) -> Vec<Outcome> {
        let generation = event.generation;
        let mut outcomes = Vec::new();
        match &event.kind {
            EventKind::Spawned { .. } => {
                outcomes.extend(self.settle_before(generation));
                self.pending.insert(generation, Pending::default());
            }
            EventKind::WatchdogBark { stack, .. } => {
                if let Some(pending) = self.pending.get_mut(&generation) {
                    pending.bark = Some(watchdog_stack(stack));
                }
            }
            EventKind::Exited {
                code,
                signal,
                description,
                expected,
            } => {
                if let Some(pending) = self.pending.get_mut(&generation) {
                    pending.exit = Some(Exit {
                        code: *code,
                        signal: *signal,
                        description: description.clone(),
                        expected: *expected,
                    });
                }
            }
            EventKind::CrashDumps { paths, removed } => {
                if !removed.is_empty() {
                    outcomes.push(Outcome::Removed(removed.clone()));
                }
                match self.pending.get_mut(&generation) {
                    Some(pending) => pending.dumps.extend(paths.iter().cloned()),
                    None => outcomes.extend(found(paths.clone())),
                }
            }
            EventKind::HangSnapshot { path, removed, .. } => {
                if !removed.is_empty() {
                    outcomes.push(Outcome::Removed(removed.clone()));
                }
                if let Some(pending) = self.pending.get_mut(&generation) {
                    pending.snapshot.clone_from(path);
                }
            }
            EventKind::State { to, cause, .. } => {
                outcomes.extend(self.settle_before(generation));
                if let (State::Hung, Cause::Hang { cause }) = (to, cause)
                    && let Some(pending) = self.pending.get_mut(&generation)
                {
                    pending.hang_cause = Some(hang_cause(*cause));
                }
                let ended = self
                    .pending
                    .get(&generation)
                    .is_some_and(|pending| pending.exit.is_some());
                if ended && let Some(pending) = self.pending.remove(&generation) {
                    outcomes.extend(settle(generation, pending, *to, cause));
                }
            }
            _ => {}
        }
        outcomes
    }

    /// Runs older than `generation` that ended without a state of their own: a restart's.
    fn settle_before(&mut self, generation: u32) -> Vec<Outcome> {
        let older: Vec<u32> = self
            .pending
            .iter()
            .filter(|(run, pending)| **run < generation && pending.exit.is_some())
            .map(|(run, _)| *run)
            .collect();
        older
            .into_iter()
            .filter_map(|run| self.pending.remove(&run))
            .flat_map(|pending| found(pending.dumps))
            .collect()
    }
}

fn settle(generation: u32, pending: Pending, to: State, cause: &Cause) -> Vec<Outcome> {
    let kind = match (to, cause) {
        (State::Crashed, Cause::Exited { .. }) => CrashKind::Crash,
        (State::Crashed, Cause::Hang { .. }) => CrashKind::Hang,
        _ => return found(pending.dumps),
    };
    let Some(exit) = pending.exit else {
        return found(pending.dumps);
    };
    let mut files = pending.dumps;
    files.extend(pending.snapshot);
    vec![Outcome::Crash {
        kind,
        generation,
        exit,
        hang_cause: pending.hang_cause,
        watchdog_stack: pending.bark,
        files,
    }]
}

/// One found record per dump stem: a minidump and its full dump go together.
fn found(paths: Vec<PathBuf>) -> Vec<Outcome> {
    let mut groups: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for path in paths {
        let stem = stem(&path);
        match groups.iter_mut().find(|(known, _)| *known == stem) {
            Some((_, group)) => group.push(path),
            None => groups.push((stem, vec![path])),
        }
    }
    groups
        .into_iter()
        .map(|(_, group)| Outcome::Found(group))
        .collect()
}

fn stem(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = name.strip_suffix(".dmp").unwrap_or(&name);
    name.strip_suffix("-full").unwrap_or(name).to_owned()
}

fn hang_cause(cause: health::HangCause) -> HangCause {
    match cause {
        health::HangCause::NoHeartbeat => HangCause::NoHeartbeat,
        health::HangCause::NoHttp => HangCause::NoHttp,
    }
}

/// A watchdog report as a record keeps it: control sequences out, secrets masked, at most 4 KiB.
fn watchdog_stack(stack: &str) -> String {
    let plain = plain(&sanitize(stack));
    let mut kept = mask(&plain).unwrap_or(plain);
    if kept.len() > MAX_STACK {
        let mut end = MAX_STACK;
        while !kept.is_char_boundary(end) {
            end -= 1;
        }
        kept.truncate(end);
    }
    kept
}

/// The files of a record, each with its size now; a file already gone is listed as not kept.
pub(super) fn files(paths: &[PathBuf]) -> Vec<CrashFile> {
    paths
        .iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            let bytes = std::fs::metadata(path).map(|metadata| metadata.len()).ok();
            Some(CrashFile {
                kind: CrashFileKind::of(&name),
                name,
                bytes: bytes.unwrap_or(0),
                kept: bytes.is_some(),
            })
        })
        .collect()
}

/// The file the analysis reads: the minidump, Observer's own hang dump or report, FXServer's live
/// dump, and a full dump only when it is the only one.
pub(super) fn readable(files: &[CrashFile]) -> Option<&CrashFile> {
    let kept = || files.iter().filter(|file| file.kept);
    [
        CrashFileKind::Minidump,
        CrashFileKind::HangDump,
        CrashFileKind::HangReport,
        CrashFileKind::LiveDump,
    ]
    .into_iter()
    .find_map(|kind| kept().find(|file| file.kind == kind))
    .or_else(|| {
        let mut full = kept().filter(|file| file.kind == CrashFileKind::FullDump);
        let only = full.next();
        only.filter(|_| kept().count() == 1)
    })
}

/// The state and signature of a record with nothing to read: Linux leaves no dump, and a
/// Windows crash may not either.
pub(super) fn without_dump(exit: &Exit) -> (AnalysisState, Option<String>) {
    let signature = match (exit.signal, exit.code) {
        (Some(signal), _) => Some(match observer_fxserver::exit::signal_name(signal) {
            Some((name, _)) => format!("signal {signal} {name}"),
            None => format!("signal {signal}"),
        }),
        (None, Some(code)) => Some(format!("exit 0x{code:08X}")),
        (None, None) => None,
    };
    (AnalysisState::NoDump, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(generation: u32, kind: EventKind) -> Event {
        Event {
            at_ms: 0,
            generation,
            kind,
        }
    }

    fn spawned(generation: u32) -> Event {
        event(
            generation,
            EventKind::Spawned {
                pid: 3312,
                probe: "127.0.0.1:30120".parse().unwrap(),
                build: "/srv/observer/artifacts/35245".into(),
                launch: Box::new(observer_domain::launch::LaunchSettings {
                    data_path: "/srv/fivem".to_owned(),
                    cfg_file: "server.cfg".to_owned(),
                    onesync: observer_domain::launch::OneSync::On,
                    auto_start: false,
                    txadmin_events: false,
                    extra_arguments: Vec::new(),
                    hang_detection: true,
                    hang_timeout_seconds: 360,
                }),
            },
        )
    }

    fn exited(generation: u32, expected: bool) -> Event {
        event(
            generation,
            EventKind::Exited {
                code: Some(0xFFFF_FFFE),
                signal: None,
                description: "exit code 0xFFFFFFFE".to_owned(),
                expected,
            },
        )
    }

    fn dumps(generation: u32, names: &[&str]) -> Event {
        event(
            generation,
            EventKind::CrashDumps {
                paths: names.iter().map(PathBuf::from).collect(),
                removed: Vec::new(),
            },
        )
    }

    fn state(generation: u32, to: State, cause: Cause) -> Event {
        event(
            generation,
            EventKind::State {
                from: State::Online,
                to,
                cause,
            },
        )
    }

    fn crashed() -> Cause {
        Cause::Exited {
            description: "exit code 0xFFFFFFFE".to_owned(),
        }
    }

    fn run(tracker: &mut Tracker, events: &[Event]) -> Vec<Outcome> {
        events.iter().flat_map(|event| tracker.event(event)).collect()
    }

    #[test]
    fn a_crash_with_dumps_is_one_record_holding_them() {
        let mut tracker = Tracker::default();

        let outcomes = run(
            &mut tracker,
            &[
                spawned(1),
                exited(1, false),
                dumps(1, &["a1b2.dmp", "a1b2-full.dmp"]),
                state(1, State::Crashed, crashed()),
            ],
        );

        assert_eq!(
            outcomes,
            [Outcome::Crash {
                kind: CrashKind::Crash,
                generation: 1,
                exit: Exit {
                    code: Some(0xFFFF_FFFE),
                    signal: None,
                    description: "exit code 0xFFFFFFFE".to_owned(),
                    expected: false
                },
                hang_cause: None,
                watchdog_stack: None,
                files: vec![PathBuf::from("a1b2.dmp"), PathBuf::from("a1b2-full.dmp")],
            }]
        );
    }

    #[test]
    fn a_hang_holds_its_snapshot_its_live_dump_and_the_last_watchdog_stack() {
        let mut tracker = Tracker::default();

        let outcomes = run(
            &mut tracker,
            &[
                spawned(2),
                event(
                    2,
                    EventKind::WatchdogBark {
                        thread: "svMain".to_owned(),
                        seconds: 45,
                        stack: "\u{1b}[31mroot\u{1b}[0m sv_licenseKey cfxk_secret".to_owned(),
                    },
                ),
                state(
                    2,
                    State::Hung,
                    Cause::Hang {
                        cause: health::HangCause::NoHeartbeat,
                    },
                ),
                event(
                    2,
                    EventKind::HangSnapshot {
                        path: Some(PathBuf::from("hang-2-20261029T101500Z.dmp")),
                        bytes: 40_000,
                        took_ms: 120,
                        failure: None,
                        removed: vec![PathBuf::from("old.dmp")],
                    },
                ),
                exited(2, false),
                dumps(2, &["livedump-3.dmp"]),
                state(
                    2,
                    State::Crashed,
                    Cause::Hang {
                        cause: health::HangCause::NoHeartbeat,
                    },
                ),
            ],
        );

        assert_eq!(outcomes[0], Outcome::Removed(vec![PathBuf::from("old.dmp")]));
        let Outcome::Crash {
            kind,
            hang_cause,
            watchdog_stack,
            files,
            ..
        } = &outcomes[1]
        else {
            panic!("{outcomes:?}")
        };
        assert_eq!(*kind, CrashKind::Hang);
        assert_eq!(*hang_cause, Some(HangCause::NoHeartbeat));
        assert_eq!(
            watchdog_stack.as_deref(),
            Some("root sv_licenseKey cfxk_[REDACTED]")
        );
        assert_eq!(
            files,
            &[
                PathBuf::from("livedump-3.dmp"),
                PathBuf::from("hang-2-20261029T101500Z.dmp")
            ]
        );
    }

    #[test]
    fn an_expected_stop_leaving_a_live_dump_is_a_found_one() {
        let mut tracker = Tracker::default();

        let outcomes = run(
            &mut tracker,
            &[
                spawned(1),
                exited(1, true),
                dumps(1, &["livedump-1.dmp"]),
                state(
                    1,
                    State::Stopped,
                    Cause::Exited {
                        description: "exit code 1".to_owned(),
                    },
                ),
            ],
        );

        assert_eq!(outcomes, [Outcome::Found(vec![PathBuf::from("livedump-1.dmp")])]);
    }

    #[test]
    fn a_bind_error_is_no_crash() {
        let mut tracker = Tracker::default();

        let outcomes = run(
            &mut tracker,
            &[
                spawned(1),
                exited(1, false),
                state(
                    1,
                    State::Crashed,
                    Cause::BindError {
                        address: "127.0.0.1:30120".to_owned(),
                    },
                ),
            ],
        );

        assert_eq!(outcomes, []);
    }

    #[test]
    fn a_crash_loop_is_one_record_per_crash() {
        let mut tracker = Tracker::default();
        let mut events = Vec::new();
        for generation in 1..=3 {
            events.extend([
                spawned(generation),
                exited(generation, false),
                state(generation, State::Crashed, crashed()),
            ]);
        }
        events.push(state(3, State::CrashLoop, Cause::CrashLoop { failures: 3 }));

        let outcomes = run(&mut tracker, &events);

        let generations: Vec<u32> = outcomes
            .iter()
            .map(|outcome| match outcome {
                Outcome::Crash { generation, .. } => *generation,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(generations, [1, 2, 3]);
    }

    #[test]
    fn a_restart_s_dumps_are_found_ones_and_so_are_those_of_an_earlier_observer() {
        let mut tracker = Tracker::default();

        let outcomes = run(
            &mut tracker,
            &[
                dumps(0, &["old.dmp", "old-full.dmp"]),
                spawned(1),
                exited(1, true),
                dumps(1, &["livedump-1.dmp"]),
                spawned(2),
            ],
        );

        assert_eq!(
            outcomes,
            [
                Outcome::Found(vec![PathBuf::from("old.dmp"), PathBuf::from("old-full.dmp")]),
                Outcome::Found(vec![PathBuf::from("livedump-1.dmp")]),
            ]
        );
    }

    #[test]
    fn the_analysis_reads_the_minidump_before_a_full_dump() {
        let file = |name: &str, kept: bool| CrashFile {
            name: name.to_owned(),
            bytes: 1,
            kind: CrashFileKind::of(name),
            kept,
        };

        let both = [file("a-full.dmp", true), file("a.dmp", true)];
        let full_only = [file("a-full.dmp", true)];
        let gone = [
            file("a.dmp", false),
            file("a-full.dmp", true),
            file("notes.txt", true),
        ];

        assert_eq!(readable(&both).unwrap().name, "a.dmp");
        assert_eq!(readable(&full_only).unwrap().name, "a-full.dmp");
        assert_eq!(readable(&gone), None);
    }

    #[test]
    fn a_crash_without_a_dump_is_signed_by_its_exit() {
        let exit = |code, signal| Exit {
            code,
            signal,
            description: String::new(),
            expected: false,
        };

        assert_eq!(
            without_dump(&exit(None, Some(11))),
            (AnalysisState::NoDump, Some("signal 11 SIGSEGV".to_owned()))
        );
        assert_eq!(
            without_dump(&exit(Some(0xC000_0005), None)).1.as_deref(),
            Some("exit 0xC0000005")
        );
    }
}
