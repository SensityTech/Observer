//! Bans and what each one holds, what was changed on them since, the identifiers a connecting
//! player is checked against, and the bans' settings with their templates.

use std::fmt::Write as _;

use jiff::Timestamp;
use observer_domain::bans::{
    Ban, BanCounts, BanEdit, BanEndChange, BanFilter, BanLift, BanReasonChange, BanState, BanTemplate,
    BansSettings,
};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, Transaction, params, params_from_iter};

use crate::database::StoreError;
use crate::players::like_escaped;
use crate::settings;
use crate::time::Utc;

const MESSAGE: &str = "bans.message";
const TOKEN_MATCHES: &str = "bans.token_matches";
/// How a hardware token and the licence are stored among a ban's identifiers.
pub const TOKEN: &str = "token";
pub const LICENSE: &str = "license";

/// A ban about to be written. `held` is `(kind, value)` in the players' canonical form, the
/// licence as `("license", hex)`.
#[derive(Debug, Clone)]
pub struct NewBan<'a> {
    pub code: &'a str,
    pub player: Option<i64>,
    pub name: Option<&'a str>,
    pub license: Option<&'a str>,
    pub reason: &'a str,
    pub by_user: Option<i64>,
    pub by_name: &'a str,
    pub at: Timestamp,
    pub ends_at: Option<Timestamp>,
    pub held: &'a [(String, String)],
}

pub fn code_taken(conn: &Connection, code: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row("SELECT 1 FROM ban WHERE code = ?1", [code], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn insert(tx: &Transaction<'_>, ban: &NewBan<'_>) -> Result<i64, StoreError> {
    tx.execute(
        "INSERT INTO ban (code, player, name, license, reason, by_user, by_name, created_at, ends_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            ban.code,
            ban.player,
            ban.name,
            ban.license,
            ban.reason,
            ban.by_user,
            ban.by_name,
            Utc(ban.at),
            ban.ends_at.map(Utc),
        ],
    )?;
    let id = tx.last_insert_rowid();
    let mut statement =
        tx.prepare_cached("INSERT OR IGNORE INTO ban_identifier (ban, kind, value) VALUES (?1, ?2, ?3)")?;
    for (kind, value) in ban.held {
        statement.execute(params![id, kind, value])?;
    }
    Ok(id)
}

/// A known player's licence, latest name, and every identifier and token their profile links to.
pub struct Identity {
    pub license: String,
    pub name: String,
    pub others: Vec<(String, String)>,
}

pub fn identity(conn: &Connection, player: i64) -> Result<Option<Identity>, StoreError> {
    let Some((license, name)) = conn
        .query_row(
            "SELECT license, name FROM player WHERE id = ?1",
            [player],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut statement = conn.prepare(
        "SELECT i.kind, i.value FROM player_identifier pi JOIN identifier i ON i.id = pi.identifier
         WHERE pi.player = ?1 AND i.kind <> 'ip' ORDER BY i.kind, i.value",
    )?;
    let others = statement
        .query_map([player], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(Some(Identity {
        license,
        name,
        others,
    }))
}

/// An active ban that holds some of what a player shows, and how much of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: i64,
    pub code: String,
    pub reason: String,
    pub by: String,
    pub ends_at: Option<Timestamp>,
    /// Identifiers matched, the licence included, tokens apart.
    pub identifiers: u32,
    pub tokens: u32,
}

/// The bans active at `now` that hold any of `shown`, permanent ones first, then the latest to
/// end.
pub fn candidates(
    conn: &Connection,
    shown: &[(String, String)],
    now: Timestamp,
) -> Result<Vec<Candidate>, StoreError> {
    let mut found: Vec<Candidate> = Vec::new();
    let mut statement = conn.prepare_cached(
        "SELECT b.id, b.code, b.reason, b.by_name, b.ends_at FROM ban_identifier i JOIN ban b ON b.id = i.ban
         WHERE i.kind = ?1 AND i.value = ?2 AND b.lifted_at IS NULL AND (b.ends_at IS NULL OR b.ends_at > ?3)",
    )?;
    for (kind, value) in shown {
        let rows = statement.query_map(params![kind, value, Utc(now)], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                code: row.get(1)?,
                reason: row.get(2)?,
                by: row.get(3)?,
                ends_at: row.get::<_, Option<Utc>>(4)?.map(|at| at.0),
                identifiers: 0,
                tokens: 0,
            })
        })?;
        for row in rows {
            let row = row?;
            let index = if let Some(index) = found.iter().position(|candidate| candidate.id == row.id) {
                index
            } else {
                found.push(row);
                found.len() - 1
            };
            if kind == TOKEN {
                found[index].tokens += 1;
            } else {
                found[index].identifiers += 1;
            }
        }
    }
    found.sort_by_key(|candidate| {
        std::cmp::Reverse(candidate.ends_at.map_or(i64::MAX, Timestamp::as_millisecond))
    });
    Ok(found)
}

