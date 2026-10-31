//! Server tools: their settings, runs, reports and the files they changed, as the panel shows them.
//!
//! A tool reads the server folder, predicts what the FiveM client does with its resources, and
//! changes a file only where the change cannot alter what players see. Everything it leaves alone
//! is reported with every reason, so that the owner knows which setting would change the answer and
//! what to pass on to a resource's author. The first tool finds the archetypes the client registers
//! more than once, which it logs as `Duplicate Archetype` warnings.
//!
//! Every file a tool changes stays in a journal until it no longer holds the change, with the
//! entries taken out and the copy kept for each, so that the change can be healed when a kept copy
//! goes and undone on request.

use std::fmt::Write as _;

use jiff::Timestamp;
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

/// Who started a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolTrigger {
    /// Observer, before FXServer starts.
    Start,
    /// The owner, on the Tools page.
    Panel,
    /// The command line.
    Cli,
}

impl ToolTrigger {
    pub const ALL: [Self; 3] = [Self::Start, Self::Panel, Self::Cli];

    /// The name on the wire and in the store.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Panel => "panel",
            Self::Cli => "cli",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|trigger| trigger.as_str() == name)
    }
}

/// What a run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolRunMode {
    /// Scans and reports, and writes nothing.
    Report,
    /// Scans and applies the plan.
    Fix,
    /// Puts back only the entries whose kept copy is gone.
    Heal,
    /// Puts back the entries the owner names.
    Undo,
}

impl ToolRunMode {
    pub const ALL: [Self; 4] = [Self::Report, Self::Fix, Self::Heal, Self::Undo];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Fix => "fix",
            Self::Heal => "heal",
            Self::Undo => "undo",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolRunOutcome {
    Done,
    /// A stop or a shutdown ended it between two files.
    Cancelled,
    /// It ran past the time a start gives it, and ended between two files.
    TimedOut,
    /// An error ended it: its summary's `problem` says which.
    Failed,
}

impl ToolRunOutcome {
    pub const ALL: [Self; 4] = [Self::Done, Self::Cancelled, Self::TimedOut, Self::Failed];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timedOut",
            Self::Failed => "failed",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.as_str() == name)
    }
}

/// What a run found and did, in counts. Stored as written: a field added later reads as its default
/// in older runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ToolRunSummary {
    /// Hashes registered more than once.
    pub groups: u32,
    /// Groups the plan fixes.
    pub fixable: u32,
    /// Groups whose fix this run wrote.
    pub fixed: u32,
    /// Groups left as they are, with their reasons in the report.
    pub left: u32,
    /// Files this run changed.
    pub files_changed: u32,
    /// Files this run gave entries back to, or their original.
    pub files_restored: u32,
    /// Entries this run took out.
    pub removals: u32,
    pub elapsed_ms: u64,
    /// Why the run failed, or what ended it early.
    pub problem: Option<String>,
}

/// One run of a tool, as its history keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolRun {
    pub id: i64,
    pub tool: ToolKind,
    pub trigger: ToolTrigger,
    /// The account that asked, or null when Observer ran it by itself.
    pub actor: Option<String>,
    pub mode: ToolRunMode,
    pub started_at: Timestamp,
    /// Null while it runs, as are its outcome and summary.
    pub finished_at: Option<Timestamp>,
    pub outcome: Option<ToolRunOutcome>,
    pub summary: Option<ToolRunSummary>,
    /// The SHA-256 of the plan its report holds, in hex: a fix names it to confirm the plan the
    /// owner saw.
    pub plan: Option<String>,
    /// Kept for the latest report and for the runs that changed files; null in lists.
    pub report: Option<ArchetypeReport>,
}

/// What a changed file holds, which says what its entries are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolFileFormat {
    /// A `.ytyp`: entries are positions in its archetype list.
    Ytyp,
    /// A model-info `.meta`: entries are the ordinals of its `Item`s.
    ModelInfo,
}

