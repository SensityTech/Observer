use std::sync::Weak;

use jiff::{SignedDuration, Timestamp};
use observer_domain::server::ServerState;
use observer_fxserver::health::{PlayerList, Players};
use observer_fxserver::supervisor::Stamped;
use observer_platform::{Clock as CpuClock, Member, Memory};
use observer_store::{Database, StoreError};
use tokio::sync::{oneshot, watch};

use super::*;
use crate::{Accounts, Clock, MetricsSetup, PasswordCost, Servers, Setup, Supervision};

const SLOT: Duration = Duration::from_secs(5);

/// A `/perf/` answer in FXServer 35245's bounds: svMain at `count` ticks of `ms` each so far, and
/// as many svSync ticks of 2 ms.
fn perf(count: u32, ms: u32) -> TickReport {
    use std::fmt::Write as _;
    let mut text = String::new();
    for (thread, ms) in [("svMain", ms), ("svSync", 2)] {
        let sum = f64::from(count * ms) / 1000.0;
        let _ = writeln!(text, "tickTime_count{{name=\"{thread}\"}} {count}");
        let _ = writeln!(text, "tickTime_sum{{name=\"{thread}\"}} {sum}");
        for (bound, bound_ms) in [("0.008", 8), ("0.05", 50), ("0.25", 250)] {
            let within = if ms <= bound_ms { count } else { 0 };
            let _ = writeln!(
                text,
                "tickTime_bucket{{name=\"{thread}\",le=\"{bound}\"}} {within}"
            );
        }
        let _ = writeln!(text, "tickTime_bucket{{name=\"{thread}\",le=\"+Inf\"}} {count}");
    }
    TickReport::parse(&text).unwrap()
}

fn list(players: &[(u32, u32)]) -> PlayerList {
    PlayerList {
        players: players.iter().map(|&(id, ping)| (id, Some(ping))).collect(),
        anonymous: false,
    }
}

struct Readings {
    dynamic: Option<Stamped<Players>>,
    perf: Option<Stamped<TickReport>>,
    players: Option<Stamped<PlayerList>>,
}

fn run(generation: u32, readings: Readings) -> RunReadings {
    RunReadings {
        generation,
        started_ms: 0,
        probe: std::net::SocketAddr::from(([127, 0, 0, 1], 30120)),
        tree: Weak::new(),
        dynamic: readings.dynamic,
        perf: readings.perf,
        players: readings.players,
    }
}

fn at<T>(at: Instant, value: T) -> Stamped<T> {
    Stamped { at, value }
}

/// The middle of slot `slot`, whose wake is at `start + slot × 5 s`.
fn mid_slot(start: Instant, slot: u32) -> Instant {
    start + SLOT * (slot - 1) + SLOT / 2
}

fn usage(cpu_ms: u64, started: u64) -> Usage {
    Usage {
        user: Duration::from_millis(cpu_ms),
        kernel: Duration::ZERO,
        working_set: 300 << 20,
        working_set_peak: Some(320 << 20),
        private: 250 << 20,
        faults: None,
        hard_faults: None,
        threads: Some(40),
        handles: Some(900),
        read: Some(cpu_ms * 1000),
        written: Some(cpu_ms * 10),
        started,
    }
}

fn wake(at: Instant, run: Option<RunReadings>, measures: Measures) -> Taken {
    Taken {
        at,
        run,
        measures,
        sockets: 2,
        collect_ms: 12,
    }
}

fn no_readings() -> Readings {
    Readings {
        dynamic: None,
        perf: None,
        players: None,
    }
}

/// Wakes the assembler at `start + slot × 5 s` with `run`, and gives the sample's values.
fn sample_at(
    assembler: &mut Assembler,
    start: Instant,
    slot: u32,
    run: Option<RunReadings>,
    measures: Measures,
) -> Values {
    assembler.sample(wake(start + SLOT * slot, run, measures)).values
}

fn thread(id: u32, name: &str, cpu_ms: u64) -> ThreadUsage {
    ThreadUsage {
        id,
        started: u64::from(id) * 10,
        name: name.to_owned(),
        cpu: Duration::from_millis(cpu_ms),
    }
}

/// A tree whose main process, started at 7, lists `threads`.
fn listed(threads: Vec<ThreadUsage>) -> Measures {
    Measures {
        tree: Some(TreeUsage {
            main: Some(usage(1000, 7)),
            children: Vec::new(),
            threads: Some(threads),
            totals: None,
        }),
        ..Measures::default()
    }
}

fn groups(values: &Values) -> [Option<f64>; 6] {
    GROUP_COLUMNS.map(|(_, column)| values.get(column).map(|cpu| (cpu * 1e6).round() / 1e6))
}

fn event(generation: u32, kind: EventKind) -> Event {
    Event {
        at_ms: 0,
        generation,
        kind,
    }
}

#[test]
fn the_wake_comes_after_the_slot_ends_whatever_events_come_meanwhile() {
    let mut wake = Wake::after(1_761_642_004_950);

    assert_eq!(wake.wait(1_761_642_004_950), Some(150));
    assert_eq!(
        wake.wait(1_761_642_005_050),
        Some(50),
        "an event just after the end keeps the wake"
    );
    assert_eq!(wake.wait(1_761_642_005_100), None);
    assert_eq!(wake.wait(1_761_642_027_000), None, "a late wake samples at once");
    assert_eq!(
        Wake::after(1_761_642_005_110),
        Wake {
            at: 1_761_642_010_100
        }
    );
}

