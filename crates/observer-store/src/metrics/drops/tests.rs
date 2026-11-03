use super::*;
use crate::metrics::memory;
use crate::metrics::runs::{self, RunKey, StartCause};
use crate::metrics::sessions::{self, Closure, NewSession};

/// 2025-10-31 08:00 UTC, on an hour.
const START_MS: i64 = 1_761_897_600_000;
const RUN: RunKey = RunKey {
    observer_started: START_MS,
    generation: 1,
};
const MINUTE: i64 = 60_000;
const ADA: [u8; 16] = [1; 16];
const GRACE: [u8; 16] = [2; 16];

fn lab() -> Connection {
    let mut conn = memory();
    let tx = conn.transaction().unwrap();
    runs::start(&tx, RUN, START_MS, StartCause::Requested).unwrap();
    tx.commit().unwrap();
    conn
}

/// A session seen at `seen` minutes, ended at `ended` minutes past the start in `category`.
fn session(conn: &mut Connection, id: i64, seen: i64, ended: Option<(i64, DropCategory)>) {
    let tx = conn.transaction().unwrap();
    assert!(
        sessions::insert(
            &tx,
            &NewSession {
                id,
                run: RUN,
                net_id: u32::try_from(id).unwrap(),
                joined_ms: Some(START_MS + seen * MINUTE),
                seen_ms: START_MS + seen * MINUTE,
            },
        )
        .unwrap()
    );
    tx.commit().unwrap();
    if let Some((ended, category)) = ended {
        close(conn, id, seen, ended, category);
    }
}

fn close(conn: &mut Connection, id: i64, seen: i64, ended: i64, category: DropCategory) {
    let tx = conn.transaction().unwrap();
    sessions::close(
        &tx,
        &Closure {
            id,
            ended_ms: START_MS + ended * MINUTE,
            length_ms: Some((ended - seen) * MINUTE),
            category,
            kind: match category {
                DropCategory::Timeout => Some(DropKind::Connection),
                DropCategory::Kick => Some(DropKind::Resource),
                DropCategory::Quit => Some(DropKind::Exit),
                _ => None,
            },
            native: None,
            resource: (category == DropCategory::Kick).then(|| "afk".to_owned()),
            crash_text: (category == DropCategory::Crash).then(|| "ERR_GFX_D3D_INIT".to_owned()),
            reason_text: (category == DropCategory::Kick).then(|| "AFK for 31 minutes".to_owned()),
            loading: Some(false),
            synthesized: false,
        },
    )
    .unwrap();
    tx.commit().unwrap();
}

fn roll(conn: &mut Connection, hours: &[i64], now_minutes: i64) {
    let tx = conn.transaction().unwrap();
    for &hour in hours {
        roll_up(
            &tx,
            START_MS / 1000 + hour * 3600,
            START_MS + now_minutes * MINUTE,
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

fn drops_of(counts: &Counts, category: DropCategory) -> i64 {
    counts
        .groups
        .iter()
        .filter(|group| group.category == category)
        .map(|group| group.drops)
        .sum()
}

#[test]
fn an_hour_rolled_up_counts_what_its_sessions_say() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Crash)));
    session(&mut conn, 2, 10, Some((50, DropCategory::Quit)));
    session(&mut conn, 3, 20, None);

    roll(&mut conn, &[0], 70);
    let rolled = hourly(
        &conn,
        START_MS / 1000,
        START_MS / 1000 + 3600,
        START_MS + 70 * MINUTE,
    )
    .unwrap();
    let live = live(&conn, START_MS, START_MS + HOUR_MS, START_MS + 70 * MINUTE).unwrap();

    let hour = &rolled[&(START_MS / 1000)];
    assert_eq!(drops_of(hour, DropCategory::Crash), 1);
    assert_eq!(drops_of(hour, DropCategory::Quit), 1);
    // 30 + 40 + 40 minutes played in the hour; the open session counts up to the hour's end.
    assert_eq!(hour.player_ms, 110 * MINUTE);
    assert_eq!(hour.sessions, 2);
    assert_eq!(hour.player_ms, live.player_ms);
}

