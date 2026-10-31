//! The duplicate archetypes tool: which archetype hashes the clients register more than once, which
//! the FiveM client logs as `Duplicate Archetype '<name>' (<hash>), seen in '<A>' and '<B>'`, and
//! which of those copies can be taken out without changing what players see.
//!
//! The analysis reads the `.ytyp` files the clients load (one per streaming slot) and the model-info
//! files the started resources' data files name, lists every registration in the order the client
//! is predicted to make them, and groups them by hash. A group is fixed only when every condition
//! holds, each of them a fact read on disk, never a prediction:
//!
//! 1. the copies are identical (or, with `differences = keepLatest`, the last copy is kept whatever
//!    the others hold);
//! 2. they all lie in one resource, which the asset escrow does not protect (unless allowed): the
//!    copy kept starts and stops with the copies taken out;
//! 3. none is an interior, and `.ytyp` copies do not meet model-info ones;
//! 4. no `.ytyp` involved can be replaced by another file: no other `.ytyp` of its stem anywhere in
//!    the folder, no other resource requesting or listing its stem;
//! 5. copies in two files of the resource are not listed by one of its `.ymf` manifests, which may
//!    load them only with the maps that need them;
//! 6. every copy's file can be changed;
//! 7. the fix leaves exactly one copy: nothing is taken out of a group that cannot be fully fixed.
//!
//! The copy kept is the last registered, the one predicted in use. The plan names, for each file,
//! the entries taken out and the copy each relies on, so that a later check ([`check_keeper`]) can
//! undo a removal whose copy kept is gone. Nothing here writes.

mod read;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use observer_domain::files::built_in_refusal;
use observer_domain::tools::{
    ArchetypeCopy, ArchetypeCounts, ArchetypeGroup, ArchetypeReport, ArchetypeType, Contest, CopySource,
    Difference, Differences, EscrowPolicy, FileProblem, GroupAction, LeaveReason, MAX_GROUPS,
    MAX_LISTED_FILES, ModelInfoKind, NotWritable, ShadowedFile, ToolNoteKind, ToolSettings, ToolTimings,
    UnreadableFile,
};
use observer_gamefiles::hash::name_hash;
use observer_gamefiles::model_info::{self, ModelInfoFile, ModelItem};
use observer_gamefiles::ytyp::{self, Archetype, ArchetypeKind, MapTypes};
use sha2::{Digest, Sha256};

use crate::edit::{Edit, Format, Keeper, Removal};
use crate::resources::{Error, ServerIndex, StreamRef};
use read::{Loaded, Want};

/// The extensions of the streamed files whose stems name the archetypes a report shows.
const NAMED_EXTENSIONS: [&str; 4] = [".ydr", ".yft", ".ydd", ".ytyp"];

/// What the tool found and what it would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub report: ArchetypeReport,
    /// One per file to change, in the order the files register.
    pub edits: Vec<Edit>,
}

