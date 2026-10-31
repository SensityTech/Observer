use std::time::{Duration, Instant};

use observer_fxserver::health::Listed;

use super::*;

const LICENSE: &str = "license:0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "license:ffffffffffffffffffffffffffffffffffffffff";
const START_MS: i64 = 1_761_900_000_000;
const RUN: RunKey = RunKey {
    observer_started: START_MS,
    generation: 1,
};

struct Bench {
    sessions: Sessions,
    fingerprints: Fingerprints,
    origin: Instant,
}

impl Bench {
    fn new() -> Self {
        let mut next = 0u8;
        let mut sessions = Sessions::new(
            1,
            Box::new(move || {
                next += 1;
                [next; 16]
            }),
        );
        sessions.spawned(RUN, START_MS);
        Self {
            sessions,
            fingerprints: Fingerprints::new([1; 32]),
            origin: Instant::now(),
        }
    }

    fn at(&self, second: u64) -> (i64, Instant) {
        (
            START_MS + i64::try_from(second).unwrap() * 1000,
            self.origin + Duration::from_secs(second),
        )
    }

    fn send(&mut self, second: u64, kind: PlayerReportKind) {
        let (at_ms, received) = self.at(second);
        let report = PlayerReport {
            generation: 1,
            at_ms,
            received,
            kind,
        };
        self.sessions.report(&report, &self.fingerprints);
    }

    fn join(&mut self, second: u64, net_id: u32, license: &str, name: &str) {
        self.send(
            second,
            PlayerReportKind::Join(Joined {
                net_id,
                timer_ms: Some(i64::try_from(second).unwrap() * 1000),
                shown: shown(license, name),
            }),
        );
    }

    fn drop(&mut self, second: u64, net_id: u32, timer_ms: Option<i64>, category: i64, reason: &str) {
        self.send(
            second,
            PlayerReportKind::Drop(Dropped {
                net_id,
                timer_ms,
                reason: reason.to_owned(),
                resource: Some("anticheat".to_owned()),
                category: Some(category),
                ped: Some(true),
            }),
        );
    }

    fn list(&mut self, second: u64, players: &[(u32, &str, &str)]) {
        let (list_ms, at) = self.at(second);
        let list = PlayerList {
            players: players.iter().map(|(id, _, _)| (*id, Some(40))).collect(),
            shown: players
                .iter()
                .map(|(id, license, name)| Listed {
                    id: *id,
                    identifiers: vec![(*license).to_owned(), "discord:123456789012345678".to_owned()],
                    name: Some((*name).to_owned()),
                })
                .collect(),
            anonymous: false,
        };
        self.sessions.listed(1, &list, list_ms, at);
    }

    fn closures(&mut self) -> Vec<ClosureWrite> {
        mem::take(&mut self.sessions.writes.closures)
    }
}

fn shown(license: &str, name: &str) -> Shown {
    Shown {
        identifiers: vec![license.to_owned(), "discord:123456789012345678".to_owned()],
        tokens: vec![format!("4:{}", "ab".repeat(32))],
        name: Some(name.to_owned()),
        ..Shown::default()
    }
}

fn listed_shown(license: &str, name: &str) -> Shown {
    Shown {
        identifiers: vec![license.to_owned()],
        name: Some(name.to_owned()),
        ..Shown::default()
    }
}

fn summary(write: &ClosureWrite) -> (i64, DropCategory, Option<DropKind>, Option<i64>, bool) {
    (
        write.closure.id,
        write.closure.category,
        write.closure.kind,
        write.closure.length_ms,
        write.closure.synthesized,
    )
}

#[test]
fn a_confirmed_join_writes_its_identity_and_its_drop_credits_the_session() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    let list = |bench: &mut Bench, second| {
        let (list_ms, at) = bench.at(second);
        let list = PlayerList {
            players: vec![(7, Some(40))],
            shown: vec![Listed {
                id: 7,
                identifiers: vec![LICENSE.to_owned(), "discord:123456789012345678".to_owned()],
                name: Some("Ada".to_owned()),
            }],
            anonymous: false,
        };
        bench.sessions.listed(1, &list, list_ms, at);
    };

    list(&mut bench, 15);
    bench.drop(3610, 7, Some(3_610_000), 2, "Exiting");

    let writes = bench.sessions.take_writes();
    assert_eq!(writes.inserts.len(), 1);
    assert_eq!(writes.inserts[0].joined_ms, Some(START_MS + 10_000));
    assert_eq!(writes.identities.len(), 1);
    let identity = &writes.identities[0];
    assert_eq!(identity.license, LICENSE);
    assert_eq!(identity.name.as_deref(), Some("Ada"));
    let kinds: Vec<IdentifierKind> = identity.others.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(kinds, [IdentifierKind::Discord, IdentifierKind::Token]);
    assert_eq!(writes.closures.len(), 1);
    assert_eq!(
        summary(&writes.closures[0]),
        (
            1,
            DropCategory::Quit,
            Some(DropKind::Exit),
            Some(3_600_000),
            false
        )
    );
    assert_eq!(writes.closures[0].credit, Some(identity.credit));
    assert_eq!(writes.closures[0].closure.loading, Some(false));
}

