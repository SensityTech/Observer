use jiff::Timestamp;

use super::*;
use crate::database::memory;

const ADA: &str = "license:0000000000000000000000000000000000000001";
const GRACE: &str = "license:0000000000000000000000000000000000000002";

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn token(n: u8) -> (IdentifierKind, String) {
    (
        IdentifierKind::Token,
        format!("4:{}", format!("{n:02x}").repeat(32)),
    )
}

fn discord(id: &str) -> (IdentifierKind, String) {
    (IdentifierKind::Discord, id.to_owned())
}

fn see(
    conn: &mut Connection,
    license: &str,
    name: &str,
    others: &[(IdentifierKind, String)],
    when: &str,
) -> (i64, [u8; 16]) {
    let tx = conn.transaction().unwrap();
    let mut pseudonym = [0; 16];
    pseudonym[..license.len().min(16)].copy_from_slice(&license.as_bytes()[license.len() - 16..]);
    let seen = record_seen(
        &tx,
        &Seen {
            license,
            name: Some(name),
            others,
            at: at(when),
            pseudonym,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    seen
}

fn credit_session(conn: &mut Connection, player: i64, key: u8, length_ms: i64, days: &[(&str, i64)]) -> bool {
    let tx = conn.transaction().unwrap();
    open_credit(&tx, [key; 16], player, at("2026-10-30T20:00:00Z")).unwrap();
    let credited = credit(
        &tx,
        &Credit {
            key: [key; 16],
            length_ms,
            ended: at("2026-10-31T01:00:00Z"),
            days: days.iter().map(|(day, ms)| ((*day).to_owned(), *ms)).collect(),
            crashed: true,
            timed_out: false,
            kicked: false,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    credited
}

#[test]
fn a_returning_player_keeps_their_row_and_pseudonym() {
    let mut conn = memory();

    let first = see(
        &mut conn,
        ADA,
        "Ada",
        &[discord("123456789012345678")],
        "2026-10-30T10:00:00Z",
    );
    let again = see(
        &mut conn,
        ADA,
        "Ada L.",
        &[discord("123456789012345678")],
        "2026-10-31T10:00:00Z",
    );

    assert_eq!(first, again);
    let (name, times): (String, i64) = conn
        .query_row(
            "SELECT p.name, pi.times FROM player p JOIN player_identifier pi ON pi.player = p.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((name.as_str(), times), ("Ada L.", 2));
    assert_eq!(names(&conn, first.0).unwrap().len(), 2);
}

#[test]
fn an_older_sighting_does_not_replace_the_latest_name() {
    let mut conn = memory();
    let (id, _) = see(&mut conn, ADA, "Ada L.", &[], "2026-10-31T10:00:00Z");

    see(&mut conn, ADA, "Ada", &[], "2026-10-01T10:00:00Z");

    let profile = profile(&conn, id, "2026-10-01").unwrap().unwrap();
    assert_eq!(profile.summary.name, "Ada L.");
    assert_eq!(profile.summary.first_seen, at("2026-10-01T10:00:00Z"));
}

#[test]
fn only_the_twenty_latest_names_are_kept() {
    let mut conn = memory();
    let mut id = 0;
    for minute in 0..25 {
        id = see(
            &mut conn,
            ADA,
            &format!("Ada {minute}"),
            &[],
            &format!("2026-10-31T10:{minute:02}:00Z"),
        )
        .0;
    }

    let kept = names(&conn, id).unwrap();

    assert_eq!(kept.len(), 20);
    assert_eq!(kept[0].name, "Ada 24");
    assert_eq!(kept[19].name, "Ada 5");
}

#[test]
fn a_session_is_credited_once_across_its_days() {
    let mut conn = memory();
    let (id, _) = see(&mut conn, ADA, "Ada", &[], "2026-10-30T20:00:00Z");

    assert!(credit_session(
        &mut conn,
        id,
        1,
        5 * 3_600_000,
        &[("2026-10-30", 4 * 3_600_000), ("2026-10-31", 3_600_000)]
    ));
    let tx = conn.transaction().unwrap();
    let again = credit(
        &tx,
        &Credit {
            key: [1; 16],
            length_ms: 5 * 3_600_000,
            ended: at("2026-10-31T01:00:00Z"),
            days: vec![("2026-10-31".to_owned(), 1)],
            crashed: true,
            timed_out: false,
            kicked: false,
        },
    )
    .unwrap();
    tx.commit().unwrap();

    assert!(!again);
    let profile = profile(&conn, id, "2026-10-01").unwrap().unwrap();
    assert_eq!(profile.summary.play_ms, 5 * 3_600_000);
    assert_eq!(profile.summary.sessions, 1);
    assert_eq!(profile.summary.recent_ms, 5 * 3_600_000);
    assert_eq!(profile.crashes, 1);
    assert_eq!(profile.longest_ms, 5 * 3_600_000);
    assert_eq!(
        days(&conn, id, "2026-10-01").unwrap(),
        [
            PlayDay {
                day: "2026-10-30".to_owned(),
                play_ms: 4 * 3_600_000
            },
            PlayDay {
                day: "2026-10-31".to_owned(),
                play_ms: 3_600_000
            },
        ]
    );
    let sessions_per_day: Vec<i64> = conn
        .prepare("SELECT sessions FROM player_day ORDER BY day")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(sessions_per_day, [0, 1]);
    assert_eq!(open_records(&conn).unwrap(), []);
}

#[test]
fn a_token_shared_by_four_licences_is_a_weak_link() {
    let mut conn = memory();
    let mut ids = Vec::new();
    for n in 1..=4u8 {
        let license = format!("license:{}", format!("{n:02x}").repeat(20));
        ids.push(
            see(
                &mut conn,
                &license,
                &format!("P{n}"),
                &[token(9)],
                "2026-10-31T10:00:00Z",
            )
            .0,
        );
    }

    let linked = links(&conn, ids[0]).unwrap();

    assert_eq!(linked.len(), 3);
    assert!(linked.iter().all(|licence| !licence.strong && licence.steps == 1));
}

#[test]
fn strong_links_reach_two_steps_and_weak_ones_are_not_followed() {
    let mut conn = memory();
    let license = |n: u8| format!("license:{}", format!("{n:02x}").repeat(20));
    let a = see(
        &mut conn,
        &license(1),
        "A",
        &[discord("100000000000000001")],
        "2026-10-31T10:00:00Z",
    )
    .0;
    see(
        &mut conn,
        &license(2),
        "B",
        &[discord("100000000000000001"), token(2)],
        "2026-10-31T10:00:00Z",
    );
    see(
        &mut conn,
        &license(3),
        "C",
        &[token(2), (IdentifierKind::Ip, "ff".repeat(16))],
        "2026-10-31T10:00:00Z",
    );
    see(
        &mut conn,
        &license(4),
        "D",
        &[(IdentifierKind::Ip, "ff".repeat(16))],
        "2026-10-31T10:00:00Z",
    );
    see(&mut conn, &license(5), "E", &[token(5)], "2026-10-31T10:00:00Z");

    let linked = links(&conn, a).unwrap();

    let found: Vec<(&str, bool, u8)> = linked
        .iter()
        .map(|licence| (licence.name.as_str(), licence.strong, licence.steps))
        .collect();
    assert_eq!(found, [("B", true, 1), ("C", true, 2)]);
    assert_eq!(linked[0].through[0].kind, IdentifierKind::Discord);
}

#[test]
fn an_address_link_is_weak_and_its_value_never_leaves() {
    let mut conn = memory();
    let fingerprint = (IdentifierKind::Ip, "ab".repeat(16));
    let a = see(
        &mut conn,
        ADA,
        "Ada",
        std::slice::from_ref(&fingerprint),
        "2026-10-31T10:00:00Z",
    )
    .0;
    see(&mut conn, GRACE, "Grace", &[fingerprint], "2026-10-31T10:00:00Z");

    let linked = links(&conn, a).unwrap();
    let shown = identifiers(&conn, a).unwrap();

    assert_eq!(linked.len(), 1);
    assert!(!linked[0].strong);
    assert_eq!(linked[0].through[0].value, None);
    assert_eq!(shown[0].value, None);
    assert_eq!(shown[0].other_licences, 1);
}

#[test]
fn retention_runs_per_link_then_drops_identifiers_nobody_holds() {
    let mut conn = memory();
    let now = at("2026-10-31T00:00:00Z");
    let id = see(
        &mut conn,
        ADA,
        "Ada",
        &[
            discord("123456789012345678"),
            (IdentifierKind::Ip, "ab".repeat(16)),
        ],
        "2026-07-01T00:00:00Z",
    )
    .0;
    see(&mut conn, ADA, "Ada", &[token(1)], "2026-10-30T00:00:00Z");

    let tx = conn.transaction().unwrap();
    let expired = expire(&tx, now, &PlayersSettings::default(), "2025-09-26").unwrap();
    tx.commit().unwrap();

    assert_eq!(expired.links, 1);
    assert_eq!(expired.identifiers, 1);
    let kinds: Vec<IdentifierKind> = identifiers(&conn, id)
        .unwrap()
        .iter()
        .map(|shown| shown.kind)
        .collect();
    assert_eq!(kinds, [IdentifierKind::Discord, IdentifierKind::Token]);
}

#[test]
fn a_profile_goes_after_its_retention_unless_the_player_is_online() {
    let mut conn = memory();
    let gone = see(&mut conn, ADA, "Ada", &[], "2024-01-01T00:00:00Z").0;
    let online = see(&mut conn, GRACE, "Grace", &[], "2024-01-01T00:00:00Z").0;
    let tx = conn.transaction().unwrap();
    open_credit(&tx, [7; 16], online, at("2024-01-01T00:00:00Z")).unwrap();
    let expired = expire(
        &tx,
        at("2026-10-31T00:00:00Z"),
        &PlayersSettings::default(),
        "2025-09-26",
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(expired.players, 1);
    assert!(profile(&conn, gone, "2026-10-01").unwrap().is_none());
    assert!(profile(&conn, online, "2026-10-01").unwrap().is_some());
}

#[test]
fn forgetting_a_player_keeps_what_another_licence_holds() {
    let mut conn = memory();
    let shared = discord("123456789012345678");
    let ada = see(
        &mut conn,
        ADA,
        "Ada",
        &[shared.clone(), token(1)],
        "2026-10-31T10:00:00Z",
    )
    .0;
    let grace = see(&mut conn, GRACE, "Grace", &[shared], "2026-10-31T10:00:00Z").0;

    let tx = conn.transaction().unwrap();
    let forgotten = forget(&tx, ada, [3; 16], at("2026-10-31T12:00:00Z"))
        .unwrap()
        .unwrap();
    tx.commit().unwrap();

    assert_eq!(forgotten.license, ADA);
    let left: i64 = conn
        .query_row("SELECT count(*) FROM player WHERE license = ?1", [ADA], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(identifiers(&conn, grace).unwrap().len(), 1);
    let tokens: i64 = conn
        .query_row(
            "SELECT count(*) FROM identifier WHERE kind = 'token'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tokens, 0);
    assert_eq!(forgotten_fingerprints(&conn), [[3; 16]]);
}

fn forgotten_fingerprints(conn: &Connection) -> Vec<[u8; 16]> {
    forgotten(conn).unwrap().into_keys().collect()
}

#[test]
fn the_list_sorts_by_play_time_and_pages_by_key() {
    let mut conn = memory();
    let license = |n: u8| format!("license:{}", format!("{n:02x}").repeat(20));
    for n in 1..=5u8 {
        let id = see(
            &mut conn,
            &license(n),
            &format!("P{n}"),
            &[],
            "2026-10-31T10:00:00Z",
        )
        .0;
        credit_session(
            &mut conn,
            id,
            n,
            i64::from(n % 3) * 1000,
            &[("2026-10-31", i64::from(n % 3) * 1000)],
        );
    }
    let query = |after| PlayerQuery {
        filter: PlayerFilter::All,
        sort: PlayerSort::PlayTime,
        recent_from: "2026-10-02".to_owned(),
        after,
        limit: 2,
    };

    let first = page(&conn, &query(None)).unwrap();
    let second = page(&conn, &query(first.last().map(|(row, key)| (*key, row.id)))).unwrap();
    let third = page(&conn, &query(second.last().map(|(row, key)| (*key, row.id)))).unwrap();

    let pages = [first, second, third];
    let names: Vec<&str> = pages.iter().flatten().map(|(row, _)| row.name.as_str()).collect();
    assert_eq!(names, ["P5", "P2", "P4", "P1", "P3"]);
}

#[test]
fn a_name_search_matches_the_start_of_a_name_literally() {
    let mut conn = memory();
    see(&mut conn, ADA, "Ada_99", &[], "2026-10-31T10:00:00Z");
    see(&mut conn, GRACE, "Adax99", &[], "2026-10-31T10:00:00Z");
    let query = |filter| PlayerQuery {
        filter,
        sort: PlayerSort::LastSeen,
        recent_from: "2026-10-02".to_owned(),
        after: None,
        limit: 10,
    };

    let found = page(&conn, &query(PlayerFilter::Name("ada_".to_owned()))).unwrap();
    let by_discord = page(
        &conn,
        &query(PlayerFilter::Identifier(IdentifierKind::Discord, "1".to_owned())),
    )
    .unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0.name, "Ada_99");
    assert_eq!(by_discord, []);
}

#[test]
fn retention_settings_round_trip_with_a_profile_kept_for_good() {
    let mut conn = memory();
    let kept = PlayersSettings {
        profile_days: None,
        ..PlayersSettings::default()
    };

    assert_eq!(settings(&conn).unwrap(), PlayersSettings::default());
    let tx = conn.transaction().unwrap();
    save_settings(&tx, &kept).unwrap();
    tx.commit().unwrap();

    assert_eq!(settings(&conn).unwrap(), kept);
}

#[test]
fn the_fingerprint_key_is_made_once() {
    let mut conn = memory();
    let tx = conn.transaction().unwrap();

    let first = fingerprint_key(&tx, [1; 32]).unwrap();
    let second = fingerprint_key(&tx, [2; 32]).unwrap();

    assert_eq!(first, [1; 32]);
    assert_eq!(second, first);
}

#[test]
fn every_sort_pages_through_every_player_once() {
    let mut conn = memory();
    let license = |n: u8| format!("license:{}", format!("{n:02x}").repeat(20));
    for n in 1..=7u8 {
        let when = format!("2026-10-{:02}T10:00:00Z", 10 + n % 4);
        let id = see(&mut conn, &license(n), &format!("P{n}"), &[], &when).0;
        let day = format!("2026-10-{:02}", 20 + n % 3);
        credit_session(&mut conn, id, n, 1000, &[(&day, i64::from(n % 3) * 1000)]);
    }

    for sort in [
        PlayerSort::PlayTime,
        PlayerSort::Recent,
        PlayerSort::LastSeen,
        PlayerSort::FirstSeen,
        PlayerSort::Sessions,
    ] {
        let mut seen = Vec::new();
        let mut after = None;
        loop {
            let rows = page(
                &conn,
                &PlayerQuery {
                    filter: PlayerFilter::All,
                    sort,
                    recent_from: "2026-10-21".to_owned(),
                    after,
                    limit: 3,
                },
            )
            .unwrap();
            let Some((last, key)) = rows.last() else {
                break;
            };
            after = Some((*key, last.id));
            seen.extend(rows.iter().map(|(row, _)| row.id));
        }
        seen.sort_unstable();
        assert_eq!(seen, (1..=7).collect::<Vec<i64>>(), "{sort:?}");
    }
}
