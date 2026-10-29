//! The diagnostics' driver fed with supervisor events, over a real database and a manual clock.

use std::path::PathBuf;

use observer_domain::diagnostics::{SyscallGroup, ThreadState};
use observer_fxserver::crashes::report::{HangReport, VERSION};
use observer_fxserver::health;
use observer_fxserver::supervisor::{Cause, State};
use tempfile::TempDir;

use super::*;
use crate::{Accounts, PasswordCost, Running, Setup};

const MAIN: u32 = 3312;

struct Lab {
    _dir: TempDir,
    clock: Clock,
    db: Database,
    data_dir: PathBuf,
    diagnostics: Diagnostics,
    driver: Driver,
    feed: broadcast::Receiver<DiagnosticsEvent>,
    supervision: Supervision,
    _running: Running,
}

async fn lab() -> Lab {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
        .await
        .unwrap();
    let clock = Clock::manual("2026-10-29T10:00:00Z".parse().unwrap());
    let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
        .await
        .unwrap();
    let data_dir = dir.path().join("data");
    let (supervision, running) = Supervision::start(
        Setup {
            servers: Servers::new(db.clone(), clock.clone()),
            db: db.clone(),
            clock: clock.clone(),
            accounts,
            data_dir: data_dir.clone(),
            console_logs: data_dir.join("logs/console"),
            log: None,
        },
        |_| {},
    )
    .await
    .unwrap();
    let (diagnostics, driver, _) = Diagnostics::prepare(DiagnosticsSetup {
        db: db.clone(),
        clock: clock.clone(),
        supervision: supervision.clone(),
        data_dir: data_dir.clone(),
    })
    .await
    .unwrap();
    let feed = diagnostics.inner.feed.subscribe();
    Lab {
        _dir: dir,
        clock,
        db,
        data_dir,
        diagnostics,
        driver,
        feed,
        supervision,
        _running: running,
    }
}

impl Lab {
    fn at_ms(&self) -> i64 {
        self.clock.now().as_millisecond()
    }

    async fn event(&mut self, generation: u32, kind: EventKind) {
        let event = Event {
            at_ms: self.at_ms(),
            generation,
            kind,
        };
        self.driver.event(event).await;
    }

    async fn spawn(&mut self, generation: u32) {
        self.event(
            generation,
            EventKind::Spawned {
                pid: MAIN,
                probe: "127.0.0.1:30120".parse().unwrap(),
            },
        )
        .await;
    }

    async fn exit(&mut self, generation: u32, code: Option<u32>, signal: Option<i32>) {
        self.event(
            generation,
            EventKind::Exited {
                code,
                signal,
                description: "exit code 0xC0000005 (access violation)".to_owned(),
                expected: false,
            },
        )
        .await;
    }

    async fn crash_state(&mut self, generation: u32, cause: Cause) {
        self.event(
            generation,
            EventKind::State {
                from: State::Online,
                to: State::Crashed,
                cause,
            },
        )
        .await;
    }

    async fn start(&mut self, generation: u32, pid: u32, image: &str, command_line: &str, kind: ProcessKind) {
        self.driver
            .tree(TreeEvent {
                generation,
                at_ms: self.at_ms(),
                change: ProcessChange::Started {
                    process: ProcessInfo {
                        pid,
                        parent: Some(MAIN),
                        image: Some(PathBuf::from(image)),
                        args: command_line.split(' ').map(str::to_owned).collect(),
                        command_line: Some(command_line.to_owned()),
                        created_ms: Some(self.at_ms()),
                        stamp: Some(1),
                    },
                    kind,
                },
            })
            .await;
    }

    async fn shell(&mut self, generation: u32, pid: u32) {
        self.start(
            generation,
            pid,
            r"C:\Windows\System32\cmd.exe",
            "cmd /c echo diag",
            ProcessKind::Shell,
        )
        .await;
    }

    fn advance(&self, minutes: i64) {
        self.clock.advance(SignedDuration::from_mins(minutes));
    }

    async fn processes(&self) -> Vec<TreeProcess> {
        self.db.read(|conn| store::processes(conn, 100)).await.unwrap()
    }

    async fn crashes(&self) -> Vec<Crash> {
        let now = self.clock.now();
        self.db
            .read(move |conn| store::crashes(conn, 100, now))
            .await
            .unwrap()
    }