#[test]
fn a_join_the_list_contradicts_gets_no_identity_and_no_credit() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.list(20, &[(7, OTHER, "Ada")]);
    bench.drop(60, 7, Some(60_000), 2, "Exiting");

    let writes = bench.sessions.take_writes();
    assert_eq!(writes.identities, []);
    assert_eq!(writes.closures[0].credit, None);
    assert_eq!(bench.sessions.counts.unconfirmed, 1);
}

#[test]
fn a_list_asked_before_the_join_neither_confirms_nor_counts_it_missing() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.list(11, &[]);
    bench.list(12, &[(7, OTHER, "Grace")]);

    assert_eq!(bench.sessions.take_writes().closures, []);
    assert!(matches!(
        bench.sessions.open().next().unwrap().identity,
        Identity::Confirming(_)
    ));
}

#[test]
fn a_forged_drop_cannot_claim_more_than_observer_measured() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.drop(70, 7, Some(10_000 + 5 * 3_600_000), 2, "Exiting");

    let closures = bench.closures();
    assert_eq!(closures[0].closure.length_ms, Some(61_000));
    assert_eq!(bench.sessions.counts.clamped, 1);
}

#[test]
fn the_game_timer_keeps_a_length_true_when_the_wall_clock_jumps() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    let (_, received) = bench.at(70);

    bench.sessions.report(
        &PlayerReport {
            generation: 1,
            at_ms: START_MS - 3_600_000,
            received,
            kind: PlayerReportKind::Drop(Dropped {
                net_id: 7,
                timer_ms: Some(70_000),
                reason: "Exiting".to_owned(),
                resource: None,
                category: Some(2),
                ped: None,
            }),
        },
        &bench.fingerprints,
    );

    assert_eq!(bench.closures()[0].closure.length_ms, Some(60_000));
}

#[test]
fn a_drop_before_its_join_opens_and_closes_one_session_and_the_late_join_is_ignored() {
    let mut bench = Bench::new();

    bench.drop(10, 7, Some(10_000), 1, "Banned");
    bench.join(12, 7, LICENSE, "Ada");

    let writes = bench.sessions.take_writes();
    assert_eq!(writes.inserts.len(), 1);
    assert_eq!(writes.inserts[0].joined_ms, None);
    assert_eq!(
        summary(&writes.closures[0]),
        (1, DropCategory::Kick, Some(DropKind::Resource), None, false)
    );
    assert_eq!(writes.closures[0].closure.resource.as_deref(), Some("anticheat"));
    assert_eq!(writes.closures[0].closure.reason_text.as_deref(), Some("Banned"));
    assert_eq!(bench.sessions.open().count(), 0);
}

#[test]
fn a_join_on_a_net_id_still_open_closes_the_old_session_as_missed() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.join(100, 7, OTHER, "Grace");

    let closures = bench.closures();
    assert_eq!(
        summary(&closures[0]),
        (
            1,
            DropCategory::Unknown,
            Some(DropKind::Missed),
            Some(90_000),
            true
        )
    );
    assert_eq!(bench.sessions.open().next().unwrap().id, 2);
}

#[test]
fn a_session_gone_from_three_lists_ends_at_its_last_sighting_and_a_late_drop_completes_it() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.list(20, &[(7, LICENSE, "Ada")]);

    for second in [25, 30] {
        bench.list(second, &[]);
    }
    assert_eq!(bench.closures(), []);
    bench.list(35, &[]);
    let closures = bench.closures();
    bench.drop(60, 7, Some(60_000), 5, "Server->client connection timed out.");

    assert_eq!(
        summary(&closures[0]),
        (
            1,
            DropCategory::Unknown,
            Some(DropKind::Missed),
            Some(10_000),
            true
        )
    );
    let reclassified = bench.sessions.take_writes().reclassified;
    assert_eq!(reclassified.len(), 1);
    assert_eq!(
        (reclassified[0].id, reclassified[0].category),
        (1, DropCategory::Timeout)
    );
}

