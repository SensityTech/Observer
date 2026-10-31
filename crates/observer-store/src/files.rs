use jiff::Timestamp;
use observer_domain::files::Rule;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// A server's named set of rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGroupRow {
    pub id: i64,
    pub server_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<Rule>,
    pub version: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// An account's access, or a pipeline's when `service_name` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAccessRow {
    pub id: i64,
    pub user_id: Option<i64>,
    pub service_name: Option<String>,
    pub enabled: bool,
    pub expires_at: Option<Timestamp>,
    /// CIDR blocks, as written; empty means anywhere.
    pub networks: Vec<String>,
    pub daily_budget: Option<u64>,
    pub max_rate: Option<u64>,
    pub version: i64,
    pub created_at: Timestamp,
    pub last_login_at: Option<Timestamp>,
    pub last_login_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileKeyRow {
    pub id: i64,
    pub access_id: i64,
    pub algorithm: String,
    /// The key in SSH wire encoding.
    pub public_key: Vec<u8>,
    /// `SHA256:` and the unpadded base64 digest, as OpenSSH prints it.
    pub fingerprint: String,
    pub comment: String,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub last_used_at: Option<Timestamp>,
    pub last_used_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyRow {
    pub algorithm: String,
    /// The OpenSSH private key format.
    pub private_key: Vec<u8>,
    pub fingerprint: String,
    pub created_at: Timestamp,
}

/// A temporary upload file that may still exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRow {
    pub id: i64,
    pub server_id: String,
    pub root: String,
    pub folder: String,
    pub name: String,
    pub created_at: Timestamp,
}

const GROUP_COLUMNS: &str = "id, server_id, name, description, rules, version, created_at, updated_at";
const ACCESS_COLUMNS: &str = "id, user_id, service_name, enabled, expires_at, networks, daily_budget, max_rate, \
    version, created_at, last_login_at, last_login_ip";
const KEY_COLUMNS: &str = "id, access_id, algorithm, public_key, fingerprint, comment, created_at, expires_at, \
    last_used_at, last_used_ip";