pub fn count_refusal(tx: &Transaction<'_>, ban: i64) -> Result<(), StoreError> {
    tx.execute("UPDATE ban SET refused = refused + 1 WHERE id = ?1", [ban])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifted {
    Done,
    NotFound,
    NotActive,
}

pub fn lift(
    tx: &Transaction<'_>,
    code: &str,
    by: &str,
    reason: Option<&str>,
    now: Timestamp,
) -> Result<Lifted, StoreError> {
    let changed = tx.execute(
        "UPDATE ban SET lifted_at = ?2, lifted_by = ?3, lift_reason = ?4
         WHERE code = ?1 AND lifted_at IS NULL AND (ends_at IS NULL OR ends_at > ?2)",
        params![code, Utc(now), by, reason],
    )?;
    if changed == 1 {
        return Ok(Lifted::Done);
    }
    Ok(if code_taken(tx, code)? {
        Lifted::NotActive
    } else {
        Lifted::NotFound
    })
}

/// An active ban as an edit finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub id: i64,
    pub reason: String,
    pub ends_at: Option<Timestamp>,
}

/// The ban of this code while it is active at `now`.
pub fn standing(conn: &Connection, code: &str, now: Timestamp) -> Result<Option<Standing>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT id, reason, ends_at FROM ban
             WHERE code = ?1 AND lifted_at IS NULL AND (ends_at IS NULL OR ends_at > ?2)",
            params![code, Utc(now)],
            |row| {
                Ok(Standing {
                    id: row.get(0)?,
                    reason: row.get(1)?,
                    ends_at: row.get::<_, Option<Utc>>(2)?.map(|at| at.0),
                })
            },
        )
        .optional()?)
}

/// A change of a ban's reason, of its end, or of both, each as `(before, after)`.
#[derive(Debug, Clone)]
pub struct NewEdit<'a> {
    pub ban: i64,
    pub by_user: Option<i64>,
    pub by_name: &'a str,
    pub at: Timestamp,
    pub reason: (&'a str, &'a str),
    pub ends_at: (Option<Timestamp>, Option<Timestamp>),
}

/// Gives the ban its new reason and end, and keeps what they replace.
pub fn edit(tx: &Transaction<'_>, edit: &NewEdit<'_>) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE ban SET reason = ?2, ends_at = ?3 WHERE id = ?1",
        params![edit.ban, edit.reason.1, edit.ends_at.1.map(Utc)],
    )?;
    tx.execute(
        "INSERT INTO ban_edit (ban, at, by_user, by_name, reason_before, reason_after, ends_before, ends_after)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            edit.ban,
            Utc(edit.at),
            edit.by_user,
            edit.by_name,
            edit.reason.0,
            edit.reason.1,
            edit.ends_at.0.map(Utc),
            edit.ends_at.1.map(Utc),
        ],
    )?;
    Ok(())
}

