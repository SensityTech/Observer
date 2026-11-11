//! The allowlist: the licences approved, the requests of refused players, and its settings.

use std::fmt::Write as _;

use jiff::Timestamp;
use observer_domain::allowlist::{AllowlistRequest, AllowlistSearch, AllowlistSettings, Approval};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, Transaction, params, params_from_iter};

use crate::database::StoreError;
use crate::players::like_escaped;
use crate::settings;
use crate::time::Utc;

const ENABLED: &str = "allowlist.enabled";
const MESSAGE: &str = "allowlist.message";

pub fn settings(conn: &Connection) -> Result<AllowlistSettings, StoreError> {
    let defaults = AllowlistSettings::default();
    let stored = AllowlistSettings {
        enabled: settings::get(conn, ENABLED)?.is_some_and(|value| value == "1"),
        message: settings::get(conn, MESSAGE)?.unwrap_or(defaults.message.clone()),
    };
    // A message edited by hand that the panel could not have saved falls back to the default.
    Ok(stored.checked().unwrap_or(AllowlistSettings {
        enabled: stored.enabled,
        ..defaults
    }))
}

pub fn save_settings(tx: &Transaction<'_>, saved: &AllowlistSettings) -> Result<(), StoreError> {
    settings::set(tx, ENABLED, if saved.enabled { "1" } else { "0" })?;
    settings::set(tx, MESSAGE, &saved.message)?;
    Ok(())
}

pub fn is_approved(conn: &Connection, license: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM allowlist_approval WHERE license = ?1",
            [license],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub fn code_taken(conn: &Connection, code: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row("SELECT 1 FROM allowlist_request WHERE code = ?1", [code], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

/// What a refused player's connection left: their request's code, and whether it is a new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub code: String,
    pub new: bool,
}

/// Counts a refused connection on the licence's request, or makes the request under `code`. Past
/// `keep` requests, those tried longest ago go.
pub fn ask(
    tx: &Transaction<'_>,
    license: &str,
    name: Option<&str>,
    code: &str,
    now: Timestamp,
    keep: u32,
) -> Result<Asked, StoreError> {
    let known: Option<String> = tx
        .query_row(
            "UPDATE allowlist_request SET last_at = ?2, tries = tries + 1, name = coalesce(?3, name)
             WHERE license = ?1 RETURNING code",
            params![license, Utc(now), name],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(code) = known {
        return Ok(Asked { code, new: false });
    }
    tx.execute(
        "INSERT INTO allowlist_request (code, license, name, first_at, last_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![code, license, name, Utc(now)],
    )?;
    tx.execute(
        "DELETE FROM allowlist_request WHERE id IN
             (SELECT id FROM allowlist_request ORDER BY last_at DESC, id DESC LIMIT -1 OFFSET ?1)",
        [keep],
    )?;
    Ok(Asked {
        code: code.to_owned(),
        new: true,
    })
}

/// What a page of a list asks for.
#[derive(Debug, Clone)]
pub struct ListQuery {
    pub search: Option<AllowlistSearch>,
    pub limit: u32,
}

/// Where a page of requests ended: the last try and the id of its last row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestCursor {
    pub last_at: Timestamp,
    pub id: i64,
}

/// A page of requests, latest try first, with where each one stands for the next page.
pub fn requests(
    conn: &Connection,
    query: &ListQuery,
    after: Option<RequestCursor>,
) -> Result<Vec<(AllowlistRequest, RequestCursor)>, StoreError> {
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();
    let mut sql =
        "SELECT id, code, license, name, first_at, last_at, tries FROM allowlist_request WHERE 1".to_owned();
    match &query.search {
        None => {}
        Some(AllowlistSearch::License(license)) => {
            values.push(Box::new(license.clone()));
            let _ = write!(sql, " AND license = ?{}", values.len());
        }
        Some(AllowlistSearch::Text { text, code }) => {
            values.push(Box::new(format!("{}%", like_escaped(text))));
            values.push(Box::new(code.clone()));
            let (name, code) = (values.len() - 1, values.len());
            let _ = write!(sql, " AND (name LIKE ?{name} ESCAPE '\\' OR code = ?{code})");
        }
    }
    if let Some(after) = after {
        values.push(Box::new(Utc(after.last_at)));
        values.push(Box::new(after.id));
        let (last_at, id) = (values.len() - 1, values.len());
        let _ = write!(sql, " AND (last_at, id) < (?{last_at}, ?{id})");
    }
    values.push(Box::new(query.limit));
    let _ = write!(sql, " ORDER BY last_at DESC, id DESC LIMIT ?{}", values.len());
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), |row| {
        let request = request(row)?;
        let cursor = RequestCursor {
            last_at: request.last_at,
            id: row.get(0)?,
        };
        Ok((request, cursor))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn request(row: &Row<'_>) -> rusqlite::Result<AllowlistRequest> {
    Ok(AllowlistRequest {
        code: row.get(1)?,
        license: row.get(2)?,
        name: row.get(3)?,
        first_at: row.get::<_, Utc>(4)?.0,
        last_at: row.get::<_, Utc>(5)?.0,
        tries: row.get(6)?,
    })
}

pub fn request_by_code(conn: &Connection, code: &str) -> Result<Option<AllowlistRequest>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT id, code, license, name, first_at, last_at, tries FROM allowlist_request WHERE code = ?1",
            [code],
            request,
        )
        .optional()?)
}