pub fn groups(conn: &Connection, server_id: &str) -> Result<Vec<FileGroupRow>, StoreError> {
    let sql = format!("SELECT {GROUP_COLUMNS} FROM file_groups WHERE server_id = ?1 ORDER BY name");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([server_id], read_group)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn group(conn: &Connection, id: i64) -> Result<Option<FileGroupRow>, StoreError> {
    let sql = format!("SELECT {GROUP_COLUMNS} FROM file_groups WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], read_group).optional()?)
}

/// The new group's id, or `None` when the server already has a group of that name.
pub fn insert_group(
    conn: &Connection,
    server_id: &str,
    name: &str,
    description: &str,
    rules: &[Rule],
    now: Timestamp,
) -> Result<Option<i64>, StoreError> {
    let inserted = conn.execute(
        "INSERT INTO file_groups (server_id, name, description, rules, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT (server_id, name) DO NOTHING",
        params![server_id, name, description, rules_json(rules)?, Utc(now)],
    )?;
    Ok((inserted == 1).then(|| conn.last_insert_rowid()))
}

/// Saves a group under the version it was read at; `false` when someone saved it since.
pub fn update_group(
    conn: &Connection,
    id: i64,
    version: i64,
    name: &str,
    description: &str,
    rules: &[Rule],
    now: Timestamp,
) -> Result<bool, StoreError> {
    let updated = conn.execute(
        "UPDATE file_groups SET name = ?3, description = ?4, rules = ?5, version = version + 1, updated_at = ?6
         WHERE id = ?1 AND version = ?2",
        params![id, version, name, description, rules_json(rules)?, Utc(now)],
    )?;
    Ok(updated == 1)
}

pub fn delete_group(conn: &Connection, id: i64) -> Result<bool, StoreError> {
    Ok(conn.execute("DELETE FROM file_groups WHERE id = ?1", [id])? == 1)
}

pub fn accesses(conn: &Connection) -> Result<Vec<FileAccessRow>, StoreError> {
    let sql = format!("SELECT {ACCESS_COLUMNS} FROM file_accesses ORDER BY service_name IS NOT NULL, id");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], read_access)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn access(conn: &Connection, id: i64) -> Result<Option<FileAccessRow>, StoreError> {
    let sql = format!("SELECT {ACCESS_COLUMNS} FROM file_accesses WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], read_access).optional()?)
}

pub fn user_access(conn: &Connection, user_id: i64) -> Result<Option<FileAccessRow>, StoreError> {
    let sql = format!("SELECT {ACCESS_COLUMNS} FROM file_accesses WHERE user_id = ?1");
    Ok(conn.query_row(&sql, [user_id], read_access).optional()?)
}

/// A pipeline's access by its name, whatever the case typed.
pub fn service_access(conn: &Connection, name: &str) -> Result<Option<FileAccessRow>, StoreError> {
    let sql = format!("SELECT {ACCESS_COLUMNS} FROM file_accesses WHERE service_name = ?1");
    Ok(conn.query_row(&sql, [name], read_access).optional()?)
}

/// The account's access, created empty the first time: it grants nothing until a group is added.
pub fn user_access_or_new(
    conn: &Connection,
    user_id: i64,
    now: Timestamp,
) -> Result<FileAccessRow, StoreError> {
    conn.execute(
        "INSERT INTO file_accesses (user_id, created_at) VALUES (?1, ?2) ON CONFLICT (user_id) DO NOTHING",
        params![user_id, Utc(now)],
    )?;
    user_access(conn, user_id)?.ok_or(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
}

/// The new pipeline's id, or `None` when the name is taken.
pub fn insert_service(conn: &Connection, access: &FileAccessRow) -> Result<Option<i64>, StoreError> {
    let inserted = conn.execute(
        "INSERT INTO file_accesses (service_name, enabled, expires_at, networks, daily_budget, max_rate, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (service_name) DO NOTHING",
        params![
            access.service_name,
            access.enabled,
            access.expires_at.map(Utc),
            networks_json(&access.networks)?,
            access.daily_budget.map(sql_size),
            access.max_rate.map(sql_size),
            Utc(access.created_at),
        ],
    )?;
    Ok((inserted == 1).then(|| conn.last_insert_rowid()))
}

/// Saves an access's settings under the version it was read at; `false` when someone saved it
/// since.
pub fn update_access(conn: &Connection, access: &FileAccessRow) -> Result<bool, StoreError> {
    let updated = conn.execute(
        "UPDATE file_accesses
         SET enabled = ?3, expires_at = ?4, networks = ?5, daily_budget = ?6, max_rate = ?7, version = version + 1
         WHERE id = ?1 AND version = ?2",
        params![
            access.id,
            access.version,
            access.enabled,
            access.expires_at.map(Utc),
            networks_json(&access.networks)?,
            access.daily_budget.map(sql_size),
            access.max_rate.map(sql_size),
        ],
    )?;
    Ok(updated == 1)
}

pub fn delete_access(conn: &Connection, id: i64) -> Result<bool, StoreError> {
    Ok(conn.execute("DELETE FROM file_accesses WHERE id = ?1", [id])? == 1)
}

pub fn record_login(
    conn: &Connection,
    access_id: i64,
    key_id: i64,
    ip: &str,
    now: Timestamp,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE file_accesses SET last_login_at = ?2, last_login_ip = ?3 WHERE id = ?1",
        params![access_id, Utc(now), ip],
    )?;
    conn.execute(
        "UPDATE file_keys SET last_used_at = ?2, last_used_ip = ?3 WHERE id = ?1",
        params![key_id, Utc(now), ip],
    )?;
    Ok(())
}

/// The groups an access holds, by name.
pub fn access_groups(conn: &Connection, access_id: i64) -> Result<Vec<FileGroupRow>, StoreError> {
    let sql = format!(
        "SELECT {GROUP_COLUMNS} FROM file_groups
         WHERE id IN (SELECT group_id FROM file_access_groups WHERE access_id = ?1) ORDER BY name"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([access_id], read_group)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The accesses that hold a group.
pub fn group_accesses(conn: &Connection, group_id: i64) -> Result<Vec<i64>, StoreError> {
    let mut statement =
        conn.prepare("SELECT access_id FROM file_access_groups WHERE group_id = ?1 ORDER BY access_id")?;
    let rows = statement.query_map([group_id], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Replaces the groups an access holds.
pub fn set_access_groups(conn: &Connection, access_id: i64, group_ids: &[i64]) -> Result<(), StoreError> {
    conn.execute("DELETE FROM file_access_groups WHERE access_id = ?1", [access_id])?;
    let mut statement =
        conn.prepare("INSERT INTO file_access_groups (access_id, group_id) VALUES (?1, ?2)")?;
    for group_id in group_ids {
        statement.execute([access_id, *group_id])?;
    }
    Ok(())
}

pub fn keys(conn: &Connection, access_id: i64) -> Result<Vec<FileKeyRow>, StoreError> {
    let sql = format!("SELECT {KEY_COLUMNS} FROM file_keys WHERE access_id = ?1 ORDER BY id");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([access_id], read_key)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The access's key with that fingerprint.
pub fn key(conn: &Connection, access_id: i64, fingerprint: &str) -> Result<Option<FileKeyRow>, StoreError> {
    let sql = format!("SELECT {KEY_COLUMNS} FROM file_keys WHERE access_id = ?1 AND fingerprint = ?2");
    Ok(conn
        .query_row(&sql, params![access_id, fingerprint], read_key)
        .optional()?)
}

/// How many accesses hold a key with this fingerprint.
pub fn key_holders(conn: &Connection, fingerprint: &str) -> Result<i64, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM file_keys WHERE fingerprint = ?1",
        [fingerprint],
        |row| row.get(0),
    )?)
}

/// The new key's id, or `None` when the access already holds it.
pub fn insert_key(conn: &Connection, key: &FileKeyRow) -> Result<Option<i64>, StoreError> {
    let inserted = conn.execute(
        "INSERT INTO file_keys (access_id, algorithm, public_key, fingerprint, comment, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (access_id, fingerprint) DO NOTHING",
        params![
            key.access_id,
            key.algorithm,
            key.public_key,
            key.fingerprint,
            key.comment,
            Utc(key.created_at),
            key.expires_at.map(Utc),
        ],
    )?;
    Ok((inserted == 1).then(|| conn.last_insert_rowid()))
}

/// Moves a key's end, as a rotation does to the key it replaces.
pub fn set_key_expiry(conn: &Connection, id: i64, expires_at: Timestamp) -> Result<bool, StoreError> {
    Ok(conn.execute(
        "UPDATE file_keys SET expires_at = ?2 WHERE id = ?1",
        params![id, Utc(expires_at)],
    )? == 1)
}

pub fn delete_key(conn: &Connection, access_id: i64, id: i64) -> Result<Option<FileKeyRow>, StoreError> {
    let sql = format!("DELETE FROM file_keys WHERE access_id = ?1 AND id = ?2 RETURNING {KEY_COLUMNS}");
    Ok(conn.query_row(&sql, [access_id, id], read_key).optional()?)
}

pub fn delete_keys(conn: &Connection, access_id: i64) -> Result<Vec<FileKeyRow>, StoreError> {
    let sql = format!("DELETE FROM file_keys WHERE access_id = ?1 RETURNING {KEY_COLUMNS}");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([access_id], read_key)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn host_key(conn: &Connection) -> Result<Option<HostKeyRow>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT algorithm, private_key, fingerprint, created_at FROM file_host_key WHERE id = 1",
            [],
            |row| {
                Ok(HostKeyRow {
                    algorithm: row.get(0)?,
                    private_key: row.get(1)?,
                    fingerprint: row.get(2)?,
                    created_at: row.get::<_, Utc>(3)?.0,
                })
            },
        )
        .optional()?)
}

/// Stores the host key unless one exists; `false` when another start stored one first.
pub fn insert_host_key(conn: &Connection, key: &HostKeyRow) -> Result<bool, StoreError> {
    let inserted = conn.execute(
        "INSERT INTO file_host_key (id, algorithm, private_key, fingerprint, created_at)
         VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT (id) DO NOTHING",
        params![
            key.algorithm,
            key.private_key,
            key.fingerprint,
            Utc(key.created_at)
        ],
    )?;
    Ok(inserted == 1)
}

/// Records a temporary file before it is created.
pub fn record_upload(
    conn: &Connection,
    server_id: &str,
    root: &str,
    folder: &str,
    name: &str,
    now: Timestamp,
) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO file_uploads (server_id, root, folder, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![server_id, root, folder, name, Utc(now)],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn forget_upload(conn: &Connection, id: i64) -> Result<(), StoreError> {
    conn.execute("DELETE FROM file_uploads WHERE id = ?1", [id])?;
    Ok(())
}

pub fn uploads(conn: &Connection) -> Result<Vec<UploadRow>, StoreError> {
    let mut statement =
        conn.prepare("SELECT id, server_id, root, folder, name, created_at FROM file_uploads ORDER BY id")?;
    let rows = statement.query_map([], |row| {
        Ok(UploadRow {
            id: row.get(0)?,
            server_id: row.get(1)?,
            root: row.get(2)?,
            folder: row.get(3)?,
            name: row.get(4)?,
            created_at: row.get::<_, Utc>(5)?.0,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Adds uploaded bytes to the access's day and returns the day's total.
pub fn add_to_budget(conn: &Connection, access_id: i64, day: &str, bytes: u64) -> Result<u64, StoreError> {
    Ok(conn.query_row(
        "INSERT INTO file_budget (access_id, day, bytes) VALUES (?1, ?2, ?3)
         ON CONFLICT (access_id, day) DO UPDATE SET bytes = bytes + excluded.bytes
         RETURNING bytes",
        params![access_id, day, sql_size(bytes)],
        |row| row.get::<_, i64>(0).map(size),
    )?)
}

pub fn budget_used(conn: &Connection, access_id: i64, day: &str) -> Result<u64, StoreError> {
    Ok(conn
        .query_row(
            "SELECT bytes FROM file_budget WHERE access_id = ?1 AND day = ?2",
            params![access_id, day],
            |row| row.get::<_, i64>(0).map(size),
        )
        .optional()?
        .unwrap_or(0))
}

pub fn trim_budget(conn: &Connection, before_day: &str) -> Result<usize, StoreError> {
    Ok(conn.execute("DELETE FROM file_budget WHERE day < ?1", [before_day])?)
}

fn json(value: &serde_json::Value) -> Result<String, StoreError> {
    serde_json::to_string(value)
        .map_err(|error| StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(error))))
}

fn rules_json(rules: &[Rule]) -> Result<String, StoreError> {
    let value = serde_json::to_value(rules)
        .map_err(|error| StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(error))))?;
    json(&value)
}

fn networks_json(networks: &[String]) -> Result<String, StoreError> {
    json(&serde_json::Value::from(networks.to_vec()))
}

/// Sizes are stored as SQLite's signed integers; no size comes near 2^63.
fn sql_size(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

fn size(stored: i64) -> u64 {
    u64::try_from(stored).unwrap_or(0)
}

fn rules_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Vec<Rule>> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error)))
}

fn networks_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Vec<String>> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error)))
}

