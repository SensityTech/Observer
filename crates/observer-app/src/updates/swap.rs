//! The executable's swap, and its undoing. The new executable is copied beside the running one
//! first, so that both renames stay on one volume. Windows refuses to overwrite or delete a
//! running executable but lets it be renamed; Linux renames over it.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// `observer.exe` as `observer.<tag>.exe`, `observer` as `observer.<tag>`, in the same folder.
pub fn sibling(executable: &Path, tag: &str) -> PathBuf {
    let stem = executable
        .file_stem()
        .map_or_else(|| "observer".into(), |stem| stem.to_string_lossy());
    let name = match executable.extension() {
        Some(extension) => format!("{stem}.{tag}.{}", extension.to_string_lossy()),
        None => format!("{stem}.{tag}"),
    };
    executable.with_file_name(name)
}

pub fn previous_of(executable: &Path) -> PathBuf {
    sibling(executable, "previous")
}

/// Whether this account can write files in `dir`, by writing one.
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".observer-write-check-{}", std::process::id()));
    let made = File::options().write(true).create_new(true).open(&probe).is_ok();
    if made {
        let _ = fs::remove_file(&probe);
    }
    made
}

/// The SHA-256 of a file, in hexadecimal.
pub fn sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

/// Puts `staged` in the place of `executable`, once its bytes still hash to `sha256`, and keeps
/// the executable it replaces as [`previous_of`]. Says where that one went.
pub fn swap(staged: &Path, sha256_hex: &str, executable: &Path) -> io::Result<PathBuf> {
    if sha256(staged)? != sha256_hex {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the verified executable changed since it was verified",
        ));
    }
    let new = sibling(executable, "new");
    let previous = previous_of(executable);
    remove_if_there(&new)?;
    fs::copy(staged, &new)?;
    // Windows flushes only a file opened for writing.
    File::options().write(true).open(&new)?.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
    }
    remove_if_there(&previous)?;
    #[cfg(windows)]
    {
        fs::rename(executable, &previous)?;
        if let Err(error) = fs::rename(&new, executable) {
            let _ = fs::rename(&previous, executable);
            return Err(error);
        }
    }
    #[cfg(unix)]
    {
        // A second name for the running executable, so that the path never goes missing.
        if fs::hard_link(executable, &previous).is_err() {
            fs::copy(executable, &previous)?;
        }
        fs::rename(&new, executable)?;
    }
    Ok(previous)
}

/// Puts `previous` back in the place of `executable`. On Windows the one that ran is set aside
/// as `observer.failed.exe` first, since it may still be running.
pub fn restore(executable: &Path, previous: &Path) -> io::Result<()> {
    if !previous.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("the previous executable is not at {}", previous.display()),
        ));
    }
    #[cfg(windows)]
    {
        let failed = sibling(executable, "failed");
        remove_if_there(&failed)?;
        if executable.exists() {
            fs::rename(executable, &failed)?;
        }
    }
    fs::rename(previous, executable)
}

/// Removes what a swap or a return leaves next to the executable, once nothing runs from it.
pub fn sweep(executable: &Path) {
    for tag in ["new", "failed"] {
        let _ = remove_if_there(&sibling(executable, tag));
    }
}

fn remove_if_there(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sibling_keeps_the_extension_last() {
        assert_eq!(
            sibling(Path::new("C:/Observer/observer.exe"), "previous"),
            Path::new("C:/Observer/observer.previous.exe")
        );
        assert_eq!(
            sibling(Path::new("/srv/observer/observer"), "new"),
            Path::new("/srv/observer/observer.new")
        );
    }

    #[test]
    fn a_swap_keeps_the_previous_executable_and_a_restore_puts_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir
            .path()
            .join(if cfg!(windows) { "observer.exe" } else { "observer" });
        fs::write(&executable, b"version 1").unwrap();
        let staged = dir.path().join("staged");
        fs::write(&staged, b"version 2").unwrap();

        let previous = swap(&staged, &sha256(&staged).unwrap(), &executable).unwrap();

        assert_eq!(fs::read(&executable).unwrap(), b"version 2");
        assert_eq!(fs::read(&previous).unwrap(), b"version 1");
        assert!(!sibling(&executable, "new").exists());

        restore(&executable, &previous).unwrap();

        assert_eq!(fs::read(&executable).unwrap(), b"version 1");
        assert!(!previous.exists());
        sweep(&executable);
        assert!(!sibling(&executable, "failed").exists());
    }

    #[test]
    fn an_executable_changed_after_its_verification_is_not_swapped_in() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("observer");
        fs::write(&executable, b"version 1").unwrap();
        let staged = dir.path().join("staged");
        fs::write(&staged, b"version 2").unwrap();
        let verified = sha256(&staged).unwrap();
        fs::write(&staged, b"something else").unwrap();

        let error = swap(&staged, &verified, &executable).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&executable).unwrap(), b"version 1");
    }

    #[test]
    fn a_folder_this_account_cannot_write_in_is_found_out() {
        let dir = tempfile::tempdir().unwrap();

        assert!(writable(dir.path()));
        assert!(!writable(&dir.path().join("missing")));
    }
}