/// Removes a request and returns it, or `None` when no request has this code.
pub fn remove_request(tx: &Transaction<'_>, code: &str) -> Result<Option<AllowlistRequest>, StoreError> {
    let found = request_by_code(tx, code)?;
    if found.is_some() {
        tx.execute("DELETE FROM allowlist_request WHERE code = ?1", [code])?;
    }
    Ok(found)
}

/// Removes every request last tried at `until` or before, and returns how many.
pub fn remove_requests_until(tx: &Transaction<'_>, until: Timestamp) -> Result<u32, StoreError> {
    let removed = tx.execute("DELETE FROM allowlist_request WHERE last_at <= ?1", [Utc(until)])?;
    Ok(u32::try_from(removed).unwrap_or(u32::MAX))
}

/// An approval about to be written.
#[derive(Debug, Clone)]
pub struct NewApproval<'a> {
    pub license: &'a str,
    pub name: Option<&'a str>,
    pub note: Option<&'a str>,
    pub request: Option<&'a str>,
    pub by_user: Option<i64>,
    pub by_name: &'a str,
    pub at: Timestamp,
}

/// Approves a licence and drops the request it may have waiting. `None` when it is approved
/// already.
pub fn approve(tx: &Transaction<'_>, approval: &NewApproval<'_>) -> Result<Option<i64>, StoreError> {
    let added = tx.execute(
        "INSERT INTO allowlist_approval (license, name, note, request, by_user, by_name, approved_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT (license) DO NOTHING",
        params![
            approval.license,
            approval.name,
            approval.note,
            approval.request,
            approval.by_user,
            approval.by_name,
            Utc(approval.at),
        ],
    )?;
    if added == 0 {
        return Ok(None);
    }
    let id = tx.last_insert_rowid();
    tx.execute(
        "DELETE FROM allowlist_request WHERE license = ?1",
        [approval.license],
    )?;
    Ok(Some(id))
}

const APPROVAL: &str =
    "SELECT a.id, a.license, p.id, coalesce(p.name, a.name), a.note, a.by_name, a.approved_at,
        a.request
    FROM allowlist_approval a LEFT JOIN player p ON p.license = a.license";

fn approval(row: &Row<'_>) -> rusqlite::Result<Approval> {
    Ok(Approval {
        id: row.get(0)?,
        license: row.get(1)?,
        player: row.get(2)?,
        name: row.get(3)?,
        note: row.get(4)?,
        by: row.get(5)?,
        approved_at: row.get::<_, Utc>(6)?.0,
        request: row.get(7)?,
    })
}

