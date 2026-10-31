//! Keeping the history within its bounds: starts past retention, whose rows go to the starts that
//! share them; trees and texts nothing references any more; bodies past the budget; and the purge
//! of a path's texts.

use std::collections::HashSet;
use std::hash::BuildHasher;

use jiff::Timestamp;
use rusqlite::{Connection, params};

use super::unsigned;
use crate::database::StoreError;
use crate::time::Utc;

/// Deletes the starts of `server` taken before `before`. One that later starts share rows with
/// hands its rows to the oldest of them, and the others point there instead.
pub fn expire(conn: &Connection, server: &str, before: Timestamp) -> Result<usize, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id FROM snapshot WHERE server_id = ?1 AND kind = 'start' AND taken_at < ?2 ORDER BY id",
    )?;
    let expired: Vec<i64> = statement
        .query_map(params![server, Utc(before)], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for &id in &expired {
        let mut sharing = conn.prepare("SELECT id FROM snapshot WHERE same_as = ?1 ORDER BY id")?;
        let pointers: Vec<i64> = sharing
            .query_map([id], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        if let Some(&heir) = pointers.first() {
            conn.execute(
                "UPDATE snapshot_resource SET snapshot_id = ?2 WHERE snapshot_id = ?1",
                params![id, heir],
            )?;
            conn.execute("UPDATE snapshot SET same_as = NULL WHERE id = ?1", [heir])?;
            conn.execute(
                "UPDATE snapshot SET same_as = ?2 WHERE same_as = ?1",
                params![id, heir],
            )?;
        }
        conn.execute("DELETE FROM snapshot WHERE id = ?1", [id])?;
    }
    Ok(expired.len())
}

/// Deletes the checks of `server` but `keep`, the latest.
pub fn replace_checks(conn: &Connection, server: &str, keep: i64) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM snapshot WHERE server_id = ?1 AND kind = 'check' AND id <> ?2",
        params![server, keep],
    )?)
}

/// Deletes the trees no snapshot lists and the texts no tree or snapshot lists.
pub fn unreferenced(conn: &Connection) -> Result<(usize, usize), StoreError> {
    let trees = conn.execute(
        "DELETE FROM tree WHERE NOT EXISTS (SELECT 1 FROM snapshot_resource WHERE tree = tree.sha256)",
        [],
    )?;
    let texts = conn.execute(
        "DELETE FROM text_version
         WHERE NOT EXISTS (SELECT 1 FROM tree_text WHERE digest = text_version.digest)
           AND NOT EXISTS (SELECT 1 FROM snapshot_text WHERE digest = text_version.digest)",
        [],
    )?;
    Ok((trees, texts))
}

