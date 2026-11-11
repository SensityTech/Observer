//! Maintenance mode: whether the server is closed, the owner's message, and who still gets in.

use jiff::Timestamp;
use observer_domain::maintenance::{MaintenancePass, MaintenanceSettings, MaintenanceStatus};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::database::StoreError;
use crate::settings;
use crate::time::Utc;

const ON: &str = "maintenance.on";
const SINCE: &str = "maintenance.since";
const BY: &str = "maintenance.by";
const MESSAGE: &str = "maintenance.message";
const SERVER_NAME: &str = "maintenance.server_name";

pub fn settings(conn: &Connection) -> Result<MaintenanceSettings, StoreError> {
    let defaults = MaintenanceSettings::default();
    let stored = MaintenanceSettings {
        message: settings::get(conn, MESSAGE)?.unwrap_or(defaults.message),
        server_name: settings::get(conn, SERVER_NAME)?,
    };
    // What was edited by hand and the panel could not have saved falls back to the defaults.
    Ok(stored.checked().unwrap_or_default())
}

pub fn save_settings(tx: &Transaction<'_>, saved: &MaintenanceSettings) -> Result<(), StoreError> {
    settings::set(tx, MESSAGE, &saved.message)?;
    settings::set(tx, SERVER_NAME, saved.server_name.as_deref().unwrap_or_default())
}

pub fn is_on(conn: &Connection) -> Result<bool, StoreError> {
    Ok(settings::get(conn, ON)?.is_some_and(|value| value == "1"))
}

/// Whether the server is closed, since when and by whom, and how many players still get in.
pub fn status(conn: &Connection) -> Result<MaintenanceStatus, StoreError> {
    let on = is_on(conn)?;
    let (since, by) = if on {
        (
            settings::get(conn, SINCE)?.and_then(|since| since.parse().ok()),
            settings::get(conn, BY)?,
        )
    } else {
        (None, None)
    };
    Ok(MaintenanceStatus {
        on,
        since,
        by,
        passes: conn.query_row("SELECT count(*) FROM maintenance_pass", [], |row| row.get(0))?,
    })
}

/// Closes the server to players from `at`, on the word of `by`.
pub fn turn_on(tx: &Transaction<'_>, at: Timestamp, by: &str) -> Result<(), StoreError> {
    settings::set(tx, ON, "1")?;
    settings::set(tx, SINCE, &at.to_string())?;
    settings::set(tx, BY, by)
}

pub fn turn_off(tx: &Transaction<'_>) -> Result<(), StoreError> {
    settings::set(tx, ON, "0")
}

pub fn has_pass(conn: &Connection, license: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM maintenance_pass WHERE license = ?1",
            [license],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// A pass about to be written.
#[derive(Debug, Clone)]
pub struct NewPass<'a> {
    pub license: &'a str,
    pub note: Option<&'a str>,
    pub by_user: Option<i64>,
    pub by_name: &'a str,
    pub at: Timestamp,
}

/// Puts a licence on the list. `None` when it is there already.
pub fn add_pass(tx: &Transaction<'_>, pass: &NewPass<'_>) -> Result<Option<i64>, StoreError> {
    let added = tx.execute(
        "INSERT INTO maintenance_pass (license, note, by_user, by_name, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (license) DO NOTHING",
        params![pass.license, pass.note, pass.by_user, pass.by_name, Utc(pass.at)],
    )?;
    Ok((added > 0).then(|| tx.last_insert_rowid()))
}

const PASS: &str = "SELECT m.id, m.license, p.id, p.name, m.note, m.by_name, m.added_at
    FROM maintenance_pass m LEFT JOIN player p ON p.license = m.license";

fn pass(row: &Row<'_>) -> rusqlite::Result<MaintenancePass> {
    Ok(MaintenancePass {
        id: row.get(0)?,
        license: row.get(1)?,
        player: row.get(2)?,
        name: row.get(3)?,
        note: row.get(4)?,
        by: row.get(5)?,
        added_at: row.get::<_, Utc>(6)?.0,
    })
}

