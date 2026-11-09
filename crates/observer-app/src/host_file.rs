//! A file the owner names on the host for Observer to read once, such as txAdmin's players
//! database or a map image: a regular local file, never a network share, a link or a device.

use std::fs::{File, Metadata};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostFileProblem {
    /// A relative path, or one on another host's share.
    NotLocal,
    Missing,
    NotAFile,
    Unreadable,
}

/// Opens the file itself, never a link in its place, and checks from the open handle that it is a
/// regular local file.
pub(crate) fn open_local(path: &Path) -> Result<(File, Metadata), HostFileProblem> {
    if !path.is_absolute() || remote(path) {
        return Err(HostFileProblem::NotLocal);
    }
    let (file, resolved) = observer_platform::open_resolved(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            HostFileProblem::Missing
        } else if std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
            HostFileProblem::NotAFile
        } else {
            HostFileProblem::Unreadable
        }
    })?;
    // A folder link on the way may lead to another host's share.
    if remote_resolved(&resolved) {
        return Err(HostFileProblem::NotLocal);
    }
    let metadata = file.metadata().map_err(|_| HostFileProblem::Unreadable)?;
    if !metadata.is_file() || metadata.is_symlink() || reparse_point(&metadata) {
        return Err(HostFileProblem::NotAFile);
    }
    Ok((file, metadata))
}

/// A UNC or device path: on Windows, opening one reaches another host, or a pipe that could act
/// as Observer.
fn remote(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.starts_with(r"\\") || text.starts_with("//")
}

/// The path the file system took to the file: on Windows `\\?\C:\...` when it is local, and
/// `\\?\UNC\...` when a folder link led it to a share.
fn remote_resolved(resolved: &Path) -> bool {
    let text = resolved.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => rest.starts_with(r"UNC\") || rest.starts_with("GLOBALROOT"),
        None => remote(resolved),
    }
}

#[cfg(windows)]
fn reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn reparse_point(_: &Metadata) -> bool {
    false
}