/// A page of approvals, latest first. `after` is the id of the last one of the previous page.
pub fn approvals(
    conn: &Connection,
    query: &ListQuery,
    after: Option<i64>,
) -> Result<Vec<Approval>, StoreError> {
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();
    let mut sql = format!("{APPROVAL} WHERE 1");
    match &query.search {
        None => {}
        Some(AllowlistSearch::License(license)) => {
            values.push(Box::new(license.clone()));
            let _ = write!(sql, " AND a.license = ?{}", values.len());
        }
        Some(AllowlistSearch::Text { text, code }) => {
            values.push(Box::new(format!("{}%", like_escaped(text))));
            values.push(Box::new(format!("%{}%", like_escaped(text))));
            values.push(Box::new(code.clone()));
            let (start, anywhere, code) = (values.len() - 2, values.len() - 1, values.len());
            let _ = write!(
                sql,
                " AND (coalesce(p.name, a.name) LIKE ?{start} ESCAPE '\\' OR a.note LIKE ?{anywhere} ESCAPE '\\'
                       OR a.request = ?{code})"
            );
        }
    }
    if let Some(after) = after {
        values.push(Box::new(after));
        let _ = write!(sql, " AND a.id < ?{}", values.len());
    }
    values.push(Box::new(query.limit));
    let _ = write!(sql, " ORDER BY a.id DESC LIMIT ?{}", values.len());
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(values), approval)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn approval_of(conn: &Connection, license: &str) -> Result<Option<Approval>, StoreError> {
    Ok(conn
        .query_row(&format!("{APPROVAL} WHERE a.license = ?1"), [license], approval)
        .optional()?)
}

pub fn approval_by_id(conn: &Connection, id: i64) -> Result<Option<Approval>, StoreError> {
    Ok(conn
        .query_row(&format!("{APPROVAL} WHERE a.id = ?1"), [id], approval)
        .optional()?)
}

/// Removes an approval and returns it, or `None` when there is none with this id.
pub fn remove_approval(tx: &Transaction<'_>, id: i64) -> Result<Option<Approval>, StoreError> {
    let found = approval_by_id(tx, id)?;
    if found.is_some() {
        tx.execute("DELETE FROM allowlist_approval WHERE id = ?1", [id])?;
    }
    Ok(found)
}

