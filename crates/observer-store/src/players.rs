//! Players: one row per licence with its profile, play time per day, and the identifiers and names
//! each licence was seen with. The players service is the only writer.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use jiff::Timestamp;
use observer_domain::players::{
    IdentifierKind, Link, LinkedLicence, PlayDay, PlayerName, PlayerSort, PlayerSummary, PlayersSettings,
    ShownIdentifier,
};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, Transaction, params, params_from_iter};

use crate::database::StoreError;
use crate::settings;
use crate::time::Utc;

/// Names kept per player, the most recent.
const NAMES_KEPT: i64 = 20;
/// An identifier seen with more licences than this links them only weakly: a shared PC, a café,
/// a VPN.
const STRONG_UP_TO: i64 = 3;
/// A group of linked licences stops here, so one shared token cannot pull a whole server in.
const MOST_LINKED: usize = 50;

const ADDRESS_KEY: &str = "players.address_key";
const LINK_DAYS: &str = "players.link_days";
const ADDRESS_DAYS: &str = "players.address_days";
const DAY_DAYS: &str = "players.day_days";
const PROFILE_DAYS: &str = "players.profile_days";

/// A join confirmed against FXServer's own player list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen<'a> {
    pub license: &'a str,
    pub name: Option<&'a str>,
    /// Canonical values; an address is its fingerprint in hex.
    pub others: &'a [(IdentifierKind, String)],
    pub at: Timestamp,
    /// Given to the player if this is their first visit.
    pub pseudonym: [u8; 16],
}

/// Records a confirmed join: the player, created on their first visit, every identifier and the
/// link to it, and the name. Returns the player's id and pseudonym.
pub fn record_seen(tx: &Transaction<'_>, seen: &Seen<'_>) -> Result<(i64, [u8; 16]), StoreError> {
    let at = Utc(seen.at);
    let (player, pseudonym): (i64, [u8; 16]) = tx.query_row(
        "INSERT INTO player (license, pseudonym, name, first_seen, last_seen) VALUES (?1, ?2, coalesce(?3, ''), ?4, ?4)
         ON CONFLICT (license) DO UPDATE SET
             last_seen = max(last_seen, excluded.last_seen),
             first_seen = min(first_seen, excluded.first_seen),
             name = CASE WHEN ?3 IS NOT NULL AND excluded.last_seen >= last_seen THEN ?3 ELSE name END
         RETURNING id, pseudonym",
        params![seen.license, seen.pseudonym, seen.name, at],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    for (kind, value) in seen.others {
        let identifier = identifier_id(tx, *kind, value)?;
        tx.prepare_cached(
            "INSERT INTO player_identifier (player, identifier, first_seen, last_seen, times) VALUES (?1, ?2, ?3, ?3, 1)
             ON CONFLICT (player, identifier) DO UPDATE SET
                 last_seen = max(last_seen, excluded.last_seen), times = times + 1, source = 'seen'",
        )?
        .execute(params![player, identifier, at])?;
    }
    if let Some(name) = seen.name {
        record_name(tx, player, name, seen.at, seen.at)?;
    }
    Ok((player, pseudonym))
}

fn identifier_id(tx: &Transaction<'_>, kind: IdentifierKind, value: &str) -> Result<i64, StoreError> {
    tx.prepare_cached(
        "INSERT INTO identifier (kind, value) VALUES (?1, ?2) ON CONFLICT (kind, value) DO NOTHING",
    )?
    .execute(params![kind.as_str(), value])?;
    Ok(tx
        .prepare_cached("SELECT id FROM identifier WHERE kind = ?1 AND value = ?2")?
        .query_row(params![kind.as_str(), value], |row| row.get(0))?)
}

fn record_name(
    tx: &Transaction<'_>,
    player: i64,
    name: &str,
    first_seen: Timestamp,
    last_seen: Timestamp,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO player_name (player, name, first_seen, last_seen) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (player, name) DO UPDATE SET
             first_seen = min(first_seen, excluded.first_seen), last_seen = max(last_seen, excluded.last_seen)",
        params![player, name, Utc(first_seen), Utc(last_seen)],
    )?;
    tx.execute(
        "DELETE FROM player_name WHERE player = ?1 AND name NOT IN
             (SELECT name FROM player_name WHERE player = ?1 ORDER BY last_seen DESC, name LIMIT ?2)",
        params![player, NAMES_KEPT],
    )?;
    Ok(())
}