/// Analyses the server `index` describes with the owner's `settings`, checking `cancel` between
/// files. Reads only.
pub fn analyse(index: &ServerIndex, settings: &ToolSettings, cancel: &AtomicBool) -> Result<Analysis, Error> {
    let began = Instant::now();
    let mut report = empty_report(index);
    if !index.supported() {
        report.timings.total_ms = index.walk_ms();
        return Ok(Analysis {
            report,
            edits: Vec::new(),
        });
    }
    let parse_began = Instant::now();
    let mut notes = index.notes_book().clone();
    notes.add(ToolNoteKind::BaseGameUnseen, None, None, None, None);
    let sources = load(index, &mut report, &mut notes, cancel)?;
    report.timings.parse_ms = elapsed_ms(parse_began);

    let registrations: Vec<Registration> = sources
        .iter()
        .enumerate()
        .flat_map(|(source, file)| (0..file.len()).map(move |entry| Registration { source, entry }))
        .collect();
    report.counts.registrations = count(registrations.len());
    let groups = group(&sources, &registrations);
    report.counts.groups = count(groups.len());
    report.groups_total = count(groups.len());

    let mut decider = Decider {
        index,
        settings,
        sources: &sources,
        writable: HashMap::new(),
    };
    let mut plan: Vec<(usize, Removal)> = Vec::new();
    let mut outcomes = Vec::with_capacity(groups.len());
    for (hash, copies) in groups {
        crate::resources::check(cancel)?;
        let difference = decider.difference(&copies);
        let reasons = decider.reasons(&copies, &difference);
        let action = if reasons.is_empty() {
            report.counts.groups_fixed += 1;
            let keeper = copies[copies.len() - 1];
            for &copy in &copies[..copies.len() - 1] {
                plan.push((copy.source, decider.removal(copy, keeper, hash)));
            }
            GroupAction::Fix {
                keep: count(copies.len() - 1),
                remove: (0..copies.len() - 1).map(count).collect(),
            }
        } else {
            GroupAction::Leave { reasons }
        };
        outcomes.push((hash, copies, difference, action));
    }

    let edits = edits(index, &sources, plan);
    report.counts.files_changed = count(edits.len());
    report.counts.removals = count(edits.iter().map(|edit| edit.removals.len()).sum());
    let names = names(index, &sources, &outcomes);
    report.groups = outcomes
        .into_iter()
        .take(MAX_GROUPS)
        .map(|(hash, copies, difference, action)| ArchetypeGroup {
            hash,
            name: names.get(&hash).cloned(),
            same_file: copies.iter().all(|copy| copy.source == copies[0].source),
            copies: copies
                .iter()
                .map(|&copy| describe(index, &sources, copy))
                .collect(),
            difference,
            action,
        })
        .collect();
    report.notes = notes.list().to_vec();
    report.notes_total = notes.total();
    report.timings.total_ms = index.walk_ms().saturating_add(elapsed_ms(began));
    Ok(Analysis { report, edits })
}

/// Whether the copy a removal relies on still stands in for the copy taken out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeeperCheck {
    Holds,
    /// The removal must be undone.
    Gone(KeeperGone),
}

/// Why the copy a removal relies on no longer stands in for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeeperGone {
    /// Its file is no longer there.
    Missing,
    /// Its file cannot be read: what it defines is unknown.
    Unreadable(String),
    /// Its file no longer registers the hash.
    NotDefined,
    /// Another file may be loaded in its place.
    Contested(Contest),
    /// It is no longer in the resource of the file changed, or no resource holds either.
    OtherResource,
    /// Its resource is now protected by the asset escrow.
    Escrowed,
    /// The owner excluded its resource.
    Excluded,
    /// Its resource no longer sends or loads it, or the server no longer runs GTA V.
    NotLoaded,
}

impl fmt::Display for KeeperGone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("the copy kept is no longer there"),
            Self::Unreadable(error) => write!(f, "the copy kept cannot be read: {error}"),
            Self::NotDefined => f.write_str("the copy kept no longer defines the archetype"),
            Self::Contested(_) => f.write_str("another file may be loaded in place of the copy kept"),
            Self::OtherResource => f.write_str("the copy kept is no longer in the same resource"),
            Self::Escrowed => f.write_str("the resource of the copy kept is now escrowed"),
            Self::Excluded => f.write_str("the resource of the copy kept is excluded"),
            Self::NotLoaded => f.write_str("the copy kept is no longer loaded"),
        }
    }
}

/// Checks, on a fresh index, that the copy `removal` relies on still stands in for the entry taken
/// out of `edited_path`: its file is there, in the same resource, still loaded and uncontested, its
/// resource neither escrowed (unless allowed) nor excluded, and, when its SHA-256 changed since the
/// plan, it still registers the hash. When in doubt, the answer is [`KeeperCheck::Gone`]: a duplicate
/// warning that comes back is harmless, a missing archetype is not.
pub fn check_keeper(
    index: &ServerIndex,
    settings: &ToolSettings,
    edited_path: &str,
    removal: &Removal,
) -> KeeperCheck {
    match keeper_gone(index, settings, edited_path, removal) {
        Some(gone) => KeeperCheck::Gone(gone),
        None => KeeperCheck::Holds,
    }
}

