//! The resources of a snapshot, one row per folder, each naming the tree of its entries once every
//! entry is hashed.

use observer_domain::changes::{GitHead, Manifest, Package};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{deflate, inflate, json, sql, unsigned};
use crate::database::StoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRow {
    /// `resources/[qb]/qb-core`.
    pub path: String,
    pub name: String,
    pub shown_name: String,
    pub duplicate: bool,
    /// `None` until every entry is hashed.
    pub tree: Option<String>,
    /// While some entries wait to be hashed: the entries known so far as JSON, those waiting with
    /// an empty key.
    pub pending_listing: Option<Vec<u8>>,
    pub pending: u64,
    pub manifest: Option<Manifest>,
    pub escrowed: bool,
    pub git_head: Option<GitHead>,
    pub packages: Vec<Package>,
    pub files: u64,
    pub bytes: u64,
}

impl ResourceRow {
    pub fn version(&self) -> Option<&str> {
        self.manifest
            .as_ref()
            .and_then(|manifest| manifest.version.as_deref())
    }

    /// Whether two rows say the same of a resource, which lets a snapshot share another's rows.
    pub fn same_as(&self, other: &Self) -> bool {
        self.tree.is_some() && self == other
    }
}

pub fn insert(conn: &Connection, snapshot: i64, rows: &[ResourceRow]) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "INSERT INTO snapshot_resource (snapshot_id, path, name, shown_name, duplicate, tree,
             pending_listing, pending, version, manifest, escrowed, git_head, packages, files, bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?;
    for row in rows {
        statement.execute(params![
            snapshot,
            row.path,
            row.name,
            row.shown_name,
            row.duplicate,
            row.tree,
            row.pending_listing.as_deref().map(deflate),
            sql(row.pending),
            row.version(),
            row.manifest.as_ref().map(to_json),
            row.escrowed,
            row.git_head.as_ref().map(to_json),
            (!row.packages.is_empty()).then(|| to_json(&row.packages)),
            sql(row.files),
            sql(row.bytes),
        ])?;
    }
    Ok(())
}

/// Every entry of the resource at `path` is now hashed.
pub fn set_tree(conn: &Connection, snapshot: i64, path: &str, tree: &str) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot_resource SET tree = ?3, pending_listing = NULL, pending = 0
         WHERE snapshot_id = ?1 AND path = ?2",
        params![snapshot, path, tree],
    )?;
    Ok(())
}

/// The entries known so far of a resource still waiting for some.
pub fn set_pending(
    conn: &Connection,
    snapshot: i64,
    path: &str,
    listing: &[u8],
    pending: u64,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot_resource SET pending_listing = ?3, pending = ?4 WHERE snapshot_id = ?1 AND path = ?2",
        params![snapshot, path, deflate(listing), sql(pending)],
    )?;
    Ok(())
}

/// The rows of the snapshot `holder`, in path order: the snapshot itself, or the one it shares
/// its rows with.
/// The path and name of each resource of the snapshot `holder`, without the rest of its row.
pub fn names(conn: &Connection, holder: i64) -> Result<Vec<(String, String)>, StoreError> {
    let mut statement =
        conn.prepare("SELECT path, name FROM snapshot_resource WHERE snapshot_id = ?1 ORDER BY path")?;
    let rows = statement.query_map([holder], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn rows(conn: &Connection, holder: i64) -> Result<Vec<ResourceRow>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT path, name, shown_name, duplicate, tree, pending_listing, pending, manifest, escrowed,
                git_head, packages, files, bytes
         FROM snapshot_resource WHERE snapshot_id = ?1 ORDER BY path",
    )?;
    let rows = statement.query_map([holder], read)?;
    rows.map(|row| row.map_err(StoreError::from).and_then(inflated))
        .collect()
}

pub fn row(conn: &Connection, holder: i64, path: &str) -> Result<Option<ResourceRow>, StoreError> {
    conn.query_row(
        "SELECT path, name, shown_name, duplicate, tree, pending_listing, pending, manifest, escrowed,
                git_head, packages, files, bytes
         FROM snapshot_resource WHERE snapshot_id = ?1 AND path = ?2",
        params![holder, path],
        read,
    )
    .optional()?
    .map(inflated)
    .transpose()
}

fn inflated(mut row: ResourceRow) -> Result<ResourceRow, StoreError> {
    row.pending_listing = row.pending_listing.as_deref().map(inflate).transpose()?;
    Ok(row)
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn read(row: &Row<'_>) -> rusqlite::Result<ResourceRow> {
    Ok(ResourceRow {
        path: row.get(0)?,
        name: row.get(1)?,
        shown_name: row.get(2)?,
        duplicate: row.get(3)?,
        tree: row.get(4)?,
        pending_listing: row.get(5)?,
        pending: unsigned(row.get(6)?),
        manifest: json(7, row.get(7)?)?,
        escrowed: row.get(8)?,
        git_head: json(9, row.get(9)?)?,
        packages: json(10, row.get(10)?)?.unwrap_or_default(),
        files: unsigned(row.get(11)?),
        bytes: unsigned(row.get(12)?),
    })
}

#[cfg(test)]
pub(crate) fn sample(path: &str, tree: Option<&str>) -> ResourceRow {
    let name = path.rsplit('/').next().unwrap_or(path).to_owned();
    ResourceRow {
        path: path.to_owned(),
        name: name.to_ascii_lowercase(),
        shown_name: name,
        duplicate: false,
        tree: tree.map(str::to_owned),
        pending_listing: None,
        pending: 0,
        manifest: Some(Manifest {
            file: "fxmanifest.lua".to_owned(),
            version: Some("1.0.0".to_owned()),
            ..Manifest::default()
        }),
        escrowed: false,
        git_head: None,
        packages: Vec::new(),
        files: 2,
        bytes: 40,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::{memory, snapshots, trees};

    #[test]
    fn rows_read_back_with_their_pending_listing() {
        let conn = memory();
        let id = snapshots::insert(&conn, &snapshots::sample("default", 0)).unwrap();
        trees::insert(&conn, "t1", b"[]", &[], "k").unwrap();
        let checking = ResourceRow {
            pending_listing: Some(br#"[{"path":"a","size":1,"key":""}]"#.to_vec()),
            pending: 1,
            packages: vec![Package {
                name: "axios".to_owned(),
                version: Some("1.7.2".to_owned()),
            }],
            ..sample("resources/b", None)
        };
        insert(&conn, id, &[sample("resources/a", Some("t1")), checking.clone()]).unwrap();

        let rows = rows(&conn, id).unwrap();

        assert_eq!(rows, [sample("resources/a", Some("t1")), checking]);
        assert_eq!(rows[0].version(), Some("1.0.0"));
    }

    #[test]
    fn a_tree_ends_the_wait() {
        let conn = memory();
        let id = snapshots::insert(&conn, &snapshots::sample("default", 0)).unwrap();
        trees::insert(&conn, "t1", b"[]", &[], "k").unwrap();
        insert(&conn, id, &[sample("resources/a", None)]).unwrap();
        set_pending(&conn, id, "resources/a", b"[]", 3).unwrap();

        set_tree(&conn, id, "resources/a", "t1").unwrap();

        let row = row(&conn, id, "resources/a").unwrap().unwrap();
        assert_eq!(
            (row.tree.as_deref(), row.pending, row.pending_listing),
            (Some("t1"), 0, None)
        );
    }
}
