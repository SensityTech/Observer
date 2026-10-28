//! The two Windows calls artifacts need that take `unsafe`: the Authenticode check of FXServer.exe
//! and the free space of a volume. They move to `observer-platform` once that crate exists.

#![allow(unsafe_code)]

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Cryptography::{
    CERT_NAME_ATTR_TYPE, CertGetNameStringW, szOID_COMMON_NAME,
};
use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData,
    WinVerifyTrust,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// The `\\?\` form of a path, which Win32 opens past 260 characters: a staged build can sit that
/// deep under a long artifacts folder.
fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
    let absolute: Vec<u16> = std::path::absolute(path)?.as_os_str().encode_wide().collect();
    let starts = |prefix: &str| {
        absolute
            .iter()
            .copied()
            .take(prefix.len())
            .eq(prefix.encode_utf16())
    };
    let prefixed: Vec<u16> = if starts(r"\\?\") || starts(r"\\.\") {
        absolute
    } else if starts(r"\\") {
        r"\\?\UNC\"
            .encode_utf16()
            .chain(absolute[2..].iter().copied())
            .collect()
    } else {
        r"\\?\".encode_utf16().chain(absolute).collect()
    };
    Ok(prefixed.into_iter().chain([0]).collect())
}

pub(crate) fn available_space(dir: &Path) -> io::Result<u64> {
    let path = wide(dir);
    let mut available = 0;
    // SAFETY: `path` is NUL-terminated and outlives the call, `available` is a writable u64, and
    // the two totals we do not want may be null.
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &raw mut available,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(available)
    }
}

/// Checks the Authenticode signature of `file` against the machine's trusted roots, then that the
/// certificate that signed it was issued to `signer`. Revocation is not checked: that would need
/// the network at install time, for an archive that already came from Cfx.re over HTTPS.
pub(crate) fn verify_signature(file: &Path, signer: &str) -> Result<(), String> {
    let path = verbatim(file).map_err(|error| format!("its path cannot be resolved: {error}"))?;
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: struct_size::<WINTRUST_FILE_INFO>(),
        pcwszFilePath: path.as_ptr(),
        hFile: ptr::null_mut(),
        pgKnownSubject: ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: struct_size::<WINTRUST_DATA>(),
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &raw mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

    // SAFETY: `data` is fully initialised and points at `file_info`, which points at `path`; all three
    // outlive the call. INVALID_HANDLE_VALUE as the window tells WinVerifyTrust there is no user to ask.
    let status = unsafe { WinVerifyTrust(INVALID_HANDLE_VALUE, &raw mut action, (&raw mut data).cast()) };
    let signed_by = if status == 0 {
        signer_name(data.hWVTStateData)
    } else {
        None
    };

    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: the same structures, now asking WinVerifyTrust to free the state the first call kept.
    unsafe { WinVerifyTrust(INVALID_HANDLE_VALUE, &raw mut action, (&raw mut data).cast()) };

    if status != 0 {
        return Err(describe(status));
    }
    match signed_by {
        Some(name) if name == signer => Ok(()),
        Some(name) => Err(format!("it is signed by {name:?}")),
        None => Err("its signer's certificate cannot be read".to_owned()),
    }
}

/// The common name on the certificate of the file's first signer.
fn signer_name(state: HANDLE) -> Option<String> {
    // SAFETY: `state` comes from a successful verification and is only closed after this returns;
    // the helpers return pointers into it, each checked for null before it is followed.
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);
        if provider.is_null() {
            return None;
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, 0, 0);
        if signer.is_null() || (*signer).csCertChain == 0 || (*signer).pasCertChain.is_null() {
            return None;
        }
        let certificate = (*(*signer).pasCertChain).pCert;
        if certificate.is_null() {
            return None;
        }
        let attribute = szOID_COMMON_NAME.cast();
        let length = CertGetNameStringW(certificate, CERT_NAME_ATTR_TYPE, 0, attribute, ptr::null_mut(), 0);
        if length <= 1 {
            return None;
        }
        let mut name = vec![0; length as usize];
        let written = CertGetNameStringW(
            certificate,
            CERT_NAME_ATTR_TYPE,
            0,
            attribute,
            name.as_mut_ptr(),
            length,
        );
        Some(String::from_utf16_lossy(
            &name[..(written as usize).saturating_sub(1)],
        ))
    }
}

fn describe(status: i32) -> String {
    // HRESULTs are conventionally written as unsigned hexadecimal.
    let code = status.cast_unsigned();
    match code {
        0x800B_0100 => "it carries no signature".to_owned(),
        0x800B_0003 => "it is not a Windows executable".to_owned(),
        0x8009_6010 => "it was modified after it was signed".to_owned(),
        0x800B_0109 => "its certificate chain ends at an untrusted root".to_owned(),
        0x800B_0111 => "its certificate is explicitly distrusted".to_owned(),
        0x800B_0101 => "its certificate expired before it was signed".to_owned(),
        0x8009_2003 => "it cannot be opened".to_owned(),
        code => format!("WinVerifyTrust refused it with 0x{code:08X}"),
    }
}

fn struct_size<T>() -> u32 {
    u32::try_from(size_of::<T>()).expect("a Win32 structure is far smaller than 4 GiB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsigned_executable_is_refused() {
        // The test binary itself: a real executable that nobody signed.
        let unsigned = std::env::current_exe().unwrap();

        assert_eq!(
            verify_signature(&unsigned, "Rockstar Games, Inc."),
            Err("it carries no signature".to_owned())
        );
    }

    #[test]
    fn a_file_that_is_not_an_executable_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("FXServer.exe");
        std::fs::write(&path, b"MZ not really").unwrap();

        assert_eq!(
            verify_signature(&path, "Rockstar Games, Inc."),
            Err("it is not a Windows executable".to_owned())
        );
    }

    #[test]
    fn a_file_past_260_characters_is_still_checked() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("staging-".repeat(30));
        std::fs::create_dir_all(&deep).unwrap();
        let path = deep.join("FXServer.exe");
        std::fs::write(&path, b"MZ not really").unwrap();
        assert!(path.as_os_str().len() > 260);

        assert_eq!(
            verify_signature(&path, "Rockstar Games, Inc."),
            Err("it is not a Windows executable".to_owned())
        );
    }

    #[test]
    fn the_free_space_of_the_system_volume_is_known() {
        assert!(available_space(Path::new(r"C:\")).unwrap() > 0);
    }
}
