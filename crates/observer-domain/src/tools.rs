//! Server tools: their reports and settings, as the panel shows them.
//!
//! A tool reads the server folder, predicts what the FiveM client does with its resources, and
//! changes a file only where the change cannot alter what players see. Everything it leaves alone
//! is reported with every reason, so that the owner knows which setting would change the answer and
//! what to pass on to a resource's author. The first tool finds the archetypes the client registers
//! more than once, which it logs as `Duplicate Archetype` warnings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The most groups a report lists: the counts still cover them all.
pub const MAX_GROUPS: usize = 5000;
/// The most files each list of files names: its total counts them all.
pub const MAX_LISTED_FILES: usize = 200;
/// The most notes a report keeps: its total counts them all.
pub const MAX_NOTES: usize = 200;

/// A tool Observer runs over a server's resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolKind {
    /// Archetypes the client registers more than once.
    DuplicateArchetypes,
}

impl ToolKind {
    pub const ALL: [Self; 1] = [Self::DuplicateArchetypes];

    /// The name on the wire and in the store.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateArchetypes => "duplicateArchetypes",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// What a tool does at every start of the server, before FXServer runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolMode {
    /// Nothing: the tool runs only when asked.
    #[default]
    Off,
    /// Scans and reports, and writes nothing.
    Report,
    /// Scans and applies the fixes.
    Fix,
}

impl ToolMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Report, Self::Fix];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Report => "report",
            Self::Fix => "fix",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

/// Which duplicates may be fixed, by how far their copies differ.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Differences {
    /// Only copies that are the same in every field: which one the game uses cannot matter.
    #[default]
    Identical,
    /// Copies that differ too, keeping the one predicted in use, the last registered. Unverified in
    /// game: if the game uses another copy, the archetype it shows changes.
    KeepLatest,
}

impl Differences {
    pub const ALL: [Self; 2] = [Self::Identical, Self::KeepLatest];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identical => "identical",
            Self::KeepLatest => "keepLatest",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|differences| differences.as_str() == name)
    }
}

/// Whether a tool changes resources protected by the asset escrow (a `.fxap` at their root).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EscrowPolicy {
    /// They are reported only: their author ships the fix.
    #[default]
    Skip,
    Allow,
}

impl EscrowPolicy {
    pub const ALL: [Self; 2] = [Self::Skip, Self::Allow];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Allow => "allow",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|policy| policy.as_str() == name)
    }
}

/// What the owner decides for one tool on one server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ToolSettings {
    pub at_start: ToolMode,
    pub differences: Differences,
    pub escrowed: EscrowPolicy,
    /// Resources the tool never changes, by name as FXServer knows them (case included): those a
    /// pipeline or a file group keeps, for example.
    pub excluded: Vec<String>,
}

/// What the duplicate archetypes tool found in a server folder and what it would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchetypeReport {
    /// The server folder, as given.
    pub folder: String,
    pub counts: ArchetypeCounts,
    /// Each hash registered more than once, in the order the first copies register, at most
    /// [`MAX_GROUPS`].
    pub groups: Vec<ArchetypeGroup>,
    pub groups_total: u32,
    /// `.ytyp` files sent to clients that load another file in their place, at most
    /// [`MAX_LISTED_FILES`].
    pub shadowed: Vec<ShadowedFile>,
    pub shadowed_total: u32,
    /// Files the clients load that could not be read, at most [`MAX_LISTED_FILES`].
    pub unreadable: Vec<UnreadableFile>,
    pub unreadable_total: u32,
    /// What limits the prediction, at most [`MAX_NOTES`].
    pub notes: Vec<ToolNote>,
    pub notes_total: u32,
    pub timings: ToolTimings,
}

/// The counts of a report, each over the whole folder whatever the lists hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchetypeCounts {
    /// Resource folders FXServer finds.
    pub resources: u32,
    /// Resources predicted to start, server-only ones included.
    pub started: u32,
    /// Started resources the clients load.
    pub client: u32,
    /// Started client resources protected by the asset escrow.
    pub escrowed: u32,
    pub ytyp_loaded: u32,
    pub ytyp_shadowed: u32,
    pub ytyp_unreadable: u32,
    pub model_info_loaded: u32,
    pub model_info_unreadable: u32,
    /// Archetypes registered by the files loaded: every copy counted.
    pub registrations: u32,
    pub groups: u32,
    pub groups_fixed: u32,
    /// Files the fixes change.
    pub files_changed: u32,
    /// Copies the fixes take out.
    pub removals: u32,
}