impl ToolFileFormat {
    pub const ALL: [Self; 2] = [Self::Ytyp, Self::ModelInfo];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ytyp => "ytyp",
            Self::ModelInfo => "modelInfo",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.as_str() == name)
    }
}

/// One entry a tool took out of a file, as the journal keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolRemoval {
    /// Its position in the file before Observer changed it: an archetype index or an item ordinal.
    pub index: u32,
    /// The name hash it registers.
    pub hash: u32,
    /// The name, when the run's report spelled it.
    #[serde(default)]
    pub name: Option<String>,
    /// The copy that keeps registering the hash.
    pub keeper: ToolKeeper,
    /// The run that took it out.
    pub run: i64,
}

/// The copy of an archetype that stays when another is taken out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolKeeper {
    /// Relative to the server folder, with `/`: the changed file itself when both copies share it.
    pub path: String,
    pub format: ToolFileFormat,
    /// Its position in its file when the run planned the removal.
    pub index: u32,
    /// The SHA-256 of its file then, in hex.
    pub sha256: String,
}

/// Where a change stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolChangeState {
    /// A write is under way, or was when Observer stopped: the next start or run settles it.
    Pending,
    /// The file holds the change.
    Applied,
    /// Over: the file is no longer Observer's to heal or undo.
    Closed,
}

impl ToolChangeState {
    pub const ALL: [Self; 3] = [Self::Pending, Self::Applied, Self::Closed];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applied => "applied",
            Self::Closed => "closed",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == name)
    }
}

/// Why a change was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolCloseReason {
    /// The owner undid it: Observer gave the file its original back.
    Undone,
    /// Every copy it kept went away: Observer gave the file its original back.
    Healed,
    /// The file changed since, by someone else's hand: Observer leaves it alone.
    Stale,
    /// The file was found holding its original again, put back by hand or by a deploy.
    Restored,
    /// Its first write never happened.
    Abandoned,
}

impl ToolCloseReason {
    pub const ALL: [Self; 5] = [
        Self::Undone,
        Self::Healed,
        Self::Stale,
        Self::Restored,
        Self::Abandoned,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Undone => "undone",
            Self::Healed => "healed",
            Self::Stale => "stale",
            Self::Restored => "restored",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }
}

/// A file a tool changed, as the Tools page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolChange {
    pub id: i64,
    /// The server folder the file is in, as it was when the change was made.
    pub folder: String,
    /// Relative to the server folder, with `/`.
    pub path: String,
    pub resource: String,
    pub format: ToolFileFormat,
    /// The entries the file holds without; once closed, those it held without last.
    pub removals: Vec<ToolRemoval>,
    pub state: ToolChangeState,
    pub close_reason: Option<ToolCloseReason>,
    /// The SHA-256 of the file before Observer changed it, in hex: its backup's name under
    /// `data/tools/backups`.
    pub backup: String,
    pub opened_at: Timestamp,
    /// When its latest write landed; null until the first did.
    pub changed_at: Option<Timestamp>,
    pub closed_at: Option<Timestamp>,
}

/// The Tools page: each tool with its settings, its latest runs and the files it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolsView {
    pub tools: Vec<ToolView>,
}

/// One tool on the Tools page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolView {
    pub tool: ToolKind,
    pub settings: ToolSettings,
    /// The settings' version, null until they are first saved.
    pub version: Option<i64>,
    /// The latest run, with its report if it made one.
    pub last_run: Option<ToolRun>,
    /// The latest run that made a report, with it, when that is an earlier run than the latest: a
    /// heal at a start makes none.
    pub last_report: Option<ToolRun>,
    /// The changes still open, in every server folder.
    pub open: Vec<ToolChange>,
    /// The changes closed recently, the latest first.
    pub closed: Vec<ToolChange>,
    /// What the backups of these changes take on the disk.
    pub backup_bytes: u64,
}

