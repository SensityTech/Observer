use jiff::Timestamp;
use observer_domain::permissions::Permissions;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// An account as stored. The password hash never leaves the services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub is_owner: bool,
    /// Read through `Permissions::from_blob`: masked and closed.
    pub permissions: Permissions,
    pub must_change_password: bool,
    pub password_expires_at: Option<Timestamp>,
    pub auth_epoch: i64,
    pub version: i64,
    pub created_at: Timestamp,
    pub last_login_at: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct NewUser<'a> {
    pub username: &'a str,
    pub password_hash: &'a str,
    pub is_owner: bool,
    pub permissions: Permissions,
    pub must_change_password: bool,
    pub password_expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

const COLUMNS: &str = "id, username, password_hash, is_owner, permissions, must_change_password, \
    password_expires_at, auth_epoch, version, created_at, last_login_at";

/// Whether no account exists yet, which is when the owner must be created.
pub fn none(conn: &Connection) -> Result<bool, StoreError> {
    Ok(conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM users)", [], |row| row.get(0))?)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<UserRow>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM users WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], read).optional()?)
}

/// Usernames are unique whatever their case, and found whatever the case typed.
pub fn find(conn: &Connection, username: &str) -> Result<Option<UserRow>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM users WHERE username = ?1");
    Ok(conn.query_row(&sql, [username], read).optional()?)
}

/// The owner first, then everyone else by name.
pub fn list(conn: &Connection) -> Result<Vec<UserRow>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM users ORDER BY is_owner DESC, username");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The new account's id, or `None` when the username is taken.
pub fn insert(conn: &Connection, user: &NewUser<'_>) -> Result<Option<i64>, StoreError> {
    let inserted = conn.execute(
        "INSERT INTO users (username, password_hash, is_owner, permissions, must_change_password, \
             password_expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (username) DO NOTHING",
        params![
            user.username,
            user.password_hash,
            user.is_owner,
            user.permissions.to_blob(),
            user.must_change_password,
            user.password_expires_at.map(Utc),
            Utc(user.created_at),
        ],
    )?;
    Ok((inserted == 1).then(|| conn.last_insert_rowid()))
}

/// Writes every field that changes over an account's life; the id, name, role and creation
/// date never do.
pub fn update(conn: &Connection, user: &UserRow) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE users SET password_hash = ?2, permissions = ?3, must_change_password = ?4,
             password_expires_at = ?5, auth_epoch = ?6, version = ?7, last_login_at = ?8
         WHERE id = ?1",
        params![
            user.id,
            user.password_hash,
            user.permissions.to_blob(),
            user.must_change_password,
            user.password_expires_at.map(Utc),
            user.auth_epoch,
            user.version,
            user.last_login_at.map(Utc),
        ],
    )?;
    Ok(())
}

/// Deletes the account and, through the foreign key, its sessions.
pub fn delete(conn: &Connection, id: i64) -> Result<bool, StoreError> {
    Ok(conn.execute("DELETE FROM users WHERE id = ?1", [id])? == 1)
}

fn read(row: &Row<'_>) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: row.get(0)?,
        username: row.get(1)?,
        password_hash: row.get(2)?,
        is_owner: row.get(3)?,
        permissions: Permissions::from_blob(&row.get::<_, Vec<u8>>(4)?),
        must_change_password: row.get(5)?,
        password_expires_at: row.get::<_, Option<Utc>>(6)?.map(|at| at.0),
        auth_epoch: row.get(7)?,
        version: row.get(8)?,
        created_at: row.get::<_, Utc>(9)?.0,
        last_login_at: row.get::<_, Option<Utc>>(10)?.map(|at| at.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;
    use crate::sessions::{self, NewSession};

    fn new_user(username: &str, permissions: Permissions) -> NewUser<'_> {
        NewUser {
            username,
            password_hash: "$argon2id$v=19$m=8,t=1,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNo",
            is_owner: false,
            permissions,
            must_change_password: true,
            password_expires_at: Some("2026-01-13T08:00:00Z".parse().unwrap()),
            created_at: "2026-01-10T08:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn permissions_round_trip_through_the_blob_column() {
        let conn = memory();
        let set = Permissions::USERS_MANAGE | Permissions::LOGS_DOWNLOAD | Permissions::DASHBOARD_VIEW;

        let id = insert(&conn, &new_user("ada", set)).unwrap().unwrap();

        let blob: Vec<u8> = conn
            .query_row("SELECT permissions FROM users WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(blob, [0x08, 0x81]);
        assert_eq!(get(&conn, id).unwrap().unwrap().permissions, set);
    }

    #[test]
    fn unknown_bits_written_by_someone_else_are_dropped_on_read() {
        let conn = memory();
        let id = insert(&conn, &new_user("ada", Permissions::NONE))
            .unwrap()
            .unwrap();
        conn.execute(
            "UPDATE users SET permissions = x'800000000004' WHERE id = ?1",
            [id],
        )
        .unwrap();

        assert_eq!(
            get(&conn, id).unwrap().unwrap().permissions,
            Permissions::CONSOLE_WRITE | Permissions::CONSOLE_VIEW
        );
    }

    #[test]
    fn usernames_are_unique_whatever_their_case() {
        let conn = memory();
        assert!(none(&conn).unwrap());
        insert(&conn, &new_user("Ada", Permissions::NONE))
            .unwrap()
            .unwrap();

        assert_eq!(insert(&conn, &new_user("ADA", Permissions::NONE)).unwrap(), None);
        assert_eq!(find(&conn, "aDa").unwrap().unwrap().username, "Ada");
        assert!(!none(&conn).unwrap());
    }

    #[test]
    fn there_is_never_a_second_owner() {
        let conn = memory();
        let owner = NewUser {
            is_owner: true,
            ..new_user("ada", Permissions::NONE)
        };
        insert(&conn, &owner).unwrap().unwrap();

        let second = NewUser {
            username: "grace",
            ..owner
        };
        assert!(insert(&conn, &second).is_err());
    }

    #[test]
    fn deleting_a_user_deletes_their_sessions() {
        let conn = memory();
        let id = insert(&conn, &new_user("ada", Permissions::NONE))
            .unwrap()
            .unwrap();
        sessions::insert(
            &conn,
            &NewSession {
                user_id: id,
                token_hash: &[1; 32],
                key_hash: &[2; 32],
                auth_epoch: 0,
                created_at: "2026-01-10T08:00:00Z".parse().unwrap(),
                expires_at: "2026-01-17T08:00:00Z".parse().unwrap(),
                ip: Some("203.0.113.7"),
                user_agent: None,
            },
        )
        .unwrap();

        assert!(delete(&conn, id).unwrap());
        assert_eq!(sessions::find_by_token(&conn, &[1; 32]).unwrap(), None);
    }
}
