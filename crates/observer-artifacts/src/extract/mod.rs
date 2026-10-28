//! The rules both archive formats share. The archives come from Cfx.re over HTTPS, but they are
//! still read as hostile: nothing may land outside the build folder, overwrite another entry, or
//! grow past fixed bounds.

mod tar_xz;
mod zip;

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

pub(crate) use self::tar_xz::extract as extract_tar_xz;
pub(crate) use self::zip::{extract as extract_zip, plan as plan_zip};
use crate::error::{Error, io};
use crate::http::MIB;

const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED: u64 = 2048 * MIB;
const MAX_NAME: usize = 1024;

/// Why an archive entry was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    TooLong,
    Control,
    Backslash,
    Colon,
    TrailingDot,
    Device,
    Absolute,
    Parent,
    Duplicate,
    InsideFile,
    InsideLink,
    ZipLink,
    Encrypted,
    Method(String),
    EntryType(String),
    LinkOutsideAlpine,
    LinkEscapes,
    LinkLoop,
    TooManyEntries,
    TooLarge,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("has an empty name"),
            Self::TooLong => write!(f, "has a name longer than {MAX_NAME} bytes"),
            Self::Control => f.write_str("has a control character in its name"),
            Self::Backslash => f.write_str("has a backslash in its name"),
            Self::Colon => f.write_str("has a colon in its name"),
            Self::TrailingDot => f.write_str("has a part ending with a dot or a space, which Windows drops"),
            Self::Device => f.write_str("names a Windows device"),
            Self::Absolute => f.write_str("is an absolute path"),
            Self::Parent => f.write_str("climbs out of its folder with `..`"),
            Self::Duplicate => f.write_str("appears twice"),
            Self::InsideFile => f.write_str("is inside a path another entry writes as a file"),
            Self::InsideLink => f.write_str("is inside a symbolic link"),
            Self::ZipLink => f.write_str("is a symbolic link, which a Windows build never contains"),
            Self::Encrypted => f.write_str("is encrypted"),
            Self::Method(method) => write!(f, "is compressed with {method}, not stored or deflated"),
            Self::EntryType(kind) => write!(f, "is a {kind}, not a file, a folder or a symbolic link"),
            Self::LinkOutsideAlpine => f.write_str("links to an absolute path outside alpine/"),
            Self::LinkEscapes => f.write_str("links outside the build"),
            Self::LinkLoop => f.write_str("is part of a loop of symbolic links"),
            Self::TooManyEntries => write!(f, "is past the {MAX_ENTRIES} entries a build may have"),
            Self::TooLarge => write!(
                f,
                "takes the build past {} GiB unpacked",
                MAX_UNPACKED / 1024 / MIB
            ),
        }
    }
}

/// The error naming a refused entry. A name refused for its length keeps only its start.
pub(crate) fn refuse(entry: &str, refusal: Refusal) -> Error {
    let entry = if entry.len() > MAX_NAME {
        format!("{}...", &entry[..entry.floor_char_boundary(MAX_NAME)])
    } else {
        entry.to_owned()
    };
    Error::Unsafe { entry, refusal }
}

/// Checks an entry name and splits it into components. `.` and empty components are dropped, so
/// `./citizen//x` is `citizen/x`; no components at all is the archive's root.
pub(crate) fn components(name: &str) -> Result<Vec<&str>, Refusal> {
    check_characters(name)?;
    if name.starts_with('/') {
        return Err(Refusal::Absolute);
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(Refusal::Parent),
            part => parts.push(part),
        }
    }
    Ok(parts)
}

/// A colon would open an NTFS alternate stream or a drive, a backslash is a separator on Windows.
pub(crate) fn check_characters(name: &str) -> Result<(), Refusal> {
    if name.len() > MAX_NAME {
        Err(Refusal::TooLong)
    } else if name.chars().any(char::is_control) {
        Err(Refusal::Control)
    } else if name.contains('\\') {
        Err(Refusal::Backslash)
    } else if name.contains(':') {
        Err(Refusal::Colon)
    } else {
        Ok(())
    }
}