#[test]
fn a_clock_set_back_finds_the_next_wake_again() {
    let mut wake = Wake::after(1_761_642_004_950);

    assert_eq!(wake.wait(1_761_638_400_000), Some(5_100));
    assert_eq!(
        wake,
        Wake {
            at: 1_761_638_405_100
        }
    );
}

#[test]
fn a_wake_samples_the_slot_that_just_ended_even_when_it_is_late() {
    let slot = 1_761_642_000_000;

    assert_eq!(slot_just_ended(slot + 5_100), 1_761_642_000);
    assert_eq!(slot_just_ended(slot + 9_999), 1_761_642_000);
    assert_eq!(
        slot_just_ended(slot + 27_000),
        1_761_642_020,
        "the slots in between stay gaps"
    );
}

#[test]
fn a_process_s_first_sample_has_no_cpu_and_a_reused_pid_starts_over() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let tree = |cpu_ms, started| Measures {
        tree: Some(TreeUsage {
            main: Some(usage(cpu_ms, started)),
            children: Vec::new(),
            threads: None,
            totals: None,
        }),
        ..Measures::default()
    };
    let running = || Some(run(1, no_readings()));

    let first = sample_at(&mut assembler, start, 1, running(), tree(1000, 7));
    let second = sample_at(&mut assembler, start, 2, running(), tree(10_000, 7));
    let reused = sample_at(&mut assembler, start, 3, running(), tree(10_500, 8));

    assert_eq!(first.get(Column::FxsCpu), None);
    assert_eq!(first.get(Column::FxsWs), Some(f64::from(300 << 20)));
    assert_eq!(first.get(Column::FxsThreads), Some(40.0));
    assert_eq!(
        first.get(Column::Children),
        Some(0.0),
        "a run without children has none"
    );
    assert_eq!(first.get(Column::ChildrenCpu), None);
    // 9 s of CPU in 5 s: 180% of one core.
    assert!((second.get(Column::FxsCpu).unwrap() - 180.0).abs() < 1e-9);
    assert!((second.get(Column::FxsReadBps).unwrap() - 1_800_000.0).abs() < 1e-6);
    assert_eq!(second.get(Column::ChildrenCpu), Some(0.0));
    assert_eq!(reused.get(Column::FxsCpu), None, "same pid, another process");
}

#[test]
fn without_a_run_or_a_tree_the_process_measures_are_unknown() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);

    let stopped = sample_at(&mut assembler, start, 1, None, Measures::default());
    let unmeasured = sample_at(
        &mut assembler,
        start,
        2,
        Some(run(1, no_readings())),
        Measures::default(),
    );

    for values in [stopped, unmeasured] {
        for column in [Column::FxsCpu, Column::FxsWs, Column::Children, Column::Players] {
            assert_eq!(values.get(column), None, "{column:?}");
        }
        assert_eq!(values.get(Column::ObsWsClients), Some(2.0));
        assert_eq!(values.get(Column::CollectMs), Some(12.0));
    }
}

#[test]
fn a_new_child_counts_at_once_in_count_and_memory_and_in_cpu_from_its_second_sample() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let child = |pid, cpu_ms| Member {
        pid,
        name: "node.exe".to_owned(),
        usage: usage(cpu_ms, u64::from(pid)),
    };
    let tree = |children| Measures {
        tree: Some(TreeUsage {
            main: Some(usage(0, 1)),
            children,
            threads: None,
            totals: None,
        }),
        ..Measures::default()
    };
    let running = || Some(run(1, no_readings()));
    sample_at(&mut assembler, start, 1, running(), tree(vec![child(10, 0)]));

    let Assembled { values, children, .. } = assembler.sample(wake(
        start + SLOT * 2,
        running(),
        tree(vec![child(10, 2500), child(11, 4000)]),
    ));

    assert_eq!(values.get(Column::Children), Some(2.0));
    assert_eq!(values.get(Column::ChildrenWs), Some(f64::from(600 << 20)));
    assert!((values.get(Column::ChildrenCpu).unwrap() - 50.0).abs() < 1e-9);
    assert_eq!(children[1].cpu, None);
    assert!((children[0].cpu.unwrap() - 50.0).abs() < 1e-9);
}

#[test]
fn the_kernel_share_faults_and_peak_come_from_the_same_readings_as_the_cpu() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let reading = |user_ms, kernel_ms, faults: u64, hard: u64| {
        let mut main = usage(0, 7);
        main.user = Duration::from_millis(user_ms);
        main.kernel = Duration::from_millis(kernel_ms);
        main.faults = Some(faults);
        main.hard_faults = Some(hard);
        Measures {
            tree: Some(TreeUsage {
                main: Some(main),
                children: Vec::new(),
                threads: None,
                totals: None,
            }),
            ..Measures::default()
        }
    };
    let running = || Some(run(1, no_readings()));

    let first = sample_at(
        &mut assembler,
        start,
        1,
        running(),
        reading(1000, 500, 90_000, 10),
    );
    let second = sample_at(
        &mut assembler,
        start,
        2,
        running(),
        reading(3500, 1500, 96_200, 13),
    );
    let wrapped = sample_at(&mut assembler, start, 3, running(), reading(4000, 1600, 400, 13));

    assert_eq!(first.get(Column::FxsCpuKernel), None);
    assert_eq!(first.get(Column::FxsFaults), None);
    assert_eq!(first.get(Column::FxsWsPeak), Some(f64::from(320 << 20)));
    // 3.5 s of CPU in 5 s, 1 s of it in the kernel.
    assert!((second.get(Column::FxsCpu).unwrap() - 70.0).abs() < 1e-9);
    assert!((second.get(Column::FxsCpuKernel).unwrap() - 20.0).abs() < 1e-9);
    assert_eq!(second.get(Column::FxsFaults), Some(6200.0));
    assert_eq!(second.get(Column::FxsHardFaults), Some(3.0));
    assert_eq!(wrapped.get(Column::FxsFaults), None, "the 32-bit counter wrapped");
    assert_eq!(wrapped.get(Column::FxsHardFaults), Some(0.0));
}

