//! Trees: a resource's sorted entries, stored once under their hash however many snapshots share
//! them, with the text entries apart so that SQL can tell which texts are in use.

use rusqlite::{Connection, OptionalExtension, params};

use super::{deflate, inflate};
use crate::database::StoreError;

/// Stores the tree `sha256` unless it is there already. `listing` is its entries as JSON, `texts`
/// the path and digest of each text entry.
pub fn insert(
    conn: &Connection,
    sha256: &str,
    listing: &[u8],
    texts: &[(String, String)],
) -> Result<bool, StoreError> {
    let added = conn.execute(
        "INSERT OR IGNORE INTO tree (sha256, listing) VALUES (?1, ?2)",
        params![sha256, deflate(listing)],
    )? > 0;
    if added {
        let mut statement = conn.prepare("INSERT INTO tree_text (tree, path, digest) VALUES (?1, ?2, ?3)")?;
        for (path, digest) in texts {
            statement.execute(params![sha256, path, digest])?;
        }
    }
    Ok(added)
}

/// The entries of the tree as JSON, or `None` when it is not stored.
pub fn listing(conn: &Connection, sha256: &str) -> Result<Option<Vec<u8>>, StoreError> {
    conn.query_row("SELECT listing FROM tree WHERE sha256 = ?1", [sha256], |row| {
        row.get::<_, Vec<u8>>(0)
    })
    .optional()?
    .map(|listing| inflate(&listing))
    .transpose()
}

/// The digest a tree lists for a text at `path`.
pub fn text_digest(conn: &Connection, sha256: &str, path: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT digest FROM tree_text WHERE tree = ?1 AND path = ?2",
            params![sha256, path],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::memory;

    #[test]
    fn a_tree_is_stored_once_with_its_texts() {
        let conn = memory();
        let texts = vec![("resources/a/client.lua".to_owned(), "d1".to_owned())];

        assert!(insert(&conn, "t1", b"[1]", &texts).unwrap());
        assert!(!insert(&conn, "t1", b"[2]", &texts).unwrap());

        assert_eq!(listing(&conn, "t1").unwrap().as_deref(), Some(&b"[1]"[..]));
        assert_eq!(listing(&conn, "t2").unwrap(), None);
        assert_eq!(
            text_digest(&conn, "t1", "resources/a/client.lua")
                .unwrap()
                .as_deref(),
            Some("d1")
        );
    }
}
