//! A resource's entries as one value: sorted, encoded without ambiguity and hashed, so that two
//! starts that ran from the same files share one tree. Kinds, flags and times stay out of the hash:
//! the kind follows from the contents and the owner's settings, flags belong to one start, and a
//! touched file is no change.

use observer_domain::changes::FileKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One entry of a tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    /// From the data folder, `/`-separated.
    pub path: String,
    pub size: u64,
    /// What the entry compares by: the keyed digest of a file, `t:<time>` for an archive or a
    /// file never read, `?` for one that could not be read, `l:<target>` for a link.
    pub key: String,
    /// What the scan made of it, outside the hash.
    pub kind: FileKind,
    /// The plain SHA-256 of a binary, encrypted or generated file, outside the hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl TreeEntry {
    pub fn is_link(&self) -> bool {
        self.key.starts_with("l:")
    }

    /// The keyed digest, for an entry that has one.
    pub fn digest(&self) -> Option<&str> {
        (self.key.len() == 64).then_some(self.key.as_str())
    }
}

/// The key of an archive or of a file never read.
pub fn time_key(modified_ns: i64) -> String {
    format!("t:{modified_ns}")
}

pub const UNREADABLE_KEY: &str = "?";

pub fn link_key(target: &str) -> String {
    format!("l:{target}")
}

/// Sorts `entries` by path and returns the tree's hash, hex.
pub fn hash(entries: &mut [TreeEntry]) -> String {
    entries.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut hasher = Sha256::new();
    for entry in entries.iter() {
        // Lengths before each field: a path may hold any byte but NUL, a newline included.
        for field in [entry.path.as_bytes(), entry.key.as_bytes()] {
            hasher.update(u64::try_from(field.len()).unwrap_or(u64::MAX).to_le_bytes());
            hasher.update(field);
        }
        hasher.update(entry.size.to_le_bytes());
    }
    hex(&hasher.finalize())
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, key: &str, kind: FileKind) -> TreeEntry {
        TreeEntry {
            path: path.to_owned(),
            size: 10,
            key: key.to_owned(),
            kind,
            sha256: None,
        }
    }

    #[test]
    fn the_order_entries_come_in_does_not_change_the_hash() {
        let mut a = vec![
            entry("r/a.lua", "k1", FileKind::Text),
            entry("r/b.lua", "k2", FileKind::Text),
        ];
        let mut b = vec![
            entry("r/b.lua", "k2", FileKind::Text),
            entry("r/a.lua", "k1", FileKind::Text),
        ];

        assert_eq!(hash(&mut a), hash(&mut b));
    }

    #[test]
    fn kinds_and_plain_hashes_stay_out_of_the_hash() {
        let mut text = vec![entry("r/a.meta", "k1", FileKind::Text)];
        let mut binary = vec![TreeEntry {
            sha256: Some("ab".repeat(32)),
            ..entry("r/a.meta", "k1", FileKind::Binary)
        }];

        assert_eq!(hash(&mut text), hash(&mut binary));
    }

    #[test]
    fn fields_cannot_run_into_each_other() {
        let mut a = vec![entry("r/a", "bc", FileKind::Text)];
        let mut b = vec![entry("r/ab", "c", FileKind::Text)];

        assert_ne!(hash(&mut a), hash(&mut b));
    }

    #[test]
    fn a_size_or_a_key_changes_the_hash() {
        let mut base = vec![entry("r/a", "k", FileKind::Text)];
        let mut bigger = vec![TreeEntry {
            size: 11,
            ..entry("r/a", "k", FileKind::Text)
        }];
        let mut other = vec![entry("r/a", "j", FileKind::Text)];

        let base = hash(&mut base);
        assert_ne!(base, hash(&mut bigger));
        assert_ne!(base, hash(&mut other));
    }
}
