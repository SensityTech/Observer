use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// What the kernel appends to the executable of a process when its file was deleted.
const DELETED: &[u8] = b" (deleted)";

/// The processes whose executable lies under `dir`, as `/proc/<pid>/exe` names it. A process this
/// account cannot read belongs to another account, which could not have started it from a folder
/// of Observer's. Without `/proc` nothing can be told, which is an error rather than none.
pub(crate) fn processes_in(dir: &Path) -> io::Result<Vec<u32>> {
    let dir = fs::canonicalize(dir)?;
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(executable) = fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        let bytes = executable.as_os_str().as_bytes();
        let executable = bytes
            .strip_suffix(DELETED)
            .map_or(executable.clone(), |kept| PathBuf::from(OsStr::from_bytes(kept)));
        if executable.starts_with(&dir) {
            found.push(pid);
        }
    }
    found.sort_unstable();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    /// Another test forking while the copy was still open for writing makes the executable busy
    /// for a moment.
    fn start(program: &Path) -> std::process::Child {
        let mut attempts = 0;
        loop {
            let started = Command::new(program)
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match started {
                Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy && attempts < 50 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                started => return started.unwrap(),
            }
        }
    }

    #[test]
    fn a_process_started_from_a_folder_is_found_in_it_even_once_its_file_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("holder");
        fs::copy("/bin/sleep", &program).unwrap();
        let mut child = start(&program);

        let running = processes_in(dir.path()).unwrap();
        fs::remove_file(&program).unwrap();
        let deleted = processes_in(dir.path()).unwrap();
        let elsewhere = processes_in(&std::env::temp_dir().join("observer-nowhere"));
        child.kill().unwrap();
        child.wait().unwrap();

        assert_eq!(running, [child.id()]);
        assert_eq!(deleted, [child.id()]);
        assert!(
            elsewhere.is_err(),
            "a folder that does not exist cannot be checked"
        );
        assert_eq!(processes_in(dir.path()).unwrap(), Vec::<u32>::new());
    }
}
