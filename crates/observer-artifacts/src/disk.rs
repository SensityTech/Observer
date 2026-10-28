use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::Duration;

use crate::error::{Error, io};
use crate::http::MIB;

/// Antivirus scanners and indexers hold freshly written files for a moment on Windows, which makes a
/// rename or a removal fail; the same call a little later succeeds.
const BUSY_DELAYS_MS: [u64; 6] = [50, 100, 200, 400, 800, 1600];

/// Kept free beyond what an extraction needs.
pub(crate) const SPACE_MARGIN: u64 = 64 * MIB;

/// FXServer does not use long paths on Windows, and a build's deepest file sits about 125
/// characters below its root.
const DEEPEST_ENTRY: usize = 125;
const MAX_PATH: usize = 260;

/// Refuses when `dir`'s volume has less than `needed` bytes free for this user.
pub(crate) fn require_space(dir: &Path, needed: u64) -> Result<(), Error> {
    let available = observer_platform::available_space(dir).map_err(io(dir))?;
    if available < needed {
        return Err(Error::DiskSpace {
            path: dir.to_path_buf(),
            needed_mib: needed.div_ceil(MIB),
            available_mib: available / MIB,
        });
    }
    Ok(())
}

/// Runs a rename or a removal again while the file system reports it busy.
pub(crate) fn retry<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for delay in BUSY_DELAYS_MS {
        match operation() {
            Err(error) if busy(&error) => thread::sleep(Duration::from_millis(delay)),
            result => return result,
        }
    }
    operation()
}

/// Removes a file, waiting out a scanner that holds it. A file already gone is no error.
pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    match retry(|| fs::remove_file(path)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn busy(error: &io::Error) -> bool {
    // 32 and 33 are Windows' sharing and lock violations, which have no portable kind.
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ResourceBusy | io::ErrorKind::DirectoryNotEmpty
    ) || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
}

/// The length of the artifacts folder when, on Windows, it leaves FXServer too little room.
pub(crate) fn long_path(root: &Path) -> Option<usize> {
    if !cfg!(windows) {
        return None;
    }
    let absolute = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    let length = absolute.as_os_str().to_string_lossy().chars().count();
    (length + DEEPEST_ENTRY > MAX_PATH).then_some(length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_without_the_space_needed_is_refused() {
        let dir = tempfile::tempdir().unwrap();

        let error = require_space(dir.path(), u64::MAX).unwrap_err();

        assert!(
            matches!(&error, Error::DiskSpace { needed_mib, available_mib, .. } if needed_mib > available_mib),
            "{error}"
        );
        require_space(dir.path(), 1).unwrap();
    }

    #[test]
    fn a_short_artifacts_folder_gets_no_warning() {
        assert_eq!(long_path(Path::new("C:\\observer\\data\\artifacts")), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_deep_artifacts_folder_on_windows_is_reported() {
        let deep = format!("C:\\{}", "nested\\".repeat(20));

        assert_eq!(long_path(Path::new(&deep)), Some(deep.len()));
    }
}
