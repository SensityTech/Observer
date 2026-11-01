//! The changes a tool plans in the server folder, file by file. A plan names every entry it takes out
//! of a file and the copy that stays in its place, so that the change can be checked again later and
//! undone when the copy kept is gone. Its digest names it: the owner confirms a fix by the digest of
//! the plan they saw, and nothing is applied unless a fresh scan plans exactly that.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What a file holds, which says how entries are taken out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// A `.ytyp`: entries are positions in its archetype list.
    Ytyp,
    /// A model-info `.meta`: entries are ordinals of the `Item`s of its `InitDatas`.
    ModelInfo,
}

/// One file to change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    /// The file, relative to the server folder, components joined by `/`.
    pub path: String,
    /// The resource the file belongs to.
    pub resource: String,
    pub format: Format,
    /// The SHA-256 of the file when the plan was made: a file changed since is left alone.
    pub sha256: [u8; 32],
    /// The entries to take out, in no particular order.
    pub removals: Vec<Removal>,
}

/// One entry taken out of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removal {
    /// Its position in the file as it was before any change: an archetype index or an item ordinal.
    pub index: usize,
    /// The name hash it registers.
    pub hash: u32,
    /// The copy that keeps registering that hash once this one is gone.
    pub keeper: Keeper,
}

/// The copy of an archetype that stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keeper {
    /// Its file, relative to the server folder; the edited file itself when both copies share it.
    pub path: String,
    pub format: Format,
    /// Its position in its file as it was when the plan was made.
    pub index: usize,
    /// The SHA-256 of its file when the plan was made.
    pub sha256: [u8; 32],
}

impl Format {
    /// Its name in a plan's digest, which must never change.
    const fn digest_name(self) -> &'static [u8] {
        match self {
            Self::Ytyp => b"ytyp",
            Self::ModelInfo => b"modelInfo",
        }
    }
}

/// The SHA-256 that names a plan: over its files by path, each file's entries by position, and for
/// each entry its hash and the copy kept, every field length-prefixed. The order the analysis lists
/// files and entries in does not change it; any change to what would be written, or to the file it
/// would be written over, does.
pub fn plan_digest(edits: &[Edit]) -> [u8; 32] {
    fn text(digest: &mut Sha256, text: &str) {
        digest.update((text.len() as u64).to_le_bytes());
        digest.update(text.as_bytes());
    }
    fn number(digest: &mut Sha256, value: usize) {
        digest.update((value as u64).to_le_bytes());
    }

    let mut ordered: Vec<&Edit> = edits.iter().collect();
    ordered.sort_by(|a, b| a.path.cmp(&b.path));
    let mut digest = Sha256::new();
    digest.update(b"observer-plan-1");
    number(&mut digest, ordered.len());
    for edit in ordered {
        text(&mut digest, &edit.path);
        text(&mut digest, &edit.resource);
        digest.update(edit.format.digest_name());
        digest.update(edit.sha256);
        let mut removals: Vec<&Removal> = edit.removals.iter().collect();
        removals.sort_by_key(|removal| removal.index);
        number(&mut digest, removals.len());
        for removal in removals {
            number(&mut digest, removal.index);
            digest.update(removal.hash.to_le_bytes());
            text(&mut digest, &removal.keeper.path);
            digest.update(removal.keeper.format.digest_name());
            number(&mut digest, removal.keeper.index);
            digest.update(removal.keeper.sha256);
        }
    }
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removal(index: usize, hash: u32) -> Removal {
        Removal {
            index,
            hash,
            keeper: Keeper {
                path: "resources/props/stream/props.ytyp".to_owned(),
                format: Format::Ytyp,
                index: 7,
                sha256: [3; 32],
            },
        }
    }

    fn edit(path: &str, removals: Vec<Removal>) -> Edit {
        Edit {
            path: path.to_owned(),
            resource: "props".to_owned(),
            format: Format::Ytyp,
            sha256: [1; 32],
            removals,
        }
    }

    #[test]
    fn a_plan_s_digest_ignores_the_order_it_was_listed_in() {
        let a = edit(
            "resources/props/stream/a.ytyp",
            vec![removal(0, 1), removal(4, 2)],
        );
        let b = edit("resources/props/stream/b.ytyp", vec![removal(2, 3)]);
        let reversed = edit(
            "resources/props/stream/a.ytyp",
            vec![removal(4, 2), removal(0, 1)],
        );

        assert_eq!(
            plan_digest(&[a.clone(), b.clone()]),
            plan_digest(&[b.clone(), reversed])
        );
        assert_ne!(plan_digest(std::slice::from_ref(&a)), plan_digest(&[a, b]));
    }

    #[test]
    fn a_plan_s_digest_changes_with_any_file_entry_or_copy_kept() {
        let base = edit("resources/props/stream/a.ytyp", vec![removal(0, 1)]);
        let digest = plan_digest(std::slice::from_ref(&base));
        let mut other_file = base.clone();
        other_file.sha256 = [2; 32];
        let mut other_entry = base.clone();
        other_entry.removals[0].index = 1;
        let mut other_keeper = base.clone();
        other_keeper.removals[0].keeper.sha256 = [4; 32];
        let mut other_format = base.clone();
        other_format.removals[0].keeper.format = Format::ModelInfo;

        for changed in [other_file, other_entry, other_keeper, other_format] {
            assert_ne!(plan_digest(&[changed]), digest);
        }
        assert_ne!(plan_digest(&[]), digest);
        assert_eq!(plan_digest(&[]), plan_digest(&[]));
    }
}