    /// The details of each process start the audit log holds, oldest first.
    async fn audited(&self) -> Vec<Value> {
        self.db
            .read(|conn| {
                let mut statement = conn.prepare(
                    "SELECT details FROM audit_log WHERE action = 'tree_process_started' ORDER BY id",
                )?;
                let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
                Ok::<_, observer_store::StoreError>(
                    rows.map(|row| serde_json::from_str(&row.unwrap()).unwrap())
                        .collect::<Vec<Value>>(),
                )
            })
            .await
            .unwrap()
    }

    fn notes(&self) -> Vec<String> {
        let (snapshot, _) = self.supervision.console(None);
        snapshot.lines.into_iter().map(|line| line.5).collect()
    }

    fn alerts(&self) -> AlertList {
        self.diagnostics.alerts().borrow().clone()
    }

    /// Waits for the analysis the driver started and hands it the result.
    async fn analysis(&mut self) {
        let (id, result) = self.driver.analyses.join_next().await.unwrap().unwrap();
        self.driver.analysed(id, result).await;
    }
}

#[tokio::test]
async fn a_shell_is_recorded_masked_audited_noted_and_alerted() {
    let mut lab = lab().await;
    lab.spawn(1).await;

    lab.start(
        1,
        4120,
        r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        "powershell -c iwr https://fivem:hunter2@example.com/x",
        ProcessKind::Shell,
    )
    .await;

    let processes = lab.processes().await;
    assert_eq!(processes.len(), 1);
    assert_eq!(
        processes[0].command_line.as_deref(),
        Some("powershell -c iwr https://fivem:[REDACTED]@example.com/x")
    );
    assert_eq!(
        processes[0].parent.as_ref().unwrap().name.as_deref(),
        Some(FXSERVER)
    );
    let audited = lab.audited().await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0]["kind"], "shell");
    assert_eq!(audited[0]["name"], "powershell.exe");
    assert_eq!(
        audited[0]["commandLine"],
        "powershell -c iwr https://fivem:[REDACTED]@example.com/x"
    );
    assert_eq!(audited[0]["parent"], json!({ "pid": MAIN, "name": FXSERVER }));
    assert!(
        lab.notes().contains(&format!(
            "security alert: FXServer's tree started powershell.exe (pid 4120, parent {FXSERVER} {MAIN})"
        )),
        "{:?}",
        lab.notes()
    );
    let alerts = lab.alerts().alerts;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, AlertKind::Shell);
    assert_eq!(alerts[0].name.as_deref(), Some("powershell.exe"));
    assert!(matches!(
        lab.feed.try_recv(),
        Ok(DiagnosticsEvent::Process { .. })
    ));
}

#[tokio::test]
async fn known_helpers_are_recorded_but_neither_audited_nor_alerted() {
    let mut lab = lab().await;
    lab.spawn(1).await;

    lab.start(
        1,
        4000,
        r"C:\Windows\System32\conhost.exe",
        r"\??\C:\WINDOWS\system32\conhost.exe 0x4",
        ProcessKind::ConsoleHost,
    )
    .await;

    assert_eq!(lab.processes().await.len(), 1);
    assert_eq!(lab.audited().await.len(), 0);
    assert_eq!(lab.alerts().alerts.len(), 0);
}

#[tokio::test]
async fn starts_of_one_program_count_on_one_row_and_the_run_s_end_audits_those_held() {
    let mut lab = lab().await;
    lab.spawn(1).await;

    for (index, pid) in [4120, 4121, 4122].into_iter().enumerate() {
        lab.advance(i64::try_from(index).unwrap());
        lab.shell(1, pid).await;
    }
    lab.exit(1, Some(0xC000_0005), None).await;

    let processes = lab.processes().await;
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].count, 3);
    assert_eq!(processes[0].pid, 4122);
    assert!(
        processes[0].ended_at.is_some(),
        "the run's end ends its processes"
    );
    let audited = lab.audited().await;
    assert_eq!(audited.len(), 2);
    assert!(audited[0].get("repeated").is_none());
    assert_eq!(audited[1]["repeated"], 2);
    assert_eq!(audited[1]["pid"], 4122);
}

#[tokio::test]
async fn a_start_ten_minutes_after_the_last_entry_is_audited_with_the_starts_between() {
    let mut lab = lab().await;
    lab.spawn(1).await;

    lab.shell(1, 4120).await;
    lab.advance(5);
    lab.shell(1, 4121).await;
    lab.advance(5);
    lab.shell(1, 4122).await;

    let audited = lab.audited().await;
    assert_eq!(audited.len(), 2);
    assert_eq!(audited[1]["repeated"], 2);
}