fn read_group(row: &Row<'_>) -> rusqlite::Result<FileGroupRow> {
    Ok(FileGroupRow {
        id: row.get(0)?,
        server_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        rules: rules_column(row, 4)?,
        version: row.get(5)?,
        created_at: row.get::<_, Utc>(6)?.0,
        updated_at: row.get::<_, Utc>(7)?.0,
    })
}

fn read_access(row: &Row<'_>) -> rusqlite::Result<FileAccessRow> {
    Ok(FileAccessRow {
        id: row.get(0)?,
        user_id: row.get(1)?,
        service_name: row.get(2)?,
        enabled: row.get(3)?,
        expires_at: row.get::<_, Option<Utc>>(4)?.map(|at| at.0),
        networks: networks_column(row, 5)?,
        daily_budget: row.get::<_, Option<i64>>(6)?.map(size),
        max_rate: row.get::<_, Option<i64>>(7)?.map(size),
        version: row.get(8)?,
        created_at: row.get::<_, Utc>(9)?.0,
        last_login_at: row.get::<_, Option<Utc>>(10)?.map(|at| at.0),
        last_login_ip: row.get(11)?,
    })
}

fn read_key(row: &Row<'_>) -> rusqlite::Result<FileKeyRow> {
    Ok(FileKeyRow {
        id: row.get(0)?,
        access_id: row.get(1)?,
        algorithm: row.get(2)?,
        public_key: row.get(3)?,
        fingerprint: row.get(4)?,
        comment: row.get(5)?,
        created_at: row.get::<_, Utc>(6)?.0,
        expires_at: row.get::<_, Option<Utc>>(7)?.map(|at| at.0),
        last_used_at: row.get::<_, Option<Utc>>(8)?.map(|at| at.0),
        last_used_ip: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use observer_domain::files::{Depth, Effect, Rights};

    use super::*;
    use crate::migrations;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrations::apply(&mut conn, std::path::Path::new("unused")).unwrap();
        conn
    }

    fn now() -> Timestamp {
        "2026-10-31T09:00:00Z".parse().unwrap()
    }

    #[test]
    fn a_group_round_trips_its_rules_and_saves_only_at_its_version() {
        let conn = database();
        let rules = vec![Rule {
            effect: Effect::Allow,
            folder: "resources/[maps]".to_owned(),
            depth: Depth::Tree,
            names: vec!["*.ytd".to_owned()],
            rights: Rights::EDIT,
            max_file_size: Some(10_000_000),
        }];
        let id = insert_group(&conn, "default", "Mappers", "", &rules, now())
            .unwrap()
            .unwrap();
        assert_eq!(
            insert_group(&conn, "default", "MAPPERS", "", &rules, now()).unwrap(),
            None
        );

        let stored = group(&conn, id).unwrap().unwrap();
        assert_eq!(stored.rules, rules);
        assert!(update_group(&conn, id, stored.version, "Mappers", "streams", &[], now()).unwrap());
        assert!(!update_group(&conn, id, stored.version, "Mappers", "stale", &[], now()).unwrap());
        assert_eq!(group(&conn, id).unwrap().unwrap().description, "streams");
    }

    #[test]
    fn deleting_an_access_takes_its_keys_and_groups_with_it() {
        let conn = database();
        let group_id = insert_group(&conn, "default", "Mappers", "", &[], now())
            .unwrap()
            .unwrap();
        let service = FileAccessRow {
            id: 0,
            user_id: None,
            service_name: Some("deploy-main".to_owned()),
            enabled: true,
            expires_at: None,
            networks: vec!["192.0.2.0/24".to_owned()],
            daily_budget: None,
            max_rate: None,
            version: 1,
            created_at: now(),
            last_login_at: None,
            last_login_ip: None,
        };
        let id = insert_service(&conn, &service).unwrap().unwrap();
        assert_eq!(insert_service(&conn, &service).unwrap(), None);
        set_access_groups(&conn, id, &[group_id]).unwrap();
        let key = FileKeyRow {
            id: 0,
            access_id: id,
            algorithm: "ssh-ed25519".to_owned(),
            public_key: vec![1, 2, 3],
            fingerprint: "SHA256:abc".to_owned(),
            comment: "deploy".to_owned(),
            created_at: now(),
            expires_at: Some(now()),
            last_used_at: None,
            last_used_ip: None,
        };
        insert_key(&conn, &key).unwrap().unwrap();
        assert_eq!(insert_key(&conn, &key).unwrap(), None);
        assert_eq!(
            service_access(&conn, "DEPLOY-MAIN").unwrap().unwrap().networks,
            ["192.0.2.0/24"]
        );

        assert!(delete_access(&conn, id).unwrap());
        assert_eq!(key_holders(&conn, "SHA256:abc").unwrap(), 0);
        assert_eq!(group_accesses(&conn, group_id).unwrap(), Vec::<i64>::new());
    }

    #[test]
    fn the_budget_adds_up_per_day_and_the_host_key_is_written_once() {
        let conn = database();
        // No account 1 exists: the foreign key refuses its access.
        assert!(user_access_or_new(&conn, 1, now()).is_err());
        let pipeline = FileAccessRow {
            id: 0,
            user_id: None,
            service_name: Some("deploy".to_owned()),
            enabled: true,
            expires_at: None,
            networks: Vec::new(),
            daily_budget: Some(5_000_000_000),
            max_rate: None,
            version: 1,
            created_at: now(),
            last_login_at: None,
            last_login_ip: None,
        };
        let id = insert_service(&conn, &pipeline).unwrap().unwrap();
        assert_eq!(
            access(&conn, id).unwrap().unwrap().daily_budget,
            Some(5_000_000_000)
        );
        assert_eq!(add_to_budget(&conn, id, "2026-10-31", 300).unwrap(), 300);
        assert_eq!(add_to_budget(&conn, id, "2026-10-31", 200).unwrap(), 500);
        assert_eq!(add_to_budget(&conn, id, "2026-11-01", 10).unwrap(), 10);
        assert_eq!(trim_budget(&conn, "2026-11-01").unwrap(), 1);
        assert_eq!(budget_used(&conn, id, "2026-10-31").unwrap(), 0);
        assert_eq!(budget_used(&conn, id, "2026-11-01").unwrap(), 10);

        let host = HostKeyRow {
            algorithm: "ssh-ed25519".to_owned(),
            private_key: b"private".to_vec(),
            fingerprint: "SHA256:host".to_owned(),
            created_at: now(),
        };
        assert!(insert_host_key(&conn, &host).unwrap());
        assert!(
            !insert_host_key(
                &conn,
                &HostKeyRow {
                    fingerprint: "SHA256:other".to_owned(),
                    ..host.clone()
                }
            )
            .unwrap()
        );
        assert_eq!(host_key(&conn).unwrap().unwrap(), host);
    }
}
