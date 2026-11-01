//! The server folder as FXServer and the FiveM client see it, read from the disk alone: the resource
//! folders FXServer's walk finds, their manifests, which of them the cfg starts and in which order,
//! the files each one streams and the model-info files it loads, and whatever else in the folder
//! names a `.ytyp`. Nothing here parses a `.ytyp` or a model-info file: the index gathers the facts
//! the tools decide from, so that one walk serves every tool and the decisions can be tested on
//! small synthetic folders.
//!
//! The walk is bounded the way FXServer's is: `[category]` folders are opened, every other folder is
//! a resource, and of a resource only its root, its `stream/` folder and the folders its data files
//! name are read. A `files '**'` over `node_modules` is never expanded: a data file's candidates are
//! the files under its literal folder, matched against the shipped patterns.
//!
//! What only the running server decides is predicted and said in notes: constraints such as
//! `/assetpacks`, the resources scripts start or stop, and the order the system lists folders in
//! where it is not Windows. The tools fix only what holds whatever those turn out to be.

mod start;
mod walk;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use observer_domain::launch::OneSync;
use observer_domain::server_data::Game;
use observer_domain::tools::{Contest, MAX_NOTES, ToolNote, ToolNoteKind};
use observer_fxserver::cfg::{self, ResourceCommand, Scan};
use observer_fxserver::server_data;
use observer_gamefiles::hash::name_hash;
use observer_gamefiles::model_info;

use crate::glob;
use crate::manifest::{self, Manifest, NoteKind};

/// The `fx_version` names FXServer knows, oldest first: an `fxmanifest.lua` naming another does
/// not start.
const FX_VERSIONS: [&str; 3] = ["adamant", "bodacious", "cerulean"];
/// The `data_file` type of the ityp requests, compared ignoring case as the client does.
const ITYP_REQUEST: &str = "DLC_ITYP_REQUEST";
const RPF_FILE: &str = "RPF_FILE";
/// The most entries the walks of data folders may list over every resource of one index: past
/// them, a data file's folder lists nothing more and is noted as too large.
const MAX_DATA_ENTRIES: usize = 400_000;

/// Why an index could not be built.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The cancel flag was raised: nothing was decided.
    #[error("the scan was cancelled")]
    Cancelled,
    #[error("the server folder {} cannot be read: {source}", path.display())]
    Folder { path: PathBuf, source: io::Error },
}

/// Which resources the server starts.
#[derive(Debug, Clone)]
pub enum Starts {
    /// The commands of the cfg that FXServer runs at launch.
    Cfg(CfgStarts),
    /// No cfg: every resource FXServer accepts, in byte order of the names, as if each were started
    /// by hand. A guess, which the report says.
    AllResources,
}

/// The cfg's part of what starts.
#[derive(Debug, Clone, Default)]
pub struct CfgStarts {
    /// The commands that start and stop resources, in the order FXServer runs them.
    pub commands: Vec<ResourceCommand>,
    /// Whether `commands` holds them all: false when a file the cfg runs was left unread.
    pub complete: bool,
    /// The game the cfg sets, none for GTA V.
    pub game: Option<Game>,
    /// The names of the build's system resources, which FXServer finds besides the folder's.
    pub system_resources: Vec<String>,
}

impl Starts {
    /// What a scan of the cfg starts, with the system resources of the build at `system_resources`.
    pub fn from_scan(scan: &Scan, system_resources: Option<&Path>) -> Self {
        Self::Cfg(CfgStarts {
            commands: scan.resource_commands.clone(),
            complete: scan.resource_commands_complete,
            game: scan.facts.game,
            system_resources: system_resources.map(system_names).unwrap_or_default(),
        })
    }

    fn game(&self) -> Option<Game> {
        match self {
            Self::Cfg(cfg) => cfg.game,
            Self::AllResources => None,
        }
    }
}

