//! Players' drops grouped and counted: the crash signature or reason group of each closed session,
//! bursts of drops, and the hourly rollups that long ranges and every trend read. Hours are Unix
//! seconds; session and run times are Unix milliseconds.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use observer_domain::drops::{DropCategory, DropKind};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};

use crate::database::StoreError;

pub const HOUR_MS: i64 = 3_600_000;
/// Drops this close together may belong to one burst.
pub const BURST_MS: i64 = 10_000;
/// A burst is an incident from this many drops...
pub const BURST_DROPS: usize = 5;
/// ...when they are at least this share of the players online, in percent.
pub const BURST_SHARE: usize = 20;
/// The drops that count against players: their own sessions ending badly, never the server's stops
/// and crashes, nor what Observer closed without a report.
const UNEXPECTED: &str = "category IN ('timeout', 'crash', 'security', 'unknown') AND coalesce(kind, '') NOT IN ('lost', 'missed')";

/// The hour that holds `ms`, in Unix seconds.
pub fn hour_of(ms: i64) -> i64 {
    ms.div_euclid(HOUR_MS) * 3600
}

/// A closed session the normaliser has not classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unclassified {
    pub id: i64,
    pub category: DropCategory,
    pub crash_text: Option<String>,
    pub reason_text: Option<String>,
    pub player: Option<[u8; 16]>,
    pub ended_ms: i64,
}

/// The oldest closed sessions not classified yet.
pub fn unclassified(conn: &Connection, limit: u32) -> Result<Vec<Unclassified>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, category, crash_text, reason_text, player, ended_ms FROM player_session
         WHERE classified = 0 AND ended_ms IS NOT NULL ORDER BY ended_ms, id LIMIT ?1",
    )?;
    let rows = statement.query_map([limit], |row| {
        let category: String = row.get(1)?;
        Ok(Unclassified {
            id: row.get(0)?,
            category: category_of(1, &category)?,
            crash_text: row.get(2)?,
            reason_text: row.get(3)?,
            player: row.get(4)?,
            ended_ms: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Sessions a former normaliser classified are classified again by this one.
pub fn outdated(tx: &Transaction<'_>, normaliser: u32) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "UPDATE player_session SET classified = 0 WHERE classified <> 0 AND classified < ?1",
        [normaliser],
    )?)
}

/// A crash's signature as the normaliser built it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewSignature<'a> {
    pub signature: &'a str,
    pub form: &'a str,
    pub module: Option<&'a str>,
    pub game_build: Option<u32>,
    pub label: Option<&'a str>,
    pub resource: Option<&'a str>,
    pub normaliser: u32,
}

/// What a closed session is grouped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grouping<'a> {
    Signature(NewSignature<'a>),
    Reason(&'a str),
    None,
}

/// Whether a signature is known already.
pub fn signature_known(conn: &Connection, signature: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM crash_signature WHERE signature = ?1",
            [signature],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// How many signatures a player's crashes made since `since_ms`.
pub fn signatures_made(conn: &Connection, player: [u8; 16], since_ms: i64) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM crash_signature WHERE first_player = ?1 AND first_seen_ms >= ?2",
        params![player, since_ms],
        |row| row.get(0),
    )?)
}

/// Gives a closed session its group and the normaliser's version, and takes its hour out of the
/// rolled ones, whose drops it changes.
pub fn classify(
    tx: &Transaction<'_>,
    session: &Unclassified,
    grouping: Grouping<'_>,
    normaliser: u32,
) -> Result<(), StoreError> {
    let (signature, reason) = match grouping {
        Grouping::Signature(signature) => (
            Some(signature_id(tx, &signature, session.ended_ms, session.player)?),
            None,
        ),
        Grouping::Reason(template) => (None, Some(reason_id(tx, template)?)),
        Grouping::None => (None, None),
    };
    tx.execute(
        "UPDATE player_session SET signature = ?2, reason = ?3, classified = ?4 WHERE id = ?1",
        params![session.id, signature, reason, normaliser],
    )?;
    tx.execute(
        "DELETE FROM drop_rolled WHERE hour = ?1",
        [hour_of(session.ended_ms)],
    )?;
    Ok(())
}