#[test]
fn rolling_an_hour_up_twice_gives_the_same_rows() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Crash)));

    roll(&mut conn, &[0], 70);
    roll(&mut conn, &[0], 70);

    let rows: i64 = conn
        .query_row("SELECT count(*) FROM drop_1h", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn a_session_closed_late_takes_its_hours_out_of_the_rolled_ones() {
    let mut conn = lab();
    session(&mut conn, 1, 0, None);
    roll(&mut conn, &[0, 1, 2], 190);

    // Observer saw it last at 1 h 30: it was not playing for the whole of hours 1 and 2.
    close(&mut conn, 1, 0, 90, DropCategory::Unknown);

    assert_eq!(
        pending_hours(&conn, START_MS + 190 * MINUTE).unwrap(),
        [START_MS / 1000 + 3600, START_MS / 1000 + 7200]
    );
    let counts = window(&conn, START_MS, START_MS + 3 * HOUR_MS, START_MS + 190 * MINUTE).unwrap();
    assert_eq!(counts.player_ms, 90 * MINUTE);
}

#[test]
fn a_window_adds_rolled_hours_hours_not_rolled_and_its_edges() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Crash)));
    session(&mut conn, 2, 70, Some((80, DropCategory::Crash)));
    session(&mut conn, 3, 125, Some((130, DropCategory::Crash)));
    roll(&mut conn, &[0], 200);

    let counts = window(
        &conn,
        START_MS + 20 * MINUTE,
        START_MS + 140 * MINUTE,
        START_MS + 200 * MINUTE,
    )
    .unwrap();

    assert_eq!(drops_of(&counts, DropCategory::Crash), 3);
    assert_eq!(counts.player_ms, (10 + 10 + 5) * MINUTE);
}

#[test]
fn a_burst_of_unexpected_drops_is_one_incident_when_enough_of_the_players_left() {
    let mut conn = lab();
    for id in 1..=20 {
        session(&mut conn, id, 0, None);
    }
    for id in 1..=6 {
        close(&mut conn, id, 0, 30, DropCategory::Timeout);
    }

    let tx = conn.transaction().unwrap();
    let joined = find_incidents(&tx, START_MS, START_MS + HOUR_MS).unwrap();
    tx.commit().unwrap();

    assert_eq!(joined, 6);
    let found = incidents(&conn, START_MS, START_MS + HOUR_MS).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].drops, found[0].players), (6, Some(20)));
    let counts = live(&conn, START_MS, START_MS + HOUR_MS, START_MS + HOUR_MS).unwrap();
    let timeouts = counts
        .groups
        .iter()
        .find(|group| group.category == DropCategory::Timeout)
        .unwrap();
    assert_eq!(timeouts.events(), 1);
}

#[test]
fn a_few_drops_on_a_full_server_are_no_incident() {
    let mut conn = lab();
    for id in 1..=40 {
        session(&mut conn, id, 0, None);
    }
    for id in 1..=6 {
        close(&mut conn, id, 0, 30, DropCategory::Timeout);
    }

    let tx = conn.transaction().unwrap();
    let joined = find_incidents(&tx, START_MS, START_MS + HOUR_MS).unwrap();
    tx.commit().unwrap();

    assert_eq!(joined, 0);
}

#[test]
fn classifying_a_session_gives_it_its_group_and_reopens_its_hour() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Crash)));
    roll(&mut conn, &[0], 70);
    let pending = unclassified(&conn, 10).unwrap();

    let tx = conn.transaction().unwrap();
    classify(
        &tx,
        &pending[0],
        Grouping::Signature(NewSignature {
            signature: "error:ERR_GFX_D3D_INIT",
            form: "error",
            module: None,
            game_build: None,
            label: None,
            resource: None,
            normaliser: 1,
        }),
        1,
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(unclassified(&conn, 10).unwrap(), []);
    assert_eq!(
        pending_hours(&conn, START_MS + 70 * MINUTE).unwrap(),
        [START_MS / 1000]
    );
    roll(&mut conn, &[0], 70);
    let hour = &hourly(
        &conn,
        START_MS / 1000,
        START_MS / 1000 + 3600,
        START_MS + 70 * MINUTE,
    )
    .unwrap()[&(START_MS / 1000)];
    assert_ne!(hour.groups[0].subject, 0);
    let signature = signatures(&conn, &[hour.groups[0].subject]).unwrap();
    assert_eq!(signature[0].signature, "error:ERR_GFX_D3D_INIT");
}

