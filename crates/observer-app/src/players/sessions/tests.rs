use std::time::{Duration, Instant};

use observer_fxserver::health::Listed;

use super::*;

const LICENSE: &str = "license:0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "license:ffffffffffffffffffffffffffffffffffffffff";
const DISCORD: &str = "discord:123456789012345678";
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

/// A closure as the tests look at it: session, category, kind, length, synthesized.
type Summary = (i64, DropCategory, Option<DropKind>, Option<i64>, bool);

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
        let origin = Instant::now();
        sessions.spawned(RUN, origin);
        Self {
            sessions,
            fingerprints: Fingerprints::new([1; 32]),
            origin,
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
                    identifiers: vec![(*license).to_owned(), DISCORD.to_owned()],
                    name: Some((*name).to_owned()),
                })
                .collect(),
            anonymous: false,
        };
        self.sessions.listed(1, &list, list_ms, at);
    }

    fn exit(&mut self, second: u64, expected: bool) {
        let (at_ms, now) = self.at(second);
        self.sessions.exited(1, at_ms, expected, now);
    }

    fn closes(&mut self) -> Vec<Summary> {
        closes(&self.sessions.take_writes())
            .iter()
            .map(|(closure, _)| summary(closure))
            .collect()
    }
}

fn opens(writes: &Writes) -> Vec<&NewSession> {
    writes
        .sessions
        .iter()
        .filter_map(|write| match write {
            SessionWrite::Open(session) => Some(session),
            _ => None,
        })
        .collect()
}

fn closes(writes: &Writes) -> Vec<(&Closure, Option<[u8; 16]>)> {
    writes
        .sessions
        .iter()
        .filter_map(|write| match write {
            SessionWrite::Close { closure, credit } => Some((closure, *credit)),
            _ => None,
        })
        .collect()
}

fn reclassified(writes: &Writes) -> Vec<&Closure> {
    writes
        .sessions
        .iter()
        .filter_map(|write| match write {
            SessionWrite::Reclassify(closure) => Some(closure),
            _ => None,
        })
        .collect()
}

fn shown(license: &str, name: &str) -> Shown {
    Shown {
        identifiers: vec![license.to_owned(), DISCORD.to_owned()],
        tokens: vec![format!("4:{}", "ab".repeat(32))],
        name: Some(name.to_owned()),
        ..Shown::default()
    }
}

fn summary(closure: &Closure) -> Summary {
    (
        closure.id,
        closure.category,
        closure.kind,
        closure.length_ms,
        closure.synthesized,
    )
}

#[test]
fn a_confirmed_join_writes_its_identity_and_its_drop_credits_the_session() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.list(15, &[(7, LICENSE, "Ada")]);
    bench.drop(3610, 7, Some(3_610_000), 2, "Exiting");

    let writes = bench.sessions.take_writes();
    assert_eq!(opens(&writes).len(), 1);
    assert_eq!(opens(&writes)[0].joined_ms, Some(START_MS + 10_000));
    assert_eq!(writes.identities.len(), 1);
    let identity = &writes.identities[0];
    assert_eq!(identity.license, LICENSE);
    assert_eq!(identity.name.as_deref(), Some("Ada"));
    let kinds: Vec<IdentifierKind> = identity.others.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(kinds, [IdentifierKind::Discord, IdentifierKind::Token]);
    let closed = closes(&writes);
    assert_eq!(
        summary(closed[0].0),
        (
            1,
            DropCategory::Quit,
            Some(DropKind::Exit),
            Some(3_600_000),
            false
        )
    );
    assert_eq!(closed[0].1, Some(identity.credit));
    assert_eq!(closed[0].0.loading, Some(false));
}

#[test]
fn a_join_the_list_contradicts_gets_no_identity_and_no_credit() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.list(20, &[(7, OTHER, "Ada")]);
    bench.drop(60, 7, Some(60_000), 2, "Exiting");

    let writes = bench.sessions.take_writes();
    assert_eq!(writes.identities, []);
    assert_eq!(closes(&writes)[0].1, None);
    assert_eq!(bench.sessions.counts.unconfirmed, 1);
}

#[test]
fn a_list_asked_before_the_join_neither_confirms_nor_counts_it_missing() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");

    bench.list(11, &[]);
    bench.list(12, &[(7, OTHER, "Grace")]);

    assert_eq!(bench.closes(), []);
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

    assert_eq!(bench.closes()[0].3, Some(61_000));
    assert_eq!(bench.sessions.counts.clamped, 1);
}

