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
    Manual,
    /// Before an install of a new release swaps Observer's executable.
    PreUpdate,
}

impl BackupKind {
    fn label(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::PreMigration => "pre-migration",
            Self::Manual => "manual",
            Self::PreUpdate => "pre-update",
        }
    }

    fn keep(self) -> usize {
        match self {
            Self::Daily => 7,
            Self::PreMigration | Self::PreUpdate => 3,
            Self::Manual => 10,
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

/// Puts `backup` in the place of the database at `database`: the database and its `-wal` and
/// `-shm` files go aside first, as `<name>.<suffix>`, so that nothing of it meets the backup.
/// Observer must not hold the database. Says where the database went.
pub fn restore(database: &Path, backup: &Path, suffix: &str) -> io::Result<PathBuf> {
    let aside = |path: &Path| {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{suffix}"));
        PathBuf::from(name)
    };
    let set_aside = aside(database);
    for companion in ["-wal", "-shm"] {
        let mut name = database.as_os_str().to_owned();
        name.push(companion);
        let path = PathBuf::from(name);
        if path.exists() {
            fs::rename(&path, aside(&path))?;
        }
    }
    if database.exists() {
        fs::rename(database, &set_aside)?;
    }
    let partial = aside(&database.with_extension("restoring"));
    fs::copy(backup, &partial)?;
    // Windows flushes only a file opened for writing.
    fs::File::options().write(true).open(&partial)?.sync_all()?;
    fs::rename(&partial, database)?;
    Ok(set_aside)
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
    fn a_restored_backup_takes_the_place_of_the_database_and_its_companions() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("observer.sqlite3");
        fs::write(&database, b"migrated").unwrap();
        fs::write(dir.path().join("observer.sqlite3-wal"), b"wal").unwrap();
        let backup = dir.path().join("observer-20261109T100000Z-pre-update.sqlite3");
        fs::write(&backup, b"before").unwrap();

        let aside = restore(&database, &backup, "update-1").unwrap();

        assert_eq!(fs::read(&database).unwrap(), b"before");
        assert_eq!(fs::read(&aside).unwrap(), b"migrated");
        assert!(!dir.path().join("observer.sqlite3-wal").exists());
        assert_eq!(
            fs::read(dir.path().join("observer.sqlite3-wal.update-1")).unwrap(),
            b"wal"
        );
        assert!(backup.exists());
    }

    #[test]
    fn a_missing_backup_directory_has_no_backups() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("backups");

        assert_eq!(newest(&missing, BackupKind::Daily).unwrap(), None);
    }
}