#[test]
fn a_forgotten_players_free_text_signature_loses_its_text_unless_another_player_shares_it() {
    let mut conn = lab();
    let tx = conn.transaction().unwrap();
    for (signature, player) in [("text:my name is ada", ADA), ("text:shared", ADA)] {
        signature_id(
            &tx,
            &NewSignature {
                signature,
                form: "text",
                module: None,
                game_build: None,
                label: None,
                resource: None,
                normaliser: 1,
            },
            START_MS,
            Some(player),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    session(&mut conn, 1, 0, Some((10, DropCategory::Crash)));
    conn.execute(
        "UPDATE player_session SET player = ?1, signature = (SELECT id FROM crash_signature WHERE signature = 'text:shared')",
        [GRACE],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let renamed = forget(&tx, ADA).unwrap();
    tx.commit().unwrap();

    assert_eq!(renamed, 1);
    let left: Vec<String> = conn
        .prepare("SELECT signature FROM crash_signature ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(left, ["text:forgotten:1", "text:shared"]);
    let made = signatures_made(&conn, Some(ADA), 0, i64::MAX).unwrap();
    assert_eq!(made, 0);
}

#[test]
fn expiring_sessions_sweeps_the_incidents_and_reason_groups_they_held() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Kick)));
    let pending = unclassified(&conn, 10).unwrap();
    let tx = conn.transaction().unwrap();
    classify(&tx, &pending[0], Grouping::Reason("AFK for {n} minutes"), 1).unwrap();
    tx.commit().unwrap();
    roll(&mut conn, &[0], 70);

    let tx = conn.transaction().unwrap();
    sessions::expire(&tx, START_MS + HOUR_MS).unwrap();
    tx.commit().unwrap();

    let reasons: i64 = conn
        .query_row("SELECT count(*) FROM drop_reason", [], |row| row.get(0))
        .unwrap();
    assert_eq!(reasons, 0);
    // The rollups outlive the sessions.
    let counts = window(&conn, START_MS, START_MS + HOUR_MS, START_MS + 70 * MINUTE).unwrap();
    assert_eq!(drops_of(&counts, DropCategory::Kick), 1);
}

#[test]
fn the_log_pages_newest_first_and_narrows_to_its_filters() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((10, DropCategory::Crash)));
    session(&mut conn, 2, 0, Some((20, DropCategory::Kick)));
    session(&mut conn, 3, 0, Some((30, DropCategory::Crash)));
    conn.execute("UPDATE player_session SET player = ?1 WHERE id = 3", [ADA])
        .unwrap();
    let filter = LogFilter {
        from_ms: START_MS,
        to_ms: START_MS + HOUR_MS,
        ..LogFilter::default()
    };

    let first = log(&conn, &filter, 2).unwrap();
    let rest = log(
        &conn,
        &LogFilter {
            before: Some((first[1].ended_ms, first[1].id)),
            ..filter.clone()
        },
        2,
    )
    .unwrap();
    let crashes = log(
        &conn,
        &LogFilter {
            category: Some(DropCategory::Crash),
            ..filter.clone()
        },
        10,
    )
    .unwrap();
    let ada = log(
        &conn,
        &LogFilter {
            players: Some(vec![ADA]),
            ..filter.clone()
        },
        10,
    )
    .unwrap();
    let nobody = log(
        &conn,
        &LogFilter {
            players: Some(Vec::new()),
            ..filter
        },
        10,
    )
    .unwrap();

    assert_eq!(first.iter().map(|row| row.id).collect::<Vec<_>>(), [3, 2]);
    assert_eq!(rest.iter().map(|row| row.id).collect::<Vec<_>>(), [1]);
    assert_eq!(crashes.len(), 2);
    assert_eq!(ada.iter().map(|row| row.id).collect::<Vec<_>>(), [3]);
    assert_eq!(nobody, []);
    assert_eq!(first[1].resource.as_deref(), Some("afk"));
}

#[test]
fn forgetting_a_player_takes_their_reason_group_away_at_once() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Kick)));
    conn.execute("UPDATE player_session SET player = ?1", [ADA])
        .unwrap();
    let pending = unclassified(&conn, 10).unwrap();
    let tx = conn.transaction().unwrap();
    classify(
        &tx,
        &pending[0],
        Grouping::Reason("Banned: Jean harassed Marie on {n}"),
        1,
    )
    .unwrap();
    tx.commit().unwrap();

    let tx = conn.transaction().unwrap();
    sessions::forget(&tx, ADA).unwrap();
    tx.commit().unwrap();

    assert_eq!(reason_groups(&conn, START_MS, START_MS + HOUR_MS).unwrap(), []);
    let templates: i64 = conn
        .query_row("SELECT count(*) FROM drop_reason", [], |row| row.get(0))
        .unwrap();
    assert_eq!(templates, 0);
}