/// What a page of the list asks for.
#[derive(Debug, Clone)]
pub struct BanQuery {
    pub filter: BanFilter,
    pub search: BanMatch,
    /// The id of the last ban of the previous page.
    pub after: Option<i64>,
    pub limit: u32,
    pub now: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanMatch {
    All,
    Code(String),
    /// The start of a name, or words anywhere in the reason.
    Text(String),
    License(String),
    Identifier(String, String),
}

const COLUMNS: &str =
    "b.id, b.code, b.player, b.name, b.license, b.reason, b.by_name, b.created_at, b.ends_at,
    b.lifted_at, b.lifted_by, b.lift_reason, b.refused,
    (SELECT count(*) FROM ban_identifier i WHERE i.ban = b.id AND i.kind NOT IN ('license', 'token')),
    (SELECT count(*) FROM ban_identifier i WHERE i.ban = b.id AND i.kind = 'token')";

/// A page of bans, latest first, with each one's id for the next page.
pub fn page(conn: &Connection, query: &BanQuery) -> Result<Vec<(Ban, i64)>, StoreError> {
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(Utc(query.now))];
    let mut sql = format!("SELECT {COLUMNS} FROM ban b WHERE 1");
    sql.push_str(match query.filter {
        BanFilter::Active => " AND b.lifted_at IS NULL AND (b.ends_at IS NULL OR b.ends_at > ?1)",
        BanFilter::Ended => " AND b.lifted_at IS NULL AND b.ends_at <= ?1",
        BanFilter::Lifted => " AND b.lifted_at IS NOT NULL",
        BanFilter::All => " AND ?1 IS NOT NULL",
    });
    match &query.search {
        BanMatch::All => {}
        BanMatch::Code(code) => {
            values.push(Box::new(code.clone()));
            let _ = write!(sql, " AND b.code = ?{}", values.len());
        }
        BanMatch::Text(text) => {
            values.push(Box::new(format!("{}%", like_escaped(text))));
            values.push(Box::new(format!("%{}%", like_escaped(text))));
            let (start, anywhere) = (values.len() - 1, values.len());
            let _ = write!(
                sql,
                " AND (b.name LIKE ?{start} ESCAPE '\\' OR b.reason LIKE ?{anywhere} ESCAPE '\\')"
            );
        }
        BanMatch::License(license) => {
            values.push(Box::new(license.clone()));
            let _ = write!(sql, " AND b.license = ?{}", values.len());
        }
        BanMatch::Identifier(kind, value) => {
            values.push(Box::new(kind.clone()));
            values.push(Box::new(value.clone()));
            let (kind, value) = (values.len() - 1, values.len());
            let _ = write!(
                sql,
                " AND b.id IN (SELECT ban FROM ban_identifier WHERE kind = ?{kind} AND value = ?{value})"
            );
        }
    }
    if let Some(after) = query.after {
        values.push(Box::new(after));
        let _ = write!(sql, " AND b.id < ?{}", values.len());
    }
    values.push(Box::new(query.limit));
    let _ = write!(sql, " ORDER BY b.id DESC LIMIT ?{}", values.len());
    let mut statement = conn.prepare(&sql)?;
    let now = query.now;
    let rows = statement.query_map(params_from_iter(values), |row| ban(row, now))?;
    edited(conn, rows.collect::<Result<_, _>>()?)
}

pub fn counts(conn: &Connection, now: Timestamp) -> Result<BanCounts, StoreError> {
    Ok(conn.query_row(
        "SELECT
             coalesce(sum(lifted_at IS NULL AND (ends_at IS NULL OR ends_at > ?1)), 0),
             coalesce(sum(lifted_at IS NULL AND ends_at <= ?1), 0),
             coalesce(sum(lifted_at IS NOT NULL), 0)
         FROM ban",
        [Utc(now)],
        |row| {
            Ok(BanCounts {
                active: row.get(0)?,
                ended: row.get(1)?,
                lifted: row.get(2)?,
            })
        },
    )?)
}

