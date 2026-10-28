use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::Connection;

use crate::database::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    Daily,
    PreMigration,
}

impl BackupKind {
    fn label(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::PreMigration => "pre-migration",
        }
    }

    fn keep(self) -> usize {
        match self {
            Self::Daily => 7,
            Self::PreMigration => 3,
        }
    }
}

/// Writes a consistent snapshot with `VACUUM INTO`, which, unlike a file copy, includes what is
/// still in the WAL. The snapshot only gets its final name once it is complete.
pub(crate) fn create(conn: &Connection, dir: &Path, kind: BackupKind) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(dir)?;

    let stamp = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
    let name = format!("observer-{stamp}-{}.sqlite3", kind.label());
    let target = dir.join(&name);
    let partial = dir.join(format!("{name}.partial"));

    // VACUUM INTO refuses to overwrite, and an interrupted run leaves its partial file behind.
    if partial.exists() {
        fs::remove_file(&partial)?;
    }
    conn.execute("VACUUM INTO ?1", [partial.to_string_lossy()])?;
    fs::rename(&partial, &target)?;

    prune(dir, kind)?;
    Ok(target)
}

/// When the most recent backup of this kind was written, if there is one.
pub(crate) fn newest(dir: &Path, kind: BackupKind) -> io::Result<Option<SystemTime>> {
    match list(dir, kind)?.last() {
        Some(path) => fs::metadata(path)?.modified().map(Some),
        None => Ok(None),
    }
}

fn prune(dir: &Path, kind: BackupKind) -> io::Result<()> {
    let backups = list(dir, kind)?;
    let excess = backups.len().saturating_sub(kind.keep());
    for path in &backups[..excess] {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Backups of one kind, oldest first: the UTC timestamp in the name sorts chronologically.
fn list(dir: &Path, kind: BackupKind) -> io::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let suffix = format!("-{}.sqlite3", kind.label());
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let matches = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("observer-") && name.ends_with(&suffix));
        if matches {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"").unwrap();
    }

    #[test]
    fn prune_keeps_the_newest_backups_of_the_same_kind_only() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=9 {
            touch(
                dir.path(),
                &format!("observer-202610{day:02}T030000Z-daily.sqlite3"),
            );
        }
        touch(dir.path(), "observer-20261001T030000Z-pre-migration.sqlite3");
        touch(dir.path(), "notes.txt");

        prune(dir.path(), BackupKind::Daily).unwrap();

        let daily = list(dir.path(), BackupKind::Daily).unwrap();
        assert_eq!(daily.len(), 7);
        assert!(daily[0].ends_with("observer-20261003T030000Z-daily.sqlite3"));
        assert_eq!(list(dir.path(), BackupKind::PreMigration).unwrap().len(), 1);
        assert!(dir.path().join("notes.txt").exists());
    }

    #[test]
    fn a_missing_backup_directory_has_no_backups() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("backups");

        assert_eq!(newest(&missing, BackupKind::Daily).unwrap(), None);
    }
}