fn keeper_gone(
    index: &ServerIndex,
    settings: &ToolSettings,
    edited_path: &str,
    removal: &Removal,
) -> Option<KeeperGone> {
    if !index.supported() {
        return Some(KeeperGone::NotLoaded);
    }
    let keeper = &removal.keeper;
    let bytes = match read::read_bytes(&index.folder().join(&keeper.path)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(KeeperGone::Missing),
        Err(error) => return Some(KeeperGone::Unreadable(error.to_string())),
    };
    let Some(resource) = index.resource_of(&keeper.path) else {
        return Some(KeeperGone::OtherResource);
    };
    if index.resource_of(edited_path) != Some(resource) {
        return Some(KeeperGone::OtherResource);
    }
    let found = &index.resources()[resource];
    if found.escrowed && settings.escrowed == EscrowPolicy::Skip {
        return Some(KeeperGone::Escrowed);
    }
    if settings.excluded.contains(&found.name) {
        return Some(KeeperGone::Excluded);
    }
    let want = match keeper.format {
        Format::Ytyp => {
            let Some(at) = index.stream_file(&keeper.path).filter(|&at| index.file(at).sent) else {
                return Some(KeeperGone::NotLoaded);
            };
            if let Some(contest) = index.contests(at).into_iter().next() {
                return Some(KeeperGone::Contested(contest));
            }
            Want::Ytyp
        }
        Format::ModelInfo => {
            let Some(data_file) = found.model_info.iter().find(|file| file.path == keeper.path) else {
                return Some(KeeperGone::NotLoaded);
            };
            Want::ModelInfo(data_file.kind)
        }
    };
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    if sha256 == keeper.sha256 {
        return None;
    }
    match read::parse(&bytes, want) {
        Loaded::Ytyp { map, .. } if map.archetypes.iter().any(|entry| entry.name == removal.hash) => None,
        Loaded::ModelInfo { file, .. } if file.items.iter().any(|item| item.hash == removal.hash) => None,
        Loaded::Unreadable(problem) => Some(KeeperGone::Unreadable(problem_text(&problem))),
        Loaded::Ytyp { .. } | Loaded::ModelInfo { .. } | Loaded::Mismatch(_) => Some(KeeperGone::NotDefined),
    }
}

fn problem_text(problem: &FileProblem) -> String {
    match problem {
        FileProblem::Format { found } => format!("it is not a resource: it looks like {found:?}"),
        FileProblem::Invalid { error } | FileProblem::Io { error } => error.clone(),
    }
}

/// A loaded file and what it registers.
#[derive(Debug)]
struct Source {
    resource: usize,
    /// Relative to the server folder, with `/`.
    path: String,
    linked: bool,
    sha256: [u8; 32],
    content: Content,
    /// For a `.ytyp`, its place in the index.
    stream: Option<StreamRef>,
}

#[derive(Debug)]
enum Content {
    Ytyp(MapTypes),
    ModelInfo(ModelInfoFile),
}

impl Source {
    fn len(&self) -> usize {
        match &self.content {
            Content::Ytyp(map) => map.archetypes.len(),
            Content::ModelInfo(file) => file.items.len(),
        }
    }

    fn hash(&self, entry: usize) -> u32 {
        match &self.content {
            Content::Ytyp(map) => map.archetypes[entry].name,
            Content::ModelInfo(file) => file.items[entry].hash,
        }
    }

    /// The entry's position in the file as the edit names it.
    fn position(&self, entry: usize) -> usize {
        match &self.content {
            Content::Ytyp(map) => map.archetypes[entry].index,
            Content::ModelInfo(file) => file.items[entry].ordinal,
        }
    }

    fn format(&self) -> Format {
        match self.content {
            Content::Ytyp(_) => Format::Ytyp,
            Content::ModelInfo(_) => Format::ModelInfo,
        }
    }

    fn archetype(&self, entry: usize) -> Option<&Archetype> {
        match &self.content {
            Content::Ytyp(map) => map.archetypes.get(entry),
            Content::ModelInfo(_) => None,
        }
    }

