//! The changes a tool plans in the server folder, file by file. A plan names every entry it takes out
//! of a file and the copy that stays in its place, so that the change can be checked again later and
//! undone when the copy kept is gone.

use serde::{Deserialize, Serialize};

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