#[test]
fn a_session_carries_the_ping_of_the_latest_list_that_shows_it() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.join(10, 8, OTHER, "Grace");
    let pings = |bench: &Bench| {
        let mut pings: Vec<_> = bench
            .sessions
            .open()
            .map(|open| (open.net_id, open.ping))
            .collect();
        pings.sort_unstable();
        pings
    };
    let before = pings(&bench);

    let (list_ms, at) = bench.at(20);
    let list = PlayerList {
        players: vec![(7, Some(62)), (8, None)],
        shown: [(7, LICENSE, "Ada"), (8, OTHER, "Grace")]
            .into_iter()
            .map(|(id, license, name)| Listed {
                id,
                identifiers: vec![license.to_owned()],
                name: Some(name.to_owned()),
            })
            .collect(),
        anonymous: false,
    };
    bench.sessions.listed(1, &list, list_ms, at);

    assert_eq!(before, [(7, None), (8, None)]);
    assert_eq!(pings(&bench), [(7, Some(62)), (8, None)]);
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

    assert_eq!(bench.closes()[0].3, Some(60_000));
}

#[test]
fn a_drop_before_its_join_opens_and_closes_one_session_and_the_late_join_is_ignored() {
    let mut bench = Bench::new();

    bench.drop(10, 7, Some(10_000), 1, "Banned");
    bench.join(12, 7, LICENSE, "Ada");

    let writes = bench.sessions.take_writes();
    assert_eq!(opens(&writes).len(), 1);
    assert_eq!(opens(&writes)[0].joined_ms, None);
    let closed = closes(&writes)[0].0;
    assert_eq!(
        summary(closed),
        (1, DropCategory::Kick, Some(DropKind::Resource), None, false)
    );
    assert_eq!(closed.resource.as_deref(), Some("anticheat"));
    assert_eq!(closed.reason_text.as_deref(), Some("Banned"));
    assert_eq!(bench.sessions.open().count(), 0);
}

#[test]
fn a_join_on_a_net_id_still_open_closes_the_old_session_before_the_new_one_opens() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.sessions.take_writes();

    bench.join(100, 7, OTHER, "Grace");

    let writes = bench.sessions.take_writes();
    assert!(
        matches!(
            &writes.sessions[..],
            [SessionWrite::Close { closure, .. }, SessionWrite::Open(opened)]
                if closure.id == 1 && opened.id == 2
        ),
        "{writes:?}"
    );
    assert_eq!(
        summary(closes(&writes)[0].0),
        (
            1,
            DropCategory::Unknown,
            Some(DropKind::Missed),
            Some(90_000),
            true
        )
    );
}

#[test]
fn a_session_gone_from_three_lists_ends_at_its_last_sighting_and_a_late_drop_completes_it() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.list(20, &[(7, LICENSE, "Ada")]);

    for second in [25, 30] {
        bench.list(second, &[]);
    }
    assert_eq!(bench.closes(), []);
    bench.list(35, &[]);
    let closed = bench.closes();
    bench.drop(60, 7, Some(60_000), 5, "Server->client connection timed out.");

    assert_eq!(
        closed,
        [(
            1,
            DropCategory::Unknown,
            Some(DropKind::Missed),
            Some(10_000),
            true
        )]
    );
    let writes = bench.sessions.take_writes();
    let late = reclassified(&writes);
    assert_eq!((late[0].id, late[0].category), (1, DropCategory::Timeout));
}

#[test]
fn a_player_the_list_keeps_showing_without_a_session_is_followed_again() {
    let mut bench = Bench::new();

    bench.list(20, &[(4, LICENSE, "Ada")]);
    bench.list(25, &[(4, LICENSE, "Ada")]);
    let opened = bench.sessions.take_writes();
    bench.list(30, &[(4, LICENSE, "Ada")]);

    let session = opens(&opened);
    assert_eq!(session.len(), 1);
    assert_eq!(
        (session[0].net_id, session[0].joined_ms, session[0].seen_ms),
        (4, None, START_MS + 20_000)
    );
    let confirmed = bench.sessions.take_writes();
    assert_eq!(confirmed.identities.len(), 1);
    assert_eq!(confirmed.identities[0].license, LICENSE);
    assert!(
        confirmed.identities[0]
            .others
            .iter()
            .all(|(kind, _)| *kind == IdentifierKind::Discord),
        "the list shows no token and no address: {confirmed:?}"
    );
    assert_eq!(bench.sessions.counts.untracked, 1);
}