fn signature_id(
    tx: &Transaction<'_>,
    signature: &NewSignature<'_>,
    at_ms: i64,
    player: Option<[u8; 16]>,
) -> Result<i64, StoreError> {
    let resource = match signature.resource {
        Some(name) => Some(tx.query_row(
            "INSERT INTO resource_name (name) VALUES (?1)
             ON CONFLICT (name) DO UPDATE SET name = excluded.name RETURNING id",
            [name],
            |row| row.get::<_, i64>(0),
        )?),
        None => None,
    };
    Ok(tx.query_row(
        "INSERT INTO crash_signature (signature, form, module, game_build, label, resource, normaliser,
             first_seen_ms, first_player)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (signature) DO UPDATE SET first_seen_ms = min(first_seen_ms, excluded.first_seen_ms)
         RETURNING id",
        params![
            signature.signature,
            signature.form,
            signature.module,
            signature.game_build,
            signature.label,
            resource,
            signature.normaliser,
            at_ms,
            player,
        ],
        |row| row.get(0),
    )?)
}

fn reason_id(tx: &Transaction<'_>, template: &str) -> Result<i64, StoreError> {
    Ok(tx.query_row(
        "INSERT INTO drop_reason (template) VALUES (?1)
         ON CONFLICT (template) DO UPDATE SET template = excluded.template RETURNING id",
        [template],
        |row| row.get(0),
    )?)
}

/// Finds the bursts among the unexpected drops players reported from `from_ms` to `to_ms`: at
/// least five within ten seconds, and at least a fifth of the sessions open then. A burst that
/// goes on joins the incident before it. Returns the drops that joined an incident.
pub fn find_incidents(tx: &Transaction<'_>, from_ms: i64, to_ms: i64) -> Result<usize, StoreError> {
    let drops: Vec<(i64, i64, i64, Option<i64>)> = {
        let mut statement = tx.prepare(&format!(
            "SELECT id, run, ended_ms, incident FROM player_session
             WHERE ended_ms >= ?1 AND ended_ms < ?2 AND synthesized = 0 AND {UNEXPECTED}
             ORDER BY run, ended_ms, id"
        ))?;
        let rows = statement.query_map(params![from_ms - BURST_MS, to_ms], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let mut joined = 0;
    let mut at = 0;
    while at < drops.len() {
        let (_, run, started, incident) = drops[at];
        if let Some(incident) = incident {
            // A burst that goes on: the drops that follow within ten seconds of each other join it.
            let mut last = started;
            let mut next = at + 1;
            let mut added = Vec::new();
            while next < drops.len() && drops[next].1 == run && drops[next].2 - last <= BURST_MS {
                if drops[next].3.is_none() {
                    added.push(drops[next].0);
                }
                last = drops[next].2;
                next += 1;
            }
            if !added.is_empty() {
                mark(tx, incident, &added)?;
                tx.execute(
                    "UPDATE drop_incident SET drops = drops + ?2, ended_ms = max(ended_ms, ?3) WHERE id = ?1",
                    params![incident, i64::try_from(added.len()).unwrap_or(i64::MAX), last],
                )?;
                joined += added.len();
            }
            at = next;
            continue;
        }
        let mut end = at;
        while end + 1 < drops.len()
            && drops[end + 1].1 == run
            && drops[end + 1].3.is_none()
            && drops[end + 1].2 - started <= BURST_MS
        {
            end += 1;
        }
        let count = end - at + 1;
        if count < BURST_DROPS {
            at += 1;
            continue;
        }
        let online: i64 = tx.query_row(
            "SELECT count(*) FROM player_session
             WHERE run = ?1 AND seen_ms <= ?2 AND (ended_ms IS NULL OR ended_ms >= ?2)",
            params![run, started],
            |row| row.get(0),
        )?;
        if count * 100 < BURST_SHARE * usize::try_from(online).unwrap_or(usize::MAX) {
            at += 1;
            continue;
        }
        while end + 1 < drops.len()
            && drops[end + 1].1 == run
            && drops[end + 1].3.is_none()
            && drops[end + 1].2 - drops[end].2 <= BURST_MS
        {
            end += 1;
        }
        let ended = drops[end].2;
        let hitch: Option<i64> = tx.query_row(
            "SELECT max(hitch_max_ms) FROM sample_5s WHERE ts >= ?1 AND ts <= ?2",
            params![started.div_euclid(1000) - 5, ended.div_euclid(1000) + 5],
            |row| row.get(0),
        )?;
        let ids: Vec<i64> = drops[at..=end].iter().map(|drop| drop.0).collect();
        let incident: i64 = tx.query_row(
            "INSERT INTO drop_incident (run, started_ms, ended_ms, drops, players, hitch_max_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id",
            params![
                run,
                started,
                ended,
                i64::try_from(ids.len()).unwrap_or(i64::MAX),
                online,
                hitch
            ],
            |row| row.get(0),
        )?;
        mark(tx, incident, &ids)?;
        joined += ids.len();
        at = end + 1;
    }
    Ok(joined)
}

/// Puts sessions in an incident, and takes their hours out of the rolled ones.
fn mark(tx: &Transaction<'_>, incident: i64, sessions: &[i64]) -> Result<(), StoreError> {
    for &session in sessions {
        let ended: i64 = tx.query_row(
            "UPDATE player_session SET incident = ?2 WHERE id = ?1 RETURNING ended_ms",
            params![session, incident],
            |row| row.get(0),
        )?;
        tx.execute("DELETE FROM drop_rolled WHERE hour = ?1", [hour_of(ended)])?;
    }
    Ok(())
}

/// Drops of one category, kind and subject, counted over some time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub category: DropCategory,
    pub kind: Option<DropKind>,
    /// The crash's signature or the kicking resource; 0 for none.
    pub subject: i64,
    pub drops: i64,
    /// Of the drops, those in an incident, and how many incidents those were.
    pub incident_drops: i64,
    pub incidents: i64,
    /// Of the drops, those whose phase FXServer could tell, and those of a player still loading.
    pub phased: i64,
    pub loading: i64,
}

impl Group {
    /// Drops, a whole incident counting as one.
    pub fn events(&self) -> i64 {
        self.drops - self.incident_drops + self.incidents
    }

    /// Whether these drops count against players: neither the server's own stops and crashes nor
    /// the sessions Observer closed without a report.
    pub fn unexpected(&self) -> bool {
        matches!(
            self.category,
            DropCategory::Timeout | DropCategory::Crash | DropCategory::Security | DropCategory::Unknown
        ) && !matches!(self.kind, Some(DropKind::Lost | DropKind::Missed))
    }
}

/// Drops and time played over some time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub groups: Vec<Group>,
    /// Time played, every session's overlap with the time.
    pub player_ms: i64,
    /// Sessions that ended in the time.
    pub sessions: i64,
}

