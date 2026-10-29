use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::FALSE;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use super::usage::process_ids;

/// The processes whose executable lies under `dir`. A process this account cannot open belongs
/// to another account, which could not have started it from a folder of Observer's.
pub(crate) fn processes_in(dir: &Path) -> io::Result<Vec<u32>> {
    let dir = folded(&fs::canonicalize(dir)?);
    let mut found = Vec::new();
    for pid in process_ids()? {
        let Some(image) = image(pid) else {
            continue;
        };
        // The canonical form gives long names for short ones and the case the disk has.
        let image = fs::canonicalize(&image).unwrap_or(image);
        if folded(&image).starts_with(&dir) {
            found.push(pid);
        }
    }
    found.sort_unstable();
    Ok(found)
}

/// Windows compares paths without regard to case.
fn folded(path: &Path) -> PathBuf {
    PathBuf::from(path.as_os_str().to_string_lossy().to_lowercase())
}

fn image(pid: u32) -> Option<PathBuf> {
    // SAFETY: OpenProcess takes any pid and returns null when it cannot open it.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: OpenProcess returned an open handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut buffer = vec![0u16; 32_768];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: the handle is open with the query right, `buffer` holds `length` writable units,
    // and the call writes at most that many, then sets `length` to what it wrote.
    let read = unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle(),
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &raw mut length,
        )
    };
    if read == 0 {
        return None;
    }
    buffer.truncate(usize::try_from(length).ok()?);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn a_process_started_from_a_folder_is_found_in_it_whatever_the_case_of_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("Holder.exe");
        // `ping` waits between its echoes whatever its input; `timeout` refuses a redirected one.
        fs::copy(r"C:\Windows\System32\PING.EXE", &program).unwrap();
        let mut child = Command::new(&program)
            .args(["-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let exact = processes_in(dir.path());
        let upper = processes_in(Path::new(&dir.path().to_string_lossy().to_uppercase()));
        let elsewhere = processes_in(&std::env::temp_dir().join("observer-nowhere"));
        child.kill().unwrap();
        child.wait().unwrap();

        assert_eq!(exact.unwrap(), [child.id()]);
        assert_eq!(upper.unwrap(), [child.id()]);
        assert!(
            elsewhere.is_err(),
            "a folder that does not exist cannot be checked"
        );
        assert_eq!(processes_in(dir.path()).unwrap(), Vec::<u32>::new());
    }
}