#[tokio::test]
async fn a_new_run_records_and_audits_the_same_program_afresh() {
    let mut lab = lab().await;
    lab.spawn(1).await;
    lab.shell(1, 4120).await;
    lab.exit(1, Some(1), None).await;
    lab.advance(1);

    lab.spawn(2).await;
    lab.shell(2, 5120).await;

    let processes = lab.processes().await;
    assert_eq!(processes.len(), 2);
    assert_eq!(processes[0].generation, 2);
    assert_eq!(lab.audited().await.len(), 2);
}

#[tokio::test]
async fn changes_of_a_run_whose_start_has_not_come_yet_wait_for_it() {
    let mut lab = lab().await;

    lab.shell(1, 4120).await;
    assert_eq!(lab.processes().await.len(), 0);
    lab.spawn(1).await;

    assert_eq!(lab.processes().await.len(), 1);
}

#[tokio::test]
async fn alerts_leave_the_list_after_24_hours() {
    let mut lab = lab().await;
    lab.spawn(1).await;
    lab.shell(1, 4120).await;
    assert_eq!(lab.alerts().alerts.len(), 1);

    lab.advance(24 * 60 + 1);
    lab.driver.refresh_alerts().await;

    assert_eq!(lab.alerts().alerts.len(), 0);
}

fn report(path: &Path) {
    let thread = |tid, name: &str, syscall: Option<&str>| ThreadState {
        tid,
        name: name.to_owned(),
        state: ThreadRun::Sleeping,
        cpu: Some(0.0),
        syscall: syscall.map(str::to_owned),
        syscall_group: syscall.map(|_| SyscallGroup::Lock),
    };
    let report = HangReport {
        version: VERSION,
        taken_at: "2026-10-29T10:00:00Z".parse().unwrap(),
        pid: MAIN,
        syscalls_readable: true,
        threads: vec![
            thread(MAIN, "ld-musl-x86_64.", None),
            thread(812, "UV loop: svMain", Some("futex")),
        ],
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&report).unwrap()).unwrap();
}

#[tokio::test]
async fn a_hang_is_recorded_with_its_report_s_analysis_and_alerted() {
    let mut lab = lab().await;
    let snapshot = lab.data_dir.join("crashes/hang-1-20261029T100000Z.json");
    report(&snapshot);
    let hang = Cause::Hang {
        cause: health::HangCause::NoHeartbeat,
    };
    lab.spawn(1).await;

    lab.event(
        1,
        EventKind::State {
            from: State::Degraded,
            to: State::Hung,
            cause: hang.clone(),
        },
    )
    .await;
    lab.event(
        1,
        EventKind::HangSnapshot {
            path: Some(snapshot),
            bytes: 300,
            took_ms: 510,
            failure: None,
            removed: Vec::new(),
        },
    )
    .await;
    lab.exit(1, None, Some(9)).await;
    lab.crash_state(1, hang).await;
    let pending = lab.crashes().await;
    lab.analysis().await;

    assert_eq!(pending[0].analysis_state, AnalysisState::Pending);
    let crashes = lab.crashes().await;
    assert_eq!(crashes.len(), 1);
    let crash = &crashes[0];
    assert_eq!(crash.kind, CrashKind::Hang);
    assert_eq!(
        crash.hang_cause,
        Some(observer_domain::server::HangCause::NoHeartbeat)
    );
    assert_eq!(crash.analysis_state, AnalysisState::Done);
    assert_eq!(crash.signature.as_deref(), Some("hang UV loop: svMain futex"));
    assert_eq!(crash.files[0].kind, CrashFileKind::HangReport);
    let alert = lab.alerts().alerts.remove(0);
    assert_eq!(alert.kind, AlertKind::Hang);
    let details = alert.crash.unwrap();
    assert_eq!(details.waiting_in.as_deref(), Some("futex"));
    assert!(details.snapshot);
    assert_eq!(details.thread.as_deref(), Some("UV loop: svMain"));
}

#[tokio::test]
async fn a_crash_without_a_dump_is_signed_by_how_it_ended() {
    let mut lab = lab().await;
    lab.spawn(1).await;

    lab.exit(1, None, Some(11)).await;
    lab.crash_state(
        1,
        Cause::Exited {
            description: "signal 11".to_owned(),
        },
    )
    .await;

    let crash = lab.crashes().await.remove(0);
    assert_eq!(crash.analysis_state, AnalysisState::NoDump);
    assert_eq!(crash.signature.as_deref(), Some("signal 11 SIGSEGV"));
    assert_eq!(crash.run_started, Some("2026-10-29T10:00:00Z".parse().unwrap()));
    let alert = lab.alerts().alerts.remove(0).crash.unwrap();
    assert_eq!(alert.exit.as_deref(), Some("SIGSEGV"));
    assert_eq!(alert.group_count, 1);
}