/// The whole list, latest added first.
pub fn passes(conn: &Connection) -> Result<Vec<MaintenancePass>, StoreError> {
    let mut statement = conn.prepare(&format!("{PASS} ORDER BY m.id DESC"))?;
    let rows = statement.query_map([], pass)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn pass_by_id(conn: &Connection, id: i64) -> Result<Option<MaintenancePass>, StoreError> {
    Ok(conn
        .query_row(&format!("{PASS} WHERE m.id = ?1"), [id], pass)
        .optional()?)
}

/// Takes a licence off the list and returns its pass, or `None` when there is none with this id.
pub fn remove_pass(tx: &Transaction<'_>, id: i64) -> Result<Option<MaintenancePass>, StoreError> {
    let found = pass_by_id(tx, id)?;
    if found.is_some() {
        tx.execute("DELETE FROM maintenance_pass WHERE id = ?1", [id])?;
    }
    Ok(found)
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

    fn added(conn: &mut Connection, license: &str, note: Option<&str>, now: &str) -> Option<i64> {
        let tx = conn.transaction().unwrap();
        let id = add_pass(
            &tx,
            &NewPass {
                license,
                note,
                by_user: Some(1),
                by_name: "ada",
                at: at(now),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }

    #[test]
    fn maintenance_is_off_until_turned_on_and_says_since_when_and_by_whom() {
        let mut conn = memory();
        let before = status(&conn).unwrap();

        let tx = conn.transaction().unwrap();
        turn_on(&tx, at("2026-11-11T20:40:00.250Z"), "maxime").unwrap();
        tx.commit().unwrap();
        let on = status(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        turn_off(&tx).unwrap();
        tx.commit().unwrap();

        assert_eq!(before, MaintenanceStatus::default());
        assert_eq!(
            on,
            MaintenanceStatus {
                on: true,
                since: Some(at("2026-11-11T20:40:00.250Z")),
                by: Some("maxime".to_owned()),
                passes: 0,
            }
        );
        assert!(!is_on(&conn).unwrap());
        assert_eq!(status(&conn).unwrap(), MaintenanceStatus::default());
    }

    #[test]
    fn the_message_is_kept_and_one_the_panel_could_not_save_falls_back_to_the_default() {
        let mut conn = memory();
        let default = settings(&conn).unwrap();

        let tx = conn.transaction().unwrap();
        let named = MaintenanceSettings {
            message: "Back at 8, {name}.".to_owned(),
            server_name: Some("Aperture RP | maintenance".to_owned()),
        };
        save_settings(&tx, &named).unwrap();
        tx.commit().unwrap();
        let saved = settings(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        save_settings(
            &tx,
            &MaintenanceSettings {
                server_name: None,
                ..named.clone()
            },
        )
        .unwrap();
        tx.commit().unwrap();
        let unnamed = settings(&conn).unwrap();
        settings::set(&conn, MESSAGE, " \n ").unwrap();

        assert_eq!(default, MaintenanceSettings::default());
        assert_eq!(saved, named);
        assert_eq!(unnamed.server_name, None);
        assert_eq!(settings(&conn).unwrap(), MaintenanceSettings::default());
    }

    #[test]
    fn a_licence_is_on_the_list_once_with_its_note_and_goes_by_its_id() {
        let mut conn = memory();
        let (lena, tester) = (license('a'), license('b'));

        let first = added(&mut conn, &lena, None, "2026-11-09T10:00:00Z").unwrap();
        let again = added(&mut conn, &lena, Some("twice"), "2026-11-09T11:00:00Z");
        let second = added(
            &mut conn,
            &tester,
            Some("Tester from the dev team"),
            "2026-11-10T10:00:00Z",
        )
        .unwrap();
        let listed = passes(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        let removed = remove_pass(&tx, first).unwrap();
        let missing = remove_pass(&tx, 99).unwrap();
        tx.commit().unwrap();

        assert_eq!(again, None);
        assert_eq!(
            listed
                .iter()
                .map(|pass| (pass.id, pass.note.as_deref(), pass.by.as_str()))
                .collect::<Vec<_>>(),
            [
                (second, Some("Tester from the dev team"), "ada"),
                (first, None, "ada")
            ]
        );
        assert_eq!(listed[1].added_at, at("2026-11-09T10:00:00Z"));
        assert_eq!(removed.map(|pass| pass.license), Some(lena.clone()));
        assert_eq!(missing, None);
        assert!(!has_pass(&conn, &lena).unwrap());
        assert!(has_pass(&conn, &tester).unwrap());
        assert_eq!(status(&conn).unwrap().passes, 1);
    }
}