#[test]
fn thread_groups_count_a_thread_from_its_second_listing_and_empty_groups_as_zero() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let running = |generation| Some(run(generation, no_readings()));

    let first = sample_at(
        &mut assembler,
        start,
        1,
        running(1),
        listed(vec![
            thread(1, "UV loop: svMain", 1000),
            thread(2, "UV loop: svNetw", 100),
        ]),
    );
    let second = sample_at(
        &mut assembler,
        start,
        2,
        running(1),
        listed(vec![
            thread(1, "UV loop: svMain", 3500),
            thread(2, "UV loop: svNetw", 600),
            thread(3, "Thread Pool Worker", 300),
        ]),
    );
    let third = sample_at(
        &mut assembler,
        start,
        3,
        running(1),
        listed(vec![
            thread(1, "UV loop: svMain", 4000),
            thread(3, "Thread Pool Worker", 550),
        ]),
    );
    let restarted = sample_at(
        &mut assembler,
        start,
        4,
        running(2),
        listed(vec![thread(1, "UV loop: svMain", 4000)]),
    );
    let unlisted = sample_at(&mut assembler, start, 5, running(2), {
        let mut measures = listed(Vec::new());
        if let Some(tree) = measures.tree.as_mut() {
            tree.threads = None;
        }
        measures
    });

    assert_eq!(groups(&first), [None; 6], "no listing to compare with");
    assert_eq!(
        groups(&second),
        [Some(50.0), Some(10.0), Some(0.0), Some(0.0), Some(0.0), Some(0.0)],
        "the new Mono thread counts from its next listing"
    );
    assert_eq!(
        groups(&third),
        [Some(10.0), Some(0.0), Some(0.0), Some(0.0), Some(5.0), Some(0.0)],
        "svNetwork ended between the two"
    );
    assert_eq!(groups(&restarted), [None; 6], "another run");
    assert_eq!(groups(&unlisted), [None; 6]);
}

#[test]
fn the_slot_keeps_the_twenty_four_busiest_names_each_once() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let running = || Some(run(1, no_readings()));
    let threads = |grown: u64| -> Vec<ThreadUsage> {
        let mut threads: Vec<ThreadUsage> = (1..=30)
            .map(|id| thread(id, &format!("worker {id:02}"), grown * u64::from(id)))
            .collect();
        threads.push(thread(40, "libuv-worker", 0));
        threads.push(thread(41, "  libuv-worker\u{7}", grown * 100));
        threads.push(thread(42, "DevCon Thread", grown.min(1)));
        threads
    };
    sample_at(&mut assembler, start, 1, running(), listed(threads(0)));

    let Assembled { values, threads, .. } =
        assembler.sample(wake(start + SLOT * 2, running(), listed(threads(50))));

    assert_eq!(threads.len(), 24);
    // 100 × 50 ms over 5 s: 100% of one core, under one name.
    assert_eq!(threads[0].0, "libuv-worker");
    assert!((threads[0].1 - 100.0).abs() < 1e-9);
    assert_eq!(threads[1].0, "worker 30");
    assert!((threads[1].1 - 30.0).abs() < 1e-9);
    assert_eq!(threads[23].0, "worker 08");
    assert!(
        threads.iter().all(|(name, _)| name != "DevCon Thread"),
        "0.02% of a core"
    );
    let other = values.get(Column::ThrOtherCpu).unwrap();
    assert!((other - (100.0 + 465.0 + 0.02)).abs() < 1e-6, "{other}");
}

#[test]
fn the_tree_totals_give_cpu_and_started_processes_until_a_new_run() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let totals = |cpu_ms, started| {
        let mut measures = listed(Vec::new());
        if let Some(tree) = measures.tree.as_mut() {
            tree.totals = Some(TreeTotals {
                cpu: Duration::from_millis(cpu_ms),
                processes_started: started,
                private_peak: Some(900 << 20),
            });
        }
        measures
    };
    let running = |generation| Some(run(generation, no_readings()));

    let first = sample_at(&mut assembler, start, 1, running(1), totals(10_000, Some(2)));
    let second = sample_at(&mut assembler, start, 2, running(1), totals(19_500, Some(5)));
    let linux = sample_at(&mut assembler, start, 3, running(1), totals(20_000, None));
    let back = sample_at(&mut assembler, start, 4, running(1), totals(15_000, None));
    let restarted = sample_at(&mut assembler, start, 5, running(2), totals(400, Some(2)));

    assert_eq!(
        (first.get(Column::TreeCpu), first.get(Column::TreeStarted)),
        (None, None)
    );
    assert_eq!(first.get(Column::TreePrivatePeak), Some(f64::from(900 << 20)));
    assert!((second.get(Column::TreeCpu).unwrap() - 190.0).abs() < 1e-9);
    assert_eq!(second.get(Column::TreeStarted), Some(3.0));
    assert!((linux.get(Column::TreeCpu).unwrap() - 10.0).abs() < 1e-9);
    assert_eq!(linux.get(Column::TreeStarted), None);
    assert_eq!(
        back.get(Column::TreeCpu),
        None,
        "a member left without being waited for"
    );
    assert_eq!(restarted.get(Column::TreeCpu), None);
}

