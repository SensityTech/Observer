//! Crash dumps: FXServer writes them into the build, where they would pile up and get in the way
//! of the build's verification. Observer moves them into its own folder and keeps a few, with its
//! own pictures of hung servers, and reads what they say.

pub mod analysis;
pub mod report;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use observer_platform::ProcessTree;
use serde::Serialize;

/// A partial file this old was left by an Observer that stopped while writing it.
const STALE_PARTIAL: Duration = Duration::from_mins(1);

#[derive(Debug, Clone, Copy)]
pub struct DumpLimits {
    pub files: usize,
    pub bytes: u64,
}

impl Default for DumpLimits {
    fn default() -> Self {
        Self {
            files: 10,
            bytes: 20 * 1024 * 1024 * 1024,
        }
    }
}

/// What [`collect`] did: where the moved files went, and the older files the limits removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collected {
    pub moved: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

/// Moves every file in `from` (the build's `crashes/`) into `to`, then removes the oldest dumps in
/// `to` beyond the limits. Blocks: a dump can be hundreds of megabytes, copied if the two folders
/// are on different volumes.
pub fn collect(from: &Path, to: &Path, limits: DumpLimits) -> io::Result<Collected> {
    let mut moved = Vec::new();
    let entries = match fs::read_dir(from) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Collected::default()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        fs::create_dir_all(to)?;
        let target = free_name(to, &entry.file_name().to_string_lossy());
        move_file(&entry.path(), &target)?;
        moved.push(target);
    }
    let mut removed = Vec::new();
    if !moved.is_empty() {
        removed = trim(to, limits)?;
        moved.retain(|path| !removed.contains(path));
    }
    Ok(Collected { moved, removed })
}

#[derive(Debug, Clone, Copy)]
pub struct HangSnapshotLimits {
    /// Below this much free space on the data volume, no snapshot is taken.
    pub min_free: u64,
    /// The kill waits no longer than this for the snapshot.
    pub timeout: Duration,
}

impl Default for HangSnapshotLimits {
    fn default() -> Self {
        Self {
            min_free: 1024 * 1024 * 1024,
            timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotFailure {
    LowDisk,
    Timeout,
    Failed,
}

/// Writes a picture of the tree's hung main process into `dir`: a minidump of its stacks on
/// Windows, a report of its threads on Linux. It goes to a `.partial` file renamed once complete,
/// and is deleted instead when `abandoned` is set meanwhile: the kill did not wait for it. Returns
/// the file and its size. Blocks.
pub fn snapshot(
    tree: &ProcessTree,
    pid: u32,
    dir: &Path,
    generation: u32,
    abandoned: &AtomicBool,
) -> io::Result<(PathBuf, u64)> {
    fs::create_dir_all(dir)?;
    let stamp = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
    let extension = if cfg!(windows) { "dmp" } else { "json" };
    let path = dir.join(format!("hang-{generation}-{stamp}.{extension}"));
    let partial = path.with_extension(format!("{extension}.partial"));
    let written = write_snapshot(tree, pid, &partial);
    match written {
        Ok(()) if !abandoned.load(Ordering::Relaxed) => {
            fs::rename(&partial, &path)?;
            let bytes = fs::metadata(&path)?.len();
            Ok((path, bytes))
        }
        Ok(()) => {
            let _ = fs::remove_file(&partial);
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the snapshot came too late",
            ))
        }
        Err(error) => {
            let _ = fs::remove_file(&partial);
            Err(error)
        }
    }
}

#[cfg(windows)]
fn write_snapshot(tree: &ProcessTree, _pid: u32, partial: &Path) -> io::Result<()> {
    let file = fs::File::create(partial)?;
    tree.write_dump(&file)?;
    file.sync_all()
}

#[cfg(unix)]
fn write_snapshot(tree: &ProcessTree, pid: u32, partial: &Path) -> io::Result<()> {
    let states = tree.thread_states(Duration::from_millis(500))?;
    let report = report::report(pid, states);
    fs::write(partial, serde_json::to_vec(&report).map_err(io::Error::other)?)
}

/// Deletes the `.partial` files an earlier Observer left in `dir`: snapshots cut short.
pub fn remove_stale_partials(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() >= STALE_PARTIAL);
        if stale && path.extension().is_some_and(|extension| extension == "partial") {
            let _ = fs::remove_file(path);
        }
    }
}

/// Whether a dump is being written now: a crash looks like a hang while the dump server writes.
pub fn being_written(crashes: &Path, within: Duration) -> bool {
    let Ok(entries) = fs::read_dir(crashes) else {
        return false;
    };
    let now = SystemTime::now();
    entries.filter_map(Result::ok).any(|entry| {
        // Not the directory entry's own metadata: on Windows the listing keeps the size and time
        // a file had when it was opened, for as long as the dump server holds it open.
        fs::metadata(entry.path())
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() <= within)
    })
}

fn free_name(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (name, String::new()),
    };
    (1..u32::MAX)
        .map(|index| dir.join(format!("{stem}-{index}{extension}")))
        .find(|path| !path.exists())
        .unwrap_or(candidate)
}

fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from)
}

/// Removes the oldest files of `dir` beyond the limits, and returns them.
pub fn trim(dir: &Path, limits: DumpLimits) -> io::Result<Vec<PathBuf>> {
    let mut dumps = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            dumps.push((metadata.modified()?, metadata.len(), entry.path()));
        }
    }
    dumps.sort();
    let mut count = dumps.len();
    let mut total: u64 = dumps.iter().map(|(_, size, _)| size).sum();
    let mut removed = Vec::new();
    for (_, size, path) in dumps {
        if count <= limits.files && total <= limits.bytes {
            break;
        }
        fs::remove_file(&path)?;
        removed.push(path);
        count -= 1;
        total -= size;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write as _;
    use std::thread;
    use std::time::Instant;

    use super::*;

    fn dump(dir: &Path, name: &str, size: usize, age: Duration) {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, vec![0; size]).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn dumps_move_out_of_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let (build, data) = (dir.path().join("build/crashes"), dir.path().join("data/crashes"));
        dump(&build, "a1b2.dmp", 10, Duration::ZERO);
        dump(&build, "a1b2-full.dmp", 30, Duration::ZERO);

        let collected = collect(&build, &data, DumpLimits::default()).unwrap();

        assert_eq!(collected.moved.len(), 2);
        assert_eq!(collected.removed, Vec::<PathBuf>::new());
        assert_eq!(names(&build), Vec::<String>::new());
        assert_eq!(names(&data), ["a1b2-full.dmp", "a1b2.dmp"]);
    }

    #[test]
    fn a_build_without_crashes_has_nothing_to_move() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            collect(
                &dir.path().join("none"),
                &dir.path().join("data"),
                DumpLimits::default()
            )
            .unwrap(),
            Collected::default()
        );
        assert!(!dir.path().join("data").exists());
    }

    #[test]
    fn an_empty_crashes_folder_needs_no_folder_of_observer_s_own() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("build/crashes")).unwrap();

        let moved = collect(
            &dir.path().join("build/crashes"),
            &dir.path().join("data/crashes"),
            DumpLimits::default(),
        );

        assert_eq!(moved.unwrap(), Collected::default());
    }

    #[test]
    fn the_oldest_dumps_go_past_the_file_count() {
        let dir = tempfile::tempdir().unwrap();
        let (build, data) = (dir.path().join("build"), dir.path().join("data"));
        for age in 1..=3 {
            dump(&data, &format!("old-{age}.dmp"), 1, Duration::from_hours(age));
        }
        dump(&build, "new.dmp", 1, Duration::ZERO);
        let limits = DumpLimits {
            files: 2,
            ..DumpLimits::default()
        };

        let collected = collect(&build, &data, limits).unwrap();

        assert_eq!(names(&data), ["new.dmp", "old-1.dmp"]);
        assert_eq!(
            collected.removed,
            [data.join("old-3.dmp"), data.join("old-2.dmp")]
        );
    }

    #[test]
    fn the_oldest_dumps_go_past_the_size_cap_even_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let (build, data) = (dir.path().join("build"), dir.path().join("data"));
        dump(&data, "old.dmp", 60, Duration::from_hours(1));
        dump(&build, "new.dmp", 50, Duration::ZERO);
        dump(&build, "newer-full.dmp", 200, Duration::ZERO);
        let limits = DumpLimits {
            bytes: 100,
            ..DumpLimits::default()
        };

        let moved = collect(&build, &data, limits).unwrap();

        assert_eq!(names(&data), Vec::<String>::new());
        assert_eq!(moved.moved, Vec::<PathBuf>::new());
        assert_eq!(moved.removed.len(), 3);
    }

    #[test]
    fn a_name_already_taken_gets_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let (build, data) = (dir.path().join("build"), dir.path().join("data"));
        dump(&data, "same.dmp", 1, Duration::from_hours(1));
        dump(&build, "same.dmp", 2, Duration::ZERO);

        collect(&build, &data, DumpLimits::default()).unwrap();

        assert_eq!(names(&data), ["same-1.dmp", "same.dmp"]);
    }

    #[test]
    fn a_dump_written_just_now_counts_as_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        dump(dir.path(), "old.dmp", 1, Duration::from_mins(5));
        assert!(!being_written(dir.path(), Duration::from_secs(10)));

        dump(dir.path(), "new.dmp", 1, Duration::ZERO);

        assert!(being_written(dir.path(), Duration::from_secs(10)));
        assert!(!being_written(&dir.path().join("none"), Duration::from_secs(10)));
    }

    #[test]
    fn a_dump_still_growing_past_the_window_counts_as_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let mut dump = File::create(dir.path().join("0a1b-full.dmp")).unwrap();
        let memory = vec![0; 64 * 1024];
        let within = Duration::from_millis(500);
        let started = Instant::now();

        while started.elapsed() < Duration::from_secs(2) {
            dump.write_all(&memory).unwrap();
            thread::sleep(Duration::from_millis(100));

            assert!(
                being_written(dir.path(), within),
                "{:?} into the dump",
                started.elapsed()
            );
        }
    }
}