/// Every ban made from a player's profile, latest first.
pub fn of_player(conn: &Connection, player: i64, now: Timestamp) -> Result<Vec<Ban>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM ban b WHERE b.player = ?1 ORDER BY b.id DESC"
    ))?;
    let rows = statement.query_map([player], |row| ban(row, now))?;
    Ok(edited(conn, rows.collect::<Result<_, _>>()?)?
        .into_iter()
        .map(|(ban, _)| ban)
        .collect())
}

pub fn by_code(conn: &Connection, code: &str, now: Timestamp) -> Result<Option<Ban>, StoreError> {
    let found = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM ban b WHERE b.code = ?1"),
            [code],
            |row| ban(row, now),
        )
        .optional()?;
    Ok(edited(conn, found.into_iter().collect())?
        .pop()
        .map(|(ban, _)| ban))
}

/// The bans with what was changed on each, latest first.
fn edited(conn: &Connection, mut bans: Vec<(Ban, i64)>) -> Result<Vec<(Ban, i64)>, StoreError> {
    let mut statement = conn.prepare_cached(
        "SELECT at, by_name, reason_before, reason_after, ends_before, ends_after FROM ban_edit
         WHERE ban = ?1 ORDER BY id DESC",
    )?;
    for (ban, id) in &mut bans {
        let rows = statement.query_map([*id], |row| {
            let reason: (String, String) = (row.get(2)?, row.get(3)?);
            let ends = (
                row.get::<_, Option<Utc>>(4)?.map(|at| at.0),
                row.get::<_, Option<Utc>>(5)?.map(|at| at.0),
            );
            Ok(BanEdit {
                at: row.get::<_, Utc>(0)?.0,
                by: row.get(1)?,
                reason: (reason.0 != reason.1).then_some(BanReasonChange {
                    before: reason.0,
                    after: reason.1,
                }),
                ends: (ends.0 != ends.1).then_some(BanEndChange {
                    before: ends.0,
                    after: ends.1,
                }),
            })
        })?;
        ban.edits = rows.collect::<Result<_, _>>()?;
    }
    Ok(bans)
}

/// A ban without its edits, and its id.
fn ban(row: &Row<'_>, now: Timestamp) -> rusqlite::Result<(Ban, i64)> {
    let ends_at = row.get::<_, Option<Utc>>(8)?.map(|at| at.0);
    let lifted_at = row.get::<_, Option<Utc>>(9)?.map(|at| at.0);
    let lifted = match (lifted_at, row.get::<_, Option<String>>(10)?) {
        (Some(at), Some(by)) => Some(BanLift {
            at,
            by,
            reason: row.get(11)?,
        }),
        _ => None,
    };
    let state = if lifted.is_some() {
        BanState::Lifted
    } else if ends_at.is_some_and(|ends| ends <= now) {
        BanState::Ended
    } else {
        BanState::Active
    };
    let ban = Ban {
        code: row.get(1)?,
        player: row.get(2)?,
        name: row.get(3)?,
        license: row.get(4)?,
        reason: row.get(5)?,
        by: row.get(6)?,
        created_at: row.get::<_, Utc>(7)?.0,
        ends_at,
        state,
        lifted,
        refused: row.get(12)?,
        identifiers: row.get(13)?,
        tokens: row.get(14)?,
        edits: Vec::new(),
    };
    Ok((ban, row.get(0)?))
}

pub fn settings(conn: &Connection) -> Result<BansSettings, StoreError> {
    let defaults = BansSettings::default();
    let stored = BansSettings {
        message: settings::get(conn, MESSAGE)?.unwrap_or(defaults.message.clone()),
        token_matches: settings::get(conn, TOKEN_MATCHES)?
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.token_matches),
        templates: Vec::new(),
    };
    // A value edited by hand that the panel could not have saved falls back to the default, and
    // such a template is left out.
    Ok(BansSettings {
        templates: templates(conn)?,
        ..stored.checked().unwrap_or(defaults)
    })
}

