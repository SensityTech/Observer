use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_NAME_NORMALIZED, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, GetFinalPathNameByHandleW,
    GetLogicalDrives, VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

pub(crate) fn local_drives() -> Vec<PathBuf> {
    // SAFETY: GetLogicalDrives takes nothing and returns a bit per drive letter.
    let drives = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|letter| drives & (1 << letter) != 0)
        .map(|letter| format!("{}:\\", char::from(b'A' + letter)))
        .filter(|root| {
            let wide: Vec<u16> = root.encode_utf16().chain([0]).collect();
            // SAFETY: `wide` is a nul-terminated root path that outlives the call.
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE
        })
        .map(PathBuf::from)
        .collect()
}

pub(crate) fn open_resolved(path: &Path) -> io::Result<(File, PathBuf)> {
    // A link or a junction in the file's place is opened itself, not what it leads to.
    let file = File::options()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let resolved = final_path(&file)?;
    Ok((file, resolved))
}

pub(crate) fn hold_folder(path: &Path) -> io::Result<(File, PathBuf)> {
    // Shared for reading and writing but not for deletion: while the handle is open nobody
    // renames or removes the folder, so its path keeps leading to it. A link or a junction in its
    // place is opened itself, and refused.
    let folder = File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = folder.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_dir() {
        return Err(io::Error::from(io::ErrorKind::NotADirectory));
    }
    Ok((folder, path.to_path_buf()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_drive_is_one_of_the_local_drives() {
        let system = std::env::var("SystemDrive").unwrap();

        assert!(local_drives().contains(&PathBuf::from(format!("{system}\\"))));
    }
}
