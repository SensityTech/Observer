use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
};

pub(crate) fn open_resolved(path: &Path) -> io::Result<(File, PathBuf)> {
    // A link or a junction in the file's place is opened itself, not what it leads to.
    let file = File::options()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let resolved = final_path(&file)?;
    Ok((file, resolved))
}

/// The path of an open file, every link and junction on the way to it resolved, in the
/// `\\?\C:\…` form `fs::canonicalize` gives.
fn final_path(file: &File) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 1024];
    loop {
        let capacity = u32::try_from(buffer.len()).map_err(io::Error::other)?;
        // SAFETY: the handle is open for as long as `file` lives, and `buffer` holds `capacity`
        // writable units; the call writes at most that many and returns what it wrote, or the
        // size it needs when that is more.
        let written = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        let written = usize::try_from(written).map_err(io::Error::other)?;
        if written == 0 {
            return Err(io::Error::last_os_error());
        }
        if written < buffer.len() {
            buffer.truncate(written);
            return Ok(PathBuf::from(OsString::from_wide(&buffer)));
        }
        buffer.resize(written, 0);
    }
}