#[test]
fn a_tree_total_that_went_back_is_measured_against_the_one_before_until_it_does_twice() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let totals = |cpu_ms| {
        let mut measures = listed(Vec::new());
        if let Some(tree) = measures.tree.as_mut() {
            tree.totals = Some(TreeTotals {
                cpu: Duration::from_millis(cpu_ms),
                processes_started: None,
                private_peak: None,
            });
        }
        measures
    };
    let running = || Some(run(1, no_readings()));

    sample_at(&mut assembler, start, 1, running(), totals(100_000));
    let steady = sample_at(&mut assembler, start, 2, running(), totals(100_050));
    // A member that used 60 s is missing from one total, then counted in its parent's.
    let missing = sample_at(&mut assembler, start, 3, running(), totals(40_100));
    let returned = sample_at(&mut assembler, start, 4, running(), totals(100_150));
    // One that left for good.
    let left = sample_at(&mut assembler, start, 5, running(), totals(40_200));
    let still_less = sample_at(&mut assembler, start, 6, running(), totals(40_250));
    let after = sample_at(&mut assembler, start, 7, running(), totals(40_300));

    let share = |values: &Values| values.get(Column::TreeCpu);
    assert!((share(&steady).unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(share(&missing), None);
    let returned = share(&returned).unwrap();
    assert!(
        (returned - 1.0).abs() < 1e-9,
        "100 ms over the two slots, not the member's time again: {returned}%"
    );
    assert_eq!((share(&left), share(&still_less)), (None, None));
    assert!((share(&after).unwrap() - 1.0).abs() < 1e-9);
}

#[test]
fn the_disk_latency_is_the_time_per_transfer_and_unknown_without_any() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let disk = |transfers, busy_ms| Measures {
        disk: Some(Disk {
            counters: Some(DiskCounters {
                transfers,
                busy: Duration::from_millis(busy_ms),
            }),
            ..Disk::default()
        }),
        ..Measures::default()
    };

    let first = sample_at(&mut assembler, start, 1, None, disk(1000, 800));
    let busy = sample_at(&mut assembler, start, 2, None, disk(1250, 900));
    let idle = sample_at(&mut assembler, start, 3, None, disk(1250, 900));
    let back = sample_at(&mut assembler, start, 4, None, disk(10, 900));

    assert_eq!(
        (first.get(Column::HostDiskOps), first.get(Column::HostDiskMs)),
        (None, None)
    );
    assert_eq!(busy.get(Column::HostDiskOps), Some(250.0));
    assert!((busy.get(Column::HostDiskMs).unwrap() - 0.4).abs() < 1e-9);
    assert_eq!(
        (idle.get(Column::HostDiskOps), idle.get(Column::HostDiskMs)),
        (Some(0.0), None)
    );
    assert_eq!(
        (back.get(Column::HostDiskOps), back.get(Column::HostDiskMs)),
        (None, None)
    );
}

#[test]
fn steal_and_pressure_are_shares_of_the_slot_and_the_clock_is_as_read() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let host = |busy: u64, steal: u64, stalled: Option<u64>| Measures {
        host: Some(HostUsage {
            cpu: Some(CpuTimes {
                busy,
                total: busy * 4,
                steal: Some(steal),
            }),
            processors: Some(4),
            memory: None,
            interfaces: None,
            clock: Some(CpuClock {
                mhz: 4760.0,
                max_mhz: 3400.0,
                limit: Some(72.0),
            }),
            pressure: Some(Pressure {
                cpu: stalled,
                memory: Some(0),
                io: Some(1_000_000),
            }),
        }),
        ..Measures::default()
    };

    let first = sample_at(&mut assembler, start, 1, None, host(1000, 10, Some(1_000)));
    let second = sample_at(&mut assembler, start, 2, None, host(2000, 30, Some(151_000)));
    let gone = sample_at(&mut assembler, start, 3, None, host(3000, 30, None));

    assert_eq!(first.get(Column::HostSteal), None);
    assert_eq!(first.get(Column::HostPsiCpu), None);
    assert_eq!(first.get(Column::HostCpuMhz), Some(4760.0));
    assert_eq!(first.get(Column::HostCpuMaxMhz), Some(3400.0));
    assert_eq!(first.get(Column::HostCpuLimit), Some(72.0));
    // 20 of 4000 ticks.
    assert!((second.get(Column::HostSteal).unwrap() - 0.5).abs() < 1e-9);
    // 150 ms stalled over 5 s.
    let cpu = second.get(Column::HostPsiCpu).unwrap();
    assert!((cpu - 3.0).abs() < 0.01, "{cpu}");
    assert_eq!(second.get(Column::HostPsiMemory), Some(0.0));
    assert_eq!(second.get(Column::HostPsiIo), Some(0.0));
    assert_eq!(gone.get(Column::HostPsiCpu), None);
    assert_eq!(gone.get(Column::HostSteal), Some(0.0));
}