/// The folders of a build's `system_resources`, by name.
fn system_names(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Reads and scans the cfg `cfg_file` of the server folder `folder` the way a launch does, for a run
/// outside the supervisor (the command line): its first MiB, the files it runs, the build's system
/// resources when the build is known.
pub fn scan_cfg(folder: &Path, cfg_file: &str, system_resources: Option<PathBuf>) -> io::Result<Scan> {
    let mut bytes = Vec::new();
    File::open(folder.join(cfg_file))?
        .take(cfg::MAX_BYTES)
        .read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let context = cfg::Context {
        panel: None,
        onesync: OneSync::On,
        probe_ports: false,
        system_resources,
        follow_outside: true,
    };
    let resources = server_data::walk(folder);
    Ok(cfg::scan(folder, cfg_file, &text, &resources, &context))
}

/// A resource folder FXServer's walk finds.
#[derive(Debug, Clone)]
pub struct Resource {
    /// Its folder's name, which FXServer knows it by, case included.
    pub name: String,
    /// Its folder, relative to the server folder, with `/`.
    pub path: String,
    /// The `[category]` folders on its path, which `ensure [category]` starts.
    pub categories: Vec<String>,
    pub manifest: ManifestState,
    pub status: Status,
    /// A `.fxap` lies at its root: the asset escrow protects it.
    pub escrowed: bool,
    /// Its folder, or a category above it, is reached through a link or junction.
    pub linked: bool,
    /// It never reaches clients (`server_only`).
    pub server_only: bool,
    /// Every file of its `stream/` folder, in the order FXServer lists them.
    pub stream: Vec<StreamFile>,
    /// The lower-cased stems its `DLC_ITYP_REQUEST` data files name.
    pub ytyp_requests: Vec<String>,
    /// Its `.ymf` manifests and `_cache_y.dat`, with the `.ytyp` stems they name.
    pub references: Vec<Reference>,
    /// The model-info files the client loads for it, in the order it loads them. Empty unless it
    /// starts and reaches clients.
    pub model_info: Vec<DataFile>,
}

impl Resource {
    /// It starts and reaches clients.
    pub fn is_client(&self) -> bool {
        self.status == Status::Started && !self.server_only
    }
}

/// What FXServer makes of a resource folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A folder of the same name was found first: FXServer uses that one.
    Duplicate,
    /// No `fxmanifest.lua` nor `__resource.lua`: FXServer does not load it.
    NoManifest,
    /// FXServer fails to load its manifest.
    Refused,
    /// Loaded and not started.
    Stopped,
    Started,
}

/// The manifest of a resource, as read.
#[derive(Debug, Clone)]
pub enum ManifestState {
    Missing,
    Unreadable(String),
    Read {
        /// `fxmanifest.lua` or `__resource.lua`.
        file: &'static str,
        manifest: Box<Manifest>,
    },
}

impl ManifestState {
    fn manifest(&self) -> Option<&Manifest> {
        match self {
            Self::Read { manifest, .. } => Some(manifest),
            Self::Missing | Self::Unreadable(_) => None,
        }
    }

    /// FXServer reads it, and nothing here can: its entries are unknown.
    fn opaque(&self) -> bool {
        self.manifest().is_some_and(|manifest| {
            manifest
                .notes
                .iter()
                .any(|note| matches!(note.kind, NoteKind::Encrypted | NoteKind::TooLarge))
        })
    }
}

/// A file of a resource's `stream/` folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFile {
    /// Its name, which the server keys it by, case included.
    pub name: String,
    /// Relative to the server folder, with `/`.
    pub path: String,
    /// FXServer sends it: the first file of its name in the resource, `.stream_raw` ones aside.
    pub sent: bool,
    /// It is reached through a link or junction.
    pub linked: bool,
}

/// A `.ymf` manifest or `_cache_y.dat`, which the client follows to load `.ytyp` files with the
/// maps that need them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Relative to the server folder, with `/`.
    pub path: String,
    /// The hashes of the folder's `.ytyp` stems found in it.
    pub stems: HashSet<u32>,
    /// Why it could not be read: what it names is then unknown.
    pub problem: Option<String>,
}

/// A model-info file a resource's data files load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFile {
    /// The kind the data file declares.
    pub kind: model_info::Kind,
    /// Relative to the server folder, with `/`.
    pub path: String,
    /// The manifest line of its `data_file`.
    pub line: u32,
    /// It is reached through a link or junction.
    pub linked: bool,
}

/// A file of the index: a resource and the position of the file in its `stream` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamRef {
    pub resource: usize,
    pub file: usize,
}

/// The `.ytyp` files sent that share a lower-cased stem, which is one streaming slot on the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// The lower-cased stem.
    pub stem: String,
    /// The file the client loads: the greatest `cache:/<resource>/<name>`.
    pub owner: StreamRef,
    /// The others, which it never loads.
    pub shadowed: Vec<StreamRef>,
}