    fn item(&self, entry: usize) -> Option<&ModelItem> {
        match &self.content {
            Content::Ytyp(_) => None,
            Content::ModelInfo(file) => file.items.get(entry),
        }
    }
}

/// One registration: an entry of a loaded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Registration {
    source: usize,
    entry: usize,
}

fn empty_report(index: &ServerIndex) -> ArchetypeReport {
    let resources = index.resources();
    let client: Vec<usize> = index.client_order().collect();
    let (notes, notes_total) = index.notes();
    ArchetypeReport {
        folder: index.folder().display().to_string(),
        counts: ArchetypeCounts {
            resources: count(resources.len()),
            started: count(index.started().len()),
            client: count(client.len()),
            escrowed: count(client.iter().filter(|&&at| resources[at].escrowed).count()),
            ..ArchetypeCounts::default()
        },
        groups: Vec::new(),
        groups_total: 0,
        shadowed: Vec::new(),
        shadowed_total: 0,
        unreadable: Vec::new(),
        unreadable_total: 0,
        notes: notes.to_vec(),
        notes_total,
        timings: ToolTimings {
            walk_ms: index.walk_ms(),
            parse_ms: 0,
            total_ms: 0,
        },
    }
}

/// A file to read, and where the report places it.
struct Job {
    resource: usize,
    path: String,
    linked: bool,
    stream: Option<StreamRef>,
    want: Want,
    /// The manifest line of a model-info file's data file.
    line: u32,
}

/// Lists the `.ytyp` files sent that the clients never load, since another file owns their slot.
fn shadowed(index: &ServerIndex, slots: &[crate::resources::Slot], report: &mut ArchetypeReport) {
    let resources = index.resources();
    for slot in slots {
        let owner = index.file(slot.owner);
        for &at in &slot.shadowed {
            report.counts.ytyp_shadowed += 1;
            report.shadowed_total += 1;
            if report.shadowed.len() < MAX_LISTED_FILES {
                report.shadowed.push(ShadowedFile {
                    resource: resources[at.resource].name.clone(),
                    path: index.file(at).path.clone(),
                    owner_resource: resources[slot.owner.resource].name.clone(),
                    owner_path: owner.path.clone(),
                });
            }
        }
    }
}

/// The files the clients load: the `.ytyp` slot owners in registration order, then the model-info
/// files in start order.
fn jobs(index: &ServerIndex, slots: &[crate::resources::Slot]) -> Vec<Job> {
    let mut jobs: Vec<Job> = slots
        .iter()
        .map(|slot| {
            let file = index.file(slot.owner);
            Job {
                resource: slot.owner.resource,
                path: file.path.clone(),
                linked: file.linked,
                stream: Some(slot.owner),
                want: Want::Ytyp,
                line: 0,
            }
        })
        .collect();
    for resource in index.client_order() {
        for data_file in &index.resources()[resource].model_info {
            jobs.push(Job {
                resource,
                path: data_file.path.clone(),
                linked: data_file.linked,
                stream: None,
                want: Want::ModelInfo(data_file.kind),
                line: data_file.line,
            });
        }
    }
    jobs
}