#[test]
fn the_first_tree_of_a_new_run_has_no_children_cpu_as_it_has_no_main_cpu() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let tree = |main_started, child_pid, child_cpu_ms| Measures {
        tree: Some(TreeUsage {
            main: Some(usage(1000, main_started)),
            children: vec![Member {
                pid: child_pid,
                name: "conhost.exe".to_owned(),
                usage: usage(child_cpu_ms, u64::from(child_pid)),
            }],
            threads: None,
            totals: None,
        }),
        ..Measures::default()
    };
    let generation = |generation| Some(run(generation, no_readings()));
    sample_at(&mut assembler, start, 1, generation(1), tree(7, 10, 0));
    sample_at(&mut assembler, start, 2, generation(1), tree(7, 10, 50));

    // Restarted between two wakes: another main process and another console host.
    let restarted = sample_at(&mut assembler, start, 3, generation(2), tree(9, 20, 0));
    let next = sample_at(&mut assembler, start, 4, generation(2), tree(9, 20, 250));

    assert_eq!(restarted.get(Column::FxsCpu), None);
    assert_eq!(restarted.get(Column::Children), Some(1.0));
    assert_eq!(restarted.get(Column::ChildrenCpu), None);
    assert!((next.get(Column::ChildrenCpu).unwrap() - 5.0).abs() < 1e-9);
}

#[test]
fn players_come_from_a_fresh_dynamic_json_answer_only() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let players = Players {
        clients: 187,
        max_clients: Some(256),
    };
    let answered = |when| {
        Some(run(
            1,
            Readings {
                dynamic: Some(at(when, players)),
                ..no_readings()
            },
        ))
    };

    let fresh = sample_at(
        &mut assembler,
        start,
        1,
        answered(start + Duration::from_secs(4)),
        Measures::default(),
    );
    let stale = sample_at(
        &mut assembler,
        start,
        3,
        answered(start + Duration::from_millis(9900)),
        Measures::default(),
    );

    assert_eq!(
        (fresh.get(Column::Players), fresh.get(Column::Slots)),
        (Some(187.0), Some(256.0))
    );
    assert_eq!(stale.get(Column::Players), None);
}

#[test]
fn ticks_are_deltas_between_readings_of_one_run_and_a_missing_one_keeps_the_baseline() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let reading = |slot: u32, count, ms| {
        Some(run(
            1,
            Readings {
                perf: Some(at(mid_slot(start, slot), perf(count, ms))),
                ..no_readings()
            },
        ))
    };

    let first = sample_at(&mut assembler, start, 1, reading(1, 100, 12), Measures::default());
    let second = sample_at(&mut assembler, start, 2, reading(2, 200, 12), Measures::default());
    // No answer in slot 3: the reading of slot 2 is still the newest.
    let missing = sample_at(&mut assembler, start, 3, reading(2, 200, 12), Measures::default());
    let spanning = sample_at(&mut assembler, start, 4, reading(4, 400, 12), Measures::default());
    let new_run = sample_at(
        &mut assembler,
        start,
        5,
        Some(run(
            2,
            Readings {
                perf: Some(at(mid_slot(start, 5), perf(50, 12))),
                ..no_readings()
            },
        )),
        Measures::default(),
    );

    assert_eq!(
        first.get(Column::MainTicks),
        None,
        "the first reading of a run is the baseline"
    );
    assert_eq!(second.get(Column::MainTicks), Some(100.0));
    assert_eq!(second.get(Column::MainTickMs), Some(1200.0));
    assert_eq!(second.get(Column::MainOver), Some(0.0));
    assert_eq!(second.get(Column::SyncTicks), Some(100.0));
    assert_eq!(
        second.get(Column::NetworkTicks),
        None,
        "svNetwork is not in this answer"
    );
    assert_eq!(missing.get(Column::MainTicks), None);
    assert_eq!(spanning.get(Column::MainTicks), Some(200.0));
    assert_eq!(new_run.get(Column::MainTicks), None);
}

#[test]
fn ticks_over_the_budget_are_counted_and_a_reset_gives_nothing() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let reading = |slot: u32, count, ms| {
        Some(run(
            1,
            Readings {
                perf: Some(at(mid_slot(start, slot), perf(count, ms))),
                ..no_readings()
            },
        ))
    };
    sample_at(&mut assembler, start, 1, reading(1, 100, 60), Measures::default());

    let slow = sample_at(&mut assembler, start, 2, reading(2, 150, 60), Measures::default());
    let reset = sample_at(&mut assembler, start, 3, reading(3, 20, 60), Measures::default());

    assert_eq!(slow.get(Column::MainOver), Some(50.0));
    assert_eq!(slow.get(Column::SyncOver), Some(0.0));
    assert_eq!(reset.get(Column::MainTicks), None);
}