/// The templates in the order the settings list them.
pub fn templates(conn: &Connection) -> Result<Vec<BanTemplate>, StoreError> {
    let mut statement = conn.prepare_cached("SELECT reason, minutes FROM ban_template ORDER BY position")?;
    let rows = statement.query_map([], |row| {
        Ok(BanTemplate {
            reason: row.get(0)?,
            minutes: row.get(1)?,
        })
    })?;
    let stored: Vec<BanTemplate> = rows.collect::<Result<_, _>>()?;
    Ok(stored.iter().filter_map(BanTemplate::checked).collect())
}

pub fn save_settings(tx: &Transaction<'_>, saved: &BansSettings) -> Result<(), StoreError> {
    settings::set(tx, MESSAGE, &saved.message)?;
    settings::set(tx, TOKEN_MATCHES, &saved.token_matches.to_string())?;
    tx.execute("DELETE FROM ban_template", [])?;
    let mut statement =
        tx.prepare_cached("INSERT INTO ban_template (position, reason, minutes) VALUES (?1, ?2, ?3)")?;
    for (position, template) in (0_u32..).zip(&saved.templates) {
        statement.execute(params![position, template.reason, template.minutes])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    const LICENSE_HEX: &str = "00000000000000000000000000000000000000aa";

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn token(n: u8) -> (String, String) {
        (TOKEN.to_owned(), format!("4:{}", format!("{n:02x}").repeat(32)))
    }

    fn write(conn: &mut Connection, code: &str, ends: Option<&str>, held: &[(String, String)]) -> i64 {
        let tx = conn.transaction().unwrap();
        let license = format!("license:{LICENSE_HEX}");
        let id = insert(
            &tx,
            &NewBan {
                code,
                player: None,
                name: Some("kaito"),
                license: Some(&license),
                reason: "Aimbot on stream",
                by_user: Some(1),
                by_name: "maxime",
                at: at("2026-11-09T20:00:00Z"),
                ends_at: ends.map(at),
                held,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }

    #[test]
    fn a_ban_is_found_by_any_identifier_it_holds_and_counts_tokens_apart() {
        let mut conn = memory();
        let held = [
            (LICENSE.to_owned(), LICENSE_HEX.to_owned()),
            ("discord".to_owned(), "123456789012345678".to_owned()),
            token(1),
            token(2),
        ];
        write(&mut conn, "AAAA-AAAA", None, &held);
        let now = at("2026-11-10T00:00:00Z");

        let by_tokens = candidates(&conn, &[token(1), token(2), token(3)], now).unwrap();
        let by_discord = candidates(&conn, &[held[1].clone()], now).unwrap();
        let stranger = candidates(&conn, &[token(9)], now).unwrap();

        assert_eq!(by_tokens.len(), 1);
        assert_eq!((by_tokens[0].identifiers, by_tokens[0].tokens), (0, 2));
        assert_eq!((by_discord[0].identifiers, by_discord[0].tokens), (1, 0));
        assert_eq!(stranger, []);
    }

    #[test]
    fn ended_and_lifted_bans_hold_nothing_and_permanent_ones_come_first() {
        let mut conn = memory();
        let held = [token(1)];
        write(&mut conn, "AAAA-AAAA", Some("2026-11-09T21:00:00Z"), &held);
        write(&mut conn, "BBBB-BBBB", Some("2026-12-01T00:00:00Z"), &held);
        write(&mut conn, "CCCC-CCCC", None, &held);
        write(&mut conn, "DDDD-DDDD", None, &held);
        let now = at("2026-11-10T00:00:00Z");
        let tx = conn.transaction().unwrap();
        assert_eq!(
            lift(&tx, "DDDD-DDDD", "sarah", Some("appeal"), now).unwrap(),
            Lifted::Done
        );
        tx.commit().unwrap();

        let found: Vec<String> = candidates(&conn, &held, now)
            .unwrap()
            .into_iter()
            .map(|candidate| candidate.code)
            .collect();

        assert_eq!(found, ["CCCC-CCCC", "BBBB-BBBB"]);
    }

    #[test]
    fn only_an_active_ban_is_lifted() {
        let mut conn = memory();
        write(&mut conn, "AAAA-AAAA", Some("2026-11-09T21:00:00Z"), &[token(1)]);
        write(&mut conn, "BBBB-BBBB", None, &[token(1)]);
        let now = at("2026-11-10T00:00:00Z");
        let tx = conn.transaction().unwrap();

        assert_eq!(
            lift(&tx, "AAAA-AAAA", "sarah", None, now).unwrap(),
            Lifted::NotActive
        );
        assert_eq!(
            lift(&tx, "ZZZZ-ZZZZ", "sarah", None, now).unwrap(),
            Lifted::NotFound
        );
        assert_eq!(lift(&tx, "BBBB-BBBB", "sarah", None, now).unwrap(), Lifted::Done);
        assert_eq!(
            lift(&tx, "BBBB-BBBB", "sarah", None, now).unwrap(),
            Lifted::NotActive
        );
        tx.commit().unwrap();

        let lifted = by_code(&conn, "BBBB-BBBB", now).unwrap().unwrap();
        assert_eq!(lifted.state, BanState::Lifted);
        assert_eq!(lifted.lifted.unwrap().by, "sarah");
        assert_eq!(
            by_code(&conn, "AAAA-AAAA", now).unwrap().unwrap().state,
            BanState::Ended
        );
    }

    #[test]
    fn the_list_filters_searches_and_pages_latest_first() {
        let mut conn = memory();
        let held = [
            (LICENSE.to_owned(), LICENSE_HEX.to_owned()),
            ("discord".to_owned(), "123456789012345678".to_owned()),
            token(1),
        ];
        let first = write(&mut conn, "AAAA-AAAA", None, &held);
        write(&mut conn, "BBBB-BBBB", Some("2026-11-09T21:00:00Z"), &held);
        let third = write(&mut conn, "CCCC-CCCC", None, &held);
        let now = at("2026-11-10T00:00:00Z");
        let query = |filter, search, after| BanQuery {
            filter,
            search,
            after,
            limit: 1,
            now,
        };

        let active = page(&conn, &query(BanFilter::Active, BanMatch::All, None)).unwrap();
        let next = page(&conn, &query(BanFilter::Active, BanMatch::All, Some(third))).unwrap();
        let ended = page(&conn, &query(BanFilter::Ended, BanMatch::All, None)).unwrap();
        let by_code = page(
            &conn,
            &query(BanFilter::All, BanMatch::Code("BBBB-BBBB".into()), None),
        )
        .unwrap();
        let by_reason = page(
            &conn,
            &query(BanFilter::All, BanMatch::Text("stream".into()), None),
        )
        .unwrap();
        let by_identifier = page(
            &conn,
            &query(
                BanFilter::Active,
                BanMatch::Identifier("discord".into(), "123456789012345678".into()),
                Some(first),
            ),
        )
        .unwrap();

        assert_eq!(active[0].0.code, "CCCC-CCCC");
        assert_eq!((active[0].0.identifiers, active[0].0.tokens), (1, 1));
        assert_eq!(next[0].0.code, "AAAA-AAAA");
        assert_eq!(ended[0].0.state, BanState::Ended);
        assert_eq!(by_code[0].0.code, "BBBB-BBBB");
        assert_eq!(by_reason[0].0.code, "CCCC-CCCC");
        assert_eq!(by_identifier, []);
        assert_eq!(
            counts(&conn, now).unwrap(),
            BanCounts {
                active: 2,
                ended: 1,
                lifted: 0
            }
        );
    }

    fn change(
        conn: &mut Connection,
        ban: i64,
        by: &str,
        when: &str,
        reason: (&str, &str),
        ends_at: (Option<&str>, Option<&str>),
    ) {
        let tx = conn.transaction().unwrap();
        edit(
            &tx,
            &NewEdit {
                ban,
                by_user: Some(3),
                by_name: by,
                at: at(when),
                reason,
                ends_at: (ends_at.0.map(at), ends_at.1.map(at)),
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn an_edit_gives_the_ban_its_new_values_and_keeps_what_they_replace() {
        let mut conn = memory();
        let id = write(&mut conn, "AAAA-AAAA", Some("2026-11-12T20:00:00Z"), &[token(1)]);
        let now = at("2026-11-10T00:00:00Z");

        change(
            &mut conn,
            id,
            "sarah",
            "2026-11-10T08:00:00Z",
            ("Aimbot on stream", "Aimbot on stream"),
            (Some("2026-11-12T20:00:00Z"), None),
        );
        change(
            &mut conn,
            id,
            "maxime",
            "2026-11-10T09:00:00Z",
            ("Aimbot on stream", "Aimbot, confirmed by two clips"),
            (None, None),
        );

        let ban = by_code(&conn, "AAAA-AAAA", now).unwrap().unwrap();
        assert_eq!(ban.reason, "Aimbot, confirmed by two clips");
        assert_eq!((ban.ends_at, ban.state), (None, BanState::Active));
        assert_eq!(
            (ban.by.as_str(), ban.created_at),
            ("maxime", at("2026-11-09T20:00:00Z"))
        );
        assert_eq!(
            ban.edits,
            [
                BanEdit {
                    at: at("2026-11-10T09:00:00Z"),
                    by: "maxime".to_owned(),
                    reason: Some(BanReasonChange {
                        before: "Aimbot on stream".to_owned(),
                        after: "Aimbot, confirmed by two clips".to_owned(),
                    }),
                    ends: None,
                },
                BanEdit {
                    at: at("2026-11-10T08:00:00Z"),
                    by: "sarah".to_owned(),
                    reason: None,
                    ends: Some(BanEndChange {
                        before: Some(at("2026-11-12T20:00:00Z")),
                        after: None,
                    }),
                },
            ]
        );
        let listed = page(
            &conn,
            &BanQuery {
                filter: BanFilter::Active,
                search: BanMatch::All,
                after: None,
                limit: 10,
                now,
            },
        )
        .unwrap();
        assert_eq!(listed[0].0.edits.len(), 2);
    }

    #[test]
    fn an_edit_that_changes_nothing_is_refused_by_the_schema() {
        let mut conn = memory();
        let id = write(&mut conn, "AAAA-AAAA", None, &[token(1)]);
        let tx = conn.transaction().unwrap();

        let same = edit(
            &tx,
            &NewEdit {
                ban: id,
                by_user: None,
                by_name: "sarah",
                at: at("2026-11-10T08:00:00Z"),
                reason: ("Aimbot on stream", "Aimbot on stream"),
                ends_at: (None, None),
            },
        );

        assert!(same.is_err());
    }

    #[test]
    fn a_player_is_checked_against_the_end_an_edit_gave() {
        let mut conn = memory();
        let held = [token(1)];
        let shortened = write(&mut conn, "AAAA-AAAA", None, &held);
        let extended = write(&mut conn, "BBBB-BBBB", Some("2026-11-10T06:00:00Z"), &held);
        change(
            &mut conn,
            shortened,
            "sarah",
            "2026-11-10T00:00:00Z",
            ("Aimbot on stream", "Aimbot on stream"),
            (None, Some("2026-11-10T12:00:00Z")),
        );
        change(
            &mut conn,
            extended,
            "sarah",
            "2026-11-10T00:00:00Z",
            ("Aimbot on stream", "Ban evasion"),
            (Some("2026-11-10T06:00:00Z"), Some("2026-11-11T00:00:00Z")),
        );
        let codes = |now: &str| -> Vec<(String, String)> {
            candidates(&conn, &held, at(now))
                .unwrap()
                .into_iter()
                .map(|candidate| (candidate.code, candidate.reason))
                .collect()
        };

        assert_eq!(
            codes("2026-11-10T11:00:00Z"),
            [
                ("BBBB-BBBB".to_owned(), "Ban evasion".to_owned()),
                ("AAAA-AAAA".to_owned(), "Aimbot on stream".to_owned()),
            ]
        );
        assert_eq!(
            codes("2026-11-10T13:00:00Z"),
            [("BBBB-BBBB".to_owned(), "Ban evasion".to_owned())]
        );
        assert_eq!(codes("2026-11-11T00:00:00Z"), []);
    }

    #[test]
    fn only_an_active_ban_stands_to_be_edited() {
        let mut conn = memory();
        write(&mut conn, "AAAA-AAAA", Some("2026-11-09T21:00:00Z"), &[token(1)]);
        write(&mut conn, "BBBB-BBBB", None, &[token(1)]);
        let active = write(&mut conn, "CCCC-CCCC", Some("2026-11-12T20:00:00Z"), &[token(1)]);
        let now = at("2026-11-10T00:00:00Z");
        let tx = conn.transaction().unwrap();
        lift(&tx, "BBBB-BBBB", "sarah", None, now).unwrap();
        tx.commit().unwrap();

        assert_eq!(standing(&conn, "AAAA-AAAA", now).unwrap(), None);
        assert_eq!(standing(&conn, "BBBB-BBBB", now).unwrap(), None);
        assert_eq!(standing(&conn, "ZZZZ-ZZZZ", now).unwrap(), None);
        assert_eq!(
            standing(&conn, "CCCC-CCCC", now).unwrap(),
            Some(Standing {
                id: active,
                reason: "Aimbot on stream".to_owned(),
                ends_at: Some(at("2026-11-12T20:00:00Z")),
            })
        );
    }

    fn template(reason: &str, minutes: Option<u32>) -> BanTemplate {
        BanTemplate {
            reason: reason.to_owned(),
            minutes,
        }
    }

    #[test]
    fn templates_are_kept_in_their_order_and_replaced_whole() {
        let mut conn = memory();
        let save = |conn: &mut Connection, templates: Vec<BanTemplate>| {
            let tx = conn.transaction().unwrap();
            save_settings(
                &tx,
                &BansSettings {
                    templates,
                    ..BansSettings::default()
                },
            )
            .unwrap();
            tx.commit().unwrap();
        };

        save(
            &mut conn,
            vec![
                template("Cheating", None),
                template("Insults", Some(3 * 24 * 60)),
                template("Combat logging", Some(24 * 60)),
            ],
        );
        save(
            &mut conn,
            vec![
                template("Combat logging", Some(24 * 60)),
                template("Cheating", None),
            ],
        );

        assert_eq!(
            settings(&conn).unwrap().templates,
            [
                template("Combat logging", Some(24 * 60)),
                template("Cheating", None)
            ]
        );
        assert_eq!(templates(&conn).unwrap(), settings(&conn).unwrap().templates);
    }

    #[test]
    fn a_template_edited_into_one_no_ban_could_be_made_from_is_left_out() {
        let conn = memory();
        conn.execute_batch(
            "INSERT INTO ban_template (position, reason, minutes) VALUES (0, ' ', NULL), (1, 'Cheating', 60)",
        )
        .unwrap();

        assert_eq!(templates(&conn).unwrap(), [template("Cheating", Some(60))]);
        assert_eq!(settings(&conn).unwrap().message, BansSettings::default().message);
    }

    #[test]
    fn settings_edited_out_of_range_fall_back_to_the_defaults() {
        let mut conn = memory();
        assert_eq!(settings(&conn).unwrap(), BansSettings::default());
        let tx = conn.transaction().unwrap();
        save_settings(
            &tx,
            &BansSettings {
                message: "Banned: {reason}".to_owned(),
                token_matches: 2,
                templates: vec![template("Cheating", None)],
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(settings(&conn).unwrap().token_matches, 2);

        settings::set(&conn, TOKEN_MATCHES, "99").unwrap();

        assert_eq!(
            settings(&conn).unwrap(),
            BansSettings {
                templates: vec![template("Cheating", None)],
                ..BansSettings::default()
            }
        );
    }
}