/// The record that the metrics database's `session` of `player`, credited under `key`, is open
/// and not credited yet.
pub fn open_credit(
    tx: &Transaction<'_>,
    key: [u8; 16],
    player: i64,
    started: Timestamp,
    session: i64,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO player_open_session (key, player, started, session) VALUES (?1, ?2, ?3, ?4)",
        params![key, player, Utc(started), session],
    )?;
    Ok(())
}

/// A session to add to its player's profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    pub key: [u8; 16],
    pub length_ms: i64,
    pub ended: Timestamp,
    /// The length split across the days it covered, in the schedule's time zone; the last one
    /// counts the session.
    pub days: Vec<(String, i64)>,
    pub crashed: bool,
    pub timed_out: bool,
    pub kicked: bool,
}

/// Adds a session to its profile, once: only while its record is there, which this deletes.
/// Returns whether it was credited.
pub fn credit(tx: &Transaction<'_>, credit: &Credit) -> Result<bool, StoreError> {
    let Some(player): Option<i64> = tx
        .query_row(
            "DELETE FROM player_open_session WHERE key = ?1 RETURNING player",
            [credit.key],
            |row| row.get(0),
        )
        .optional()?
    else {
        return Ok(false);
    };
    let length = credit.length_ms.max(0);
    tx.execute(
        "UPDATE player SET play_ms = play_ms + ?2, sessions = sessions + 1, longest_ms = max(longest_ms, ?2),
             crashes = crashes + ?3, timeouts = timeouts + ?4, kicks = kicks + ?5,
             last_seen = max(last_seen, ?6)
         WHERE id = ?1",
        params![
            player,
            length,
            i64::from(credit.crashed),
            i64::from(credit.timed_out),
            i64::from(credit.kicked),
            Utc(credit.ended),
        ],
    )?;
    let last = credit.days.len().saturating_sub(1);
    for (index, (day, ms)) in credit.days.iter().enumerate() {
        tx.execute(
            "INSERT INTO player_day (player, day, play_ms, sessions) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (player, day) DO UPDATE SET
                 play_ms = play_ms + excluded.play_ms, sessions = sessions + excluded.sessions",
            params![player, day, (*ms).max(0), i64::from(index == last)],
        )?;
    }
    Ok(true)
}

/// A crediting record still there, and the metrics session it credits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRecord {
    pub key: [u8; 16],
    pub session: i64,
}