/// Reads the files the clients load, and lists in the report those shadowed or unreadable.
fn load(
    index: &ServerIndex,
    report: &mut ArchetypeReport,
    notes: &mut crate::resources::Notes,
    cancel: &AtomicBool,
) -> Result<Vec<Source>, Error> {
    let resources = index.resources();
    let slots = index.ytyp_slots();
    shadowed(index, &slots, report);
    let jobs = jobs(index, &slots);
    let paths: Vec<(PathBuf, Want)> = jobs
        .iter()
        .map(|job| (PathBuf::from(&job.path), job.want))
        .collect();
    let loaded = read::read_all(index.folder(), &paths, cancel)?;

    let mut sources = Vec::new();
    for (job, loaded) in jobs.into_iter().zip(loaded) {
        let (sha256, content) = match loaded {
            Loaded::Ytyp { sha256, map } => {
                report.counts.ytyp_loaded += 1;
                (sha256, Content::Ytyp(map))
            }
            Loaded::ModelInfo { sha256, file } => {
                report.counts.model_info_loaded += 1;
                (sha256, Content::ModelInfo(file))
            }
            Loaded::Mismatch(found) => {
                let detail = found.map_or("no model-info root", |kind| kind.data_file());
                notes.add(
                    ToolNoteKind::DataFileMismatch,
                    Some(resources[job.resource].name.clone()),
                    Some(job.path.clone()),
                    Some(job.line),
                    Some(detail.to_owned()),
                );
                continue;
            }
            Loaded::Unreadable(problem) => {
                match job.want {
                    Want::Ytyp => report.counts.ytyp_unreadable += 1,
                    Want::ModelInfo(_) => report.counts.model_info_unreadable += 1,
                }
                report.unreadable_total += 1;
                if report.unreadable.len() < MAX_LISTED_FILES {
                    report.unreadable.push(UnreadableFile {
                        resource: resources[job.resource].name.clone(),
                        path: job.path.clone(),
                        problem,
                    });
                }
                continue;
            }
        };
        sources.push(Source {
            resource: job.resource,
            path: job.path,
            linked: job.linked,
            sha256,
            content,
            stream: job.stream,
        });
    }
    Ok(sources)
}

/// The hashes registered more than once, in the order of their first registration, each with its
/// copies in registration order.
fn group(sources: &[Source], registrations: &[Registration]) -> Vec<(u32, Vec<Registration>)> {
    let mut by_hash: HashMap<u32, Vec<Registration>> = HashMap::new();
    let mut order = Vec::new();
    for &copy in registrations {
        let hash = sources[copy.source].hash(copy.entry);
        let copies = by_hash.entry(hash).or_default();
        if copies.is_empty() {
            order.push(hash);
        }
        copies.push(copy);
    }
    order
        .into_iter()
        .filter_map(|hash| {
            let copies = by_hash.remove(&hash)?;
            (copies.len() > 1).then_some((hash, copies))
        })
        .collect()
}

/// Classifies and decides groups, with what it learnt of each file.
struct Decider<'a> {
    index: &'a ServerIndex,
    settings: &'a ToolSettings,
    sources: &'a [Source],
    /// Why each file may not be changed, once asked.
    writable: HashMap<usize, Option<NotWritable>>,
}