#[test]
fn pings_joins_and_leaves_come_from_the_run_s_lists_while_svmain_ticks() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let pings: Vec<(u32, u32)> = (1..=20).map(|id| (id, id * 10)).collect();
    let mut later = pings[2..].to_vec();
    later.push((21, 400));
    let slot = |slot: u32, ticks, players: &[(u32, u32)], anonymous: bool| {
        let mut list = list(players);
        if anonymous {
            list = PlayerList::anonymous();
        }
        Some(run(
            1,
            Readings {
                perf: Some(at(mid_slot(start, slot), perf(ticks, 1))),
                players: Some(at(mid_slot(start, slot), list)),
                ..no_readings()
            },
        ))
    };

    let first = sample_at(
        &mut assembler,
        start,
        1,
        slot(1, 100, &pings, false),
        Measures::default(),
    );
    let second = sample_at(
        &mut assembler,
        start,
        2,
        slot(2, 200, &later, false),
        Measures::default(),
    );
    let frozen = sample_at(
        &mut assembler,
        start,
        3,
        slot(3, 200, &later, false),
        Measures::default(),
    );
    let anonymous = sample_at(
        &mut assembler,
        start,
        4,
        slot(4, 300, &[], true),
        Measures::default(),
    );
    let empty = sample_at(
        &mut assembler,
        start,
        5,
        slot(5, 400, &[], false),
        Measures::default(),
    );

    for column in [Column::PingAvgMs, Column::Joins] {
        assert_eq!(
            first.get(column),
            None,
            "svMain's ticks are not known yet: {column:?}"
        );
    }
    // 18 pings from 30 to 200 and one of 400.
    let average = (f64::from((3..=20).sum::<u32>() * 10) + 400.0) / 19.0;
    assert!((second.get(Column::PingAvgMs).unwrap() - average).abs() < 1e-9);
    assert_eq!(
        second.get(Column::PingP95Ms),
        Some(400.0),
        "95% of 19 players rounds up to the 19th"
    );
    assert_eq!(second.get(Column::PingMaxMs), Some(400.0));
    assert_eq!(
        (second.get(Column::Joins), second.get(Column::Leaves)),
        (Some(1.0), Some(2.0))
    );
    for column in [Column::PingAvgMs, Column::Joins, Column::Leaves] {
        assert_eq!(frozen.get(column), None, "svMain did not tick: {column:?}");
        assert_eq!(anonymous.get(column), None, "the public list: {column:?}");
    }
    assert_eq!(empty.get(Column::PingAvgMs), None, "nobody to ping");
    assert_eq!(
        (empty.get(Column::Joins), empty.get(Column::Leaves)),
        (Some(0.0), Some(19.0))
    );
}

#[test]
fn the_p95_ping_is_the_nearest_rank() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let pings: Vec<(u32, u32)> = (1..=40).map(|id| (id, id)).collect();
    let slot = |slot: u32, ticks| {
        Some(run(
            1,
            Readings {
                perf: Some(at(mid_slot(start, slot), perf(ticks, 1))),
                players: Some(at(mid_slot(start, slot), list(&pings))),
                ..no_readings()
            },
        ))
    };
    sample_at(&mut assembler, start, 1, slot(1, 10), Measures::default());

    let values = sample_at(&mut assembler, start, 2, slot(2, 20), Measures::default());

    assert_eq!(values.get(Column::PingP95Ms), Some(38.0));
}

#[test]
fn a_player_without_a_ping_counts_in_joins_and_leaves_but_not_in_the_pings() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let slot = |slot: u32, ticks, players: Vec<(u32, Option<u32>)>| {
        Some(run(
            1,
            Readings {
                perf: Some(at(mid_slot(start, slot), perf(ticks, 1))),
                players: Some(at(
                    mid_slot(start, slot),
                    PlayerList {
                        players,
                        anonymous: false,
                    },
                )),
                ..no_readings()
            },
        ))
    };
    let first = slot(1, 10, vec![(3, Some(40))]);
    sample_at(&mut assembler, start, 1, first, Measures::default());

    let joined = slot(2, 20, vec![(3, Some(40)), (4, None)]);
    let joined = sample_at(&mut assembler, start, 2, joined, Measures::default());
    let unpinged = slot(3, 30, vec![(4, None)]);
    let unpinged = sample_at(&mut assembler, start, 3, unpinged, Measures::default());

    assert_eq!(joined.get(Column::Joins), Some(1.0));
    assert_eq!(
        (joined.get(Column::PingAvgMs), joined.get(Column::PingMaxMs)),
        (Some(40.0), Some(40.0))
    );
    assert_eq!(unpinged.get(Column::Leaves), Some(1.0));
    assert_eq!(unpinged.get(Column::PingAvgMs), None, "no ping to average");
}

#[test]
fn hitches_and_barks_are_counted_per_slot_while_a_run_exists() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let hitch = |thread: &str, ms| {
        event(
            1,
            EventKind::Hitch {
                thread: thread.to_owned(),
                ms,
            },
        )
    };
    let idle = sample_at(&mut assembler, start, 1, None, Measures::default());
    assembler.event(
        &event(
            1,
            EventKind::Spawned {
                pid: 4,
                probe: "127.0.0.1:30120".parse().unwrap(),
                build: "/srv/observer/artifacts/35245".into(),
            },
        ),
        start,
    );
    for event in [
        hitch("svMain", 412),
        hitch("svMain", 150),
        hitch("svSync", 90),
        hitch("svScripts", 999),
    ] {
        assembler.event(&event, start);
    }
    assembler.event(
        &event(
            1,
            EventKind::WatchdogBark {
                thread: "svMain".to_owned(),
                seconds: 60,
                stack: String::new(),
            },
        ),
        start,
    );

    let busy = sample_at(&mut assembler, start, 2, None, Measures::default());
    let quiet = sample_at(
        &mut assembler,
        start,
        3,
        Some(run(1, no_readings())),
        Measures::default(),
    );

    assert_eq!(idle.get(Column::HitchMain), None, "no run, no hitch count");
    assert_eq!(
        busy.get(Column::HitchMain),
        Some(2.0),
        "the run started and ended in the slot"
    );
    assert_eq!(busy.get(Column::HitchSync), Some(1.0));
    assert_eq!(busy.get(Column::HitchNetwork), Some(0.0));
    assert_eq!(busy.get(Column::Barks), Some(1.0));
    assert_eq!(busy.get(Column::HitchMaxMs), Some(412.0));
    assert_eq!(quiet.get(Column::HitchMain), Some(0.0));
    assert_eq!(quiet.get(Column::HitchMaxMs), None);
}