/// The owner's change of a tool's settings.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ToolSettingsChange {
    pub settings: ToolSettings,
    /// The version the change was made from, null before the first save.
    pub version: Option<i64>,
}

/// A fix the owner confirmed: it is applied only if a fresh scan plans exactly what the owner saw.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ToolFixRequest {
    /// The `plan` of the run whose report the owner saw.
    pub plan: String,
}

impl ToolFixRequest {
    /// The plan's SHA-256, or `None` when the request does not hold one.
    pub fn plan_sha256(&self) -> Option<[u8; 32]> {
        sha256_from_hex(&self.plan)
    }
}

/// The changes the owner asks to undo.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ToolUndoRequest {
    pub scope: ToolUndoScope,
}

/// What an undo puts back. Written `"all"`, `{ "run": id }` or `{ "changes": [id, ...] }`: one key
/// only, so that a request never names two scopes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolUndoScope {
    /// Every open change in the server folder.
    All,
    /// The entries one run took out, in every file it changed.
    Run(i64),
    /// These changes, whole.
    Changes(Vec<i64>),
}

/// A SHA-256 as the journal, the backups and the panel write it: 64 lowercase hex digits.
pub fn sha256_hex(digest: &[u8; 32]) -> String {
    digest.iter().fold(String::with_capacity(64), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// Reads what [`sha256_hex`] writes, and nothing else: no sign, no upper case, no other length.
pub fn sha256_from_hex(text: &str) -> Option<[u8; 32]> {
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    if pairs.len() != 32 || !rest.is_empty() {
        return None;
    }
    let nibble = |digit: u8| match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    };
    let mut digest = [0; 32];
    for (byte, [high, low]) in digest.iter_mut().zip(pairs) {
        *byte = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Some(digest)
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
        for trigger in ToolTrigger::ALL {
            assert_eq!(serde_json::to_value(trigger).unwrap(), trigger.as_str());
            assert_eq!(ToolTrigger::from_name(trigger.as_str()), Some(trigger));
        }
        for mode in ToolRunMode::ALL {
            assert_eq!(serde_json::to_value(mode).unwrap(), mode.as_str());
            assert_eq!(ToolRunMode::from_name(mode.as_str()), Some(mode));
        }
        for outcome in ToolRunOutcome::ALL {
            assert_eq!(serde_json::to_value(outcome).unwrap(), outcome.as_str());
            assert_eq!(ToolRunOutcome::from_name(outcome.as_str()), Some(outcome));
        }
        for format in ToolFileFormat::ALL {
            assert_eq!(serde_json::to_value(format).unwrap(), format.as_str());
            assert_eq!(ToolFileFormat::from_name(format.as_str()), Some(format));
        }
        for state in ToolChangeState::ALL {
            assert_eq!(serde_json::to_value(state).unwrap(), state.as_str());
            assert_eq!(ToolChangeState::from_name(state.as_str()), Some(state));
        }
        for reason in ToolCloseReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), reason.as_str());
            assert_eq!(ToolCloseReason::from_name(reason.as_str()), Some(reason));
        }
        assert_eq!(ToolRunOutcome::from_name("timed_out"), None);
    }

    #[test]
    fn a_summary_stored_by_an_older_build_reads_with_the_fields_it_lacks_at_their_default() {
        let summary: ToolRunSummary = serde_json::from_value(json!({ "groups": 4, "fixed": 2 })).unwrap();

        assert_eq!(
            summary,
            ToolRunSummary {
                groups: 4,
                fixed: 2,
                ..ToolRunSummary::default()
            }
        );
        assert_eq!(
            serde_json::to_value(&summary).unwrap(),
            json!({
                "groups": 4, "fixable": 0, "fixed": 2, "left": 0, "filesChanged": 0, "filesRestored": 0,
                "removals": 0, "elapsedMs": 0, "problem": null
            })
        );
    }

    #[test]
    fn a_removal_names_its_keeper_and_its_run() {
        let removal = ToolRemoval {
            index: 3,
            hash: 0x1234_abcd,
            name: Some("bz_prop_gift".to_owned()),
            keeper: ToolKeeper {
                path: "resources/gifts/stream/bz_prop_gift2.ytyp".to_owned(),
                format: ToolFileFormat::Ytyp,
                index: 0,
                sha256: "ab".repeat(32),
            },
            run: 7,
        };
        let wire = json!({
            "index": 3, "hash": 0x1234_abcd_u32, "name": "bz_prop_gift",
            "keeper": {
                "path": "resources/gifts/stream/bz_prop_gift2.ytyp", "format": "ytyp", "index": 0,
                "sha256": "ab".repeat(32)
            },
            "run": 7
        });

        assert_eq!(serde_json::to_value(&removal).unwrap(), wire);
        assert_eq!(serde_json::from_value::<ToolRemoval>(wire).unwrap(), removal);
        let mut unnamed = serde_json::to_value(&removal).unwrap();
        unnamed.as_object_mut().unwrap().remove("name");
        assert_eq!(serde_json::from_value::<ToolRemoval>(unnamed).unwrap().name, None);
    }

    #[test]
    fn an_undo_names_everything_a_run_or_changes_and_nothing_else() {
        let undo = |text: &str| serde_json::from_str::<ToolUndoRequest>(text).map(|request| request.scope);

        assert_eq!(undo(r#"{ "scope": "all" }"#).unwrap(), ToolUndoScope::All);
        assert_eq!(
            undo(r#"{ "scope": { "run": 12 } }"#).unwrap(),
            ToolUndoScope::Run(12)
        );
        assert_eq!(
            undo(r#"{ "scope": { "changes": [3, 4] } }"#).unwrap(),
            ToolUndoScope::Changes(vec![3, 4])
        );
        assert!(undo(r#"{ "scope": "run" }"#).is_err());
        assert!(undo(r#"{ "scope": { "run": 12, "changes": [3] } }"#).is_err());
        assert!(undo(r#"{ "scope": "everything" }"#).is_err());
        assert!(undo(r#"{ "scope": "all", "force": true }"#).is_err());
    }

    #[test]
    fn a_fix_confirms_a_plan_by_its_digest_and_settings_by_their_version() {
        let digest = [0x5a; 32];
        let fix: ToolFixRequest = serde_json::from_value(json!({ "plan": sha256_hex(&digest) })).unwrap();
        assert_eq!(fix.plan_sha256(), Some(digest));
        let unknown: ToolFixRequest = serde_json::from_value(json!({ "plan": "5A".repeat(32) })).unwrap();
        assert_eq!(unknown.plan_sha256(), None);
        assert!(serde_json::from_value::<ToolFixRequest>(json!({ "plan": "", "force": true })).is_err());

        let change: ToolSettingsChange = serde_json::from_value(json!({
            "settings": {
                "atStart": "report", "differences": "identical", "escrowed": "skip", "excluded": []
            },
            "version": null
        }))
        .unwrap();
        assert_eq!(change.settings.at_start, ToolMode::Report);
        assert_eq!(change.version, None);
    }

    #[test]
    fn a_digest_is_written_in_lowercase_hex_and_read_back_only_in_that_form() {
        let mut digest = [0; 32];
        for (byte, value) in digest.iter_mut().zip(0u8..) {
            *byte = value.wrapping_mul(37);
        }

        let hex = sha256_hex(&digest);

        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("00254a6f94b9"));
        assert_eq!(sha256_from_hex(&hex), Some(digest));
        assert_eq!(sha256_from_hex(&hex.to_uppercase()), None);
        assert_eq!(sha256_from_hex(&hex[..62]), None);
        assert_eq!(sha256_from_hex(&format!("{hex}0")), None);
        assert_eq!(sha256_from_hex(&format!("+{}", &hex[1..])), None);
        assert_eq!(sha256_from_hex(&"g".repeat(64)), None);
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
