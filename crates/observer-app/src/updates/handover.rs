//! What an Observer that swapped its executable leaves the next one, in `updates/handover.json`
//! under the data folder: which versions, where the previous executable and the database's backup
//! are, whether FXServer must start again, and whether the new version already started once. And
//! how the last install ended, in `updates/last.json`, for the panel.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use observer_domain::updates::{RollbackReason, UpdateOutcome};
use serde::{Deserialize, Serialize};

pub const FOLDER: &str = "updates";
const HANDOVER: &str = "handover.json";
const LAST: &str = "last.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Handover {
    pub from: String,
    pub to: String,
    /// Where Observer runs from, which now holds the new version.
    pub executable: PathBuf,
    /// The previous version's executable, kept beside it.
    pub previous: PathBuf,
    /// The state database as it was just before the swap.
    pub database_backup: Option<PathBuf>,
    /// FXServer ran, or was stopped in the place of a scheduled restart: the next Observer starts
    /// it again.
    pub fxserver: bool,
    pub at: Timestamp,
    /// The starts of the new version so far: a second one means the first never finished.
    pub attempts: u32,
    pub state: HandoverState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HandoverState {
    /// The new executable is in place.
    Swapped,
    /// The previous executable is back; it settles the rest as it starts.
    RolledBack { reason: RollbackReason, at: Timestamp },
}

pub fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join(FOLDER)
}

/// The handover a previous Observer left, if any. One that cannot be read is an error: guessing
/// could start the wrong executable.
pub fn read(data_dir: &Path) -> io::Result<Option<Handover>> {
    read_json(&folder(data_dir).join(HANDOVER))
}

pub fn write(data_dir: &Path, handover: &Handover) -> io::Result<()> {
    write_json(&folder(data_dir).join(HANDOVER), handover)
}

pub fn clear(data_dir: &Path) -> io::Result<()> {
    match fs::remove_file(folder(data_dir).join(HANDOVER)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// How the last install that restarted Observer ended; nothing when it cannot be read.
pub fn read_last(data_dir: &Path) -> Option<UpdateOutcome> {
    read_json(&folder(data_dir).join(LAST)).ok().flatten()
}

pub fn write_last(data_dir: &Path, outcome: &UpdateOutcome) -> io::Result<()> {
    write_json(&folder(data_dir).join(LAST), outcome)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Written whole to a temporary file, then renamed: a crash leaves the old file or the new one.
fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let partial = path.with_extension("json.partial");
    let mut file = File::create(&partial)?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handover(dir: &Path) -> Handover {
        Handover {
            from: "0.1.0".to_owned(),
            to: "0.2.0".to_owned(),
            executable: dir.join("observer"),
            previous: dir.join("observer.previous"),
            database_backup: None,
            fxserver: true,
            at: "2026-11-09T10:00:00Z".parse().unwrap(),
            attempts: 0,
            state: HandoverState::Swapped,
        }
    }

    #[test]
    fn a_handover_is_read_back_as_written_and_cleared() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).unwrap(), None);

        let written = handover(dir.path());
        write(dir.path(), &written).unwrap();

        assert_eq!(read(dir.path()).unwrap(), Some(written));
        clear(dir.path()).unwrap();
        assert_eq!(read(dir.path()).unwrap(), None);
        clear(dir.path()).unwrap();
    }

    #[test]
    fn a_handover_that_cannot_be_read_is_an_error_not_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(folder(dir.path())).unwrap();
        fs::write(folder(dir.path()).join(HANDOVER), b"{\"from\":").unwrap();

        assert_eq!(read(dir.path()).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