/// The compressed bytes of every body kept.
pub fn stored(conn: &Connection) -> Result<u64, StoreError> {
    let stored: i64 = conn.query_row(
        "SELECT coalesce(sum(stored), 0) FROM text_version WHERE body IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(unsigned(stored))
}

/// Drops bodies, least recently listed first, until what is kept fits `budget`, sparing every
/// text the snapshots `protected` list. Returns how many went and the bytes they held.
pub fn enforce_budget(conn: &Connection, budget: u64, protected: &[i64]) -> Result<(usize, u64), StoreError> {
    let mut kept = stored(conn)?;
    if kept <= budget {
        return Ok((0, 0));
    }
    let spared = protected_digests(conn, protected)?;
    let mut statement = conn.prepare(
        "SELECT digest, stored FROM text_version WHERE body IS NOT NULL ORDER BY last_seen, digest",
    )?;
    let candidates: Vec<(String, u64)> = statement
        .query_map([], |row| Ok((row.get(0)?, unsigned(row.get(1)?))))?
        .collect::<Result<_, _>>()?;
    let mut drop = conn.prepare("UPDATE text_version SET body = NULL, stored = 0 WHERE digest = ?1")?;
    let (mut dropped, mut freed) = (0, 0);
    for (digest, bytes) in candidates {
        if kept <= budget {
            break;
        }
        if spared.contains(&digest) {
            continue;
        }
        drop.execute([&digest])?;
        kept = kept.saturating_sub(bytes);
        dropped += 1;
        freed += bytes;
    }
    Ok((dropped, freed))
}

fn protected_digests(conn: &Connection, snapshots: &[i64]) -> Result<HashSet<String>, StoreError> {
    let mut digests = HashSet::new();
    let mut in_trees = conn.prepare(
        "SELECT tt.digest FROM tree_text tt
         JOIN snapshot_resource r ON r.tree = tt.tree
         JOIN snapshot s ON r.snapshot_id = coalesce(s.same_as, s.id)
         WHERE s.id = ?1",
    )?;
    let mut outside = conn.prepare("SELECT digest FROM snapshot_text WHERE snapshot_id = ?1")?;
    for id in snapshots {
        for digest in in_trees.query_map([id], |row| row.get(0))? {
            digests.insert(digest?);
        }
        for digest in outside.query_map([id], |row| row.get(0))? {
            digests.insert(digest?);
        }
    }
    Ok(digests)
}

/// Every digest a start or a check, or the index, ever recorded for `path`.
pub fn digests_of(conn: &Connection, path: &str) -> Result<HashSet<String>, StoreError> {
    let mut digests = HashSet::new();
    for sql in [
        "SELECT digest FROM tree_text WHERE path = ?1",
        "SELECT digest FROM snapshot_text WHERE path = ?1",
        "SELECT digest FROM file_index WHERE path = ?1 AND digest IS NOT NULL",
    ] {
        let mut statement = conn.prepare(sql)?;
        for digest in statement.query_map([path], |row| row.get(0))? {
            digests.insert(digest?);
        }
    }
    Ok(digests)
}

/// Clears the bodies of `digests`. Returns how many held one and the bytes they held.
pub fn clear_bodies<S: BuildHasher>(
    conn: &Connection,
    digests: &HashSet<String, S>,
) -> Result<(usize, u64), StoreError> {
    let mut read = conn.prepare("SELECT stored FROM text_version WHERE digest = ?1 AND body IS NOT NULL")?;
    let mut clear = conn.prepare("UPDATE text_version SET body = NULL, stored = 0 WHERE digest = ?1")?;
    let (mut cleared, mut bytes) = (0, 0u64);
    for digest in digests {
        let held: Option<u64> = read.query_row([digest], |row| row.get(0)).ok().map(unsigned);
        if let Some(held) = held {
            clear.execute([digest])?;
            cleared += 1;
            bytes += held;
        }
    }
    Ok((cleared, bytes))
}

/// Gives the pages freed back to the file system, then truncates the log, so that no copy of a
/// cleared body stays in either.
pub fn compact(conn: &mut Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA incremental_vacuum;")?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::resources::{self, sample as row};
    use crate::changes::texts::{self, sample as text};
    use crate::changes::{memory, snapshots, trees};

    fn start(conn: &Connection, minutes: i64, tree: &str) -> i64 {
        let id = snapshots::insert(conn, &snapshots::sample("default", minutes)).unwrap();
        resources::insert(conn, id, &[row("resources/a", Some(tree))]).unwrap();
        id
    }

    #[test]
    fn an_expired_start_hands_its_rows_to_the_starts_that_share_them() {
        let conn = memory();
        trees::insert(&conn, "t1", b"[]", &[]).unwrap();
        let old = start(&conn, 0, "t1");
        let later: Vec<i64> = (1..4)
            .map(|minutes| {
                let id = snapshots::insert(&conn, &snapshots::sample("default", minutes)).unwrap();
                snapshots::share_rows(&conn, id, old).unwrap();
                id
            })
            .collect();

        let cutoff = Timestamp::from_second(1_793_000_000 + 30).unwrap();
        assert_eq!(expire(&conn, "default", cutoff).unwrap(), 1);

        let heir = snapshots::get(&conn, later[0]).unwrap().unwrap();
        assert_eq!(heir.same_as, None);
        assert_eq!(resources::rows(&conn, heir.id).unwrap().len(), 1);
        for id in &later[1..] {
            assert_eq!(snapshots::get(&conn, *id).unwrap().unwrap().holder(), later[0]);
        }
    }

    #[test]
    fn unreferenced_trees_and_texts_go() {
        let conn = memory();
        trees::insert(
            &conn,
            "kept",
            b"[]",
            &[("resources/a/x.lua".to_owned(), "d1".to_owned())],
        )
        .unwrap();
        trees::insert(
            &conn,
            "gone",
            b"[]",
            &[("resources/a/x.lua".to_owned(), "d2".to_owned())],
        )
        .unwrap();
        let id = start(&conn, 0, "kept");
        texts::attach(&conn, id, "server.cfg", "d3").unwrap();
        for digest in ["d1", "d2", "d3", "d4"] {
            texts::store(&conn, &text(digest, "x"), id).unwrap();
        }

        assert_eq!(unreferenced(&conn).unwrap(), (1, 2));
        assert!(texts::get(&conn, "d1").unwrap().is_some());
        assert!(texts::get(&conn, "d3").unwrap().is_some());
    }

    #[test]
    fn the_budget_drops_the_oldest_bodies_and_spares_the_protected_starts() {
        let conn = memory();
        trees::insert(
            &conn,
            "t1",
            b"[]",
            &[("resources/a/x.lua".to_owned(), "new".to_owned())],
        )
        .unwrap();
        let protected = start(&conn, 10, "t1");
        texts::store(&conn, &text("oldest", &"a".repeat(4000)), 1).unwrap();
        texts::store(&conn, &text("older", &"b".repeat(4000)), 2).unwrap();
        texts::store(&conn, &text("new", &"c".repeat(4000)), 1).unwrap();
        let each = stored(&conn).unwrap() / 3;

        let (dropped, freed) = enforce_budget(&conn, each, &[protected]).unwrap();

        assert_eq!((dropped, freed), (2, 2 * each));
        assert_eq!(texts::get(&conn, "oldest").unwrap().unwrap().text, None);
        assert!(texts::get(&conn, "new").unwrap().unwrap().text.is_some());
    }

    #[test]
    fn a_purge_clears_every_body_a_path_had() {
        let conn = memory();
        trees::insert(
            &conn,
            "t1",
            b"[]",
            &[("resources/a/x.lua".to_owned(), "v1".to_owned())],
        )
        .unwrap();
        trees::insert(
            &conn,
            "t2",
            b"[]",
            &[("resources/a/x.lua".to_owned(), "v2".to_owned())],
        )
        .unwrap();
        for digest in ["v1", "v2", "other"] {
            texts::store(&conn, &text(digest, "token"), 1).unwrap();
        }

        let digests = digests_of(&conn, "resources/a/x.lua").unwrap();
        let (cleared, _) = clear_bodies(&conn, &digests).unwrap();

        assert_eq!(cleared, 2);
        assert_eq!(texts::get(&conn, "v2").unwrap().unwrap().text, None);
        assert!(texts::get(&conn, "other").unwrap().unwrap().text.is_some());
    }
}