#[test]
fn a_slot_whose_events_were_lost_has_no_counts() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    assembler.lagged();

    let lagged = sample_at(
        &mut assembler,
        start,
        1,
        Some(run(1, no_readings())),
        Measures::default(),
    );
    let next = sample_at(
        &mut assembler,
        start,
        2,
        Some(run(1, no_readings())),
        Measures::default(),
    );

    assert_eq!(lagged.get(Column::HitchMain), None);
    assert_eq!(lagged.get(Column::Barks), None);
    assert_eq!(next.get(Column::HitchMain), Some(0.0));
}

#[test]
fn the_heap_and_resources_are_the_run_s_latest_reports_and_the_heap_goes_stale() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    assembler.event(
        &event(
            1,
            EventKind::Heap {
                heap_used: 18 << 20,
                heap_total: 24 << 20,
                rss: 0,
                external: 0,
            },
        ),
        start,
    );
    let started = vec!["hardcap".to_owned(), "observer".to_owned(), "chat".to_owned()];
    assembler.event(&event(1, EventKind::Resources { started }), start);
    let running = |generation| Some(run(generation, no_readings()));

    let fresh = sample_at(&mut assembler, start, 1, running(1), Measures::default());
    let other_run = sample_at(&mut assembler, start, 2, running(2), Measures::default());
    let stale = sample_at(&mut assembler, start, 6, running(1), Measures::default());

    assert_eq!(fresh.get(Column::JsHeapUsed), Some(f64::from(18 << 20)));
    assert_eq!(fresh.get(Column::JsHeapTotal), Some(f64::from(24 << 20)));
    assert_eq!(fresh.get(Column::Resources), Some(3.0));
    assert_eq!(other_run.get(Column::JsHeapUsed), None);
    assert_eq!(other_run.get(Column::Resources), None);
    assert_eq!(stale.get(Column::JsHeapUsed), None, "30 s old");
    assert_eq!(
        stale.get(Column::Resources),
        Some(3.0),
        "kept until the next report"
    );
}

#[test]
fn the_host_rates_leave_out_an_adapter_whose_counters_went_back() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let interface = |key: &str, received, sent| Interface {
        key: key.to_owned(),
        received,
        sent,
    };
    let host = |busy, interfaces| Measures {
        host: Some(HostUsage {
            cpu: Some(CpuTimes {
                busy,
                total: busy * 4,
                steal: None,
            }),
            processors: Some(28),
            memory: Some(Memory {
                used: 7 << 30,
                total: 16 << 30,
            }),
            interfaces: Some(interfaces),
            clock: None,
            pressure: None,
        }),
        ..Measures::default()
    };

    let first = sample_at(
        &mut assembler,
        start,
        1,
        None,
        host(
            1000,
            vec![interface("eth0", 1000, 500), interface("eth1", 9000, 9000)],
        ),
    );
    let second = sample_at(
        &mut assembler,
        start,
        2,
        None,
        host(
            3000,
            vec![
                interface("eth0", 11_000, 5500),
                interface("eth1", 10, 10),
                interface("eth2", 50, 50),
            ],
        ),
    );
    let reset = sample_at(
        &mut assembler,
        start,
        3,
        None,
        host(4000, vec![interface("eth0", 0, 0)]),
    );

    assert_eq!(first.get(Column::HostCpu), None);
    assert_eq!(first.get(Column::HostRxBps), None);
    assert_eq!(first.get(Column::HostMemUsed), Some(real(7 << 30)));
    assert_eq!(second.get(Column::HostCpu), Some(25.0));
    assert_eq!(
        second.get(Column::HostRxBps),
        Some(2000.0),
        "eth0 alone: 10 kB in 5 s"
    );
    assert_eq!(second.get(Column::HostTxBps), Some(1000.0));
    assert_eq!(reset.get(Column::HostRxBps), None, "no adapter left to compare");
}

#[test]
fn shares_of_the_machine_count_the_processors_the_host_cpu_counts() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let host = |processors| Measures {
        host: Some(HostUsage {
            cpu: None,
            processors,
            memory: None,
            interfaces: None,
            clock: None,
            pressure: None,
        }),
        ..Measures::default()
    };
    let sample = |assembler: &mut Assembler, slot, measures| {
        let values = sample_at(assembler, start, slot, None, measures);
        live(
            &Sample {
                ts: 1_761_642_000,
                state: ServerState::Stopped,
                values,
                threads: Vec::new(),
            },
            Vec::new(),
            assembler.processors,
        )
        .cores
    };

    assert_eq!(sample(&mut assembler, 1, Measures::default()), None);
    // A process limited to 2 of the host's 28 processors still reads the 28 of the host's share.
    assert_eq!(sample(&mut assembler, 2, host(Some(28))), Some(28));
    assert_eq!(
        sample(&mut assembler, 3, host(None)),
        Some(28),
        "kept while unsaid"
    );
}

