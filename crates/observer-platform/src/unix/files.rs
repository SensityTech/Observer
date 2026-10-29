use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;

pub(crate) fn open_resolved(path: &Path) -> io::Result<(File, PathBuf)> {
    // A link in the file's place is refused rather than followed, and a pipe never makes the open
    // wait for a writer.
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = File::from(rustix::fs::open(path, flags, Mode::empty())?);
    let resolved = fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    Ok((file, resolved))
}

pub(crate) fn hold_folder(path: &Path) -> io::Result<(File, PathBuf)> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let folder = match rustix::fs::open(path, flags, Mode::empty()) {
        Ok(folder) => File::from(folder),
        // Older kernels report a link in the folder's place as a loop rather than as no folder.
        Err(Errno::LOOP) => return Err(io::Error::from(io::ErrorKind::NotADirectory)),
        Err(error) => return Err(error.into()),
    };
    // The descriptor's entry in /proc leads to the folder it holds, wherever that is now.
    let inside = PathBuf::from(format!("/proc/self/fd/{}", folder.as_raw_fd()));
    Ok((folder, inside))
}
