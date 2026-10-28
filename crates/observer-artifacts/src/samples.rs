//! Small archives crafted for tests, including the malformed ones no well-behaved writer produces.

use std::io::{Cursor, Write};

use lzma_rust2::{XzOptions, XzWriter};
use tar::EntryType;
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::Platform;

pub(crate) enum ZipEntry<'a> {
    Stored(&'a str, &'a [u8]),
    Deflated(&'a str, &'a [u8]),
    Folder(&'a str),
    Link(&'a str, &'a str),
}

pub(crate) fn zip_archive(entries: &[ZipEntry]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for entry in entries {
        match entry {
            ZipEntry::Stored(name, data) => {
                writer.start_file(*name, stored).unwrap();
                writer.write_all(data).unwrap();
            }
            ZipEntry::Deflated(name, data) => {
                writer.start_file(*name, deflated).unwrap();
                writer.write_all(data).unwrap();
            }
            ZipEntry::Folder(name) => writer.add_directory(*name, stored).unwrap(),
            ZipEntry::Link(name, target) => writer.add_symlink(*name, *target, stored).unwrap(),
        }
    }
    writer.finish().unwrap().into_inner()
}

/// Calls `patch` on every local and central header of a zip, with the offsets of the compression
/// method and of the uncompressed size in that header. The flags sit just before the method in both.
pub(crate) fn patch_zip_entry(bytes: &mut [u8], patch: impl Fn(&mut [u8], usize, usize)) {
    let headers: Vec<(usize, usize, usize)> = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(at, window)| match window {
            b"PK\x03\x04" => Some((at, 8, 22)),
            b"PK\x01\x02" => Some((at, 10, 24)),
            _ => None,
        })
        .collect();
    for (at, method, size) in headers {
        patch(&mut bytes[at..], method, size);
    }
}

pub(crate) enum TarEntry<'a> {
    File(&'a str, &'a [u8], u32),
    Folder(&'a str),
    Link(&'a str, &'a str),
    HardLink(&'a str, &'a str),
    Fifo(&'a str),
    /// A file header declaring this many bytes, with none following it.
    Declared(&'a str, u64),
    /// A GNU long name header, which names the entry after it.
    LongName(&'a [u8]),
    /// A pax global header declaring this many bytes, with none following it.
    Global(u64),
}

/// Writes the names straight into the headers: `tar`'s own setters refuse `..` and absolute paths,
/// which are exactly what the tests need.
pub(crate) fn tar_xz(entries: &[TarEntry]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for entry in entries {
        let (name, kind, data, mode, link): (&str, EntryType, &[u8], u32, Option<&str>) = match entry {
            TarEntry::File(name, data, mode) => (name, EntryType::Regular, data, *mode, None),
            TarEntry::Folder(name) => (name, EntryType::Directory, &[], 0o755, None),
            TarEntry::Link(name, target) => (name, EntryType::Symlink, &[], 0o777, Some(target)),
            TarEntry::HardLink(name, target) => (name, EntryType::Link, &[], 0o755, Some(target)),
            TarEntry::Fifo(name) => (name, EntryType::Fifo, &[], 0o644, None),
            TarEntry::Declared(name, _) => (name, EntryType::Regular, &[], 0o644, None),
            TarEntry::LongName(name) => ("././@LongLink", EntryType::GNULongName, name, 0o644, None),
            TarEntry::Global(_) => ("pax_global_header", EntryType::XGlobalHeader, &[], 0o644, None),
        };
        let mut header = tar::Header::new_gnu();
        let old = header.as_old_mut();
        old.name[..name.len()].copy_from_slice(name.as_bytes());
        if let Some(link) = link {
            old.linkname[..link.len()].copy_from_slice(link.as_bytes());
        }
        header.set_entry_type(kind);
        header.set_mode(mode);
        header.set_size(match entry {
            TarEntry::Declared(_, size) | TarEntry::Global(size) => *size,
            _ => data.len() as u64,
        });
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut writer = XzWriter::new(Vec::new(), XzOptions::with_preset(1)).unwrap();
    writer.write_all(&tar).unwrap();
    writer.finish().unwrap()
}

/// Bytes that do not compress, so a cut in the compressed stream lands inside the data.
pub(crate) fn noise(length: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

/// The smallest archive that passes the layout check of the host's platform. Its `FXServer.exe` is
/// not signed.
pub(crate) fn host_build() -> Vec<u8> {
    match Platform::host() {
        Platform::Windows => zip_archive(&[
            ZipEntry::Stored("FXServer.exe", b"MZ not really"),
            ZipEntry::Deflated("components.json", b"[]"),
            ZipEntry::Deflated("citizen/scripting/lua/scheduler.lua", b"-- scheduler"),
        ]),
        Platform::Linux => tar_xz(&[
            TarEntry::Folder("./"),
            TarEntry::File("run.sh", b"#!/bin/bash\n", 0o777),
            TarEntry::File("alpine/opt/cfx-server/FXServer", b"\x7fELF", 0o755),
            TarEntry::File("alpine/opt/cfx-server/ld-musl-x86_64.so.1", b"\x7fELF", 0o755),
            TarEntry::File(
                "alpine/opt/cfx-server/citizen/scripting/lua/scheduler.lua",
                b"--",
                0o644,
            ),
            TarEntry::File("alpine/bin/busybox", b"\x7fELF", 0o755),
            TarEntry::Link("alpine/bin/kill", "/bin/busybox"),
        ]),
    }
}