/// How many requests wait, and how many licences are approved.
pub fn counts(conn: &Connection) -> Result<(u32, u32), StoreError> {
    Ok(conn.query_row(
        "SELECT (SELECT count(*) FROM allowlist_request), (SELECT count(*) FROM allowlist_approval)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn license(digit: char) -> String {
        format!("license:{}", digit.to_string().repeat(40))
    }

    fn asked(conn: &mut Connection, license: &str, name: &str, code: &str, now: &str) -> Asked {
        let tx = conn.transaction().unwrap();
        let asked = ask(&tx, license, Some(name), code, at(now), 100).unwrap();
        tx.commit().unwrap();
        asked
    }

    fn approved(
        conn: &mut Connection,
        license: &str,
        note: Option<&str>,
        request: Option<&str>,
    ) -> Option<i64> {
        let tx = conn.transaction().unwrap();
        let id = approve(
            &tx,
            &NewApproval {
                license,
                name: None,
                note,
                request,
                by_user: Some(1),
                by_name: "maxime",
                at: at("2026-11-11T12:00:00Z"),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }

    fn all() -> ListQuery {
        ListQuery {
            search: None,
            limit: 50,
        }
    }

    fn text(text: &str, code: Option<&str>) -> ListQuery {
        ListQuery {
            search: Some(AllowlistSearch::Text {
                text: text.to_owned(),
                code: code.map(str::to_owned),
            }),
            limit: 50,
        }
    }

    #[test]
    fn a_player_who_tries_again_keeps_their_request_and_its_code() {
        let mut conn = memory();

        let first = asked(
            &mut conn,
            &license('a'),
            "kaito",
            "AAA-AAA",
            "2026-11-11T20:00:00Z",
        );
        let again = asked(
            &mut conn,
            &license('a'),
            "kaito_",
            "BBB-BBB",
            "2026-11-11T20:05:00Z",
        );

        assert_eq!((first.code.as_str(), first.new), ("AAA-AAA", true));
        assert_eq!((again.code.as_str(), again.new), ("AAA-AAA", false));
        let listed = requests(&conn, &all(), None).unwrap();
        assert_eq!(listed.len(), 1);
        let request = &listed[0].0;
        assert_eq!(request.name.as_deref(), Some("kaito_"));
        assert_eq!(request.tries, 2);
        assert_eq!(request.first_at, at("2026-11-11T20:00:00Z"));
        assert_eq!(request.last_at, at("2026-11-11T20:05:00Z"));
        assert!(code_taken(&conn, "AAA-AAA").unwrap());
        assert!(!code_taken(&conn, "BBB-BBB").unwrap());
    }

    #[test]
    fn requests_list_the_latest_try_first_page_by_page_and_past_the_cap_the_oldest_go() {
        let mut conn = memory();
        asked(&mut conn, &license('a'), "Ada", "AAA-AAA", "2026-11-11T20:00:00Z");
        asked(&mut conn, &license('b'), "Bea", "BBB-BBB", "2026-11-11T20:01:00Z");
        asked(&mut conn, &license('a'), "Ada", "ZZZ-ZZZ", "2026-11-11T20:02:00Z");
        let one = ListQuery {
            search: None,
            limit: 1,
        };

        let first = requests(&conn, &one, None).unwrap();
        let second = requests(&conn, &one, Some(first[0].1)).unwrap();
        let third = requests(&conn, &one, Some(second[0].1)).unwrap();
        let tx = conn.transaction().unwrap();
        ask(&tx, &license('c'), None, "CCC-CCC", at("2026-11-11T20:03:00Z"), 2).unwrap();
        tx.commit().unwrap();
        let kept: Vec<String> = requests(&conn, &all(), None)
            .unwrap()
            .into_iter()
            .map(|(request, _)| request.code)
            .collect();

        assert_eq!(first[0].0.code, "AAA-AAA");
        assert_eq!(second[0].0.code, "BBB-BBB");
        assert_eq!(third, []);
        assert_eq!(kept, ["CCC-CCC", "AAA-AAA"]);
    }

    #[test]
    fn a_search_finds_a_request_by_its_code_its_licence_or_the_start_of_its_name() {
        let mut conn = memory();
        asked(
            &mut conn,
            &license('a'),
            "Nathan",
            "AAA-AAA",
            "2026-11-11T20:00:00Z",
        );
        asked(
            &mut conn,
            &license('b'),
            "kaito",
            "NAT-HAN",
            "2026-11-11T20:01:00Z",
        );
        let codes = |query: &ListQuery| -> Vec<String> {
            requests(&conn, query, None)
                .unwrap()
                .into_iter()
                .map(|(request, _)| request.code)
                .collect()
        };
        let by_license = ListQuery {
            search: Some(AllowlistSearch::License(license('b'))),
            limit: 50,
        };

        assert_eq!(codes(&text("Nathan", Some("NAT-HAN"))), ["NAT-HAN", "AAA-AAA"]);
        assert_eq!(codes(&text("kai", None)), ["NAT-HAN"]);
        assert_eq!(codes(&text("%", None)), Vec::<String>::new());
        assert_eq!(codes(&by_license), ["NAT-HAN"]);
    }

    #[test]
    fn approving_a_licence_drops_its_request_and_happens_once() {
        let mut conn = memory();
        asked(
            &mut conn,
            &license('a'),
            "kaito",
            "AAA-AAA",
            "2026-11-11T20:00:00Z",
        );

        let id = approved(&mut conn, &license('a'), None, Some("AAA-AAA"));
        let again = approved(&mut conn, &license('a'), Some("twice"), None);

        assert!(id.is_some());
        assert_eq!(again, None);
        assert!(is_approved(&conn, &license('a')).unwrap());
        assert!(!is_approved(&conn, &license('b')).unwrap());
        assert_eq!(requests(&conn, &all(), None).unwrap(), []);
        assert_eq!(counts(&conn).unwrap(), (0, 1));
    }

    #[test]
    fn approvals_take_the_name_of_the_profile_and_are_found_by_name_note_licence_or_request() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        crate::players::record_seen(
            &tx,
            &crate::players::Seen {
                license: &license('a'),
                name: Some("Nyx"),
                others: &[],
                at: at("2026-11-01T00:00:00Z"),
                pseudonym: [7; 16],
            },
        )
        .unwrap();
        tx.commit().unwrap();
        let nyx = approved(&mut conn, &license('a'), None, Some("K2P-7VD")).unwrap();
        let ahead = approved(&mut conn, &license('b'), Some("Sarah's brother"), None).unwrap();
        let ids = |query: &ListQuery, after: Option<i64>| -> Vec<i64> {
            approvals(&conn, query, after)
                .unwrap()
                .into_iter()
                .map(|approval| approval.id)
                .collect()
        };
        let by_license = ListQuery {
            search: Some(AllowlistSearch::License(license('b'))),
            limit: 50,
        };

        let listed = approvals(&conn, &all(), None).unwrap();

        assert_eq!(ids(&all(), None), [ahead, nyx]);
        assert_eq!(ids(&all(), Some(ahead)), [nyx]);
        assert_eq!(
            (listed[1].name.as_deref(), listed[1].player.is_some()),
            (Some("Nyx"), true)
        );
        assert_eq!((listed[0].name.as_deref(), listed[0].player), (None, None));
        assert_eq!(listed[0].note.as_deref(), Some("Sarah's brother"));
        assert_eq!(listed[1].request.as_deref(), Some("K2P-7VD"));
        assert_eq!(ids(&text("ny", None), None), [nyx]);
        assert_eq!(ids(&text("brother", None), None), [ahead]);
        assert_eq!(ids(&text("k2p7vd", Some("K2P-7VD")), None), [nyx]);
        assert_eq!(ids(&by_license, None), [ahead]);
    }

    #[test]
    fn requests_and_approvals_are_removed_one_by_one_or_up_to_a_time() {
        let mut conn = memory();
        asked(&mut conn, &license('a'), "Ada", "AAA-AAA", "2026-11-11T20:00:00Z");
        asked(&mut conn, &license('b'), "Bea", "BBB-BBB", "2026-11-11T20:01:00Z");
        asked(&mut conn, &license('c'), "Cy", "CCC-CCC", "2026-11-11T20:02:00Z");
        let approval = approved(&mut conn, &license('d'), None, None).unwrap();
        let tx = conn.transaction().unwrap();

        let denied = remove_request(&tx, "BBB-BBB").unwrap();
        let unknown = remove_request(&tx, "ZZZ-ZZZ").unwrap();
        let swept = remove_requests_until(&tx, at("2026-11-11T20:00:00Z")).unwrap();
        let removed = remove_approval(&tx, approval).unwrap();
        let gone = remove_approval(&tx, approval).unwrap();
        tx.commit().unwrap();

        assert_eq!(denied.map(|request| request.license), Some(license('b')));
        assert_eq!(unknown, None);
        assert_eq!(swept, 1);
        assert_eq!(removed.map(|approval| approval.license), Some(license('d')));
        assert_eq!(gone, None);
        assert_eq!(counts(&conn).unwrap(), (1, 0));
    }

    #[test]
    fn settings_default_to_off_and_a_message_edited_out_of_range_falls_back() {
        let mut conn = memory();
        assert_eq!(settings(&conn).unwrap(), AllowlistSettings::default());
        let tx = conn.transaction().unwrap();
        save_settings(
            &tx,
            &AllowlistSettings {
                enabled: true,
                message: "Ask us: {id}".to_owned(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(settings(&conn).unwrap().message, "Ask us: {id}");

        settings::set(&conn, MESSAGE, " ").unwrap();

        let fallen = settings(&conn).unwrap();
        assert!(fallen.enabled);
        assert_eq!(fallen.message, AllowlistSettings::default().message);
    }
}
