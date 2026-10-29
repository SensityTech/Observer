use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags};

pub(crate) fn open_resolved(path: &Path) -> io::Result<(File, PathBuf)> {
    // A link in the file's place is refused rather than followed, and a pipe never makes the open
    // wait for a writer.
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = File::from(rustix::fs::open(path, flags, Mode::empty())?);
    let resolved = fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    Ok((file, resolved))
}
