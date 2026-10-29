//! `fx.tar.xz`, the Linux archive: an Alpine root file system next to `run.sh`. A tar has no index,
//! so entries are checked as they stream into the staging folder; symbolic links wait until the end,
//! when nothing more can be written through them, and are confined before they are created.
//!
//! The Alpine links are absolute (`alpine/bin/kill -> /bin/busybox`) because Alpine expects to be
//! the root. FXServer runs it in place instead, so such a link would point into the host: it is
//! rewritten relative to `alpine/` (`busybox`), which is what the musl loader then finds.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek};
use std::path::Path;

use lzma_rust2::XzReader;
use tar::EntryType;

use super::{
    Budget, MAX_UNPACKED, Refusal, Track, check_characters, components, confine, refuse, write_entry,
};
use crate::disk::{self, SPACE_MARGIN};
use crate::error::{Error, io};
use crate::http::MIB;

const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const BLOCK: u64 = 512;
/// What the headers of an entry may take, beyond the data of the entry before it. FXServer's
/// longest is a GNU long name of 119 bytes.
const HEADER_ROOM: u64 = 64 * 1024;
/// The whole tar stream: the data a build may have, with its headers and padding.
const MAX_STREAM: u64 = MAX_UNPACKED + 64 * MIB;

/// Extracts into `into`, which must not exist yet. A damaged stream can fail halfway, after some
/// files are written: `into` is always a staging folder for that reason.
///
/// `space` is what the volume was found to have free for the files. A tar does not say what it
/// unpacks to, so once the files pass that, the volume is checked again, for as much more.
pub(crate) fn extract(archive: &Path, into: &Path, space: u64, track: &Track) -> Result<(), Error> {
    let mut file = File::open(archive).map_err(io(archive))?;
    let mut magic = [0; 6];
    if file.read_exact(&mut magic).is_err() || magic != XZ_MAGIC {
        return Err(Error::Damaged("it is not an xz archive".to_owned()));
    }
    file.rewind().map_err(io(archive))?;

    let room = Cell::new(HEADER_ROOM);
    let mut tar = tar::Archive::new(Bounded {
        inner: XzReader::new(BufReader::new(file), true),
        room: &room,
        read: 0,
    });
    fs::create_dir(into).map_err(io(into))?;
    let mut tree = Tree::default();
    let mut budget = Budget::default();
    let mut checked = space;
    for entry in tar.entries().map_err(|error| damaged(&error))? {
        let mut entry = entry.map_err(|error| damaged(&error))?;
        let kind = entry.header().entry_type();
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let refuse = |refusal| refuse(&name, refusal);
        // A pax size wins over the header's, and is what `tar` reads or skips.
        let size = entry.size();
        budget.take(size).map_err(refuse)?;
        room.set(size.next_multiple_of(BLOCK) + HEADER_ROOM);
        if kind == EntryType::XGlobalHeader {
            continue;
        }
        let parts = components(&name).map_err(refuse)?;
        if parts.is_empty() && kind.is_dir() {
            continue;
        }
        if parts.is_empty() {
            return Err(refuse(Refusal::Empty));
        }
        track.entry()?;
        let path = parts.join("/");
        let target = into.join(&path);
        match kind {
            EntryType::Regular | EntryType::Continuous => {
                tree.claim(&path, Claim::File).map_err(refuse)?;
                if budget.bytes() > checked {
                    disk::require_space(into, size.saturating_add(space).saturating_add(SPACE_MARGIN))?;
                    checked = budget.bytes().saturating_add(space);
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(io(parent))?;
                }
                let mode = entry.header().mode().map_err(|error| damaged(&error))?;
                let file = write_entry(&name, &mut entry, &target, size)?;
                keep_mode(&file, &target, mode)?;
            }
            EntryType::Directory => {
                tree.claim(&path, Claim::Folder).map_err(refuse)?;
                fs::create_dir_all(&target).map_err(io(&target))?;
            }
            EntryType::Symlink => {
                let link = entry
                    .link_name_bytes()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default();
                check_characters(&link).map_err(refuse)?;
                if link.is_empty() {
                    return Err(refuse(Refusal::Empty));
                }
                tree.claim(&path, Claim::Link).map_err(refuse)?;
                tree.links.insert(path, link);
            }
            other => return Err(refuse(Refusal::EntryType(describe(other)))),
        }
    }
    // The tar ends before the xz stream does: only reading the rest checks the last block's CRC,
    // the index and the footer, the one guard a Linux build has against a damaged download. What
    // follows the end is padding, 512 bytes in FXServer's archives.
    room.set(HEADER_ROOM);
    io::copy(&mut tar.into_inner(), &mut io::sink()).map_err(|error| damaged(&error))?;

    for (path, link) in confine(&tree.links)? {
        create_link(&link, &into.join(path))?;
    }
    Ok(())
}

