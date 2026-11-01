//! Text versions: a file's masked text, compressed, stored once under the keyed digest of its
//! bytes on disk, however many files, resources and starts share it.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use super::{deflate, inflate, sql, unsigned};
use crate::database::StoreError;

/// A masked text as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewText {
    pub digest: String,
    pub text: String,
    /// The bytes of the file on disk.
    pub bytes: u64,
    pub masking: u32,
    /// Where each redaction lies and its tag, six bytes each.
    pub redactions: Vec<u8>,
}

/// A stored text version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// `None` once the budget or a purge dropped it.
    pub text: Option<String>,
    pub bytes: u64,
    pub lines: u64,
    pub masking: u32,
    pub redactions: Vec<u8>,
}

/// Stores the text, or fills a body the budget dropped, and records that `snapshot` lists it.
pub fn store(conn: &Connection, text: &NewText, snapshot: i64) -> Result<(), StoreError> {
    write(conn, text, snapshot, false)
}

/// Stores the text over the body kept for its digest: a file of the cfg exec chain, masked with
/// the cfg's rules, replaces what the same bytes gave as an ordinary file.
pub fn store_over(conn: &Connection, text: &NewText, snapshot: i64) -> Result<(), StoreError> {
    write(conn, text, snapshot, true)
}

fn write(conn: &Connection, text: &NewText, snapshot: i64, over: bool) -> Result<(), StoreError> {
    let body = deflate(text.text.as_bytes());
    let lines = text.text.lines().count();
    conn.execute(
        "INSERT INTO text_version (digest, body, stored, bytes, lines, masking, redactions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (digest) DO UPDATE SET
             body = CASE WHEN body IS NULL OR ?8 THEN excluded.body ELSE body END,
             stored = CASE WHEN body IS NULL OR ?8 THEN excluded.stored ELSE stored END,
             masking = CASE WHEN body IS NULL OR ?8 THEN excluded.masking ELSE masking END,
             redactions = CASE WHEN body IS NULL OR ?8 THEN excluded.redactions ELSE redactions END",
        params![
            text.digest,
            body,
            sql(body.len() as u64),
            sql(text.bytes),
            sql(lines as u64),
            text.masking,
            text.redactions,
            over
        ],
    )?;
    conn.execute(
        "INSERT INTO text_seen (digest, first_seen, last_seen) VALUES (?1, ?2, ?2)
         ON CONFLICT (digest) DO UPDATE SET
             first_seen = min(first_seen, excluded.first_seen),
             last_seen = max(last_seen, excluded.last_seen)",
        params![text.digest, snapshot],
    )?;
    Ok(())
}

/// Records that `snapshot` lists texts already stored, which keeps them last in the budget's order.
pub fn seen(conn: &Connection, digests: &[String], snapshot: i64) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "UPDATE text_seen SET last_seen = max(last_seen, ?2), first_seen = min(first_seen, ?2)
         WHERE digest = ?1",
    )?;
    for digest in digests {
        statement.execute(params![digest, snapshot])?;
    }
    Ok(())
}

pub fn get(conn: &Connection, digest: &str) -> Result<Option<Text>, StoreError> {
    let row = conn
        .query_row(
            "SELECT body, bytes, lines, masking, redactions FROM text_version WHERE digest = ?1",
            [digest],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    unsigned(row.get(1)?),
                    unsigned(row.get(2)?),
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((body, bytes, lines, masking, redactions)) = row else {
        return Ok(None);
    };
    let text = body
        .map(|body| {
            inflate(&body).and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| StoreError::Corrupt("a stored text is not UTF-8".to_owned()))
            })
        })
        .transpose()?;
    Ok(Some(Text {
        text,
        bytes,
        lines,
        masking,
        redactions,
    }))
}