impl Counts {
    /// Adds `other` in, group by group.
    pub fn add(&mut self, other: &Self) {
        self.player_ms += other.player_ms;
        self.sessions += other.sessions;
        for group in &other.groups {
            match self.groups.iter_mut().find(|known| {
                (known.category, known.kind, known.subject) == (group.category, group.kind, group.subject)
            }) {
                Some(known) => {
                    known.drops += group.drops;
                    known.incident_drops += group.incident_drops;
                    known.incidents += group.incidents;
                    known.phased += group.phased;
                    known.loading += group.loading;
                }
                None => self.groups.push(*group),
            }
        }
    }
}

/// Counts the sessions themselves from `from_ms` to `to_ms`, open sessions played up to `now_ms`.
pub fn live(conn: &Connection, from_ms: i64, to_ms: i64, now_ms: i64) -> Result<Counts, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT category, kind,
                CASE category WHEN 'crash' THEN coalesce(signature, 0) WHEN 'kick' THEN coalesce(resource, 0)
                    ELSE 0 END AS subject,
                count(*), count(incident), count(DISTINCT incident), count(phase),
                count(*) FILTER (WHERE phase = 'loading')
         FROM player_session WHERE ended_ms >= ?1 AND ended_ms < ?2
         GROUP BY category, coalesce(kind, ''), subject",
    )?;
    let groups = statement
        .query_map(params![from_ms, to_ms], group)?
        .collect::<Result<_, _>>()?;
    // Ended in or after the time, or still open: two index ranges rather than one scan.
    let player_ms: i64 = conn.query_row(
        "SELECT coalesce(sum(played), 0) FROM (
             SELECT min(ended_ms, ?2) - max(seen_ms, ?1) AS played FROM player_session
                 WHERE ended_ms > ?1 AND seen_ms < ?2
             UNION ALL
             SELECT min(?3, ?2) - max(seen_ms, ?1) FROM player_session
                 WHERE ended_ms IS NULL AND seen_ms < ?2 AND ?3 > ?1)",
        params![from_ms, to_ms, now_ms],
        |row| row.get(0),
    )?;
    let sessions: i64 = conn.query_row(
        "SELECT count(*) FROM player_session WHERE ended_ms >= ?1 AND ended_ms < ?2",
        params![from_ms, to_ms],
        |row| row.get(0),
    )?;
    Ok(Counts {
        groups,
        player_ms: player_ms.max(0),
        sessions,
    })
}