/// One archetype hash registered more than once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchetypeGroup {
    /// The joaat of the lower-cased name, as the client's warning writes it.
    pub hash: u32,
    /// The name, when a streamed file or a model-info item spells it.
    pub name: Option<String>,
    /// Every copy, in predicted registration order: the last one is predicted in use.
    pub copies: Vec<ArchetypeCopy>,
    /// How the other copies differ from the last one.
    pub difference: Difference,
    /// Every copy is in one file, which the warning shows as `'x.ytyp' and 'x.ytyp'`.
    pub same_file: bool,
    pub action: GroupAction,
}

/// One registration of an archetype.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArchetypeCopy {
    pub resource: String,
    /// Relative to the server folder, with `/`.
    pub path: String,
    pub source: CopySource,
    /// Its resource is protected by the asset escrow.
    pub escrowed: bool,
}

/// Where in its file a copy is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "format", rename_all = "camelCase")]
#[ts(export)]
pub enum CopySource {
    /// An entry of a `.ytyp`'s archetype list, from 0.
    Ytyp { index: u32, archetype: ArchetypeType },
    /// An `Item` of a model-info `.meta` (the warning's `Extra`): its place among the items, from 0,
    /// and the line it starts on, from 1.
    ModelInfo {
        kind: ModelInfoKind,
        ordinal: u32,
        line: u32,
    },
}

/// The structure an archetype of a `.ytyp` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ArchetypeType {
    /// `CBaseArchetypeDef`, a prop.
    Base,
    /// `CTimeArchetypeDef`, a prop shown at some hours.
    Time,
    /// `CMloArchetypeDef`, an interior.
    Mlo,
}

/// What a model-info file defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ModelInfoKind {
    Weapon,
    Vehicle,
    Ped,
}

/// How the copies of a group differ from the last one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Difference {
    Identical,
    /// Only in draw distances and bounds (`lodDist`, `hdTextureDist`, `bbMin`, `bbMax`,
    /// `bsCentre`, `bsRadius`), named as the files write them.
    Minor {
        fields: Vec<String>,
    },
    /// In anything else too. `source` when a `.ytyp` copy meets a model-info one, which cannot be
    /// compared; `order` when model-info items hold the same fields written in another order.
    Major {
        fields: Vec<String>,
    },
}

/// What the tool does with a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum GroupAction {
    /// The copies at the positions `remove` of the group's list are taken out, and the one at
    /// `keep` stays the only one.
    Fix { keep: u32, remove: Vec<u32> },
    /// Nothing is taken out, for every one of these reasons.
    Leave { reasons: Vec<LeaveReason> },
}

/// Why a group is left as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum LeaveReason {
    /// The copies differ, and only identical copies may be removed.
    DefinitionsDiffer,
    /// The copies are in several resources, which can start and stop apart.
    SeveralResources,
    /// A copy's resource is protected by the asset escrow.
    Escrowed,
    /// A copy defines an interior, whose removal could take a whole interior away.
    Interior,
    /// A `.ytyp` copy meets a model-info one.
    MixedSources,
    /// Which copy the clients load depends on more than this file.
    Contested {
        file: String,
        by: Contest,
    },
    /// Two files of the group are listed by a `.ymf` manifest or `_cache_y.dat` of their resource,
    /// which may load them only with the maps that need them.
    ManifestDependent {
        file: String,
        manifest: String,
    },
    NotWritable {
        file: String,
        why: NotWritable,
    },
    /// The owner excluded the resource.
    Excluded {
        resource: String,
    },
}

/// What else could put another file in a `.ytyp`'s place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Contest {
    /// A `.ytyp` of the same name in another resource, which may own the name instead.
    SameName { resource: String, path: String },
    /// A `.ytyp` of the same name in the same resource: which one is sent depends on the order the
    /// system lists the folders in.
    Sibling { path: String },
    /// Another resource requests the name with `DLC_ITYP_REQUEST`.
    Requested { resource: String },
    /// A `.ymf` manifest or `_cache_y.dat` of another resource names it.
    Manifest { resource: String, path: String },
}

