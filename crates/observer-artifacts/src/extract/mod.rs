//! The rules both archive formats share. The archives come from Cfx.re over HTTPS, but they are
//! still read as hostile: nothing may land outside the build folder, overwrite another entry, or
//! grow past fixed bounds.

mod tar_xz;
mod zip;

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio_util::sync::CancellationToken;

pub(crate) use self::tar_xz::create_link;
pub(crate) use self::tar_xz::extract as extract_tar_xz;
pub(crate) use self::zip::{Plan, extract as extract_zip, plan as plan_zip};
use crate::error::{Error, io};
use crate::http::MIB;

pub(crate) const MAX_ENTRIES: usize = 20_000;
pub(crate) const MAX_UNPACKED: u64 = 2048 * MIB;
const MAX_NAME: usize = 1024;
/// What Linux allows when it resolves a path.
const MAX_LINK_HOPS: usize = 40;
const ALPINE: &str = "alpine";

/// What an extraction or a copy reports as it goes, and what stops it: it counts the entries it
/// wrote and checks for a cancellation before each.
#[derive(Debug, Clone, Default)]
pub(crate) struct Track {
    pub cancel: CancellationToken,
    pub done: Arc<AtomicU64>,
}

impl Track {
    pub fn entry(&self) -> Result<(), Error> {
        if self.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.done.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

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

/// Rewrites absolute links under `alpine/` relative to it, then follows every link through the
/// others the way the kernel would, to prove that none leads outside the build. Returns the links
/// to create, with the targets to give them.
pub(crate) fn confine(links: &HashMap<String, String>) -> Result<Vec<(String, String)>, Error> {
    let mut relative = HashMap::with_capacity(links.len());
    for (path, link) in links {
        let target = if link.starts_with('/') {
            if !path.starts_with("alpine/") {
                return Err(refuse(path, Refusal::LinkOutsideAlpine));
            }
            inside_alpine(path, link)
        } else {
            link.clone()
        };
        relative.insert(path.clone(), target);
    }
    for path in relative.keys() {
        resolve(path, &relative).map_err(|refusal| refuse(path, refusal))?;
    }
    let mut ordered: Vec<(String, String)> = relative.into_iter().collect();
    ordered.sort();
    Ok(ordered)
}

/// The relative target that reaches, from the link's folder, what `link` names inside `alpine/`.
/// As at the root of a real system, `..` above `alpine/` stays there.
fn inside_alpine(path: &str, link: &str) -> String {
    let mut target = vec![ALPINE];
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if target.len() > 1 {
                    target.pop();
                }
            }
            part => target.push(part),
        }
    }
    let from: Vec<&str> = parent(path).collect();
    let common = from.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut relative: Vec<&str> = vec![".."; from.len() - common];
    relative.extend(&target[common..]);
    if relative.is_empty() {
        ".".to_owned()
    } else {
        relative.join("/")
    }
}

fn parent(path: &str) -> impl Iterator<Item = &str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.into_iter()
}

/// Walks a link's target component by component from the link's folder, replacing any component
/// that is itself a link by that link's target, and fails as soon as the walk would leave the root.
fn resolve(link: &str, links: &HashMap<String, String>) -> Result<(), Refusal> {
    let mut position: Vec<String> = parent(link).map(str::to_owned).collect();
    let mut pending: VecDeque<String> = links[link].split('/').map(str::to_owned).collect();
    let mut hops = 0;
    while let Some(part) = pending.pop_front() {
        match part.as_str() {
            "" | "." => {}
            ".." => {
                position.pop().ok_or(Refusal::LinkEscapes)?;
            }
            _ => {
                position.push(part);
                if let Some(target) = links.get(&position.join("/")) {
                    hops += 1;
                    if hops > MAX_LINK_HOPS {
                        return Err(Refusal::LinkLoop);
                    }
                    position.pop();
                    for part in target.split('/').rev() {
                        pending.push_front(part.to_owned());
                    }
                }
            }
        }
    }
    Ok(())
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