/// Holds `tar` to the room the entries seen so far leave it. It reads GNU long names and pax
/// headers whole into memory before it hands over the entry they describe, and skips the data of
/// entries nobody reads: unbounded, one header declaring gigabytes would be decompressed, and kept,
/// before any check could refuse it.
struct Bounded<'a, R> {
    inner: R,
    /// What may still be read before the next entry comes out, set again for each entry.
    room: &'a Cell<u64>,
    read: u64,
}

impl<R: Read> Read for Bounded<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let room = self.room.get().min(MAX_STREAM - self.read);
        if room == 0 && !buffer.is_empty() {
            return Err(io::Error::other(if self.read == MAX_STREAM {
                format!("it unpacks to more than {} GiB", MAX_STREAM / 1024 / MIB)
            } else {
                format!("it has headers longer than {} KiB", HEADER_ROOM / 1024)
            }));
        }
        let wanted = buffer.len().min(usize::try_from(room).unwrap_or(usize::MAX));
        let read = self.inner.read(&mut buffer[..wanted])?;
        self.room.set(self.room.get() - read as u64);
        self.read += read as u64;
        Ok(read)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    File,
    Folder,
    Link,
}

/// Every path the archive has used so far, to refuse duplicates and writes through files or links.
#[derive(Debug, Default)]
struct Tree {
    claimed: HashSet<String>,
    /// Folders that other entries were written into, named or not.
    parents: HashSet<String>,
    files: HashSet<String>,
    links: HashMap<String, String>,
}

impl Tree {
    fn claim(&mut self, path: &str, claim: Claim) -> Result<(), Refusal> {
        if !self.claimed.insert(path.to_owned()) || (claim != Claim::Folder && self.parents.contains(path)) {
            return Err(Refusal::Duplicate);
        }
        for (at, _) in path.match_indices('/') {
            let ancestor = &path[..at];
            if self.files.contains(ancestor) {
                return Err(Refusal::InsideFile);
            }
            if self.links.contains_key(ancestor) {
                return Err(Refusal::InsideLink);
            }
            self.parents.insert(ancestor.to_owned());
        }
        if claim == Claim::File {
            self.files.insert(path.to_owned());
        }
        Ok(())
    }
}

/// Files keep their executable bits, owner read and write are always granted, and nothing becomes
/// writable by others than the owner.
#[cfg(unix)]
pub(crate) fn keep_mode(file: &File, path: &Path, mode: u32) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode((mode & 0o755) | 0o600))
        .map_err(io(path))?;
    file.sync_all().map_err(io(path))
}

#[cfg(not(unix))]
pub(crate) fn keep_mode(file: &File, path: &Path, _mode: u32) -> Result<(), Error> {
    file.sync_all().map_err(io(path))
}

#[cfg(unix)]
pub(crate) fn create_link(target: &str, path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    std::os::unix::fs::symlink(target, path).map_err(io(path))
}

#[cfg(not(unix))]
pub(crate) fn create_link(_target: &str, _path: &Path) -> Result<(), Error> {
    Err(Error::WrongPlatform(crate::Platform::Linux))
}