fn group(row: &Row<'_>) -> rusqlite::Result<Group> {
    let category: String = row.get(0)?;
    let kind: Option<String> = row.get(1)?;
    Ok(Group {
        category: category_of(0, &category)?,
        kind: kind
            .as_deref()
            .filter(|kind| !kind.is_empty())
            .and_then(DropKind::from_name),
        subject: row.get(2)?,
        drops: row.get(3)?,
        incident_drops: row.get(4)?,
        incidents: row.get(5)?,
        phased: row.get(6)?,
        loading: row.get(7)?,
    })
}

fn category_of(index: usize, name: &str) -> rusqlite::Result<DropCategory> {
    DropCategory::from_name(name).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Text,
            format!("unknown category {name}").into(),
        )
    })
}

/// The closed hours that hold sessions and have no rollup, oldest first.
pub fn pending_hours(conn: &Connection, now_ms: i64) -> Result<Vec<i64>, StoreError> {
    let first: Option<i64> =
        conn.query_row("SELECT min(seen_ms) FROM player_session", [], |row| row.get(0))?;
    let Some(first) = first else {
        return Ok(Vec::new());
    };
    let first = hour_of(first);
    let closed = hour_of(now_ms);
    let rolled: HashSet<i64> = {
        let mut statement = conn.prepare("SELECT hour FROM drop_rolled WHERE hour >= ?1")?;
        let rows = statement.query_map([first], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    Ok((first..closed)
        .step_by(3600)
        .filter(|hour| !rolled.contains(hour))
        .collect())
}

/// Writes a closed hour's rollups from its sessions, and marks it rolled.
pub fn roll_up(tx: &Transaction<'_>, hour: i64, now_ms: i64) -> Result<(), StoreError> {
    let from_ms = hour * 1000;
    let counts = live(tx, from_ms, from_ms + HOUR_MS, now_ms)?;
    tx.execute("DELETE FROM drop_1h WHERE hour = ?1", [hour])?;
    tx.execute("DELETE FROM exposure_1h WHERE hour = ?1", [hour])?;
    for group in &counts.groups {
        tx.execute(
            "INSERT INTO drop_1h (hour, category, kind, subject, drops, incident_drops, incidents, phased, loading)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                hour,
                group.category.as_str(),
                group.kind.map_or("", DropKind::as_str),
                group.subject,
                group.drops,
                group.incident_drops,
                group.incidents,
                group.phased,
                group.loading,
            ],
        )?;
    }
    if counts.player_ms > 0 || counts.sessions > 0 {
        tx.execute(
            "INSERT INTO exposure_1h (hour, player_ms, sessions) VALUES (?1, ?2, ?3)",
            params![hour, counts.player_ms, counts.sessions],
        )?;
    }
    tx.execute("INSERT OR IGNORE INTO drop_rolled (hour) VALUES (?1)", [hour])?;
    Ok(())
}

/// The counts of each hour from `from` to `to` (Unix seconds, on hours): the rollups of the hours
/// rolled, the sessions for the others, the hour under way up to `now_ms`.
pub fn hourly(
    conn: &Connection,
    from: i64,
    to: i64,
    now_ms: i64,
) -> Result<BTreeMap<i64, Counts>, StoreError> {
    let mut hours: BTreeMap<i64, Counts> = BTreeMap::new();
    let rolled: HashSet<i64> = {
        let mut statement =
            conn.prepare_cached("SELECT hour FROM drop_rolled WHERE hour >= ?1 AND hour < ?2")?;
        let rows = statement.query_map(params![from, to], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    {
        let mut statement = conn.prepare_cached(
            "SELECT d.hour, d.category, d.kind, d.subject, d.drops, d.incident_drops, d.incidents, d.phased,
                 d.loading
             FROM drop_1h d JOIN drop_rolled r ON r.hour = d.hour WHERE d.hour >= ?1 AND d.hour < ?2",
        )?;
        let mut rows = statement.query(params![from, to])?;
        while let Some(row) = rows.next()? {
            let hour: i64 = row.get(0)?;
            let category: String = row.get(1)?;
            let kind: String = row.get(2)?;
            hours.entry(hour).or_default().groups.push(Group {
                category: category_of(1, &category)?,
                kind: DropKind::from_name(&kind),
                subject: row.get(3)?,
                drops: row.get(4)?,
                incident_drops: row.get(5)?,
                incidents: row.get(6)?,
                phased: row.get(7)?,
                loading: row.get(8)?,
            });
        }
    }
    {
        let mut statement = conn.prepare_cached(
            "SELECT e.hour, e.player_ms, e.sessions FROM exposure_1h e JOIN drop_rolled r ON r.hour = e.hour
             WHERE e.hour >= ?1 AND e.hour < ?2",
        )?;
        let mut rows = statement.query(params![from, to])?;
        while let Some(row) = rows.next()? {
            let counts = hours.entry(row.get(0)?).or_default();
            counts.player_ms = row.get(1)?;
            counts.sessions = row.get(2)?;
        }
    }
    let now_hour = hour_of(now_ms);
    let mut hour = from;
    while hour < to && hour <= now_hour {
        if !rolled.contains(&hour) {
            let from_ms = hour * 1000;
            let counts = live(conn, from_ms, (from_ms + HOUR_MS).min(now_ms), now_ms)?;
            if counts != Counts::default() {
                hours.insert(hour, counts);
            }
        }
        hour += 3600;
    }
    Ok(hours)
}

/// The counts from `from_ms` to `to_ms`: whole hours as [`hourly`] reads them, the edges from the
/// sessions.
pub fn window(conn: &Connection, from_ms: i64, to_ms: i64, now_ms: i64) -> Result<Counts, StoreError> {
    let mut counts = Counts::default();
    if to_ms <= from_ms {
        return Ok(counts);
    }
    let first = (from_ms + HOUR_MS - 1).div_euclid(HOUR_MS) * HOUR_MS;
    let last = to_ms.div_euclid(HOUR_MS) * HOUR_MS;
    if first >= last {
        return live(conn, from_ms, to_ms, now_ms);
    }
    if from_ms < first {
        counts.add(&live(conn, from_ms, first, now_ms)?);
    }
    for hour in hourly(conn, first / 1000, last / 1000, now_ms)?.values() {
        counts.add(hour);
    }
    if last < to_ms {
        counts.add(&live(conn, last, to_ms, now_ms)?);
    }
    Ok(counts)
}

/// A crash signature as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureRow {
    pub id: i64,
    pub signature: String,
    pub form: String,
    pub module: Option<String>,
    pub game_build: Option<u32>,
    pub label: Option<String>,
    pub resource: Option<String>,
    pub first_seen_ms: i64,
}

pub fn signatures(conn: &Connection, ids: &[i64]) -> Result<Vec<SignatureRow>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(&format!(
        "SELECT c.id, c.signature, c.form, c.module, c.game_build, c.label, r.name, c.first_seen_ms
         FROM crash_signature c LEFT JOIN resource_name r ON r.id = c.resource WHERE c.id IN ({})",
        placeholders(ids.len())
    ))?;
    let rows = statement.query_map(params_from_iter(ids), |row| {
        Ok(SignatureRow {
            id: row.get(0)?,
            signature: row.get(1)?,
            form: row.get(2)?,
            module: row.get(3)?,
            game_build: row.get(4)?,
            label: row.get(5)?,
            resource: row.get(6)?,
            first_seen_ms: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The players each signature or each kicking resource dropped from `from_ms` to `to_ms` (or over
/// every session kept), counted, never named.
pub fn players_by_subject(
    conn: &Connection,
    category: DropCategory,
    range: Option<(i64, i64)>,
) -> Result<HashMap<i64, i64>, StoreError> {
    let column = if category == DropCategory::Crash {
        "signature"
    } else {
        "resource"
    };
    let (from_ms, to_ms) = range.unwrap_or((i64::MIN, i64::MAX));
    let mut statement = conn.prepare(&format!(
        "SELECT {column}, count(DISTINCT player) FROM player_session
         WHERE category = ?1 AND ended_ms >= ?2 AND ended_ms < ?3 AND {column} IS NOT NULL GROUP BY {column}"
    ))?;
    let rows = statement.query_map(params![category.as_str(), from_ms, to_ms], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The latest distinct redacted texts of a signature's crashes.
pub fn examples(conn: &Connection, signature: i64, limit: u32) -> Result<Vec<String>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT crash_text FROM player_session WHERE signature = ?1 AND crash_text IS NOT NULL
         GROUP BY crash_text ORDER BY max(ended_ms) DESC LIMIT ?2",
    )?;
    let rows = statement.query_map(params![signature, limit], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn resource_names(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, String>, StoreError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut statement = conn.prepare(&format!(
        "SELECT id, name FROM resource_name WHERE id IN ({})",
        placeholders(ids.len())
    ))?;
    let rows = statement.query_map(params_from_iter(ids), |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn resource_id(conn: &Connection, name: &str) -> Result<Option<i64>, StoreError> {
    Ok(conn
        .query_row("SELECT id FROM resource_name WHERE name = ?1", [name], |row| {
            row.get(0)
        })
        .optional()?)
}

/// A kick's reason group and how many kicks it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasonCount {
    pub resource: i64,
    pub template: String,
    pub kicks: i64,
}

/// The reason groups of the kicks from `from_ms` to `to_ms`, the largest first.
pub fn reason_groups(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Vec<ReasonCount>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT s.resource, r.template, count(*) FROM player_session s JOIN drop_reason r ON r.id = s.reason
         WHERE s.category = 'kick' AND s.ended_ms >= ?1 AND s.ended_ms < ?2 AND s.resource IS NOT NULL
         GROUP BY s.resource, s.reason ORDER BY 3 DESC, 2",
    )?;
    let rows = statement.query_map(params![from_ms, to_ms], |row| {
        Ok(ReasonCount {
            resource: row.get(0)?,
            template: row.get(1)?,
            kicks: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The players with the most drops of a category from `from_ms` to `to_ms`, by pseudonym.
pub fn top_players(
    conn: &Connection,
    category: Option<DropCategory>,
    from_ms: i64,
    to_ms: i64,
    least: i64,
    limit: u32,
) -> Result<Vec<([u8; 16], i64)>, StoreError> {
    let filter = match category {
        Some(category) => format!("category = '{}'", category.as_str()),
        None => UNEXPECTED.to_owned(),
    };
    let mut statement = conn.prepare(&format!(
        "SELECT player, count(*) FROM player_session
         WHERE {filter} AND synthesized = 0 AND ended_ms >= ?1 AND ended_ms < ?2 AND player IS NOT NULL
         GROUP BY player HAVING count(*) >= ?3 ORDER BY 2 DESC, player LIMIT ?4"
    ))?;
    let rows = statement.query_map(params![from_ms, to_ms, least, limit], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A run of FXServer as the changes and the chart read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: i64,
    pub generation: u32,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    pub start_cause: String,
    pub end_cause: Option<String>,
    pub build: Option<u32>,
    pub spawned_ms: Option<i64>,
    /// The sessions the server's own crash or hang closed.
    pub crash_drops: i64,
}

const RUN_ROW: &str = "SELECT r.id, r.generation, r.started_ms, r.ended_ms, r.start_cause, r.end_cause, r.build,
        r.spawned_ms, (SELECT count(*) FROM player_session s WHERE s.run = r.id AND s.category = 'serverCrash')
    FROM server_run r";

fn run_row(row: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        generation: row.get(1)?,
        started_ms: row.get(2)?,
        ended_ms: row.get(3)?,
        start_cause: row.get(4)?,
        end_cause: row.get(5)?,
        build: row.get(6)?,
        spawned_ms: row.get(7)?,
        crash_drops: row.get(8)?,
    })
}

/// The runs that overlap `from_ms` to `to_ms`, and the one before them, oldest first.
pub fn runs(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Vec<RunRow>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "{RUN_ROW} WHERE r.started_ms < ?2 AND (r.ended_ms IS NULL OR r.ended_ms >= ?1
             OR r.id = (SELECT id FROM server_run WHERE started_ms < ?1 ORDER BY started_ms DESC LIMIT 1))
         ORDER BY r.started_ms, r.id"
    ))?;
    let rows = statement.query_map(params![from_ms, to_ms], run_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The run FXServer's process started at `spawned_ms` with this generation, as the state
/// database's crashes know it.
pub fn run_spawned(
    conn: &Connection,
    spawned_ms: i64,
    generation: u32,
) -> Result<Option<RunRow>, StoreError> {
    Ok(conn
        .query_row(
            &format!("{RUN_ROW} WHERE r.spawned_ms = ?1 AND r.generation = ?2 ORDER BY r.id DESC LIMIT 1"),
            params![spawned_ms, generation],
            run_row,
        )
        .optional()?)
}

/// A burst of drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentRow {
    pub id: i64,
    pub run: i64,
    pub started_ms: i64,
    pub ended_ms: i64,
    pub drops: i64,
    pub players: Option<i64>,
    pub hitch_max_ms: Option<i64>,
    /// Its drops by category, the largest first.
    pub categories: Vec<(DropCategory, i64)>,
}

pub fn incidents(conn: &Connection, from_ms: i64, to_ms: i64) -> Result<Vec<IncidentRow>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, run, started_ms, ended_ms, drops, players, hitch_max_ms FROM drop_incident
         WHERE started_ms >= ?1 AND started_ms < ?2 ORDER BY started_ms",
    )?;
    let mut found: Vec<IncidentRow> = statement
        .query_map(params![from_ms, to_ms], |row| {
            Ok(IncidentRow {
                id: row.get(0)?,
                run: row.get(1)?,
                started_ms: row.get(2)?,
                ended_ms: row.get(3)?,
                drops: row.get(4)?,
                players: row.get(5)?,
                hitch_max_ms: row.get(6)?,
                categories: Vec::new(),
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut categories = conn.prepare_cached(
        "SELECT category, count(*) FROM player_session WHERE incident = ?1 GROUP BY category ORDER BY 2 DESC",
    )?;
    for incident in &mut found {
        incident.categories = categories
            .query_map([incident.id], |row| {
                let category: String = row.get(0)?;
                Ok((category_of(0, &category)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
    }
    Ok(found)
}

/// A session as the uptime card reads it: when it was played, how long after its run started, and
/// when it ended unexpectedly, if it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Played {
    pub seen_ms: i64,
    /// Its end, or now.
    pub until_ms: i64,
    pub run_started_ms: i64,
    pub unexpected_ms: Option<i64>,
}

pub fn played(conn: &Connection, from_ms: i64, to_ms: i64, now_ms: i64) -> Result<Vec<Played>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT s.seen_ms, coalesce(s.ended_ms, ?3), r.started_ms,
                CASE WHEN s.synthesized = 0 AND s.ended_ms IS NOT NULL AND {UNEXPECTED} THEN s.ended_ms END
         FROM player_session s JOIN server_run r ON r.id = s.run
         WHERE s.seen_ms < ?2 AND (s.ended_ms IS NULL OR s.ended_ms > ?1)"
    ))?;
    let rows = statement.query_map(params![from_ms, to_ms, now_ms], |row| {
        Ok(Played {
            seen_ms: row.get(0)?,
            until_ms: row.get(1)?,
            run_started_ms: row.get(2)?,
            unexpected_ms: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What the drop log is narrowed to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    pub from_ms: i64,
    pub to_ms: i64,
    pub category: Option<DropCategory>,
    pub kind: Option<DropKind>,
    pub resource: Option<i64>,
    pub signature: Option<i64>,
    pub run: Option<i64>,
    /// Only these players' sessions; an empty list matches none.
    pub players: Option<Vec<[u8; 16]>>,
    /// The page after this row, newest first.
    pub before: Option<(i64, i64)>,
}

/// A row of the drop log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRow {
    pub id: i64,
    pub ended_ms: i64,
    pub length_ms: Option<i64>,
    pub category: DropCategory,
    pub kind: Option<DropKind>,
    pub resource: Option<String>,
    pub signature: Option<i64>,
    pub reason_text: Option<String>,
    pub loading: Option<bool>,
    pub run: i64,
    pub generation: u32,
    pub player: Option<[u8; 16]>,
    pub synthesized: bool,
    pub incident: Option<i64>,
}

pub fn log(conn: &Connection, filter: &LogFilter, limit: u32) -> Result<Vec<LogRow>, StoreError> {
    if filter.players.as_ref().is_some_and(Vec::is_empty) {
        return Ok(Vec::new());
    }
    let mut sql = String::from(
        "SELECT s.id, s.ended_ms, s.length_ms, s.category, s.kind, r.name, s.signature, s.reason_text, s.phase,
                s.run, sr.generation, s.player, s.synthesized, s.incident
         FROM player_session s JOIN server_run sr ON sr.id = s.run LEFT JOIN resource_name r ON r.id = s.resource
         WHERE s.ended_ms IS NOT NULL AND s.ended_ms >= ? AND s.ended_ms < ?",
    );
    let mut values: Vec<rusqlite::types::Value> = vec![filter.from_ms.into(), filter.to_ms.into()];
    if let Some(category) = filter.category {
        sql.push_str(" AND s.category = ?");
        values.push(category.as_str().to_owned().into());
    }
    if let Some(kind) = filter.kind {
        sql.push_str(" AND s.kind = ?");
        values.push(kind.as_str().to_owned().into());
    }
    if let Some(resource) = filter.resource {
        sql.push_str(" AND s.resource = ?");
        values.push(resource.into());
    }
    if let Some(signature) = filter.signature {
        sql.push_str(" AND s.signature = ?");
        values.push(signature.into());
    }
    if let Some(run) = filter.run {
        sql.push_str(" AND s.run = ?");
        values.push(run.into());
    }
    if let Some(players) = &filter.players {
        let _ = write!(sql, " AND s.player IN ({})", placeholders(players.len()));
        values.extend(players.iter().map(|player| player.to_vec().into()));
    }
    if let Some((ended_ms, id)) = filter.before {
        sql.push_str(" AND (s.ended_ms, s.id) < (?, ?)");
        values.push(ended_ms.into());
        values.push(id.into());
    }
    sql.push_str(" ORDER BY s.ended_ms DESC, s.id DESC LIMIT ?");
    values.push(i64::from(limit).into());
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), |row| {
        let category: String = row.get(3)?;
        let kind: Option<String> = row.get(4)?;
        let phase: Option<String> = row.get(8)?;
        Ok(LogRow {
            id: row.get(0)?,
            ended_ms: row.get(1)?,
            length_ms: row.get(2)?,
            category: category_of(3, &category)?,
            kind: kind.as_deref().and_then(DropKind::from_name),
            resource: row.get(5)?,
            signature: row.get(6)?,
            reason_text: row.get(7)?,
            loading: phase.map(|phase| phase == "loading"),
            run: row.get(9)?,
            generation: row.get(10)?,
            player: row.get(11)?,
            synthesized: row.get(12)?,
            incident: row.get(13)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Deletes the incidents no kept session holds and the reason groups no kept session uses.
pub fn sweep(tx: &Transaction<'_>) -> Result<usize, StoreError> {
    let incidents = tx.execute(
        "DELETE FROM drop_incident WHERE id NOT IN (SELECT incident FROM player_session WHERE incident IS NOT NULL)",
        [],
    )?;
    let reasons = tx.execute(
        "DELETE FROM drop_reason WHERE id NOT IN (SELECT reason FROM player_session WHERE reason IS NOT NULL)",
        [],
    )?;
    Ok(incidents + reasons)
}

/// A forgotten player made no signature: a free-text one only their crashes hold loses its text.
pub fn forget(tx: &Transaction<'_>, pseudonym: [u8; 16]) -> Result<usize, StoreError> {
    let renamed = tx.execute(
        "UPDATE crash_signature SET signature = 'text:forgotten:' || id, label = NULL
         WHERE first_player = ?1 AND form IN ('text', 'exception') AND NOT EXISTS (
             SELECT 1 FROM player_session s
             WHERE s.signature = crash_signature.id AND s.player IS NOT NULL AND s.player <> ?1)",
        [pseudonym],
    )?;
    tx.execute(
        "UPDATE crash_signature SET first_player = NULL WHERE first_player = ?1",
        [pseudonym],
    )?;
    Ok(renamed)
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

#[cfg(test)]
mod tests;