/// Why the tool may not change a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum NotWritable {
    /// Its layout is not one the change is proven on: what the reader found.
    Layout { detail: String },
    /// It is marked read-only.
    ReadOnly,
    /// It is reached through a link or junction.
    Linked,
    /// Observer never changes it: a cfg, FXServer's cache, Observer's own files or a name that
    /// holds secrets.
    Refused,
}

/// A `.ytyp` sent to clients whose name another file owns: the clients never load it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShadowedFile {
    pub resource: String,
    pub path: String,
    /// The file loaded in its place.
    pub owner_resource: String,
    pub owner_path: String,
}

/// A file the clients load that the tool could not read: its archetypes are unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnreadableFile {
    pub resource: String,
    pub path: String,
    pub problem: FileProblem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum FileProblem {
    /// It is not the format its name says: it looks like `found`.
    Format { found: FileFormat },
    /// It is that format, with contents the reader refuses: why.
    Invalid { error: String },
    /// It could not be read from the disk.
    Io { error: String },
}

/// What a file looks like from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileFormat {
    Rsc7,
    /// Encrypted by the asset escrow.
    Escrowed,
    Pso,
    Rbf,
    Xml,
    Empty,
    Unknown,
}

/// Something that limits the prediction or changes what FXServer does, with where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolNote {
    pub kind: ToolNoteKind,
    pub resource: Option<String>,
    /// Relative to the server folder, with `/`.
    pub file: Option<String>,
    pub line: Option<u32>,
    /// The value the note is about, as written: a resource name, a constraint, a manifest note.
    pub detail: Option<String>,
    /// How many times it was found: notes about a constraint or a manifest construct are gathered
    /// over the folder (one per resource), those about data files of the wrong kind or archives in
    /// `stream/` per resource (one per file).
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolNoteKind {
    /// Not on Windows, the system lists folders in no set order: which of two same-named stream
    /// files FXServer sends may differ from the prediction.
    OrderNotGuaranteed,
    /// A file the cfg runs was not read: the resources it starts or stops are missing.
    CfgIncomplete,
    /// No cfg was given: every resource FXServer accepts is taken as started, in name order.
    AllResources,
    /// The cfg runs another game than GTA V: the tool does nothing there.
    UnsupportedGame,
    /// The cfg names a resource FXServer does not find.
    MissingResource,
    /// A resource depends on one FXServer does not find, and does not start.
    MissingDependency,
    /// A resource's dependency did not start, so neither did it.
    DependencyNotStarted,
    /// A dependency on the server itself (`/assetpacks`, `/gameBuild:…`), which only FXServer
    /// decides: taken as met.
    ConstraintNotChecked,
    /// Two resource folders have the same name: FXServer uses the first one found.
    DuplicateResourceName,
    /// A resource folder without `fxmanifest.lua` or `__resource.lua`, which never starts.
    NoManifest,
    /// FXServer refuses the manifest (`detail` says why), so the resource never starts.
    ManifestRefused,
    /// The manifest is encrypted by the asset escrow: its dependencies and data files are unknown.
    ManifestEncrypted,
    ManifestUnreadable,
    /// What the manifest reader did not follow, by the reader's name for it.
    ManifestNote,
    /// An `.rpf` in `stream/`, which the client refuses.
    RpfInStream,
    /// An `RPF_FILE` data file, whose archive may define archetypes the tool does not see.
    RpfDataFile,
    /// A model-info data file names no file the resource ships.
    DataFileMissing,
    /// A model-info data file's type differs from the file's own.
    DataFileMismatch,
    /// A folder held more entries than the tool lists: what lies past them is unknown.
    FolderTooLarge,
    /// A `.ymf` manifest or `_cache_y.dat` could not be read: the files it names are unknown.
    ReferenceUnreadable,
    /// The archetypes of the game itself and of Rockstar's updates are not read, since no game
    /// install is: a resource that defines one of them again is not seen.
    BaseGameUnseen,
}