#[tokio::test]
async fn a_file_the_dump_policy_removed_stays_listed_as_no_longer_kept() {
    let mut lab = lab().await;
    let dump = lab.data_dir.join("crashes/livedump-1.dmp");
    std::fs::create_dir_all(dump.parent().unwrap()).unwrap();
    std::fs::write(&dump, b"MDMP").unwrap();
    lab.event(
        0,
        EventKind::CrashDumps {
            paths: vec![dump.clone()],
            removed: Vec::new(),
        },
    )
    .await;
    lab.analysis().await;
    std::fs::remove_file(&dump).unwrap();

    lab.event(
        0,
        EventKind::CrashDumps {
            paths: Vec::new(),
            removed: vec![dump],
        },
    )
    .await;

    let crash = lab.crashes().await.remove(0);
    assert_eq!(crash.kind, CrashKind::Found);
    assert_eq!(crash.analysis_state, AnalysisState::Unreadable);
    assert_eq!(crash.files[0].name, "livedump-1.dmp");
    assert!(!crash.files[0].kept);
    assert_eq!(lab.alerts().alerts.len(), 0, "found dumps raise no alert");
}

#[tokio::test]
async fn the_retention_deletes_what_is_past_its_time() {
    let mut lab = lab().await;
    lab.spawn(1).await;
    lab.shell(1, 4120).await;
    lab.exit(1, None, Some(11)).await;
    lab.crash_state(
        1,
        Cause::Exited {
            description: "signal 11".to_owned(),
        },
    )
    .await;

    lab.advance(31 * 24 * 60);
    lab.driver.upkeep().await;
    assert_eq!(lab.processes().await.len(), 0);
    assert_eq!(lab.crashes().await.len(), 1, "crashes stay 180 days");

    lab.advance(150 * 24 * 60);
    lab.driver.upkeep().await;
    assert_eq!(lab.crashes().await.len(), 0);
}

#[tokio::test]
async fn the_snapshot_holds_the_records_and_the_crash_folder() {
    let mut lab = lab().await;
    lab.spawn(1).await;
    lab.shell(1, 4120).await;

    let (snapshot, _) = lab.diagnostics.follow().await.unwrap();

    assert_eq!(snapshot.processes.len(), 1);
    assert_eq!(snapshot.crash_folder.max_files, 10);
    assert_eq!(snapshot.crash_folder.files, 0);
}

#[tokio::test]
async fn the_processes_an_earlier_observer_left_running_end_when_the_next_one_starts() {
    let mut lab = lab().await;
    lab.spawn(1).await;
    lab.shell(1, 4120).await;
    lab.advance(5);

    let (next, mut driver, _) = Diagnostics::prepare(DiagnosticsSetup {
        db: lab.db.clone(),
        clock: lab.clock.clone(),
        supervision: lab.supervision.clone(),
        data_dir: lab.data_dir.clone(),
    })
    .await
    .unwrap();
    driver
        .event(Event {
            at_ms: lab.at_ms(),
            generation: 1,
            kind: EventKind::Spawned {
                pid: MAIN,
                probe: "127.0.0.1:30120".parse().unwrap(),
            },
        })
        .await;

    let (snapshot, _) = next.follow().await.unwrap();
    assert_eq!(snapshot.processes.len(), 1);
    assert_eq!(snapshot.processes[0].ended_at, Some(lab.clock.now()));
}

#[tokio::test]
async fn an_analysis_an_earlier_observer_did_not_finish_runs_at_the_next_start() {
    let mut lab = lab().await;
    let dump = lab.data_dir.join("crashes/livedump-1.dmp");
    std::fs::create_dir_all(dump.parent().unwrap()).unwrap();
    std::fs::write(&dump, b"MDMP").unwrap();
    lab.event(
        0,
        EventKind::CrashDumps {
            paths: vec![dump],
            removed: Vec::new(),
        },
    )
    .await;
    assert_eq!(lab.crashes().await[0].analysis_state, AnalysisState::Pending);

    let (_, mut driver, _) = Diagnostics::prepare(DiagnosticsSetup {
        db: lab.db.clone(),
        clock: lab.clock.clone(),
        supervision: lab.supervision.clone(),
        data_dir: lab.data_dir.clone(),
    })
    .await
    .unwrap();
    let (id, result) = driver.analyses.join_next().await.unwrap().unwrap();
    driver.analysed(id, result).await;

    assert_eq!(lab.crashes().await[0].analysis_state, AnalysisState::Unreadable);
}