fn describe(kind: EntryType) -> String {
    match kind {
        EntryType::Link => "hard link".to_owned(),
        EntryType::Char => "character device".to_owned(),
        EntryType::Block => "block device".to_owned(),
        EntryType::Fifo => "named pipe".to_owned(),
        other => format!("tar entry of type {:?}", char::from(other.as_byte())),
    }
}

fn damaged(error: &io::Error) -> Error {
    Error::Damaged(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples::{TarEntry, noise, tar_xz};

    /// Free space found on the volume for the files, as much as the tests could need.
    const PLENTY: u64 = u64::MAX;

    fn extracted(entries: &[TarEntry]) -> (tempfile::TempDir, Result<(), Error>) {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("fx.tar.xz");
        fs::write(&archive, tar_xz(entries)).unwrap();
        let result = extract(&archive, &dir.path().join("tree"), PLENTY, &Track::default());
        (dir, result)
    }

    fn refusal(entries: &[TarEntry]) -> (String, Refusal) {
        match extracted(entries).1 {
            Err(Error::Unsafe { entry, refusal }) => (entry, refusal),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn confined(links: &[(&str, &str)]) -> Result<Vec<(String, String)>, Error> {
        let links = links
            .iter()
            .map(|(path, target)| ((*path).to_owned(), (*target).to_owned()))
            .collect();
        confine(&links)
    }

    #[test]
    fn absolute_alpine_links_are_rewritten_inside_the_rootfs() {
        let links = confined(&[
            ("alpine/bin/kill", "/bin/busybox"),
            (
                "alpine/lib/ld-linux-x86-64.so.2",
                "/usr/glibc-compat/lib/ld-linux-x86-64.so.2",
            ),
            (
                "alpine/etc/ssl/certs/ca.pem",
                "/usr/share/ca-certificates/mozilla/ca.crt",
            ),
            ("alpine/root", "/"),
            ("alpine/etc/mtab", "/../../proc/mounts"),
        ])
        .unwrap();

        assert_eq!(
            links,
            [
                ("alpine/bin/kill".to_owned(), "busybox".to_owned()),
                ("alpine/etc/mtab".to_owned(), "../proc/mounts".to_owned()),
                (
                    "alpine/etc/ssl/certs/ca.pem".to_owned(),
                    "../../../usr/share/ca-certificates/mozilla/ca.crt".to_owned()
                ),
                (
                    "alpine/lib/ld-linux-x86-64.so.2".to_owned(),
                    "../usr/glibc-compat/lib/ld-linux-x86-64.so.2".to_owned()
                ),
                ("alpine/root".to_owned(), ".".to_owned()),
            ]
        );
    }

    #[test]
    fn relative_links_inside_the_build_are_kept() {
        let links = confined(&[
            ("alpine/usr/lib/libstdc++.so.6", "libstdc++.so.6.0.33"),
            ("alpine/opt/cfx-server/run.sh", "../../../run.sh"),
        ])
        .unwrap();

        assert_eq!(links[0].1, "../../../run.sh");
        assert_eq!(links[1].1, "libstdc++.so.6.0.33");
    }

    #[test]
    fn a_link_climbing_out_of_the_build_is_refused() {
        let error = confined(&[("alpine/escape", "../../outside")]).unwrap_err();

        assert!(
            matches!(
                error,
                Error::Unsafe {
                    refusal: Refusal::LinkEscapes,
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_chain_of_links_escaping_together_is_refused() {
        // Each link alone stays inside; followed through `a`, `b` leaves the build.
        let error = confined(&[("alpine/a", ".."), ("alpine/b", "a/..")]).unwrap_err();

        assert!(
            matches!(&error, Error::Unsafe { entry, refusal: Refusal::LinkEscapes } if entry == "alpine/b"),
            "{error}"
        );
    }

    #[test]
    fn a_loop_of_links_is_refused() {
        let error = confined(&[("alpine/a", "b"), ("alpine/b", "a")]).unwrap_err();

        assert!(
            matches!(
                error,
                Error::Unsafe {
                    refusal: Refusal::LinkLoop,
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn an_absolute_link_outside_alpine_is_refused() {
        let error = confined(&[("run.sh", "/bin/sh")]).unwrap_err();

        assert!(
            matches!(
                error,
                Error::Unsafe {
                    refusal: Refusal::LinkOutsideAlpine,
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn an_entry_under_a_link_is_refused() {
        let refused = refusal(&[
            TarEntry::Link("alpine/etc", "/tmp"),
            TarEntry::File("alpine/etc/passwd", b"root::0:0::/:/bin/sh", 0o644),
        ]);

        assert_eq!(refused, ("alpine/etc/passwd".to_owned(), Refusal::InsideLink));
    }

    #[test]
    fn a_link_over_a_folder_already_written_is_refused() {
        let refused = refusal(&[
            TarEntry::File("alpine/etc/passwd", b"root::0:0::/:/bin/sh", 0o644),
            TarEntry::Link("alpine/etc", "/tmp"),
        ]);

        assert_eq!(refused, ("alpine/etc".to_owned(), Refusal::Duplicate));
    }

    #[test]
    fn a_hard_link_is_refused() {
        let refused = refusal(&[TarEntry::HardLink("alpine/bin/sh", "alpine/bin/busybox")]);

        assert_eq!(
            refused,
            (
                "alpine/bin/sh".to_owned(),
                Refusal::EntryType("hard link".to_owned())
            )
        );
    }

    #[test]
    fn a_device_is_refused() {
        let refused = refusal(&[TarEntry::Fifo("alpine/dev/pipe")]);

        assert_eq!(refused.1, Refusal::EntryType("named pipe".to_owned()));
    }

    #[test]
    fn entries_escaping_the_build_are_refused() {
        for (name, expected) in [
            ("../../etc/cron.d/evil", Refusal::Parent),
            ("/etc/cron.d/evil", Refusal::Absolute),
            ("alpine\\evil", Refusal::Backslash),
        ] {
            assert_eq!(
                refusal(&[TarEntry::File(name, b"x", 0o644)]),
                (name.to_owned(), expected)
            );
        }
    }

    #[test]
    fn an_entry_written_twice_is_refused() {
        let refused = refusal(&[
            TarEntry::File("run.sh", b"#!/bin/sh", 0o755),
            TarEntry::File("./run.sh", b"#!/bin/sh evil", 0o755),
        ]);

        assert_eq!(refused, ("./run.sh".to_owned(), Refusal::Duplicate));
    }

    #[test]
    fn an_entry_declaring_more_than_two_gibibytes_is_refused_before_reading_it() {
        let refused = refusal(&[TarEntry::Declared("alpine/huge", 3 << 30)]);

        assert_eq!(refused, ("alpine/huge".to_owned(), Refusal::TooLarge));
    }

    #[test]
    fn a_pax_global_header_counts_against_the_size_limit() {
        let refused = refusal(&[TarEntry::Global(3 << 30)]);

        assert_eq!(refused, ("pax_global_header".to_owned(), Refusal::TooLarge));
    }

    #[test]
    fn a_gnu_long_name_names_the_entry_after_it() {
        let name = format!(
            "alpine/opt/cfx-server/citizen/system_resources/{}.lua",
            "a".repeat(120)
        );
        let (dir, result) = extracted(&[
            TarEntry::LongName(name.as_bytes()),
            TarEntry::File("placeholder", b"--", 0o644),
        ]);
        result.unwrap();

        assert_eq!(fs::read(dir.path().join("tree").join(&name)).unwrap(), b"--");
    }

    #[test]
    fn a_long_name_header_larger_than_any_name_is_refused_before_it_is_read() {
        let name = vec![b'a'; 1 << 20];

        let (_dir, result) = extracted(&[
            TarEntry::LongName(&name),
            TarEntry::File("placeholder", b"", 0o644),
        ]);

        let error = result.unwrap_err();
        assert!(matches!(error, Error::Damaged(_)), "{:.200}", error.to_string());
        assert!(error.to_string().len() < 200, "{:.200}", error.to_string());
    }

    #[test]
    fn a_file_that_is_not_xz_is_damaged() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("fx.tar.xz");
        fs::write(&archive, b"<html>maintenance</html>").unwrap();

        let error = extract(&archive, &dir.path().join("tree"), PLENTY, &Track::default()).unwrap_err();

        assert!(matches!(error, Error::Damaged(_)), "{error}");
    }

    #[test]
    fn a_truncated_stream_is_damaged() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("fx.tar.xz");
        let bytes = tar_xz(&[TarEntry::File(
            "alpine/opt/cfx-server/FXServer",
            &noise(300_000),
            0o755,
        )]);
        fs::write(&archive, &bytes[..bytes.len() / 2]).unwrap();

        let error = extract(&archive, &dir.path().join("tree"), PLENTY, &Track::default()).unwrap_err();

        assert!(matches!(error, Error::Damaged(_)), "{error}");
    }

    fn extracted_bytes(bytes: &[u8]) -> Result<(), Error> {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("fx.tar.xz");
        fs::write(&archive, bytes).unwrap();
        extract(&archive, &dir.path().join("tree"), PLENTY, &Track::default())
    }

    #[test]
    fn a_byte_changed_in_the_last_block_is_damaged() {
        // Noise does not compress, so LZMA2 stores the chunks in the middle of it as they are and
        // the changed byte reaches the file; only the block's CRC64, at the end of the stream, tells.
        let data = noise(300_000);
        let mut bytes = tar_xz(&[TarEntry::File("alpine/opt/cfx-server/FXServer", &data, 0o755)]);
        let at = bytes
            .windows(64)
            .position(|window| window == &data[150_000..150_064])
            .unwrap();
        bytes[at] ^= 1;

        let error = extracted_bytes(&bytes).unwrap_err();

        assert!(matches!(error, Error::Damaged(_)), "{error}");
    }

    #[test]
    fn a_broken_xz_footer_is_damaged() {
        let mut bytes = tar_xz(&[TarEntry::File("run.sh", b"#!/bin/bash", 0o755)]);
        let end = bytes.len();
        bytes[end - 2..].copy_from_slice(b"XX");

        let error = extracted_bytes(&bytes).unwrap_err();

        assert!(matches!(error, Error::Damaged(_)), "{error}");
    }

    #[test]
    fn files_past_the_space_found_free_get_the_volume_checked_again() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("fx.tar.xz");
        fs::write(
            &archive,
            tar_xz(&[
                TarEntry::File("run.sh", b"#!/bin/bash", 0o755),
                TarEntry::File("alpine/opt/cfx-server/FXServer", &noise(4096), 0o755),
            ]),
        )
        .unwrap();

        // Two bytes were found free: both files are past them, and the volume has room for more.
        extract(&archive, &dir.path().join("tree"), 2, &Track::default()).unwrap();

        assert_eq!(fs::read(dir.path().join("tree/run.sh")).unwrap(), b"#!/bin/bash");
    }

    #[cfg(unix)]
    #[test]
    fn files_keep_their_executable_bits_and_links_are_created_confined() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, result) = extracted(&[
            TarEntry::Folder("./"),
            TarEntry::File("run.sh", b"#!/bin/bash", 0o777),
            TarEntry::File("alpine/bin/busybox", b"\x7fELF", 0o755),
            TarEntry::Link("alpine/bin/kill", "/bin/busybox"),
            TarEntry::File("alpine/etc/motd", b"hi", 0o666),
        ]);
        result.unwrap();
        let tree = dir.path().join("tree");

        let mode = |path: &str| fs::metadata(tree.join(path)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("run.sh"), 0o755);
        assert_eq!(mode("alpine/bin/busybox"), 0o755);
        assert_eq!(mode("alpine/etc/motd"), 0o644);
        assert_eq!(
            fs::read_link(tree.join("alpine/bin/kill")).unwrap(),
            Path::new("busybox")
        );
        assert_eq!(fs::read(tree.join("alpine/bin/kill")).unwrap(), b"\x7fELF");
    }
}