/// Counts entries and unpacked bytes against the bounds of a build.
#[derive(Debug, Default)]
pub(crate) struct Budget {
    entries: usize,
    bytes: u64,
}

impl Budget {
    pub fn take(&mut self, bytes: u64) -> Result<(), Refusal> {
        self.entries += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        if self.entries > MAX_ENTRIES {
            Err(Refusal::TooManyEntries)
        } else if self.bytes > MAX_UNPACKED {
            Err(Refusal::TooLarge)
        } else {
            Ok(())
        }
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Writes one entry into a new file, holding it to the size its header declared. A read error is
/// the archive's fault, a write error the disk's.
pub(crate) fn write_entry(name: &str, mut reader: impl Read, path: &Path, size: u64) -> Result<File, Error> {
    let mut file = File::create_new(path).map_err(io(path))?;
    let mut buffer = vec![0; 64 * 1024];
    let mut written = 0u64;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::Damaged(format!("{name}: {error}"))),
        };
        written += read as u64;
        if written > size {
            return Err(Error::Damaged(format!(
                "{name} is larger than its declared {size} bytes"
            )));
        }
        file.write_all(&buffer[..read]).map_err(io(path))?;
    }
    if written < size {
        return Err(Error::Damaged(format!(
            "{name} is truncated at {written} of {size} bytes"
        )));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_are_split_into_safe_components() {
        assert_eq!(
            components("citizen/scripting/lua/scheduler.lua"),
            Ok(vec!["citizen", "scripting", "lua", "scheduler.lua"])
        );
        assert_eq!(
            components("./alpine//opt/./cfx-server/"),
            Ok(vec!["alpine", "opt", "cfx-server"])
        );
        assert_eq!(components("./"), Ok(vec![]));
    }

    #[test]
    fn unsafe_entry_names_are_refused() {
        let long = "a/".repeat(600);
        for (name, refusal) in [
            ("/etc/passwd", Refusal::Absolute),
            ("citizen/../../evil.dll", Refusal::Parent),
            ("..", Refusal::Parent),
            ("citizen\\evil.dll", Refusal::Backslash),
            ("C:/Windows/evil.dll", Refusal::Colon),
            ("FXServer.exe:stream", Refusal::Colon),
            ("bad\u{7}name", Refusal::Control),
            ("line\nbreak", Refusal::Control),
            (long.as_str(), Refusal::TooLong),
        ] {
            assert_eq!(components(name), Err(refusal), "{name:?}");
        }
    }

    #[test]
    fn a_refused_name_is_printed_with_its_control_characters_escaped() {
        let error = refuse(
            "citizen/\u{1b}]0;owned\u{7}\u{1b}[2K\rinstalled.lua",
            Refusal::Control,
        );

        assert_eq!(
            error.to_string(),
            r"the archive entry `citizen/\u{1b}]0;owned\u{7}\u{1b}[2K\rinstalled.lua` has a control character in its name"
        );
    }

    #[test]
    fn a_name_refused_for_its_length_keeps_only_its_start() {
        let name = "é".repeat(1 << 20);

        let Error::Unsafe { entry, .. } = refuse(&name, Refusal::TooLong) else {
            panic!("a refusal is an unsafe entry");
        };

        assert!(entry.len() <= MAX_NAME + 3, "{}", entry.len());
        assert!(name.starts_with(entry.trim_end_matches("...")));
    }

    #[test]
    fn the_budget_stops_at_twenty_thousand_entries_or_two_gibibytes() {
        let mut entries = Budget::default();
        for _ in 0..MAX_ENTRIES {
            entries.take(0).unwrap();
        }
        assert_eq!(entries.take(0), Err(Refusal::TooManyEntries));

        let mut bytes = Budget::default();
        bytes.take(MAX_UNPACKED).unwrap();
        assert_eq!(bytes.take(1), Err(Refusal::TooLarge));
    }
}