#[test]
fn the_roster_opens_listed_players_and_closes_sessions_it_no_longer_shows() {
    let mut bench = Bench::new();
    bench.join(10, 3, OTHER, "Grace");
    let page = |page, last, players: Vec<(u32, Shown)>| {
        PlayerReportKind::Roster(RosterPage {
            timer_ms: Some(50_000),
            page,
            last,
            players,
            dropped: 0,
        })
    };

    bench.send(50, page(1, false, vec![(7, listed_shown(LICENSE, "Ada"))]));
    bench.drop(51, 8, Some(51_000), 2, "Exiting");
    bench.send(51, page(2, true, vec![(8, listed_shown(LICENSE, "Bob"))]));

    let writes = bench.sessions.take_writes();
    let missed: Vec<_> = writes.closures.iter().map(summary).collect();
    assert_eq!(
        missed,
        [
            (2, DropCategory::Quit, Some(DropKind::Exit), None, false),
            (
                1,
                DropCategory::Unknown,
                Some(DropKind::Missed),
                Some(40_000),
                true
            ),
        ]
    );
    let opened: Vec<(u32, Option<i64>, i64)> = writes
        .inserts
        .iter()
        .map(|insert| (insert.net_id, insert.joined_ms, insert.seen_ms))
        .collect();
    assert_eq!(
        opened,
        [
            (3, Some(START_MS + 10_000), START_MS + 10_000),
            (8, None, START_MS + 51_000),
            (7, None, START_MS + 50_000),
        ]
    );
}

#[test]
fn a_roster_with_a_missing_page_is_given_up() {
    let mut bench = Bench::new();
    bench.join(10, 3, OTHER, "Grace");
    let page = |page, last| {
        PlayerReportKind::Roster(RosterPage {
            timer_ms: Some(50_000),
            page,
            last,
            players: vec![(7, listed_shown(LICENSE, "Ada"))],
            dropped: 0,
        })
    };

    bench.send(50, page(1, false));
    bench.send(51, page(3, true));

    let writes = bench.sessions.take_writes();
    assert_eq!(writes.inserts.len(), 1);
    assert_eq!(writes.closures, []);
    assert_eq!(bench.sessions.counts.rosters_given_up, 1);
}

#[test]
fn a_stop_closes_what_fxserver_did_not_report_with_the_stop_reason() {
    let mut bench = Bench::new();
    bench.join(10, 1, LICENSE, "Ada");
    bench.join(10, 2, OTHER, "Grace");

    bench.sessions.stopping(1, DropKind::Restart);
    bench.drop(100, 1, Some(100_000), 7, "Server shutting down: restart");
    bench.sessions.exited(1, START_MS + 101_000, true);

    let closures: Vec<_> = bench.closures().iter().map(summary).collect();
    assert_eq!(
        closures,
        [
            (
                1,
                DropCategory::ServerStop,
                Some(DropKind::Restart),
                Some(90_000),
                false
            ),
            (
                2,
                DropCategory::ServerStop,
                Some(DropKind::Restart),
                Some(91_000),
                true
            ),
        ]
    );
    assert_eq!(bench.sessions.open().count(), 0);
}

#[test]
fn a_crash_and_a_hang_close_every_session_as_the_servers() {
    for (hung, kind) in [(false, DropKind::ServerCrashed), (true, DropKind::Hang)] {
        let mut bench = Bench::new();
        bench.join(10, 1, LICENSE, "Ada");
        if hung {
            bench.sessions.hung(1);
        }

        bench.sessions.exited(1, START_MS + 20_000, false);

        assert_eq!(
            bench.closures().iter().map(summary).collect::<Vec<_>>(),
            [(1, DropCategory::ServerCrash, Some(kind), Some(10_000), true)]
        );
    }
}

#[test]
fn an_operators_quit_is_a_console_stop() {
    let mut bench = Bench::new();
    bench.join(10, 1, LICENSE, "Ada");

    bench.sessions.exited(1, START_MS + 20_000, true);

    assert_eq!(bench.closures()[0].closure.kind, Some(DropKind::Console));
}

#[test]
fn reports_of_another_run_are_ignored() {
    let mut bench = Bench::new();
    let (at_ms, received) = bench.at(10);

    bench.sessions.report(
        &PlayerReport {
            generation: 2,
            at_ms,
            received,
            kind: PlayerReportKind::Join(Joined {
                net_id: 1,
                timer_ms: None,
                shown: shown(LICENSE, "Ada"),
            }),
        },
        &bench.fingerprints,
    );

    assert_eq!(bench.sessions.take_writes(), Writes::default());
}

#[test]
fn a_forgotten_licence_earns_nothing_more_in_this_run() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.list(15, &[(7, LICENSE, "Ada")]);
    bench.sessions.take_writes();

    bench.sessions.forget(LICENSE);
    bench.drop(70, 7, Some(70_000), 2, "Exiting");

    assert_eq!(bench.closures()[0].credit, None);
}

#[test]
fn a_crash_keeps_its_redacted_text() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.drop(
        20,
        7,
        Some(20_000),
        2,
        r"Game crashed: Recursive error in C:\Users\jdupont\FiveM\cars.meta",
    );

    let closure = &bench.closures()[0].closure;
    assert_eq!(closure.category, DropCategory::Crash);
    assert_eq!(
        closure.crash_text.as_deref(),
        Some(r"Recursive error in C:\Users\[user]\FiveM\cars.meta")
    );
    assert_eq!(closure.reason_text, None);
}
