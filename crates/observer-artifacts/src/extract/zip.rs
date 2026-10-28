//! `server.zip`, the Windows archive. Its central directory lists every entry up front, so the whole
//! archive is checked before anything is written.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use zip::result::ZipError;
use zip::{CompressionMethod, ZipArchive};

use super::{Budget, Refusal, components, refuse, write_entry};
use crate::error::{Error, io};

/// A `server.zip` whose every entry passed the checks, and what it will take on disk.
#[derive(Debug)]
pub(crate) struct Plan {
    entries: Vec<Planned>,
    pub unpacked: u64,
}

#[derive(Debug)]
struct Planned {
    index: usize,
    name: String,
    /// The path in lower case, as Windows compares it.
    key: String,
    path: PathBuf,
    /// `None` for a folder.
    size: Option<u64>,
}

pub(crate) fn plan(archive: &Path) -> Result<Plan, Error> {
    let mut zip = open(archive)?;
    let mut budget = Budget::default();
    let mut entries = Vec::new();
    // Windows paths ignore case, so `Citizen/x` and `citizen/x` are the same file there.
    let mut taken = HashSet::new();
    let mut files = HashSet::new();
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|error| damaged(&error))?;
        let name = entry.name().map_err(|error| damaged(&error))?.into_owned();
        let refuse = |refusal| refuse(&name, refusal);
        budget.take(entry.size()).map_err(refuse)?;
        let parts = components(&name).map_err(refuse)?;
        if entry.encrypted() {
            return Err(refuse(Refusal::Encrypted));
        }
        if entry.is_symlink() {
            return Err(refuse(Refusal::ZipLink));
        }
        let method = entry.compression();
        if !matches!(method, CompressionMethod::Stored | CompressionMethod::Deflated) {
            return Err(refuse(Refusal::Method(format!("{method:?}"))));
        }
        let folder = entry.is_dir();
        if parts.is_empty() {
            if folder {
                continue;
            }
            return Err(refuse(Refusal::Empty));
        }
        let key = parts.join("/").to_lowercase();
        if !taken.insert(key.clone()) {
            return Err(refuse(Refusal::Duplicate));
        }
        if !folder {
            files.insert(key.clone());
        }
        entries.push(Planned {
            index,
            key,
            path: parts.iter().collect(),
            size: (!folder).then(|| entry.size()),
            name,
        });
    }
    for planned in &entries {
        let key = &planned.key;
        let mut ancestors = key.match_indices('/').map(|(at, _)| &key[..at]);
        if ancestors.any(|ancestor| files.contains(ancestor)) {
            return Err(refuse(&planned.name, Refusal::InsideFile));
        }
    }
    Ok(Plan {
        entries,
        unpacked: budget.bytes(),
    })
}