impl Decider<'_> {
    /// How the copies differ from the last one.
    fn difference(&self, copies: &[Registration]) -> Difference {
        let last = copies[copies.len() - 1];
        let others = &copies[..copies.len() - 1];
        let source = |copy: Registration| &self.sources[copy.source];
        let mut fields: Vec<String> = Vec::new();
        let mut add = |names: Vec<String>| {
            for name in names {
                if !fields.contains(&name) {
                    fields.push(name);
                }
            }
        };
        if let Some(kept) = source(last).archetype(last.entry) {
            let mut identical = true;
            let mut minor = true;
            for &copy in others {
                let Some(other) = source(copy).archetype(copy.entry) else {
                    return mixed();
                };
                identical &= other.digest.is_some() && other.digest == kept.digest;
                minor &= other.major_digest.is_some() && other.major_digest == kept.major_digest;
                add(ytyp::differences(other, kept)
                    .into_iter()
                    .map(str::to_owned)
                    .collect());
            }
            return match (identical, minor) {
                (true, _) => Difference::Identical,
                (false, true) => Difference::Minor { fields },
                (false, false) => Difference::Major { fields },
            };
        }
        let Some(kept) = source(last).item(last.entry) else {
            return mixed();
        };
        let mut identical = true;
        for &copy in others {
            let Some(other) = source(copy).item(copy.entry) else {
                return mixed();
            };
            identical &= other.canonical == kept.canonical;
            add(model_info::differing_fields(other, kept));
        }
        if identical {
            Difference::Identical
        } else {
            if fields.is_empty() {
                fields.push("order".to_owned());
            }
            Difference::Major { fields }
        }
    }

    /// Every reason the group is left, none when it can be fixed.
    fn reasons(&mut self, copies: &[Registration], difference: &Difference) -> Vec<LeaveReason> {
        let mut reasons = Vec::new();
        if *difference != Difference::Identical && self.settings.differences != Differences::KeepLatest {
            reasons.push(LeaveReason::DefinitionsDiffer);
        }
        let mut files: Vec<usize> = Vec::new();
        for copy in copies {
            if !files.contains(&copy.source) {
                files.push(copy.source);
            }
        }
        let resources = self.index.resources();
        let first = self.sources[files[0]].resource;
        if files.iter().any(|&file| self.sources[file].resource != first) {
            reasons.push(LeaveReason::SeveralResources);
        }
        if self.settings.escrowed == EscrowPolicy::Skip
            && files
                .iter()
                .any(|&file| resources[self.sources[file].resource].escrowed)
        {
            reasons.push(LeaveReason::Escrowed);
        }
        let sources = self.sources;
        let archetypes = || {
            copies
                .iter()
                .filter_map(|copy| sources[copy.source].archetype(copy.entry))
        };
        if archetypes().any(|archetype| archetype.kind == ArchetypeKind::Mlo) {
            reasons.push(LeaveReason::Interior);
        }
        let ytyp_copies = archetypes().count();
        if ytyp_copies != 0 && ytyp_copies != copies.len() {
            reasons.push(LeaveReason::MixedSources);
        }
        for &file in &files {
            self.contests(file, &mut reasons);
        }
        if files.len() > 1 {
            for &file in &files {
                let source = &self.sources[file];
                if let Some(manifest) = source.stream.and_then(|at| self.index.manifest_dependency(at)) {
                    reasons.push(LeaveReason::ManifestDependent {
                        file: source.path.clone(),
                        manifest: manifest.to_owned(),
                    });
                }
            }
        }
        for &file in &files {
            if let Some(why) = self.not_writable(file) {
                reasons.push(LeaveReason::NotWritable {
                    file: self.sources[file].path.clone(),
                    why,
                });
            }
        }
        let mut excluded = HashSet::new();
        for &file in &files {
            let name = &resources[self.sources[file].resource].name;
            if self.settings.excluded.contains(name) && excluded.insert(name) {
                reasons.push(LeaveReason::Excluded {
                    resource: name.clone(),
                });
            }
        }
        reasons
    }

    /// What contests the `.ytyp` `file`: the first contest of each kind.
    fn contests(&self, file: usize, reasons: &mut Vec<LeaveReason>) {
        let source = &self.sources[file];
        let Some(at) = source.stream else { return };
        let mut kinds = HashSet::new();
        for contest in self.index.contests(at) {
            if kinds.insert(std::mem::discriminant(&contest)) {
                reasons.push(LeaveReason::Contested {
                    file: source.path.clone(),
                    by: contest,
                });
            }
        }
    }

    /// Why `file` may not be changed: a layout the change is not proven on, a link on its path, a
    /// path Observer never changes, a read-only file.
    fn not_writable(&mut self, file: usize) -> Option<NotWritable> {
        if let Some(known) = self.writable.get(&file) {
            return known.clone();
        }
        let source = &self.sources[file];
        let components: Vec<String> = source.path.split('/').map(str::to_owned).collect();
        let why = if let Content::Ytyp(MapTypes {
            patch_refusal: Some(detail),
            ..
        }) = &source.content
        {
            Some(NotWritable::Layout {
                detail: (*detail).to_owned(),
            })
        } else if source.linked || self.index.resources()[source.resource].linked {
            Some(NotWritable::Linked)
        } else if !source.path.starts_with("resources/")
            || built_in_refusal(&components, self.index.refused())
        {
            Some(NotWritable::Refused)
        } else if std::fs::metadata(self.index.folder().join(&source.path))
            .is_ok_and(|metadata| metadata.permissions().readonly())
        {
            Some(NotWritable::ReadOnly)
        } else {
            None
        };
        self.writable.insert(file, why.clone());
        why
    }

    /// The removal of `copy`, relying on `keeper`.
    fn removal(&self, copy: Registration, keeper: Registration, hash: u32) -> Removal {
        let kept = &self.sources[keeper.source];
        Removal {
            index: self.sources[copy.source].position(copy.entry),
            hash,
            keeper: Keeper {
                path: kept.path.clone(),
                format: kept.format(),
                index: kept.position(keeper.entry),
                sha256: kept.sha256,
            },
        }
    }
}