#[test]
fn a_player_who_just_dropped_is_not_opened_again_from_a_list_asked_before() {
    let mut bench = Bench::new();
    bench.join(10, 4, LICENSE, "Ada");
    bench.drop(20, 4, Some(20_000), 2, "Exiting");
    bench.sessions.take_writes();

    bench.list(21, &[(4, LICENSE, "Ada")]);
    bench.list(22, &[(4, LICENSE, "Ada")]);

    assert_eq!(opens(&bench.sessions.take_writes()), Vec::<&NewSession>::new());
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

    bench.send(50, page(1, false, vec![(7, shown(LICENSE, "Ada"))]));
    bench.drop(51, 8, Some(51_000), 2, "Exiting");
    bench.send(51, page(2, true, vec![(8, shown(LICENSE, "Bob"))]));

    let writes = bench.sessions.take_writes();
    let closed: Vec<Summary> = closes(&writes)
        .iter()
        .map(|(closure, _)| summary(closure))
        .collect();
    assert_eq!(
        closed,
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
    let opened: Vec<(u32, Option<i64>, i64)> = opens(&writes)
        .iter()
        .map(|session| (session.net_id, session.joined_ms, session.seen_ms))
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
fn a_roster_never_closes_a_session_the_latest_list_still_showed() {
    let mut bench = Bench::new();
    bench.join(10, 3, LICENSE, "Ada");
    bench.list(20, &[(3, LICENSE, "Ada")]);
    bench.sessions.take_writes();

    bench.send(
        30,
        PlayerReportKind::Roster(RosterPage {
            timer_ms: Some(30_000),
            page: 1,
            last: true,
            players: Vec::new(),
            dropped: 0,
        }),
    );

    assert_eq!(bench.closes(), []);
    assert_eq!(bench.sessions.open().count(), 1);
}

#[test]
fn a_roster_with_a_missing_page_or_too_many_pages_is_given_up() {
    let mut bench = Bench::new();
    bench.join(10, 3, OTHER, "Grace");
    let page = |page, last| {
        PlayerReportKind::Roster(RosterPage {
            timer_ms: Some(50_000),
            page,
            last,
            players: vec![(7, shown(LICENSE, "Ada"))],
            dropped: 0,
        })
    };

    bench.send(50, page(1, false));
    bench.send(51, page(3, true));
    bench.send(52, page(1, false));
    for number in 2..=40 {
        bench.send(53, page(number, false));
    }

    let writes = bench.sessions.take_writes();
    assert_eq!(opens(&writes).len(), 1);
    assert_eq!(closes(&writes), []);
    assert_eq!(bench.sessions.counts.rosters_given_up, 9);
}

#[test]
fn a_roster_whose_last_page_never_comes_is_given_up_and_the_list_follows_players_again() {
    let mut bench = Bench::new();
    bench.send(
        50,
        PlayerReportKind::Roster(RosterPage {
            timer_ms: Some(50_000),
            page: 1,
            last: false,
            players: vec![(7, shown(LICENSE, "Ada"))],
            dropped: 0,
        }),
    );

    bench.list(55, &[(7, LICENSE, "Ada")]);
    bench.list(58, &[(7, LICENSE, "Ada")]);
    let waited = bench.sessions.open().count();
    bench.list(70, &[(7, LICENSE, "Ada")]);
    bench.list(75, &[(7, LICENSE, "Ada")]);

    assert_eq!(waited, 0);
    assert_eq!(bench.sessions.counts.rosters_given_up, 1);
    assert_eq!(bench.sessions.open().count(), 1);
}

#[test]
fn a_run_already_followed_is_not_started_again() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.sessions.take_writes();
    let (_, now) = bench.at(20);

    bench.sessions.spawned(RUN, now);

    assert_eq!(bench.sessions.take_writes(), Writes::default());
    assert_eq!(bench.sessions.open().count(), 1);
    assert!(bench.sessions.holds(1));
}

#[test]
fn a_stop_closes_what_fxserver_did_not_report_with_the_stop_reason() {
    let mut bench = Bench::new();
    bench.join(10, 1, LICENSE, "Ada");
    bench.join(10, 2, OTHER, "Grace");

    bench.sessions.stopping(1, DropKind::Restart);
    bench.drop(100, 1, Some(100_000), 7, "Server shutting down: restart");
    bench.exit(101, true);

    assert_eq!(
        bench.closes(),
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
fn a_drop_fxserver_reported_just_before_it_exited_still_counts_when_it_comes_after() {
    let mut bench = Bench::new();
    bench.join(10, 1, LICENSE, "Ada");
    bench.exit(101, false);
    bench.sessions.take_writes();

    bench.drop(101, 1, Some(100_000), 5, "Server->client connection timed out.");
    bench.join(101, 2, OTHER, "Grace");

    let writes = bench.sessions.take_writes();
    let late = reclassified(&writes);
    assert_eq!((late[0].id, late[0].category), (1, DropCategory::Timeout));
    assert_eq!(opens(&writes), Vec::<&NewSession>::new());
}

#[test]
fn a_crash_and_a_hang_close_every_session_as_the_servers() {
    for (hung, expected, kind) in [
        (false, false, DropKind::ServerCrashed),
        (true, false, DropKind::Hang),
        (true, true, DropKind::Hang),
    ] {
        let mut bench = Bench::new();
        bench.join(10, 1, LICENSE, "Ada");
        if hung {
            bench.sessions.hung(1);
        }

        bench.exit(20, expected);

        assert_eq!(
            bench.closes(),
            [(1, DropCategory::ServerCrash, Some(kind), Some(10_000), true)],
            "{hung} {expected}"
        );
    }
}

#[test]
fn an_operators_quit_is_a_console_stop() {
    let mut bench = Bench::new();
    bench.join(10, 1, LICENSE, "Ada");

    bench.exit(20, true);

    assert_eq!(bench.closes()[0].2, Some(DropKind::Console));
}

#[test]
fn a_clock_step_adds_nothing_to_a_session_observer_closes() {
    let mut bench = Bench::new();
    bench.join(10, 1, LICENSE, "Ada");
    let (_, now) = bench.at(20);

    bench.sessions.exited(1, START_MS + 10 * 3_600_000, false, now);

    assert_eq!(bench.closes()[0].3, Some(11_000));
}

#[test]
fn sessions_a_missed_exit_left_open_end_where_they_were_last_seen() {
    let mut bench = Bench::new();
    bench.join(10, 1, LICENSE, "Ada");
    bench.list(20, &[(1, LICENSE, "Ada")]);
    bench.sessions.take_writes();
    let (_, now) = bench.at(8 * 3600);

    bench.sessions.spawned(RunKey { generation: 2, ..RUN }, now);

    assert_eq!(
        bench.closes(),
        [(1, DropCategory::Unknown, Some(DropKind::Lost), Some(10_000), true)]
    );
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

    assert_eq!(closes(&bench.sessions.take_writes())[0].1, None);
}

#[test]
fn a_forgotten_licences_sessions_keep_no_text_from_a_drop_that_comes_after() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.list(15, &[(7, LICENSE, "Ada")]);
    // Observer closed the session itself, before FXServer's drop came.
    bench.exit(30, false);
    bench.sessions.take_writes();

    let forgotten = bench.sessions.forget(LICENSE);
    bench.drop(
        31,
        7,
        Some(31_000),
        2,
        r"Game crashed: Recursive error in C:\Users\jdupont\FiveM\cars.meta",
    );

    assert_eq!(forgotten, HashSet::from([1]));
    let writes = bench.sessions.take_writes();
    let late = reclassified(&writes);
    assert_eq!((late[0].id, late[0].category), (1, DropCategory::Crash));
    assert_eq!(late[0].crash_text, None);
}

#[test]
fn a_forgotten_licences_open_session_keeps_no_text_from_its_drop() {
    let mut bench = Bench::new();
    bench.join(10, 7, LICENSE, "Ada");
    bench.list(15, &[(7, LICENSE, "Ada")]);
    bench.sessions.take_writes();

    assert_eq!(bench.sessions.forget(LICENSE), HashSet::from([1]));
    bench.drop(
        20,
        7,
        Some(20_000),
        2,
        r"Game crashed: Recursive error in C:\Users\jdupont\FiveM\cars.meta",
    );

    let writes = bench.sessions.take_writes();
    let (closure, credit) = closes(&writes)[0];
    assert_eq!(
        (closure.category, closure.crash_text.as_deref(), credit),
        (DropCategory::Crash, None, None)
    );
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

    let writes = bench.sessions.take_writes();
    let closure = closes(&writes)[0].0;
    assert_eq!(closure.category, DropCategory::Crash);
    assert_eq!(
        closure.crash_text.as_deref(),
        Some(r"Recursive error in C:\Users\[user]\FiveM\cars.meta")
    );
    assert_eq!(closure.reason_text, None);
}