#[test]
fn observer_and_the_disk_are_measured_whether_or_not_a_server_runs() {
    let start = Instant::now();
    let mut assembler = Assembler::new(start);
    let measures = |cpu_ms| Measures {
        observer: Some(usage(cpu_ms, 1)),
        disk: Some(Disk {
            space: Some(Space {
                available: 100 << 30,
                total: 500 << 30,
            }),
            counters: None,
            state_db: Some(4096),
            metrics_db: None,
        }),
        ..Measures::default()
    };
    sample_at(&mut assembler, start, 1, None, measures(100));

    let values = sample_at(&mut assembler, start, 2, None, measures(350));

    assert!((values.get(Column::ObsCpu).unwrap() - 5.0).abs() < 1e-9);
    assert_eq!(values.get(Column::ObsPrivate), Some(f64::from(250 << 20)));
    assert_eq!(values.get(Column::HostDiskFree), Some(real(100 << 30)));
    assert_eq!(values.get(Column::ObsStateDb), Some(4096.0));
    assert_eq!(values.get(Column::ObsMetricsDb), None);
}

#[tokio::test]
async fn a_source_still_busy_from_the_previous_slot_is_not_started_again() {
    let mut busy: Option<JoinHandle<u32>> = None;
    let (release, wait) = std::sync::mpsc::channel::<()>();

    let slow = source(
        &mut busy,
        Some(move || {
            let _ = wait.recv();
            1
        }),
    )
    .await;
    let skipped = source(&mut busy, Some(|| 2)).await;
    release.send(()).unwrap();
    while busy.as_ref().is_some_and(|task| !task.is_finished()) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let next = source(&mut busy, Some(|| 3)).await;

    assert_eq!((slow, skipped, next), (None, None, Some(3)));
}

#[test]
fn a_database_s_size_includes_its_write_ahead_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("observer.sqlite3");
    std::fs::write(&path, vec![0; 4096]).unwrap();
    assert_eq!(database_size(&path), Some(4096));
    std::fs::write(dir.path().join("observer.sqlite3-wal"), vec![0; 1000]).unwrap();

    assert_eq!(database_size(&path), Some(5096));
    assert_eq!(database_size(&dir.path().join("missing.sqlite3")), None);
}

#[tokio::test]
async fn a_write_that_outlasts_the_next_slot_does_not_cost_that_slot() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("observer.sqlite3");
    let db = Database::open(&state, dir.path().join("backups")).await.unwrap();
    let services = Clock::manual("2025-01-06T09:00:00Z".parse().unwrap());
    let accounts = Accounts::new(db.clone(), services.clone(), PasswordCost::TEST)
        .await
        .unwrap();
    let (supervision, _running) = Supervision::start(
        Setup {
            servers: Servers::new(db.clone(), services.clone()),
            db,
            clock: services,
            accounts,
            data_dir: dir.path().join("data"),
            artifacts: dir.path().join("data/artifacts"),
            console_logs: dir.path().join("data/logs/console"),
            log: None,
        },
        |_| {},
    )
    .await
    .unwrap();
    let slot_end: Timestamp = "2025-01-06T09:00:00Z".parse().unwrap();
    // A millisecond before the slot ends: the sampler sleeps 101 ms at a time until the clock moves.
    let clock = Clock::manual(slot_end - SignedDuration::from_millis(1));
    let path = dir.path().join("metrics.sqlite3");
    let metrics = Metrics::open(MetricsSetup {
        path: path.clone(),
        clock: clock.clone(),
    })
    .await
    .unwrap();
    let (_, mut live) = metrics.live();
    let (_sockets, sockets) = watch::channel(0);
    let sampling = metrics.start(Sources {
        supervision,
        sockets,
        data_dir: dir.path().to_path_buf(),
        databases: DatabaseFiles { state, metrics: path },
    });
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (begun, holding) = oneshot::channel();
    let write = tokio::spawn({
        let db = metrics.inner.db.clone();
        async move {
            db.write(move |_| -> Result<(), StoreError> {
                let _ = begun.send(());
                let _ = held.recv();
                Ok(())
            })
            .await
        }
    });
    holding.await.unwrap();

    clock.advance(SignedDuration::from_millis(101));
    // The sampler's timer goes off before this one, on the same thread: it takes the slot that
    // just ended and waits behind the held write, which then outlasts the next slot's end.
    tokio::time::sleep(Duration::from_millis(500)).await;
    clock.advance(SignedDuration::from_secs(6));
    release.send(()).unwrap();
    write.await.unwrap().unwrap();
    let mut published = Vec::new();
    for _ in 0..2 {
        let sample = tokio::time::timeout(SLOT, live.recv())
            .await
            .expect("the slot after the held write was not sampled")
            .unwrap();
        published.push(sample.ts);
    }
    sampling.close().await;
    let written = metrics
        .inner
        .db
        .read(|conn| -> Result<Vec<i64>, StoreError> {
            let mut statement = conn.prepare("SELECT ts FROM sample_5s ORDER BY ts")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .await
        .unwrap();
    metrics.close().await.unwrap();

    let slots = [slot_end.as_second() - 5, slot_end.as_second()];
    assert_eq!(published, slots);
    assert_eq!(written, slots);
}
