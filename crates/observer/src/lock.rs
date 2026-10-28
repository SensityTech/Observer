use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;

use anyhow::{Context, bail};

/// Holds the data directory for this process. The OS releases the lock when the process ends, even
/// after a crash, so there is never a stale lock file to clean up.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
}

pub fn acquire(data_dir: &Path) -> anyhow::Result<DataDirLock> {
    fs::create_dir_all(data_dir).with_context(|| format!("cannot create {}", data_dir.display()))?;

    let path = data_dir.join("observer.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;

    match file.try_lock() {
        Ok(()) => Ok(DataDirLock { _file: file }),
        Err(TryLockError::WouldBlock) => {
            bail!("another Observer is already using {}", data_dir.display())
        }
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("cannot lock {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_lock_on_the_same_directory_is_refused_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();

        let first = acquire(dir.path()).unwrap();
        assert!(acquire(dir.path()).is_err());

        drop(first);
        assert!(acquire(dir.path()).is_ok());
    }
}