/// Writes a planned archive into `into`, which must not exist yet.
pub(crate) fn extract(archive: &Path, plan: &Plan, into: &Path) -> Result<(), Error> {
    let mut zip = open(archive)?;
    fs::create_dir(into).map_err(io(into))?;
    for planned in &plan.entries {
        let path = into.join(&planned.path);
        let Some(size) = planned.size else {
            fs::create_dir_all(&path).map_err(io(&path))?;
            continue;
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        // Reading the entry to its end is what makes the zip crate compare its CRC-32.
        let entry = zip.by_index(planned.index).map_err(|error| damaged(&error))?;
        let file = write_entry(&planned.name, entry, &path, size)?;
        file.sync_all().map_err(io(&path))?;
    }
    Ok(())
}

fn open(archive: &Path) -> Result<ZipArchive<BufReader<File>>, Error> {
    let file = File::open(archive).map_err(io(archive))?;
    ZipArchive::new(BufReader::new(file)).map_err(|error| damaged(&error))
}

fn damaged(error: &ZipError) -> Error {
    Error::Damaged(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples::{ZipEntry, patch_zip_entry, zip_archive};

    fn planned(entries: &[ZipEntry]) -> Result<Plan, Error> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip");
        fs::write(&path, zip_archive(entries)).unwrap();
        plan(&path)
    }

    fn refusal(result: Result<Plan, Error>) -> (String, Refusal) {
        match result {
            Err(Error::Unsafe { entry, refusal }) => (entry, refusal),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_windows_build_is_extracted_with_its_folders() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("server.zip");
        fs::write(
            &archive,
            zip_archive(&[
                ZipEntry::Folder("citizen/"),
                ZipEntry::Deflated("citizen/scripting/lua/scheduler.lua", b"-- scheduler"),
                ZipEntry::Stored("FXServer.exe", b"MZ fake"),
            ]),
        )
        .unwrap();
        let into = dir.path().join("tree");

        let plan = plan(&archive).unwrap();
        extract(&archive, &plan, &into).unwrap();

        assert_eq!(plan.unpacked, 19);
        assert_eq!(fs::read(into.join("FXServer.exe")).unwrap(), b"MZ fake");
        assert_eq!(
            fs::read(into.join("citizen/scripting/lua/scheduler.lua")).unwrap(),
            b"-- scheduler"
        );
    }

    #[test]
    fn entries_escaping_the_build_are_refused() {
        for (name, expected) in [
            ("../evil.dll", Refusal::Parent),
            ("/evil.dll", Refusal::Absolute),
            ("citizen\\..\\..\\evil.dll", Refusal::Backslash),
            ("C:/evil.dll", Refusal::Colon),
            ("FXServer.exe:hidden", Refusal::Colon),
            ("evil\u{1}.dll", Refusal::Control),
        ] {
            let (entry, refusal) = refusal(planned(&[ZipEntry::Stored(name, b"x")]));
            assert_eq!((entry.as_str(), refusal), (name, expected));
        }
    }

    #[test]
    fn entries_that_differ_only_in_case_are_duplicates() {
        let result = planned(&[
            ZipEntry::Stored("citizen/a.lua", b"1"),
            ZipEntry::Stored("Citizen/A.lua", b"2"),
        ]);

        assert_eq!(refusal(result), ("Citizen/A.lua".to_owned(), Refusal::Duplicate));
    }

    #[test]
    fn an_entry_inside_a_file_is_refused() {
        let result = planned(&[
            ZipEntry::Stored("citizen", b"file"),
            ZipEntry::Stored("citizen/a.lua", b"x"),
        ]);

        assert_eq!(refusal(result), ("citizen/a.lua".to_owned(), Refusal::InsideFile));
    }

    #[test]
    fn a_symbolic_link_is_refused() {
        let result = planned(&[ZipEntry::Link("FXServer.exe", "C:/Windows/System32/cmd.exe")]);

        assert_eq!(refusal(result), ("FXServer.exe".to_owned(), Refusal::ZipLink));
    }

    #[test]
    fn methods_other_than_stored_and_deflated_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip");
        let mut bytes = zip_archive(&[ZipEntry::Stored("FXServer.exe", b"MZ")]);
        // 12 is bzip2.
        patch_zip_entry(&mut bytes, |header, method_at, _| {
            header[method_at..method_at + 2].copy_from_slice(&12u16.to_le_bytes());
        });
        fs::write(&path, bytes).unwrap();

        let (_, refused) = refusal(plan(&path));

        assert!(matches!(refused, Refusal::Method(_)), "{refused}");
    }

    #[test]
    fn an_encrypted_entry_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip");
        let mut bytes = zip_archive(&[ZipEntry::Stored("FXServer.exe", b"MZ")]);
        patch_zip_entry(&mut bytes, |header, method_at, _| header[method_at - 2] |= 1);
        fs::write(&path, bytes).unwrap();

        assert_eq!(
            refusal(plan(&path)),
            ("FXServer.exe".to_owned(), Refusal::Encrypted)
        );
    }

    #[test]
    fn an_archive_declaring_more_than_two_gibibytes_is_refused_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip");
        let mut bytes = zip_archive(&[ZipEntry::Stored("citizen/huge.rpf", b"x")]);
        patch_zip_entry(&mut bytes, |header, _, size_at| {
            header[size_at..size_at + 4].copy_from_slice(&0xC000_0000u32.to_le_bytes());
        });
        fs::write(&path, bytes).unwrap();

        assert_eq!(
            refusal(plan(&path)),
            ("citizen/huge.rpf".to_owned(), Refusal::TooLarge)
        );
    }

    #[test]
    fn an_archive_past_twenty_thousand_entries_is_refused() {
        let names: Vec<String> = (0..20_001).map(|index| format!("citizen/{index}")).collect();
        let entries: Vec<ZipEntry> = names.iter().map(|name| ZipEntry::Stored(name, b"")).collect();

        assert_eq!(
            refusal(planned(&entries)),
            ("citizen/20000".to_owned(), Refusal::TooManyEntries)
        );
    }

    #[test]
    fn an_entry_whose_content_does_not_match_its_checksum_is_damaged() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("server.zip");
        let mut bytes = zip_archive(&[ZipEntry::Stored("FXServer.exe", b"MZ original")]);
        let at = bytes
            .windows(11)
            .position(|window| window == b"MZ original")
            .unwrap();
        bytes[at + 3] = b'X';
        fs::write(&archive, bytes).unwrap();
        let into = dir.path().join("tree");

        let plan = plan(&archive).unwrap();
        let error = extract(&archive, &plan, &into).unwrap_err();

        assert!(matches!(error, Error::Damaged(_)), "{error}");
    }

    #[test]
    fn a_truncated_archive_is_damaged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip");
        let bytes = zip_archive(&[ZipEntry::Stored("FXServer.exe", b"MZ")]);
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();

        assert!(matches!(plan(&path), Err(Error::Damaged(_))));
    }
}