pub fn open_records(conn: &Connection) -> Result<Vec<OpenRecord>, StoreError> {
    let mut statement = conn.prepare("SELECT key, session FROM player_open_session")?;
    let rows = statement.query_map([], |row| {
        Ok(OpenRecord {
            key: row.get(0)?,
            session: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn drop_record(tx: &Transaction<'_>, key: [u8; 16]) -> Result<(), StoreError> {
    tx.execute("DELETE FROM player_open_session WHERE key = ?1", [key])?;
    Ok(())
}

/// The players list, from its query. `recent_from` is the first day of the last 30.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerQuery {
    pub filter: PlayerFilter,
    pub sort: PlayerSort,
    pub recent_from: String,
    /// The last row of the previous page: its sort key and id.
    pub after: Option<(i64, i64)>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerFilter {
    All,
    /// The start of a name, any case.
    Name(String),
    License(String),
    Identifier(IdentifierKind, String),
}

/// A page of the players list, and each row's sort key for the next page.
pub fn page(conn: &Connection, query: &PlayerQuery) -> Result<Vec<(PlayerSummary, i64)>, StoreError> {
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(query.recent_from.clone())];
    let filter = match &query.filter {
        PlayerFilter::All => String::new(),
        PlayerFilter::Name(start) => {
            values.push(Box::new(format!("{}%", like_escaped(start))));
            " AND p.name LIKE ?2 ESCAPE '\\'".to_owned()
        }
        PlayerFilter::License(license) => {
            values.push(Box::new(license.clone()));
            " AND p.license = ?2".to_owned()
        }
        PlayerFilter::Identifier(kind, value) => {
            values.push(Box::new(kind.as_str()));
            values.push(Box::new(value.clone()));
            " AND p.id IN (SELECT pi.player FROM player_identifier pi JOIN identifier i ON i.id = pi.identifier
                           WHERE i.kind = ?2 AND i.value = ?3)"
                .to_owned()
        }
    };
    let key = match query.sort {
        PlayerSort::PlayTime => "play_ms + imported_ms",
        PlayerSort::Recent => "recent",
        PlayerSort::LastSeen => "unixepoch(last_seen, 'subsec') * 1000",
        PlayerSort::FirstSeen => "unixepoch(first_seen, 'subsec') * 1000",
        PlayerSort::Sessions => "sessions",
    };
    let after = match query.after {
        Some((sort, id)) => {
            values.push(Box::new(sort));
            values.push(Box::new(id));
            let (sort, id) = (values.len() - 1, values.len());
            format!(" WHERE (sort_key, id) < (?{sort}, ?{id})")
        }
        None => String::new(),
    };
    values.push(Box::new(query.limit));
    let sql = format!(
        "SELECT * FROM (
             SELECT *, CAST({key} AS INTEGER) AS sort_key FROM (
                 SELECT p.id, p.name, p.license, p.play_ms, p.imported_ms,
                     coalesce((SELECT sum(d.play_ms) FROM player_day d WHERE d.player = p.id AND d.day >= ?1), 0)
                         AS recent,
                     p.sessions, p.first_seen, p.last_seen
                 FROM player p WHERE 1{filter}
             )
         ){after} ORDER BY sort_key DESC, id DESC LIMIT ?{}",
        values.len()
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), |row| Ok((summary(row)?, row.get(9)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn like_escaped(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn summary(row: &Row<'_>) -> rusqlite::Result<PlayerSummary> {
    Ok(PlayerSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        license: row.get(2)?,
        play_ms: row.get(3)?,
        imported_ms: row.get(4)?,
        recent_ms: row.get(5)?,
        sessions: row.get(6)?,
        first_seen: row.get::<_, Utc>(7)?.0,
        last_seen: row.get::<_, Utc>(8)?.0,
        in_progress_ms: 0,
        online: false,
    })
}

/// A profile's own row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub summary: PlayerSummary,
    pub pseudonym: [u8; 16],
    pub imported_at: Option<Timestamp>,
    pub longest_ms: i64,
    pub crashes: i64,
    pub timeouts: i64,
    pub kicks: i64,
}

pub fn profile(conn: &Connection, id: i64, recent_from: &str) -> Result<Option<Profile>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT p.id, p.name, p.license, p.play_ms, p.imported_ms,
                 coalesce((SELECT sum(d.play_ms) FROM player_day d WHERE d.player = p.id AND d.day >= ?2), 0),
                 p.sessions, p.first_seen, p.last_seen, p.pseudonym, p.imported_at, p.longest_ms, p.crashes,
                 p.timeouts, p.kicks
             FROM player p WHERE p.id = ?1",
            params![id, recent_from],
            |row| {
                Ok(Profile {
                    summary: summary(row)?,
                    pseudonym: row.get(9)?,
                    imported_at: row.get::<_, Option<Utc>>(10)?.map(|at| at.0),
                    longest_ms: row.get(11)?,
                    crashes: row.get(12)?,
                    timeouts: row.get(13)?,
                    kicks: row.get(14)?,
                })
            },
        )
        .optional()?)
}

/// The players' ids, licences and total play time (imported time included), by licence.
pub fn by_license(conn: &Connection, licenses: &[String]) -> Result<HashMap<String, (i64, i64)>, StoreError> {
    let mut statement =
        conn.prepare_cached("SELECT id, play_ms + imported_ms FROM player WHERE license = ?1")?;
    let mut found = HashMap::new();
    for license in licenses {
        if let Some(row) = statement
            .query_row([license], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?
        {
            found.insert(license.clone(), row);
        }
    }
    Ok(found)
}

pub fn names(conn: &Connection, player: i64) -> Result<Vec<PlayerName>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT name, first_seen, last_seen FROM player_name WHERE player = ?1 ORDER BY last_seen DESC, name",
    )?;
    let rows = statement.query_map([player], |row| {
        Ok(PlayerName {
            name: row.get(0)?,
            first_seen: row.get::<_, Utc>(1)?.0,
            last_seen: row.get::<_, Utc>(2)?.0,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The player's play time on each day from `from`, oldest first.
pub fn days(conn: &Connection, player: i64, from: &str) -> Result<Vec<PlayDay>, StoreError> {
    let mut statement =
        conn.prepare("SELECT day, play_ms FROM player_day WHERE player = ?1 AND day >= ?2 ORDER BY day")?;
    let rows = statement.query_map(params![player, from], |row| {
        Ok(PlayDay {
            day: row.get(0)?,
            play_ms: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every identifier the player showed, an address's value left out, with how many other
/// licences showed it.
pub fn identifiers(conn: &Connection, player: i64) -> Result<Vec<ShownIdentifier>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT i.kind, i.value, pi.first_seen, pi.last_seen, pi.times, pi.source,
             (SELECT count(*) FROM player_identifier other WHERE other.identifier = i.id AND other.player <> ?1)
         FROM player_identifier pi JOIN identifier i ON i.id = pi.identifier
         WHERE pi.player = ?1 ORDER BY i.kind, pi.last_seen DESC",
    )?;
    let rows = statement.query_map([player], |row| {
        let kind = identifier_kind(row, 0)?;
        Ok(ShownIdentifier {
            kind,
            value: (kind != IdentifierKind::Ip).then(|| row.get(1)).transpose()?,
            first_seen: row.get::<_, Utc>(2)?.0,
            last_seen: row.get::<_, Utc>(3)?.0,
            times: row.get(4)?,
            imported: row.get::<_, String>(5)? == "imported",
            other_licences: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn identifier_kind(row: &Row<'_>, index: usize) -> rusqlite::Result<IdentifierKind> {
    let name: String = row.get(index)?;
    IdentifierKind::from_name(&name).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("unknown identifier kind {name}").into(),
        )
    })
}

/// The licences linked to the player's. A link is strong through an identifier Observer saw
/// itself on at most three licences; the others are weak, listed but never followed. The group
/// goes two steps through strong links, fifty licences at most.
pub fn links(conn: &Connection, player: i64) -> Result<Vec<LinkedLicence>, StoreError> {
    let mut found: Vec<LinkedLicence> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::from([player]);
    let mut frontier = vec![player];
    for steps in 1..=2u8 {
        let mut next = Vec::new();
        for from in frontier {
            let mut shared: HashMap<i64, Vec<Link>> = HashMap::new();
            for (other, link) in neighbours(conn, from)? {
                if steps == 2 && !link.strong {
                    continue;
                }
                shared.entry(other).or_default().push(link);
            }
            let mut others: Vec<_> = shared.into_iter().collect();
            // Strong links fill the cap first: fifty licences behind one café's address must not
            // hide an alternate account linked by its Discord.
            others.sort_by_key(|(other, through)| (!through.iter().any(|link| link.strong), *other));
            for (other, through) in others {
                if !seen.insert(other) || found.len() >= MOST_LINKED {
                    continue;
                }
                let strong = through.iter().any(|link| link.strong);
                if strong {
                    next.push(other);
                }
                let (name, license) =
                    conn.query_row("SELECT name, license FROM player WHERE id = ?1", [other], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?;
                found.push(LinkedLicence {
                    player: other,
                    name,
                    license,
                    through,
                    strong,
                    steps,
                });
            }
        }
        frontier = next;
    }
    Ok(found)
}

/// The other licences that share an identifier with `player`, each with that identifier. An
/// identifier more than [`MOST_LINKED`] licences show, a proxy's address for one, links nobody in
/// particular: the profile counts its holders instead.
fn neighbours(conn: &Connection, player: i64) -> Result<Vec<(i64, Link)>, StoreError> {
    let mut statement = conn.prepare_cached(
        "WITH mine AS (
             SELECT pi.identifier, pi.source, i.kind, i.value,
                 (SELECT count(*) FROM player_identifier holders WHERE holders.identifier = pi.identifier)
                     AS holders
             FROM player_identifier pi JOIN identifier i ON i.id = pi.identifier
             WHERE pi.player = ?1
         )
         SELECT other.player, mine.kind, mine.value,
             mine.source = 'seen' AND other.source = 'seen' AND mine.kind <> 'ip' AND mine.holders <= ?2
         FROM mine JOIN player_identifier other ON other.identifier = mine.identifier AND other.player <> ?1
         WHERE mine.holders <= ?3",
    )?;
    let fan_out = i64::try_from(MOST_LINKED).unwrap_or(i64::MAX) + 1;
    let rows = statement.query_map(params![player, STRONG_UP_TO, fan_out], |row| {
        let kind = identifier_kind(row, 1)?;
        Ok((
            row.get(0)?,
            Link {
                kind,
                value: (kind != IdentifierKind::Ip).then(|| row.get(2)).transpose()?,
                strong: row.get(3)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Unique players and play time of each day from `from` to `to`, both included.
pub fn days_played(conn: &Connection, from: &str, to: &str) -> Result<Vec<(String, i64, i64)>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT day, count(*), sum(play_ms) FROM player_day WHERE day >= ?1 AND day <= ?2 GROUP BY day ORDER BY day",
    )?;
    let rows = statement.query_map(params![from, to], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The days from `from` on that `players` already have play time on.
pub fn days_of(conn: &Connection, players: &[i64], from: &str) -> Result<Vec<(i64, String)>, StoreError> {
    let mut statement =
        conn.prepare_cached("SELECT player, day FROM player_day WHERE player = ?1 AND day >= ?2")?;
    let mut days = Vec::new();
    for player in players {
        let rows = statement.query_map(params![player, from], |row| Ok((row.get(0)?, row.get(1)?)))?;
        for row in rows {
            days.push(row?);
        }
    }
    Ok(days)
}

/// The players who played between `from` and `to`, both included.
pub fn unique_players(conn: &Connection, from: &str, to: &str) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT count(DISTINCT player) FROM player_day WHERE day >= ?1 AND day <= ?2",
        params![from, to],
        |row| row.get(0),
    )?)
}

/// When each player seen first at `since` or later was, for the new players of each day.
pub fn first_visits(conn: &Connection, since: Timestamp) -> Result<Vec<Timestamp>, StoreError> {
    let mut statement = conn.prepare("SELECT first_seen FROM player WHERE first_seen >= ?1")?;
    let rows = statement.query_map([Utc(since)], |row| Ok(row.get::<_, Utc>(0)?.0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn settings(conn: &Connection) -> Result<PlayersSettings, StoreError> {
    let defaults = PlayersSettings::default();
    let days = |key, default: u32| -> Result<u32, StoreError> {
        Ok(settings::get(conn, key)?
            .and_then(|value| value.parse().ok())
            .unwrap_or(default))
    };
    let profile_days = match settings::get(conn, PROFILE_DAYS)? {
        Some(value) if value == "kept" => None,
        Some(value) => value.parse().ok().or(defaults.profile_days),
        None => defaults.profile_days,
    };
    let stored = PlayersSettings {
        link_days: days(LINK_DAYS, defaults.link_days)?,
        address_days: days(ADDRESS_DAYS, defaults.address_days)?,
        day_days: days(DAY_DAYS, defaults.day_days)?,
        profile_days,
    };
    // Values edited by hand out of their ranges are brought into them, never shortened below
    // what the owner chose.
    Ok(stored.clamped())
}

pub fn save_settings(tx: &Transaction<'_>, retention: &PlayersSettings) -> Result<(), StoreError> {
    let profile = retention
        .profile_days
        .map_or_else(|| "kept".to_owned(), |days| days.to_string());
    for (key, value) in [
        (LINK_DAYS, retention.link_days.to_string()),
        (ADDRESS_DAYS, retention.address_days.to_string()),
        (DAY_DAYS, retention.day_days.to_string()),
        (PROFILE_DAYS, profile),
    ] {
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
    }
    Ok(())
}

/// Keeps `key` as the key of the keyed fingerprints.
pub fn set_fingerprint_key(tx: &Transaction<'_>, key: [u8; 32]) -> Result<(), StoreError> {
    let hex = key.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![ADDRESS_KEY, hex],
    )?;
    Ok(())
}

/// The key of the keyed fingerprints, made from `fresh` the first time.
pub fn fingerprint_key(tx: &Transaction<'_>, fresh: [u8; 32]) -> Result<[u8; 32], StoreError> {
    let hex = fresh.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
        params![ADDRESS_KEY, hex],
    )?;
    let stored = settings::get(tx, ADDRESS_KEY)?.unwrap_or_default();
    let mut key = [0; 32];
    if stored.len() != 64 {
        return Err(StoreError::Corrupt(
            "the players' fingerprint key is malformed".to_owned(),
        ));
    }
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&stored[index * 2..index * 2 + 2], 16)
            .map_err(|_| StoreError::Corrupt("the players' fingerprint key is malformed".to_owned()))?;
    }
    Ok(key)
}

/// What the retention removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Expired {
    pub links: usize,
    pub names: usize,
    pub days: usize,
    pub players: usize,
    pub identifiers: usize,
}

/// Applies the retentions at `now`; `day_cutoff` is the first day of play time kept.
pub fn expire(
    tx: &Transaction<'_>,
    now: Timestamp,
    retention: &PlayersSettings,
    day_cutoff: &str,
) -> Result<Expired, StoreError> {
    let before = |days: u32| {
        Utc(now
            .checked_sub(jiff::SignedDuration::from_hours(24 * i64::from(days)))
            .unwrap_or(Timestamp::MIN))
    };
    let mut released: HashSet<i64> = HashSet::new();
    for (kinds, days) in [("<>", retention.link_days), ("=", retention.address_days)] {
        let mut statement = tx.prepare(&format!(
            "DELETE FROM player_identifier WHERE last_seen < ?1
                 AND (SELECT kind FROM identifier WHERE id = player_identifier.identifier) {kinds} 'ip'
             RETURNING identifier"
        ))?;
        let rows = statement.query_map([before(days)], |row| row.get(0))?;
        for identifier in rows {
            released.insert(identifier?);
        }
    }
    let mut expired = Expired {
        links: released.len(),
        ..Expired::default()
    };
    expired.names = tx.execute(
        "DELETE FROM player_name WHERE last_seen < ?1",
        [before(retention.link_days)],
    )?;
    expired.days = tx.execute("DELETE FROM player_day WHERE day < ?1", [day_cutoff])?;
    if let Some(days) = retention.profile_days {
        let gone: Vec<i64> = tx
            .prepare(
                "SELECT id FROM player WHERE last_seen < ?1
                     AND NOT EXISTS (SELECT 1 FROM player_open_session o WHERE o.player = player.id)",
            )?
            .query_map([before(days)], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        // A player online now has a crediting record, and keeps their profile.
        for player in gone {
            released.extend(identifiers_of(tx, player)?);
            expired.players += tx.execute("DELETE FROM player WHERE id = ?1", [player])?;
        }
    }
    expired.identifiers = drop_unheld(tx, &released)?;
    Ok(expired)
}

fn identifiers_of(tx: &Transaction<'_>, player: i64) -> Result<Vec<i64>, StoreError> {
    let mut statement = tx.prepare_cached("SELECT identifier FROM player_identifier WHERE player = ?1")?;
    let rows = statement.query_map([player], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Deletes those of `identifiers` no link holds any more.
fn drop_unheld<'a>(
    tx: &Transaction<'_>,
    identifiers: impl IntoIterator<Item = &'a i64>,
) -> Result<usize, StoreError> {
    let mut statement = tx.prepare_cached(
        "DELETE FROM identifier WHERE id = ?1
             AND NOT EXISTS (SELECT 1 FROM player_identifier pi WHERE pi.identifier = ?1)",
    )?;
    let mut dropped = 0;
    for identifier in identifiers {
        dropped += statement.execute([identifier])?;
    }
    Ok(dropped)
}

/// What forgetting a player left behind to blank elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forgotten {
    pub license: String,
    pub pseudonym: [u8; 16],
}

/// Deletes a player with everything that hangs on them, and the identifiers nobody else holds, and
/// records the licence's fingerprint as forgotten. Both databases overwrite what they delete
/// (`secure_delete`), so the rows do not stay readable in free space.
pub fn forget(
    tx: &Transaction<'_>,
    player: i64,
    fingerprint: [u8; 16],
    at: Timestamp,
) -> Result<Option<Forgotten>, StoreError> {
    // FAST, the connection's setting, leaves whole pages freed as they were: one player's days fill
    // whole pages of `player_day`.
    tx.pragma_update(None, "secure_delete", true)?;
    let held = identifiers_of(tx, player)?;
    let forgotten = tx
        .query_row(
            "DELETE FROM player WHERE id = ?1 RETURNING license, pseudonym",
            [player],
            |row| {
                Ok(Forgotten {
                    license: row.get(0)?,
                    pseudonym: row.get(1)?,
                })
            },
        )
        .optional()?;
    if forgotten.is_some() {
        drop_unheld(tx, &held)?;
        remember_forgotten(tx, fingerprint, at)?;
    }
    tx.pragma_update(None, "secure_delete", "FAST")?;
    Ok(forgotten)
}

pub fn remember_forgotten(
    tx: &Transaction<'_>,
    fingerprint: [u8; 16],
    at: Timestamp,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO player_forgotten (fingerprint, at) VALUES (?1, ?2)
         ON CONFLICT (fingerprint) DO UPDATE SET at = max(at, excluded.at)",
        params![fingerprint, Utc(at)],
    )?;
    Ok(())
}

pub fn forgotten(conn: &Connection) -> Result<HashMap<[u8; 16], Timestamp>, StoreError> {
    let mut statement = conn.prepare("SELECT fingerprint, at FROM player_forgotten")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get::<_, Utc>(1)?.0)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The id, licence and last visit of every player not seen after `until`: those a restored
/// backup may have brought back after they were forgotten.
pub fn seen_until(conn: &Connection, until: Timestamp) -> Result<Vec<(i64, String, Timestamp)>, StoreError> {
    let mut statement = conn.prepare("SELECT id, license, last_seen FROM player WHERE last_seen <= ?1")?;
    let rows = statement.query_map([Utc(until)], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get::<_, Utc>(2)?.0))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Writes the WAL back into the database file, so that deleted rows leave it too.
pub fn checkpoint(conn: &Connection) -> Result<(), StoreError> {
    crate::database::checkpoint(conn)
}

/// A player of txAdmin's players database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported<'a> {
    pub license: &'a str,
    pub name: Option<&'a str>,
    /// txAdmin's play time, kept apart from Observer's.
    pub play_ms: i64,
    pub joined: Timestamp,
    pub last: Timestamp,
    /// Validated, never an address.
    pub others: &'a [(IdentifierKind, String)],
    pub imported_at: Timestamp,
    /// Given to the player if Observer never saw them.
    pub pseudonym: [u8; 16],
}

/// Takes one of txAdmin's players in: the first visit is the earlier, the last visit the later,
/// the name txAdmin's only when its last visit is later, and the imported time replaced, never
/// added, so that running the import twice changes nothing. Links are imported ones until Observer
/// sees them. Returns whether the player is new.
pub fn import(tx: &Transaction<'_>, player: &Imported<'_>) -> Result<bool, StoreError> {
    let known: Option<i64> = tx
        .query_row(
            "SELECT id FROM player WHERE license = ?1",
            [player.license],
            |row| row.get(0),
        )
        .optional()?;
    let (joined, last) = (Utc(player.joined), Utc(player.last));
    let id = match known {
        Some(id) => {
            tx.execute(
                "UPDATE player SET first_seen = min(first_seen, ?2),
                     name = CASE WHEN ?4 IS NOT NULL AND ?3 > last_seen THEN ?4 ELSE name END,
                     last_seen = max(last_seen, ?3), imported_ms = ?5, imported_at = ?6
                 WHERE id = ?1",
                params![
                    id,
                    joined,
                    last,
                    player.name,
                    player.play_ms,
                    Utc(player.imported_at)
                ],
            )?;
            id
        }
        None => tx.query_row(
            "INSERT INTO player (license, pseudonym, name, first_seen, last_seen, imported_ms, imported_at)
             VALUES (?1, ?2, coalesce(?3, ''), ?4, ?5, ?6, ?7) RETURNING id",
            params![
                player.license,
                player.pseudonym,
                player.name,
                joined,
                last,
                player.play_ms,
                Utc(player.imported_at)
            ],
            |row| row.get(0),
        )?,
    };
    for (kind, value) in player.others {
        let identifier = identifier_id(tx, *kind, value)?;
        tx.execute(
            "INSERT INTO player_identifier (player, identifier, first_seen, last_seen, times, source)
             VALUES (?1, ?2, ?3, ?4, 1, 'imported')
             ON CONFLICT (player, identifier) DO UPDATE SET
                 first_seen = min(first_seen, excluded.first_seen), last_seen = max(last_seen, excluded.last_seen)",
            params![id, identifier, joined, last],
        )?;
    }
    if let Some(name) = player.name {
        record_name(tx, id, name, player.joined, player.last)?;
    }
    Ok(known.is_none())
}

#[cfg(test)]
mod tests;