#[test]
fn an_incident_counts_once_in_each_category_whatever_its_kinds_and_hours() {
    let mut conn = lab();
    for id in 1..=10 {
        session(&mut conn, id, 0, None);
    }
    // Five timeouts and five crashes, one burst across the hour's end.
    for id in 1..=5 {
        close(&mut conn, id, 0, 59, DropCategory::Timeout);
    }
    for id in 6..=10 {
        close(&mut conn, id, 0, 60, DropCategory::Crash);
    }
    conn.execute(
        "UPDATE player_session SET ended_ms = ?1 + (id - 1) * 1000",
        [START_MS + 59 * MINUTE + 55_000],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    find_incidents(&tx, START_MS, START_MS + 2 * HOUR_MS).unwrap();
    tx.commit().unwrap();
    let counts = window(&conn, START_MS, START_MS + 2 * HOUR_MS, START_MS + 2 * HOUR_MS).unwrap();

    let events = |category| -> i64 {
        counts
            .groups()
            .iter()
            .filter(|group| group.category == category)
            .map(Group::events)
            .sum()
    };
    assert_eq!(
        incidents(&conn, START_MS, START_MS + 2 * HOUR_MS).unwrap().len(),
        1
    );
    assert_eq!(
        (events(DropCategory::Timeout), events(DropCategory::Crash)),
        (1, 1)
    );
}

#[test]
fn a_burst_ends_when_ten_seconds_no_longer_hold_five_drops() {
    let mut conn = lab();
    for id in 1..=12 {
        session(&mut conn, id, 0, None);
    }
    for id in 1..=12 {
        close(&mut conn, id, 0, 30, DropCategory::Timeout);
    }
    // Six drops a second apart, then one every eight seconds.
    conn.execute(
        "UPDATE player_session SET ended_ms = ?1 + CASE WHEN id <= 6 THEN id * 1000 ELSE 6000 + (id - 6) * 8000 END",
        [START_MS + 30 * MINUTE],
    )
    .unwrap();

    let tx = conn.transaction().unwrap();
    let joined = find_incidents(&tx, START_MS, START_MS + HOUR_MS).unwrap();
    tx.commit().unwrap();

    assert_eq!(joined, 6);
}

#[test]
fn a_session_written_late_takes_its_hours_out_of_the_rolled_ones() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((10, DropCategory::Quit)));
    roll(&mut conn, &[0, 1, 2], 190);

    // Written at 3 h 10 for a join at 0 h 15: the state database could not be written before.
    session(&mut conn, 2, 15, None);

    assert_eq!(
        pending_hours(&conn, START_MS + 190 * MINUTE).unwrap(),
        [START_MS / 1000, START_MS / 1000 + 3600, START_MS / 1000 + 7200]
    );
}

#[test]
fn a_free_text_signature_no_kept_session_holds_loses_its_text() {
    let mut conn = lab();
    let tx = conn.transaction().unwrap();
    signature_id(
        &tx,
        &NewSignature {
            signature: "text:call me on {n}",
            form: "text",
            module: None,
            game_build: None,
            label: None,
            resource: None,
            normaliser: 1,
        },
        START_MS,
        Some(ADA),
    )
    .unwrap();
    sweep(&tx).unwrap();
    tx.commit().unwrap();

    let left: String = conn
        .query_row("SELECT signature FROM crash_signature", [], |row| row.get(0))
        .unwrap();
    assert_eq!(left, "text:expired:1");
}

#[test]
fn hours_before_the_first_session_are_empty_without_reading_the_sessions() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Crash)));

    let before = live(&conn, START_MS - 90 * 24 * HOUR_MS, START_MS, START_MS + HOUR_MS).unwrap();
    let wide = window(
        &conn,
        START_MS - 30 * 24 * HOUR_MS,
        START_MS + HOUR_MS,
        START_MS + HOUR_MS,
    )
    .unwrap();

    assert_eq!(before, Counts::default());
    assert_eq!(wide.player_ms, 30 * MINUTE);
}