/// The digests whose body is stored: a text the listing finds unchanged but without a body is
/// read and stored again.
pub fn with_body(conn: &Connection) -> Result<HashSet<String>, StoreError> {
    let mut statement = conn.prepare("SELECT digest FROM text_version WHERE body IS NOT NULL")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Texts masked with rules older than `version`, a batch at a time, for the upkeep pass.
pub fn masked_before(conn: &Connection, version: u32, limit: u32) -> Result<Vec<(String, Text)>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT digest FROM text_version WHERE body IS NOT NULL AND masking < ?1 ORDER BY digest LIMIT ?2",
    )?;
    let digests: Vec<String> = statement
        .query_map(params![version, limit], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    digests
        .into_iter()
        .filter_map(|digest| match get(conn, &digest) {
            Ok(Some(text)) => Some(Ok((digest, text))),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

/// Replaces a body with the same text masked again with newer rules.
pub fn remasked(
    conn: &Connection,
    digest: &str,
    text: &str,
    masking: u32,
    redactions: &[u8],
) -> Result<(), StoreError> {
    let body = deflate(text.as_bytes());
    conn.execute(
        "UPDATE text_version SET body = ?2, stored = ?3, masking = ?4, redactions = ?5
         WHERE digest = ?1 AND body IS NOT NULL",
        params![digest, body, sql(body.len() as u64), masking, redactions],
    )?;
    Ok(())
}

/// A path whose text had the digest `digest`, which tells the masking rules what kind of file it is.
pub fn path_of(conn: &Connection, digest: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT path FROM tree_text WHERE digest = ?1
             UNION ALL SELECT path FROM snapshot_text WHERE digest = ?1 LIMIT 1",
            [digest],
            |row| row.get(0),
        )
        .optional()?)
}

/// The paths whose text a snapshot holds outside its trees: the files of the cfg exec chain.
pub fn attach(conn: &Connection, snapshot: i64, path: &str, digest: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR REPLACE INTO snapshot_text (snapshot_id, path, digest) VALUES (?1, ?2, ?3)",
        params![snapshot, path, digest],
    )?;
    Ok(())
}

pub fn attached(conn: &Connection, snapshot: i64, path: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT digest FROM snapshot_text WHERE snapshot_id = ?1 AND path = ?2",
            params![snapshot, path],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
pub(crate) fn sample(digest: &str, text: &str) -> NewText {
    NewText {
        digest: digest.to_owned(),
        text: text.to_owned(),
        bytes: text.len() as u64,
        masking: 1,
        redactions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::memory;

    #[test]
    fn a_text_is_stored_once_and_reads_back() {
        let conn = memory();
        store(&conn, &sample("d1", "a\nb\n"), 1).unwrap();
        store(&conn, &sample("d1", "a\nb\n"), 3).unwrap();

        let text = get(&conn, "d1").unwrap().unwrap();

        assert_eq!(text.text.as_deref(), Some("a\nb\n"));
        assert_eq!(text.lines, 2);
        let seen: (i64, i64) = conn
            .query_row("SELECT first_seen, last_seen FROM text_seen", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(seen, (1, 3));
    }

    #[test]
    fn storing_a_text_whose_body_was_dropped_fills_it_again() {
        let conn = memory();
        store(&conn, &sample("d1", "a"), 1).unwrap();
        conn.execute("UPDATE text_version SET body = NULL, stored = 0", [])
            .unwrap();
        assert_eq!(get(&conn, "d1").unwrap().unwrap().text, None);
        assert!(!with_body(&conn).unwrap().contains("d1"));

        store(&conn, &sample("d1", "a"), 2).unwrap();

        assert_eq!(get(&conn, "d1").unwrap().unwrap().text.as_deref(), Some("a"));
        assert!(with_body(&conn).unwrap().contains("d1"));
    }

    #[test]
    fn texts_masked_with_older_rules_are_found_and_replaced() {
        let conn = memory();
        store(&conn, &sample("d1", "token = 'x'"), 1).unwrap();

        let older = masked_before(&conn, 2, 10).unwrap();
        remasked(&conn, "d1", "token = '[REDACTED]'", 2, &[0, 0, 0, 9, 1, 2]).unwrap();

        assert_eq!(older.len(), 1);
        let text = get(&conn, "d1").unwrap().unwrap();
        assert_eq!(
            (text.text.as_deref(), text.masking),
            (Some("token = '[REDACTED]'"), 2)
        );
        assert_eq!(masked_before(&conn, 2, 10).unwrap(), Vec::new());
    }
}