/// Where the time of a run went, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolTimings {
    /// Walking the folder and reading the manifests.
    pub walk_ms: u64,
    /// Reading the `.ytyp` and model-info files.
    pub parse_ms: u64,
    pub total_ms: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn stored_names_match_the_wire_names() {
        for kind in ToolKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
            assert_eq!(ToolKind::from_name(kind.as_str()), Some(kind));
        }
        for mode in ToolMode::ALL {
            assert_eq!(serde_json::to_value(mode).unwrap(), mode.as_str());
            assert_eq!(ToolMode::from_name(mode.as_str()), Some(mode));
        }
        for differences in Differences::ALL {
            assert_eq!(serde_json::to_value(differences).unwrap(), differences.as_str());
            assert_eq!(Differences::from_name(differences.as_str()), Some(differences));
        }
        for policy in EscrowPolicy::ALL {
            assert_eq!(serde_json::to_value(policy).unwrap(), policy.as_str());
            assert_eq!(EscrowPolicy::from_name(policy.as_str()), Some(policy));
        }
    }

    #[test]
    fn the_settings_default_to_nothing_at_start_and_identical_copies_outside_escrow() {
        let settings = ToolSettings::default();

        assert_eq!(settings.at_start, ToolMode::Off);
        assert_eq!(settings.differences, Differences::Identical);
        assert_eq!(settings.escrowed, EscrowPolicy::Skip);
        assert_eq!(settings.excluded, Vec::<String>::new());
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            json!({ "atStart": "off", "differences": "identical", "escrowed": "skip", "excluded": [] })
        );
    }

    #[test]
    fn settings_refuse_what_they_do_not_know() {
        let settings: ToolSettings = serde_json::from_value(json!({
            "atStart": "fix", "differences": "keepLatest", "escrowed": "allow", "excluded": ["maps"]
        }))
        .unwrap();

        assert_eq!(settings.at_start, ToolMode::Fix);
        assert_eq!(settings.differences, Differences::KeepLatest);
        assert!(
            serde_json::from_value::<ToolSettings>(json!({
                "atStart": "always", "differences": "identical", "escrowed": "skip", "excluded": []
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ToolSettings>(json!({
                "atStart": "off", "differences": "identical", "escrowed": "skip", "excluded": [],
                "extra": true
            }))
            .is_err()
        );
    }

    #[test]
    fn actions_and_reasons_are_tagged_with_their_parameters() {
        let actions = [
            (
                GroupAction::Fix {
                    keep: 1,
                    remove: vec![0],
                },
                json!({ "kind": "fix", "keep": 1, "remove": [0] }),
            ),
            (
                GroupAction::Leave {
                    reasons: vec![
                        LeaveReason::Escrowed,
                        LeaveReason::Contested {
                            file: "resources/a/stream/x.ytyp".to_owned(),
                            by: Contest::SameName {
                                resource: "b".to_owned(),
                                path: "resources/b/stream/x.ytyp".to_owned(),
                            },
                        },
                    ],
                },
                json!({ "kind": "leave", "reasons": [
                    { "kind": "escrowed" },
                    { "kind": "contested", "file": "resources/a/stream/x.ytyp",
                      "by": { "kind": "sameName", "resource": "b", "path": "resources/b/stream/x.ytyp" } }
                ] }),
            ),
        ];
        for (action, wire) in actions {
            assert_eq!(serde_json::to_value(&action).unwrap(), wire);
            assert_eq!(serde_json::from_value::<GroupAction>(wire).unwrap(), action);
        }
    }

    #[test]
    fn a_copy_names_its_format_and_place() {
        let copies = [
            (
                CopySource::Ytyp {
                    index: 3,
                    archetype: ArchetypeType::Base,
                },
                json!({ "format": "ytyp", "index": 3, "archetype": "base" }),
            ),
            (
                CopySource::ModelInfo {
                    kind: ModelInfoKind::Weapon,
                    ordinal: 7,
                    line: 85,
                },
                json!({ "format": "modelInfo", "kind": "weapon", "ordinal": 7, "line": 85 }),
            ),
        ];
        for (source, wire) in copies {
            assert_eq!(serde_json::to_value(source).unwrap(), wire);
        }
    }
}