#[test]
fn a_crash_of_fxserver_is_counted_an_hour_after_its_run_ended_with_no_session_open() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::ServerCrash)));
    conn.execute(
        "UPDATE server_run SET ended_ms = ?1, end_cause = 'crashed'",
        [START_MS + 30 * MINUTE],
    )
    .unwrap();
    let count = |conn: &mut Connection, now_ms: i64| {
        let tx = conn.transaction().unwrap();
        let counted = count_crash_drops(&tx, now_ms).unwrap();
        tx.commit().unwrap();
        counted
    };

    let early = count(&mut conn, START_MS + 40 * MINUTE);
    let later = count(&mut conn, START_MS + 120 * MINUTE);

    assert_eq!((early, later), (0, 1));
    let dropped: i64 = conn
        .query_row("SELECT dropped FROM server_run", [], |row| row.get(0))
        .unwrap();
    assert_eq!(dropped, 1);
}

#[test]
fn a_client_cannot_write_a_prefix_that_keeps_its_text_past_the_sweep() {
    let mut conn = lab();
    let tx = conn.transaction().unwrap();
    signature_id(
        &tx,
        &NewSignature {
            signature: "text:expired: call me on {n}",
            form: "text",
            module: None,
            game_build: None,
            label: None,
            resource: None,
            normaliser: 1,
        },
        START_MS,
        Some(ADA),
    )
    .unwrap();
    sweep(&tx).unwrap();
    tx.commit().unwrap();

    let left: String = conn
        .query_row("SELECT signature FROM crash_signature", [], |row| row.get(0))
        .unwrap();
    assert_eq!(left, "text:expired:1");
}

#[test]
fn the_sessions_kept_start_at_the_latest_cut_of_the_retention() {
    let mut conn = lab();

    let tx = conn.transaction().unwrap();
    sessions::expire(&tx, START_MS + 90 * MINUTE).unwrap();
    sessions::expire(&tx, START_MS + 30 * MINUTE).unwrap();
    tx.commit().unwrap();

    assert_eq!(kept_since(&conn).unwrap(), Some(START_MS + HOUR_MS));
}

#[test]
fn crashes_with_a_signature_come_oldest_first_with_their_player() {
    let mut conn = lab();
    conn.execute(
        "INSERT INTO crash_signature (id, signature, form, normaliser, first_seen_ms) VALUES (7, 'hash:kilo', 'hash', 1, 0)",
        [],
    )
    .unwrap();
    session(&mut conn, 1, 0, Some((12, DropCategory::Crash)));
    session(&mut conn, 2, 0, Some((5, DropCategory::Crash)));
    session(&mut conn, 3, 0, Some((8, DropCategory::Quit)));
    session(&mut conn, 4, 0, Some((9, DropCategory::Crash)));
    conn.execute("UPDATE player_session SET signature = 7 WHERE id IN (1, 2)", [])
        .unwrap();
    conn.execute("UPDATE player_session SET player = ?1 WHERE id = 1", [ADA])
        .unwrap();

    let found = crashes(&conn, START_MS, START_MS + 60 * MINUTE).unwrap();

    assert_eq!(
        found,
        [
            CrashRow {
                session: 2,
                ended_ms: START_MS + 5 * MINUTE,
                signature: 7,
                player: None,
            },
            CrashRow {
                session: 1,
                ended_ms: START_MS + 12 * MINUTE,
                signature: 7,
                player: Some(ADA),
            },
        ],
        "a crash without a signature is no wave's"
    );
    assert_eq!(
        crashes(&conn, START_MS + 13 * MINUTE, START_MS + 60 * MINUTE).unwrap(),
        Vec::<CrashRow>::new()
    );
}

#[test]
fn sessions_started_in_a_window_keep_their_length_and_how_they_ended() {
    let mut conn = lab();
    session(&mut conn, 1, 0, Some((30, DropCategory::Quit)));
    session(&mut conn, 2, 10, None);
    session(&mut conn, 3, 70, Some((80, DropCategory::ServerStop)));

    let mut found = started(&conn, START_MS, START_MS + 60 * MINUTE).unwrap();
    found.sort_by_key(|session| session.joined_ms);

    assert_eq!(
        found,
        [
            StartedSession {
                joined_ms: START_MS,
                ended_ms: Some(START_MS + 30 * MINUTE),
                length_ms: Some(30 * MINUTE),
                category: Some(DropCategory::Quit),
                kind: Some(DropKind::Exit),
            },
            StartedSession {
                joined_ms: START_MS + 10 * MINUTE,
                ended_ms: None,
                length_ms: None,
                category: None,
                kind: None,
            },
        ]
    );
}