/// The facts of one server folder that the tools decide from.
#[derive(Debug, Clone)]
pub struct ServerIndex {
    folder: PathBuf,
    supported: bool,
    resources: Vec<Resource>,
    /// The resources started, in the order they started.
    order: Vec<usize>,
    /// Every `.ytyp` of every resource's `stream/`, by lower-cased stem.
    stems: HashMap<String, Vec<StreamRef>>,
    notes: Notes,
    walk_ms: u64,
    refused: Vec<Vec<String>>,
}

impl ServerIndex {
    /// Indexes the server folder `folder` for what `starts` starts, checking `cancel` between
    /// entries.
    pub fn build(folder: &Path, starts: &Starts, cancel: &AtomicBool) -> Result<Self, Error> {
        let began = Instant::now();
        let metadata = fs::metadata(folder).map_err(|source| Error::Folder {
            path: folder.to_path_buf(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(Error::Folder {
                path: folder.to_path_buf(),
                source: io::Error::new(io::ErrorKind::NotADirectory, "not a folder"),
            });
        }
        let mut index = Self {
            folder: folder.to_path_buf(),
            supported: true,
            resources: Vec::new(),
            order: Vec::new(),
            stems: HashMap::new(),
            notes: Notes::default(),
            walk_ms: 0,
            refused: Vec::new(),
        };
        if starts.game() == Some(Game::Rdr3) {
            index.supported = false;
            index.notes.add(
                ToolNoteKind::UnsupportedGame,
                None,
                None,
                None,
                Some("rdr3".to_owned()),
            );
            index.walk_ms = elapsed_ms(began);
            return Ok(index);
        }
        let mut cache_files = Vec::new();
        for found in walk::resources(&folder.join("resources"), cancel)? {
            let (resource, cache) = read_resource(&found, &mut index.notes, cancel)?;
            index.resources.push(resource);
            cache_files.push(cache);
        }
        index.index_stems();
        index.read_references(&cache_files, cancel)?;
        index.replay(starts);
        index.read_data_files(cancel)?;
        index.note_findings(starts);
        index.walk_ms = elapsed_ms(began);
        Ok(index)
    }

    /// The paths of Observer's own files and folders inside the server folder, as components: the
    /// tools never change them. None by default.
    #[must_use]
    pub fn with_refused(mut self, refused: Vec<Vec<String>>) -> Self {
        self.refused = refused;
        self
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Whether the tools run on this server: its cfg sets GTA V, or no game.
    pub fn supported(&self) -> bool {
        self.supported
    }

    /// Every resource folder found, in the order FXServer's walk finds them.
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    /// The resources started, in the order they started, server-only ones included.
    pub fn started(&self) -> &[usize] {
        &self.order
    }

    /// The resources clients load, in the order the server started them.
    pub fn client_order(&self) -> impl Iterator<Item = usize> + '_ {
        self.order
            .iter()
            .copied()
            .filter(|&index| !self.resources[index].server_only)
    }

    pub fn file(&self, at: StreamRef) -> &StreamFile {
        &self.resources[at.resource].stream[at.file]
    }

    /// What limits the prediction, and how many notes there were in all.
    pub fn notes(&self) -> (&[ToolNote], u32) {
        (&self.notes.list, self.notes.total)
    }

    pub(crate) fn notes_book(&self) -> &Notes {
        &self.notes
    }

    /// How long the build took.
    pub fn walk_ms(&self) -> u64 {
        self.walk_ms
    }

    pub(crate) fn refused(&self) -> &[Vec<String>] {
        &self.refused
    }

    /// The resource whose folder holds `path` (relative to the server folder, with `/`).
    pub fn resource_of(&self, path: &str) -> Option<usize> {
        self.resources.iter().position(|resource| {
            resource.status != Status::Duplicate
                && path
                    .strip_prefix(resource.path.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    /// The `.ytyp` file at `path`, among the files of its resource's `stream/`.
    pub fn stream_file(&self, path: &str) -> Option<StreamRef> {
        let resource = self.resource_of(path)?;
        let file = self.resources[resource]
            .stream
            .iter()
            .position(|file| file.path == path)?;
        Some(StreamRef { resource, file })
    }

    /// The streaming slots of the `.ytyp` files the started client resources send, in the order the
    /// client registers them: by the smallest `cache:/<resource>/<name>` of each slot, byte order.
    pub fn ytyp_slots(&self) -> Vec<Slot> {
        let mut slots: HashMap<String, Vec<(String, StreamRef)>> = HashMap::new();
        for index in self.client_order() {
            let resource = &self.resources[index];
            for (file, stream) in resource.stream.iter().enumerate() {
                let Some(stem) = ytyp_stem(&stream.name).filter(|_| stream.sent) else {
                    continue;
                };
                let cache = format!("cache:/{}/{}", resource.name, stream.name);
                slots.entry(stem.to_ascii_lowercase()).or_default().push((
                    cache,
                    StreamRef {
                        resource: index,
                        file,
                    },
                ));
            }
        }
        let mut ordered: Vec<(String, Slot)> = slots
            .into_iter()
            .filter_map(|(stem, mut files)| {
                files.sort();
                let first = files.first()?.0.clone();
                let (_, owner) = files.pop()?;
                let shadowed = files.into_iter().map(|(_, at)| at).collect();
                Some((
                    first,
                    Slot {
                        stem,
                        owner,
                        shadowed,
                    },
                ))
            })
            .collect();
        ordered.sort_by(|a, b| a.0.cmp(&b.0));
        ordered.into_iter().map(|(_, slot)| slot).collect()
    }

    /// What else could put another file in the place of the `.ytyp` at `at`: a `.ytyp` of the same
    /// stem anywhere under `resources/` (in its own resource, a sibling), another resource's
    /// `DLC_ITYP_REQUEST` of the stem, another resource's `.ymf` or `_cache_y.dat` naming it. Every
    /// resource folder counts, started or not: a resource started later takes the slot.
    pub fn contests(&self, at: StreamRef) -> Vec<Contest> {
        let Some(stem) = ytyp_stem(&self.file(at).name).map(str::to_ascii_lowercase) else {
            return Vec::new();
        };
        let mut contests = Vec::new();
        for &other in self.stems.get(&stem).into_iter().flatten() {
            if other == at {
                continue;
            }
            let path = self.file(other).path.clone();
            contests.push(if other.resource == at.resource {
                Contest::Sibling { path }
            } else {
                Contest::SameName {
                    resource: self.resources[other.resource].name.clone(),
                    path,
                }
            });
        }
        let hash = name_hash(&stem);
        for (index, resource) in self.resources.iter().enumerate() {
            if index == at.resource {
                continue;
            }
            if resource.ytyp_requests.contains(&stem) {
                contests.push(Contest::Requested {
                    resource: resource.name.clone(),
                });
            }
            for reference in &resource.references {
                if reference.stems.contains(&hash) {
                    contests.push(Contest::Manifest {
                        resource: resource.name.clone(),
                        path: reference.path.clone(),
                    });
                }
            }
        }
        contests
    }

    /// A `.ymf` manifest or `_cache_y.dat` that could not be read, in another resource than the
    /// `.ytyp` at `at`, as a [`Contest::Manifest`]: what it names is unknown, so it may name the
    /// file's stem, which [`contests`](Self::contests) would then list. Every resource folder counts,
    /// as there. Its [`ToolNoteKind::ReferenceUnreadable`] note says why it could not be read.
    pub fn unknown_reference(&self, at: StreamRef) -> Option<Contest> {
        ytyp_stem(&self.file(at).name)?;
        self.resources
            .iter()
            .enumerate()
            .filter(|&(index, _)| index != at.resource)
            .find_map(|(_, resource)| {
                let reference = resource
                    .references
                    .iter()
                    .find(|reference| reference.problem.is_some())?;
                Some(Contest::Manifest {
                    resource: resource.name.clone(),
                    path: reference.path.clone(),
                })
            })
    }

    /// A `.ymf` manifest or `_cache_y.dat` of the file's own resource that names its stem, or that
    /// could not be read and so may: the client may then load the file only with the maps that need
    /// it.
    pub fn manifest_dependency(&self, at: StreamRef) -> Option<&str> {
        let stem = ytyp_stem(&self.file(at).name)?;
        let hash = name_hash(stem);
        self.resources[at.resource]
            .references
            .iter()
            .find(|reference| reference.problem.is_some() || reference.stems.contains(&hash))
            .map(|reference| reference.path.as_str())
    }

    fn index_stems(&mut self) {
        for (resource, found) in self.resources.iter().enumerate() {
            for (file, stream) in found.stream.iter().enumerate() {
                if stream.path.contains(".stream_raw") {
                    continue;
                }
                if let Some(stem) = ytyp_stem(&stream.name) {
                    self.stems
                        .entry(stem.to_ascii_lowercase())
                        .or_default()
                        .push(StreamRef { resource, file });
                }
            }
        }
    }

    /// Searches every `.ymf` and `_cache_y.dat` of every resource for the folder's `.ytyp` stems.
    fn read_references(&mut self, cache_files: &[Option<String>], cancel: &AtomicBool) -> Result<(), Error> {
        let hashes: HashSet<u32> = self.stems.keys().map(|stem| name_hash(stem)).collect();
        for (resource, cache) in self.resources.iter_mut().zip(cache_files) {
            let streamed = resource
                .stream
                .iter()
                .filter(|file| {
                    ends_with_ignoring_case(&file.name, ".ymf")
                        || ends_with_ignoring_case(&file.name, "_cache_y.dat")
                })
                .map(|file| file.path.clone());
            let root = cache.iter().map(|name| format!("{}/{name}", resource.path));
            for path in root.chain(streamed).collect::<Vec<_>>() {
                walk::cancelled(cancel)?;
                let (stems, problem) = match walk::references(&self.folder.join(&path), &hashes) {
                    Ok(stems) => (stems, None),
                    Err(problem) => (HashSet::new(), Some(problem)),
                };
                if let Some(problem) = &problem {
                    self.notes.add(
                        ToolNoteKind::ReferenceUnreadable,
                        Some(resource.name.clone()),
                        Some(path.clone()),
                        None,
                        Some(problem.clone()),
                    );
                }
                resource.references.push(Reference { path, stems, problem });
            }
        }
        Ok(())
    }

    /// Replays the cfg's commands, or starts everything.
    fn replay(&mut self, starts: &Starts) {
        let candidates: Vec<Option<start::Candidate<'_>>> = self.resources.iter().map(candidate).collect();
        let system: HashSet<String> = match starts {
            Starts::Cfg(cfg) => cfg.system_resources.iter().cloned().collect(),
            Starts::AllResources => HashSet::new(),
        };
        let mut notes = std::mem::take(&mut self.notes);
        let mut replay = start::Replay::new(&candidates, &system, &mut notes);
        match starts {
            Starts::Cfg(cfg) => replay.run(&cfg.commands),
            Starts::AllResources => replay.start_all(),
        }
        let order = replay.finish();
        drop(candidates);
        self.notes = notes;
        for &index in &order {
            self.resources[index].status = Status::Started;
        }
        self.order = order;
    }

    /// The `DLC_ITYP_REQUEST` stems of every resource, and the model-info files of the started
    /// client resources.
    fn read_data_files(&mut self, cancel: &AtomicBool) -> Result<(), Error> {
        let mut budget = MAX_DATA_ENTRIES;
        for index in 0..self.resources.len() {
            let resource = &self.resources[index];
            let Some(manifest) = resource.manifest.manifest() else {
                continue;
            };
            let client = resource.is_client();
            let wanted = manifest.data_files.iter().any(|data_file| {
                data_file.kind.eq_ignore_ascii_case(ITYP_REQUEST)
                    || (client && model_info_kind(&data_file.kind).is_some())
            });
            if !wanted {
                continue;
            }
            let dir = self.folder.join(&resource.path);
            let mut reader = DataFiles {
                dir: &dir,
                resource,
                manifest,
                folders: walk::Folders::new(&mut budget),
                notes: &mut self.notes,
                cancel,
            };
            let requests = reader.ytyp_requests()?;
            let model_info = if client { reader.model_info()? } else { Vec::new() };
            let resource = &mut self.resources[index];
            resource.ytyp_requests = requests;
            resource.model_info = model_info;
        }
        Ok(())
    }

    /// The notes on what the started client resources hold.
    fn note_findings(&mut self, starts: &Starts) {
        if cfg!(not(windows)) {
            self.notes
                .add(ToolNoteKind::OrderNotGuaranteed, None, None, None, None);
        }
        match starts {
            Starts::Cfg(cfg) if !cfg.complete => {
                self.notes
                    .add(ToolNoteKind::CfgIncomplete, None, None, None, None);
            }
            Starts::Cfg(_) => {}
            Starts::AllResources => self.notes.add(ToolNoteKind::AllResources, None, None, None, None),
        }
        for resource in &self.resources {
            let kind = match resource.status {
                Status::Duplicate => Some(ToolNoteKind::DuplicateResourceName),
                Status::NoManifest => Some(ToolNoteKind::NoManifest),
                Status::Refused | Status::Stopped | Status::Started => None,
            };
            if let Some(kind) = kind {
                self.notes.add(
                    kind,
                    Some(resource.name.clone()),
                    Some(resource.path.clone()),
                    None,
                    None,
                );
            }
            if !resource.is_client() {
                continue;
            }
            for file in resource.stream.iter().filter(|file| file.sent) {
                if ends_with_ignoring_case(&file.name, ".rpf") {
                    self.notes.add(
                        ToolNoteKind::RpfInStream,
                        Some(resource.name.clone()),
                        Some(file.path.clone()),
                        None,
                        None,
                    );
                }
            }
            let ManifestState::Read { file, manifest } = &resource.manifest else {
                continue;
            };
            let at = format!("{}/{file}", resource.path);
            for note in &manifest.notes {
                let kind = match note.kind {
                    NoteKind::Encrypted => ToolNoteKind::ManifestEncrypted,
                    NoteKind::TooLarge => ToolNoteKind::ManifestUnreadable,
                    _ => ToolNoteKind::ManifestNote,
                };
                self.notes.add(
                    kind,
                    Some(resource.name.clone()),
                    Some(at.clone()),
                    Some(note.line),
                    Some(note.kind.name().to_owned()),
                );
            }
        }
    }
}

/// Reads a resource's root, manifest and `stream/` listing; with the name of its `_cache_y.dat`.
fn read_resource(
    found: &walk::Found,
    notes: &mut Notes,
    cancel: &AtomicBool,
) -> Result<(Resource, Option<String>), Error> {
    walk::cancelled(cancel)?;
    let root = walk::root(&found.dir, &found.name);
    let listing = walk::stream(&found.dir, &found.path, found.linked, cancel)?;
    if listing.truncated {
        notes.add(
            ToolNoteKind::FolderTooLarge,
            Some(found.name.clone()),
            Some(format!("{}/stream", found.path)),
            None,
            Some(format!("{} entries", walk::MAX_STREAM_ENTRIES)),
        );
    }
    let manifest = if found.duplicate {
        ManifestState::Missing
    } else {
        read_manifest(&found.dir)
    };
    let status = match &manifest {
        _ if found.duplicate => Status::Duplicate,
        ManifestState::Missing => Status::NoManifest,
        ManifestState::Unreadable(problem) => {
            notes.add(
                ToolNoteKind::ManifestUnreadable,
                Some(found.name.clone()),
                Some(found.path.clone()),
                None,
                Some(problem.clone()),
            );
            Status::Refused
        }
        ManifestState::Read { manifest, .. } if manifest.refused() => Status::Refused,
        ManifestState::Read { .. } => Status::Stopped,
    };
    let server_only = manifest.manifest().is_some_and(|manifest| manifest.server_only);
    let resource = Resource {
        name: found.name.clone(),
        path: found.path.clone(),
        categories: found.categories.clone(),
        manifest,
        status,
        escrowed: root.escrowed,
        linked: found.linked,
        server_only,
        stream: listing.files,
        ytyp_requests: Vec::new(),
        references: Vec::new(),
        model_info: Vec::new(),
    };
    Ok((resource, root.cache))
}

/// The manifest FXServer reads: `fxmanifest.lua` when there is one, else `__resource.lua`. Bytes
/// that are not UTF-8 are replaced, as the reader expects.
fn read_manifest(dir: &Path) -> ManifestState {
    let Some((file, path)) = ["fxmanifest.lua", "__resource.lua"]
        .into_iter()
        .map(|file| (file, dir.join(file)))
        .find(|(_, path)| path.is_file())
    else {
        return ManifestState::Missing;
    };
    let mut bytes = Vec::new();
    let limit = u64::try_from(manifest::MAX_BYTES).unwrap_or(u64::MAX) + 1;
    match File::open(&path).and_then(|opened| opened.take(limit).read_to_end(&mut bytes)) {
        Ok(_) => ManifestState::Read {
            file,
            manifest: Box::new(manifest::read(&String::from_utf8_lossy(&bytes))),
        },
        Err(error) => ManifestState::Unreadable(error.to_string()),
    }
}

/// What the replay needs of a resource, `None` when FXServer does not load it.
fn candidate(resource: &Resource) -> Option<start::Candidate<'_>> {
    if !matches!(resource.status, Status::Stopped | Status::Started) {
        return None;
    }
    let (file, manifest) = match &resource.manifest {
        ManifestState::Read { file, manifest } => (*file, manifest),
        ManifestState::Missing | ManifestState::Unreadable(_) => return None,
    };
    let opaque = resource.manifest.opaque();
    Some(start::Candidate {
        name: &resource.name,
        dependencies: &manifest.dependencies,
        provides: &manifest.provides,
        categories: &resource.categories,
        refusal: if opaque { None } else { refusal(file, manifest) },
        manifest: format!("{}/{file}", resource.path),
    })
}

/// Why FXServer, running GTA V, refuses to start a resource whose manifest it loaded: only an
/// `fxmanifest.lua` is checked, for its games and its `fx_version` (`ServerResources.cpp`).
fn refusal(file: &str, manifest: &Manifest) -> Option<String> {
    if file != "fxmanifest.lua" {
        return None;
    }
    let names = |name: &str| manifest.games.iter().any(|game| game.value == name);
    match (names("common"), names("gta5")) {
        (true, true) => return Some("names both the game common and gta5".to_owned()),
        (false, false) => return Some("does not name the game gta5 or common".to_owned()),
        _ => {}
    }
    match &manifest.fx_version {
        None => Some("names no fx_version".to_owned()),
        Some(version) if !FX_VERSIONS.contains(&version.value.as_str()) => {
            Some(format!("names the unknown fx_version {}", version.value))
        }
        Some(_) => None,
    }
}

/// The model-info kind a `data_file` type loads, compared ignoring case as the client does.
fn model_info_kind(kind: &str) -> Option<model_info::Kind> {
    model_info::Kind::ALL
        .into_iter()
        .find(|candidate| kind.eq_ignore_ascii_case(candidate.data_file()))
}

/// The stem of a `.ytyp` name, extension compared ignoring case.
pub(crate) fn ytyp_stem(name: &str) -> Option<&str> {
    let stem = name.get(..name.len().checked_sub(5)?)?;
    (ends_with_ignoring_case(name, ".ytyp") && !stem.is_empty()).then_some(stem)
}

/// Whether `name` ends with `suffix`, ASCII case ignored, as names compare on Windows.
pub(crate) fn ends_with_ignoring_case(name: &str, suffix: &str) -> bool {
    name.len() >= suffix.len()
        && name.as_bytes()[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

/// Resolves a resource's data files the way the client does, against the files the resource ships.
struct DataFiles<'a, 'n> {
    dir: &'a Path,
    resource: &'a Resource,
    manifest: &'a Manifest,
    folders: walk::Folders<'n>,
    notes: &'n mut Notes,
    cancel: &'a AtomicBool,
}

impl DataFiles<'_, '_> {
    /// The stems of the `DLC_ITYP_REQUEST` entries, lower-cased: the client keeps only the file
    /// name of the path, without its extension, and requests that streaming name.
    fn ytyp_requests(&mut self) -> Result<Vec<String>, Error> {
        let mut stems = Vec::new();
        for data_file in &self.manifest.data_files {
            if !data_file.kind.eq_ignore_ascii_case(ITYP_REQUEST) {
                continue;
            }
            let paths = if glob::is_glob(&data_file.path) {
                let (_, shipped) = self.shipped(&data_file.path)?;
                glob::data_file_paths(&data_file.path, &shipped)
            } else {
                vec![data_file.path.clone()]
            };
            for path in paths {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
                let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
                let stem = stem.to_ascii_lowercase();
                if !stem.is_empty() && !stems.contains(&stem) {
                    stems.push(stem);
                }
            }
        }
        Ok(stems)
    }

    /// The model-info files of the weapon, vehicle and ped data files, in manifest order then in the
    /// order of each data file's matches; a path named twice counts once.
    fn model_info(&mut self) -> Result<Vec<DataFile>, Error> {
        let mut files: Vec<DataFile> = Vec::new();
        for data_file in &self.manifest.data_files {
            if data_file.kind.eq_ignore_ascii_case(RPF_FILE) {
                self.note(ToolNoteKind::RpfDataFile, data_file.line, &data_file.path);
                continue;
            }
            let Some(kind) = model_info_kind(&data_file.kind) else {
                continue;
            };
            let (linked, shipped) = self.shipped(&data_file.path)?;
            for path in glob::data_file_paths(&data_file.path, &shipped) {
                let Some(found) = glob::lookup(&path, &shipped) else {
                    self.note(ToolNoteKind::DataFileMissing, data_file.line, &path);
                    continue;
                };
                let full = format!("{}/{found}", self.resource.path);
                if files.iter().any(|file| file.path == full) {
                    continue;
                }
                files.push(DataFile {
                    kind,
                    path: full,
                    line: data_file.line,
                    linked: self.resource.linked || linked.contains(found),
                });
            }
        }
        Ok(files)
    }

    /// The files the resource ships among those under the literal folder of `pattern`, sorted, with
    /// those reached through a link: what a data file of that pattern can match on the client. The
    /// folder is found ignoring case, as the client looks in the packfile; the shipped patterns are
    /// expanded the way FXServer opens their folders on this system ([`glob::Folders::SERVER`]), so
    /// that on Linux a file whose folder `files` spells in another case is not shipped, and cannot
    /// be the copy kept.
    fn shipped(&mut self, pattern: &str) -> Result<(HashSet<String>, Vec<String>), Error> {
        let Some(normalized) = glob::normalize(pattern) else {
            return Ok((HashSet::new(), Vec::new()));
        };
        let names: Vec<&str> = normalized.split('/').collect();
        let literal = names
            .iter()
            .position(|name| glob::is_glob(name))
            .unwrap_or(names.len().saturating_sub(1));
        let walked = self.folders.walk(self.dir, &names[..literal], self.cancel)?;
        if walked.truncated {
            self.notes.add(
                ToolNoteKind::FolderTooLarge,
                Some(self.resource.name.clone()),
                Some(format!("{}/{}", self.resource.path, names[..literal].join("/"))),
                None,
                None,
            );
        }
        let mut shipped = BTreeSet::new();
        for entry in self.manifest.shipped_patterns() {
            if entry.value.starts_with('@') {
                continue;
            }
            shipped.extend(
                glob::expand_with(&entry.value, &walked.files, glob::Folders::SERVER)
                    .into_iter()
                    .filter(|path| !stripped_from_packfile(path)),
            );
        }
        Ok((walked.linked.clone(), shipped.into_iter().collect()))
    }

    fn note(&mut self, kind: ToolNoteKind, line: u32, detail: &str) {
        let file = match &self.resource.manifest {
            ManifestState::Read { file, .. } => Some(format!("{}/{file}", self.resource.path)),
            ManifestState::Missing | ManifestState::Unreadable(_) => None,
        };
        self.notes.add(
            kind,
            Some(self.resource.name.clone()),
            file,
            Some(line),
            Some(detail.to_owned()),
        );
    }
}

/// FXServer leaves out of the packfile the files of `stream/` whose extension, from the first dot,
/// starts with `.y`, `.ytyp` aside: they are streamed instead (`ResourceFilesComponent.cpp`).
fn stripped_from_packfile(path: &str) -> bool {
    path.starts_with("stream/")
        && path
            .find('.')
            .map(|dot| &path[dot..])
            .is_some_and(|extension| extension.as_bytes().get(1) == Some(&b'y') && extension != ".ytyp")
}

/// [`Error::Cancelled`] once the cancel flag is raised.
pub(crate) fn check(cancel: &AtomicBool) -> Result<(), Error> {
    walk::cancelled(cancel)
}

fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The notes of a scan, at most [`MAX_NOTES`] kept and every one counted. Notes about a constraint
/// or a kind of manifest construct are gathered over the folder, one per value with the number of
/// resources; data files of the wrong kind and archives in `stream/` are gathered per resource; any
/// other note is kept once per resource, file and value.
#[derive(Debug, Clone, Default)]
pub(crate) struct Notes {
    list: Vec<ToolNote>,
    keys: HashMap<NoteKey, Option<usize>>,
    total: u32,
}

type NoteKey = (ToolNoteKind, Option<String>, Option<String>, Option<String>);

impl Notes {
    pub(crate) fn add(
        &mut self,
        kind: ToolNoteKind,
        resource: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        detail: Option<String>,
    ) {
        let (gathered, key) = match kind {
            ToolNoteKind::ConstraintNotChecked | ToolNoteKind::ManifestNote => {
                (true, (kind, None, None, detail.clone()))
            }
            ToolNoteKind::DataFileMismatch | ToolNoteKind::RpfInStream => {
                (true, (kind, resource.clone(), None, detail.clone()))
            }
            _ => (false, (kind, resource.clone(), file.clone(), detail.clone())),
        };
        if let Some(listed) = self.keys.get(&key) {
            if gathered && let Some(note) = listed.and_then(|at| self.list.get_mut(at)) {
                note.count = note.count.saturating_add(1);
            }
            return;
        }
        self.total = self.total.saturating_add(1);
        let listed = (self.list.len() < MAX_NOTES).then(|| {
            self.list.push(ToolNote {
                kind,
                resource,
                file,
                line,
                detail,
                count: 1,
            });
            self.list.len() - 1
        });
        self.keys.insert(key, listed);
    }

    pub(crate) fn list(&self) -> &[ToolNote] {
        &self.list
    }

    pub(crate) fn total(&self) -> u32 {
        self.total
    }
}