/// How a `.ytyp` copy meeting a model-info one differs: they cannot be compared.
fn mixed() -> Difference {
    Difference::Major {
        fields: vec!["source".to_owned()],
    }
}

/// The plan, one edit per file, in the order the files register.
fn edits(index: &ServerIndex, sources: &[Source], plan: Vec<(usize, Removal)>) -> Vec<Edit> {
    let mut by_file: Vec<(usize, Vec<Removal>)> = Vec::new();
    for (source, removal) in plan {
        match by_file.iter_mut().find(|(file, _)| *file == source) {
            Some((_, removals)) => removals.push(removal),
            None => by_file.push((source, vec![removal])),
        }
    }
    by_file.sort_by_key(|(source, _)| *source);
    by_file
        .into_iter()
        .map(|(source, mut removals)| {
            removals.sort_by_key(|removal| removal.index);
            let file = &sources[source];
            Edit {
                path: file.path.clone(),
                resource: index.resources()[file.resource].name.clone(),
                format: file.format(),
                sha256: file.sha256,
                removals,
            }
        })
        .collect()
}

/// The names of the grouped hashes: a model-info item's own, else the stem of a streamed drawable,
/// fragment, drawable dictionary or `.ytyp` of that hash. Only grouped hashes are looked up.
fn names(
    index: &ServerIndex,
    sources: &[Source],
    groups: &[(u32, Vec<Registration>, Difference, GroupAction)],
) -> HashMap<u32, String> {
    let mut names = HashMap::new();
    let mut wanted: HashSet<u32> = HashSet::new();
    for (hash, copies, _, _) in groups {
        let item = copies
            .iter()
            .find_map(|copy| sources[copy.source].item(copy.entry));
        match item {
            Some(item) => {
                names.insert(*hash, item.name.clone());
            }
            None => {
                wanted.insert(*hash);
            }
        }
    }
    if wanted.is_empty() {
        return names;
    }
    for resource in index.resources() {
        for file in &resource.stream {
            let Some(extension) = NAMED_EXTENSIONS
                .iter()
                .find(|extension| crate::resources::ends_with_ignoring_case(&file.name, extension))
            else {
                continue;
            };
            let stem = &file.name[..file.name.len() - extension.len()];
            let hash = name_hash(stem);
            if wanted.remove(&hash) {
                names.insert(hash, stem.to_owned());
                if wanted.is_empty() {
                    return names;
                }
            }
        }
    }
    names
}

fn describe(index: &ServerIndex, sources: &[Source], copy: Registration) -> ArchetypeCopy {
    let source = &sources[copy.source];
    let resource = &index.resources()[source.resource];
    let place = match &source.content {
        Content::Ytyp(map) => {
            let archetype = &map.archetypes[copy.entry];
            CopySource::Ytyp {
                index: count(archetype.index),
                archetype: match archetype.kind {
                    ArchetypeKind::Base => ArchetypeType::Base,
                    ArchetypeKind::Time => ArchetypeType::Time,
                    ArchetypeKind::Mlo => ArchetypeType::Mlo,
                },
            }
        }
        Content::ModelInfo(file) => {
            let item = &file.items[copy.entry];
            CopySource::ModelInfo {
                kind: match file.kind {
                    model_info::Kind::Weapon => ModelInfoKind::Weapon,
                    model_info::Kind::Vehicle => ModelInfoKind::Vehicle,
                    model_info::Kind::Ped => ModelInfoKind::Ped,
                },
                ordinal: count(item.ordinal),
                line: count(item.line),
            }
        }
    };
    ArchetypeCopy {
        resource: resource.name.clone(),
        path: source.path.clone(),
        source: place,
        escrowed: resource.escrowed,
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}
