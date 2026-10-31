//! A snapshot's job: the listing pass at once, the hashing pass once the server is online, and the
//! comparison with the previous start that lets identical rows be shared. It only reads the server
//! folder, and a new spawn cancels it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use observer_domain::artifacts::BuildPlatform;
use observer_domain::changes::{
    ChangesSettings, FileFlag, FileKind, LiveChangeKind, PartialReason, SnapshotKind, SnapshotState,
    StartCause,
};
use observer_domain::launch::LaunchSettings;
use observer_fxserver::changes::mask::{self, Redaction};
use observer_fxserver::changes::{Cancelled, Entry, Keys, ListError, Rate, ScanSettings, Shortened};
use observer_fxserver::supervisor::{State, Status};
use observer_store::changes::index::{self, IndexEntry};
use observer_store::changes::resources;
use observer_store::changes::snapshots::{self, NewSnapshot};
use observer_store::changes::texts::{self, NewText};
use observer_store::changes::{live, trees, upkeep};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::plan::{self, Chain, ListingInput, ListingPass, Planned, Read, Slot};
use super::{SERVER, Shared};
use crate::Error;

/// Hashing waits for the server to be online, or this long after the spawn.
const ONLINE_WAIT: Duration = Duration::from_mins(3);
/// Below this much free space on the data folder's volume, digests only are kept.
const DISK_FLOOR: u64 = 2_000_000_000;
/// Files read in one go of the hashing pass, between which progress is written.
const CHUNK_FILES: usize = 512;
const CHUNK_BYTES: u64 = 64 * 1024 * 1024;

/// A start of FXServer to take a snapshot of.
#[derive(Debug, Clone)]
pub(crate) struct Spawn {
    pub generation: u32,
    pub run_started: Timestamp,
    pub cause: Option<StartCause>,
    pub by: Option<String>,
    pub launch: LaunchSettings,
    /// The build folder it runs from.
    pub build: PathBuf,
}

/// How a job ended.
#[derive(Debug)]
pub(crate) enum Ended {
    Done(i64),
    Cancelled(i64),
    Failed,
}

/// Takes the snapshot of a start: the listing at once, the hashing once `status` says the run is
/// online, or three minutes later.
pub(crate) async fn start(
    shared: Arc<Shared>,
    settings: ChangesSettings,
    spawn: Spawn,
    status: watch::Receiver<Status>,
    cancel: CancellationToken,
) -> Result<Ended, Error> {
    let (build, build_sha256) = build_facts(&shared, spawn.build.clone()).await;
    let baseline = !shared
        .store
        .read(|conn| snapshots::has_starts(conn, SERVER))
        .await?;
    let taken_at = shared.clock.now();
    let snapshot = NewSnapshot {
        server: SERVER.to_owned(),
        kind: SnapshotKind::Start,
        run_started: Some(spawn.run_started),
        generation: Some(spawn.generation),
        cause: spawn.cause,
        by: spawn.by.clone(),
        build,
        build_sha256,
        game: None,
        game_build: argument(&spawn.launch, "sv_enforceGameBuild").and_then(|value| value.parse().ok()),
        launch: launch_record(&spawn.launch, &shared.keys),
        cfg: json!({}),
        baseline,
        taken_at,
    };
    let id = shared
        .store
        .write(move |tx| snapshots::insert(tx, &snapshot))
        .await?;
    shared.started(spawn.generation, id);
    shared.changed(id);
    let started_ns = i64::try_from(spawn.run_started.as_nanosecond()).unwrap_or(i64::MAX);
    let mut work = Work::new(Arc::clone(&shared), &settings, id, Some(started_ns), cancel);
    let ended = work
        .run(&spawn.launch, true, Some((spawn.generation, status)))
        .await?;
    if let Ended::Done(_) = ended {
        // A check taken before this start no longer says what the next restart applies.
        shared
            .store
            .write(move |tx| upkeep::stale_checks(tx, SERVER, taken_at))
            .await?;
    }
    if baseline {
        info!(snapshot = id, "the first snapshot of the server is a baseline");
    }
    Ok(ended)
}

/// Records a check of the disk, which [`check`] then takes: it shows at once as under way.
pub(crate) async fn new_check(shared: &Shared, launch: &LaunchSettings) -> Result<i64, Error> {
    let snapshot = NewSnapshot {
        server: SERVER.to_owned(),
        kind: SnapshotKind::Check,
        run_started: None,
        generation: None,
        cause: None,
        by: None,
        build: None,
        build_sha256: None,
        game: None,
        game_build: None,
        launch: launch_record(launch, &shared.keys),
        cfg: json!({}),
        baseline: false,
        taken_at: shared.clock.now(),
    };
    let id = shared
        .store
        .write(move |tx| snapshots::insert(tx, &snapshot))
        .await?;
    shared.changed(id);
    Ok(id)
}

/// Takes the check `id` of the disk as it is now, which replaces the previous check.
pub(crate) async fn check(
    shared: Arc<Shared>,
    settings: ChangesSettings,
    launch: LaunchSettings,
    id: i64,
    cancel: CancellationToken,
) -> Result<Ended, Error> {
    let mut work = Work::new(Arc::clone(&shared), &settings, id, None, cancel);
    let ended = work.run(&launch, true, None).await?;
    if let Ended::Done(id) = ended {
        shared
            .store
            .write(move |tx| upkeep::replace_checks(tx, SERVER, id))
            .await?;
    }
    Ok(ended)
}

/// Reads every file again that no read went through since `since`, when the round began, and
/// records what it finds there: a copy that kept both size and time shows at the next start. A
/// round cut short goes on where it stopped. No snapshot is taken.
pub(crate) async fn verify(
    shared: Arc<Shared>,
    settings: ChangesSettings,
    launch: LaunchSettings,
    since: Timestamp,
    cancel: CancellationToken,
) -> Result<bool, Error> {
    let Some(latest) = shared
        .store
        .read(|conn| snapshots::starts(conn, SERVER, None, 1))
        .await?
        .pop()
    else {
        return Ok(false);
    };
    let index = shared.store.read(index::load).await?;
    let bodies = shared.store.read(texts::with_body).await?;
    let input = ListingInput {
        data: PathBuf::from(&launch.data_path),
        launch,
        settings: shared.scan_settings(&settings),
        keys: shared.keys.clone(),
        index,
        bodies,
        read_text_now: false,
        verified_after: Some(since),
    };
    let mut rate = Rate::new(settings.hash_rate);
    let floor = below_floor(&shared.data_dir).await;
    let pass = {
        let cancel = cancel.clone();
        let (pass, returned) = tokio::task::spawn_blocking(move || {
            let pass = plan::listing_pass(&input, &mut rate, &cancel);
            (pass.map(|pass| (pass, input)), rate)
        })
        .await
        .map_err(|error| Error::Io(std::io::Error::other(error)))?;
        rate = returned;
        match pass {
            Ok(pass) => pass,
            Err(_) => return Ok(false),
        }
    };
    let (pass, input) = pass;
    let in_chain = Arc::new(chain_paths(&pass.chain));
    let waiting: Vec<((), Entry)> = pass
        .resources
        .iter()
        .flat_map(|planned| planned.slots.iter())
        .filter_map(|slot| match slot {
            Slot::Pending(entry) => Some(((), entry.clone())),
            Slot::Known(_) => None,
        })
        .collect();
    for chunk in waiting.chunks(CHUNK_FILES) {
        let (chunk, in_chain, settings, keys, cancel) = (
            chunk.to_vec(),
            Arc::clone(&in_chain),
            input.settings.clone(),
            input.keys.clone(),
            cancel.clone(),
        );
        let (reads, returned) = tokio::task::spawn_blocking(move || {
            let reads = plan::hash(chunk, &in_chain, &settings, &keys, &mut rate, &cancel);
            (reads, rate)
        })
        .await
        .map_err(|error| Error::Io(std::io::Error::other(error)))?;
        rate = returned;
        let Ok(reads) = reads else {
            return Ok(false);
        };
        let reads: Vec<Read> = reads.into_iter().map(|((), read)| read).collect();
        let mut batch = Batch::new(latest.id, None, floor);
        batch.add_reads(&reads, true);
        let now = shared.clock.now();
        shared.store.write(move |tx| batch.write(tx, now)).await?;
    }
    Ok(true)
}

/// What the launch settings say, as a snapshot keeps them: every value but the extra arguments',
/// which an argument may well hold a token in; each of those is kept as a tag that tells a value
/// changed and nothing else.
pub(crate) fn launch_record(launch: &LaunchSettings, keys: &Keys) -> Value {
    let arguments: Vec<Value> = launch
        .extra_arguments
        .iter()
        .map(|argument| {
            json!({
                "name": argument.name,
                "tag": mask::tag(&argument.value, &keys.tag),
            })
        })
        .collect();
    json!({
        "dataPath": launch.data_path,
        "cfgFile": launch.cfg_file,
        "onesync": launch.onesync,
        "autoStart": launch.auto_start,
        "txadminEvents": launch.txadmin_events,
        "arguments": arguments,
    })
}

/// The value an extra argument gives a convar, the last one winning as FXServer reads them.
fn argument<'a>(launch: &'a LaunchSettings, name: &str) -> Option<&'a str> {
    launch
        .extra_arguments
        .iter()
        .rev()
        .find(|argument| argument.name.eq_ignore_ascii_case(name))
        .map(|argument| argument.value.trim())
}

/// The build number and its archive's SHA-256, from the manifest of the build folder; a legacy
/// folder says nothing, and the number comes from `/info.json` later.
async fn build_facts(shared: &Shared, folder: PathBuf) -> (Option<u32>, Option<String>) {
    let manifest = tokio::task::spawn_blocking(move || observer_artifacts::Manifest::read(&folder).ok())
        .await
        .ok()
        .flatten();
    let Some(manifest) = manifest else {
        return (None, None);
    };
    let build = manifest.build;
    if let Some(sha256) = manifest.origin.archive_sha256() {
        return (Some(build), Some(sha256.to_owned()));
    }
    let fingerprint = shared
        .db
        .read(move |conn| observer_store::artifacts::fingerprint(conn, BuildPlatform::host(), build))
        .await
        .ok()
        .flatten();
    (Some(build), fingerprint)
}

/// The state of one job between its passes.
struct Work {
    shared: Arc<Shared>,
    scan: ScanSettings,
    id: i64,
    /// When the run started, for the files modified after it.
    started_ns: Option<i64>,
    cancel: CancellationToken,
    rate: Option<Rate>,
    resources: Vec<Planned>,
    shortened: Option<Shortened>,
    /// Categories the listing could not read: their resources are missing, not removed.
    unreadable: Vec<String>,
    /// The files of the cfg exec chain, masked as cfg files wherever they lie.
    in_chain: Arc<HashSet<String>>,
    /// No contents are kept: the data folder's volume is short of space.
    floor: bool,
}

impl Work {
    fn new(
        shared: Arc<Shared>,
        settings: &ChangesSettings,
        id: i64,
        started_ns: Option<i64>,
        cancel: CancellationToken,
    ) -> Self {
        let scan = shared.scan_settings(settings);
        let rate = Rate::new(settings.hash_rate);
        Self {
            shared,
            scan,
            id,
            started_ns,
            cancel,
            rate: Some(rate),
            resources: Vec::new(),
            shortened: None,
            unreadable: Vec::new(),
            in_chain: Arc::default(),
            floor: false,
        }
    }

    async fn run(
        &mut self,
        launch: &LaunchSettings,
        read_text_now: bool,
        online: Option<(u32, watch::Receiver<Status>)>,
    ) -> Result<Ended, Error> {
        match self.passes(launch, read_text_now, online).await {
            Ok(()) => match self.finish().await {
                Ok(()) => Ok(Ended::Done(self.id)),
                Err(error) => {
                    self.failed(&error).await;
                    Err(error)
                }
            },
            Err(Interrupted::Cancelled) => {
                self.end(SnapshotState::Partial, Some(PartialReason::Cancelled))
                    .await?;
                Ok(Ended::Cancelled(self.id))
            }
            Err(Interrupted::Failed(reason)) => {
                let (id, now) = (self.id, self.shared.clock.now());
                warn!(snapshot = id, %reason, "no snapshot could be taken");
                self.shared
                    .store
                    .write(move |tx| snapshots::fail(tx, id, &reason, now))
                    .await?;
                self.shared.changed(id);
                Ok(Ended::Failed)
            }
            Err(Interrupted::Error(error)) => {
                self.failed(&error).await;
                Err(error)
            }
        }
    }

    /// A write that failed leaves the snapshot failed rather than under way until the next start;
    /// best effort, as the database that refused one write may refuse this one.
    async fn failed(&self, error: &Error) {
        let (id, now) = (self.id, self.shared.clock.now());
        let reason = format!("the snapshot could not be stored: {error}");
        let written = self
            .shared
            .store
            .write(move |tx| snapshots::fail(tx, id, &reason, now))
            .await;
        if written.is_ok() {
            self.shared.changed(id);
        }
    }

    async fn passes(
        &mut self,
        launch: &LaunchSettings,
        read_text_now: bool,
        online: Option<(u32, watch::Receiver<Status>)>,
    ) -> Result<(), Interrupted> {
        self.floor = below_floor(&self.shared.data_dir).await;
        let pass = self.listing(launch, read_text_now).await?;
        self.write_listing(pass, launch).await?;
        if self.resources.iter().any(|planned| planned.pending() > 0) {
            if let Some((generation, status)) = online {
                self.wait_online(generation, status).await?;
            }
            self.hashing().await?;
        }
        Ok(())
    }

    async fn listing(
        &mut self,
        launch: &LaunchSettings,
        read_text_now: bool,
    ) -> Result<ListingPass, Interrupted> {
        let index = self.shared.store.read(index::load).await?;
        let bodies = self.shared.store.read(texts::with_body).await?;
        let input = ListingInput {
            data: PathBuf::from(&launch.data_path),
            launch: launch.clone(),
            settings: self.scan.clone(),
            keys: self.shared.keys.clone(),
            index,
            bodies,
            read_text_now,
            verified_after: None,
        };
        let mut rate = self.rate.take().unwrap_or_else(|| Rate::new(0));
        let cancel = self.cancel.clone();
        let (pass, rate) = tokio::task::spawn_blocking(move || {
            let pass = plan::listing_pass(&input, &mut rate, &cancel);
            (pass, rate)
        })
        .await
        .map_err(|error| Interrupted::Failed(format!("the listing stopped: {error}")))?;
        self.rate = Some(rate);
        match pass {
            Ok(pass) => Ok(pass),
            Err(ListError::Cancelled) => Err(Interrupted::Cancelled),
            Err(error @ ListError::Unreadable(_)) => Err(Interrupted::Failed(error.to_string())),
        }
    }

    async fn write_listing(&mut self, pass: ListingPass, launch: &LaunchSettings) -> Result<(), Interrupted> {
        let ListingPass {
            resources,
            reads,
            shortened,
            unreadable,
            chain,
            paths,
        } = pass;
        self.shortened = shortened;
        self.in_chain = Arc::new(chain_paths(&chain));
        if !unreadable.is_empty() {
            warn!(snapshot = self.id, categories = ?unreadable, "some categories could not be read");
        }
        self.unreadable = unreadable;
        let mut batch = Batch::new(self.id, self.started_ns, self.floor);
        let mut rows = Vec::with_capacity(resources.len());
        let mut listed = Vec::new();
        for planned in &resources {
            if let Some((sha256, entries)) = planned.tree() {
                listed.extend(plan::texts(&entries).into_iter().map(|(_, digest)| digest));
                batch.trees.push(tree_record(sha256.clone(), &entries));
                rows.push(planned.row(Some(sha256)));
            } else {
                for slot in &planned.slots {
                    if let Slot::Known(entry) = slot
                        && entry.kind == FileKind::Text
                        && let Some(digest) = entry.digest()
                    {
                        batch.attached.push((entry.path.clone(), digest.to_owned()));
                    }
                }
                rows.push(planned.row(None));
            }
        }
        batch.add_reads(&reads, true);
        batch.add_chain(&chain);
        let cfg = json!({
            "file": launch.cfg_file,
            "files": chain.files,
            "started": chain.started,
        });
        let (files, bytes) = resources.iter().fold((0, 0), |(files, bytes), planned| {
            (files + planned.resource.files(), bytes + planned.resource.bytes())
        });
        let state = if resources.iter().any(|planned| planned.pending() > 0) {
            SnapshotState::Hashing
        } else {
            SnapshotState::Listing
        };
        let (id, now, game, game_build) = (self.id, self.shared.clock.now(), chain.game, chain.game_build);
        let forget = (self.shortened.is_none() && self.unreadable.is_empty()).then_some(paths);
        self.shared
            .store
            .write(move |tx| -> Result<(), Error> {
                batch.write(tx, now)?;
                resources::insert(tx, id, &rows)?;
                texts::seen(tx, &listed, id)?;
                if let Some(paths) = &forget {
                    index::keep_only(tx, paths)?;
                }
                snapshots::set_chain(tx, id, &cfg, game, game_build)?;
                snapshots::listed(tx, id, files, bytes, state, now)?;
                Ok(())
            })
            .await?;
        self.resources = resources;
        self.shared.changed(self.id);
        Ok(())
    }

    /// Waits for the run to be online, for three minutes at most. A run that ended frees the disk,
    /// so hashing starts at once.
    async fn wait_online(
        &self,
        generation: u32,
        mut status: watch::Receiver<Status>,
    ) -> Result<(), Interrupted> {
        let deadline = tokio::time::sleep(ONLINE_WAIT);
        tokio::pin!(deadline);
        loop {
            {
                let now = status.borrow_and_update();
                let up = matches!(now.state, State::Online | State::Degraded);
                let over = now.generation != generation
                    || matches!(now.state, State::Stopped | State::Crashed | State::CrashLoop);
                if up || over {
                    return Ok(());
                }
            }
            tokio::select! {
                () = self.cancel.cancelled() => return Err(Interrupted::Cancelled),
                () = &mut deadline => return Ok(()),
                changed = status.changed() => if changed.is_err() {
                    return Ok(());
                },
            }
        }
    }

    async fn hashing(&mut self) -> Result<(), Interrupted> {
        let mut waiting: Vec<((usize, usize), Entry)> = Vec::new();
        for (resource, planned) in self.resources.iter().enumerate() {
            for (index, slot) in planned.slots.iter().enumerate() {
                if let Slot::Pending(entry) = slot {
                    waiting.push(((resource, index), entry.clone()));
                }
            }
        }
        let mut waiting = waiting.into_iter().peekable();
        while waiting.peek().is_some() {
            let mut chunk = Vec::new();
            let mut bytes = 0;
            while let Some(next) = waiting.next_if(|_| chunk.len() < CHUNK_FILES && bytes < CHUNK_BYTES) {
                bytes += next.1.stat.size;
                chunk.push(next);
            }
            let (in_chain, settings, keys, cancel) = (
                Arc::clone(&self.in_chain),
                self.scan.clone(),
                self.shared.keys.clone(),
                self.cancel.clone(),
            );
            let mut rate = self.rate.take().unwrap_or_else(|| Rate::new(0));
            let (reads, rate) = tokio::task::spawn_blocking(move || {
                let reads = plan::hash(chunk, &in_chain, &settings, &keys, &mut rate, &cancel);
                (reads, rate)
            })
            .await
            .map_err(|error| Interrupted::Failed(format!("the hashing stopped: {error}")))?;
            self.rate = Some(rate);
            let reads = reads.map_err(|Cancelled| Interrupted::Cancelled)?;
            self.write_chunk(reads).await?;
        }
        Ok(())
    }

    async fn write_chunk(&mut self, reads: Vec<((usize, usize), Read)>) -> Result<(), Interrupted> {
        let mut touched = HashSet::new();
        let mut batch = Batch::new(self.id, self.started_ns, self.floor);
        let plain: Vec<Read> = reads.iter().map(|(_, read)| read.clone()).collect();
        batch.add_reads(&plain, true);
        for ((resource, index), read) in reads {
            self.resources[resource].slots[index] = Slot::Known(read.entry());
            touched.insert(resource);
        }
        let mut done = Vec::new();
        let mut waiting = Vec::new();
        for resource in touched {
            let planned = &self.resources[resource];
            if let Some((sha256, entries)) = planned.tree() {
                batch.trees.push(tree_record(sha256.clone(), &entries));
                done.push((planned.resource.path.clone(), sha256));
            } else {
                let listing = serde_json::to_vec(&planned.partial_listing()).unwrap_or_default();
                waiting.push((planned.resource.path.clone(), listing, planned.pending() as u64));
            }
        }
        let (id, now) = (self.id, self.shared.clock.now());
        self.shared
            .store
            .write(move |tx| -> Result<(), Error> {
                batch.write(tx, now)?;
                for (path, tree) in &done {
                    resources::set_tree(tx, id, path, tree)?;
                }
                for (path, listing, pending) in &waiting {
                    resources::set_pending(tx, id, path, listing, *pending)?;
                }
                Ok(())
            })
            .await?;
        self.shared.changed(self.id);
        Ok(())
    }

    /// The snapshot is complete, or partial when the listing stopped at a limit or could not read
    /// a category. A start whose rows all equal the previous complete start's shares its rows.
    async fn finish(&self) -> Result<(), Error> {
        let reason = if self.shortened.is_some() {
            Some(PartialReason::Limit)
        } else if !self.unreadable.is_empty() {
            Some(PartialReason::Unreadable)
        } else if self.floor {
            Some(PartialReason::DiskFloor)
        } else {
            None
        };
        let state = if self.shortened.is_some() || !self.unreadable.is_empty() {
            SnapshotState::Partial
        } else {
            SnapshotState::Complete
        };
        let (id, now) = (self.id, self.shared.clock.now());
        self.shared
            .store
            .write(move |tx| -> Result<(), Error> {
                snapshots::finish(tx, id, state, reason, now)?;
                let Some(snapshot) = snapshots::get(tx, id)? else {
                    return Ok(());
                };
                if state != SnapshotState::Complete || snapshot.kind != SnapshotKind::Start {
                    return Ok(());
                }
                let Some(previous) = snapshots::previous_complete(tx, SERVER, id)? else {
                    return Ok(());
                };
                let holder = previous.holder();
                let theirs = resources::rows(tx, holder)?;
                let ours = resources::rows(tx, id)?;
                let same = theirs.len() == ours.len() && theirs.iter().zip(&ours).all(|(a, b)| a.same_as(b));
                if same {
                    snapshots::share_rows(tx, id, holder)?;
                }
                Ok(())
            })
            .await?;
        self.shared.changed(id);
        Ok(())
    }

    async fn end(&self, state: SnapshotState, reason: Option<PartialReason>) -> Result<(), Error> {
        let (id, now) = (self.id, self.shared.clock.now());
        self.shared
            .store
            .write(move |tx| snapshots::finish(tx, id, state, reason, now))
            .await?;
        self.shared.changed(id);
        Ok(())
    }
}

/// Why the passes stopped short.
enum Interrupted {
    Cancelled,
    /// Nothing could be listed; the snapshot is `failed` with this reason.
    Failed(String),
    Error(Error),
}

impl From<Error> for Interrupted {
    fn from(error: Error) -> Self {
        Self::Error(error)
    }
}

impl From<observer_store::StoreError> for Interrupted {
    fn from(error: observer_store::StoreError) -> Self {
        Self::Error(error.into())
    }
}

/// A tree as it is stored: its hash, its entries as JSON, its text entries, and the hash of its
/// entries' kinds.
type TreeRecord = (String, Vec<u8>, Vec<(String, String)>, String);

fn tree_record(sha256: String, entries: &[observer_fxserver::changes::tree::TreeEntry]) -> TreeRecord {
    let texts = plan::texts(entries);
    let kinds = observer_fxserver::changes::tree::kinds(entries);
    (
        sha256,
        serde_json::to_vec(entries).unwrap_or_default(),
        texts,
        kinds,
    )
}

/// The paths of the cfg exec chain's files.
fn chain_paths(chain: &Chain) -> HashSet<String> {
    chain.files.iter().map(|file| file.path.clone()).collect()
}

/// What one write of the job stores besides rows: trees, texts, index entries, flags and the
/// texts a snapshot holds outside its trees.
struct Batch {
    id: i64,
    started_ns: Option<i64>,
    floor: bool,
    trees: Vec<TreeRecord>,
    texts: Vec<NewText>,
    /// Texts of the cfg exec chain, masked as cfg files: they replace what the same bytes gave as
    /// an ordinary file.
    chain_texts: Vec<NewText>,
    index: Vec<(String, IndexEntry)>,
    flags: Vec<(String, FileFlag)>,
    /// Files modified after the start, with when: a live restart of their resource since then
    /// reloaded them, though it came before they were read.
    after_start: Vec<(String, i64)>,
    attached: Vec<(String, String)>,
}

impl Batch {
    fn new(id: i64, started_ns: Option<i64>, floor: bool) -> Self {
        Self {
            id,
            started_ns,
            floor,
            trees: Vec::new(),
            texts: Vec::new(),
            chain_texts: Vec::new(),
            index: Vec::new(),
            flags: Vec::new(),
            after_start: Vec::new(),
            attached: Vec::new(),
        }
    }

    fn add_reads(&mut self, reads: &[Read], indexed: bool) {
        for read in reads {
            if let (Some(masked), Some(digest), false) = (&read.read.masked, &read.read.digest, self.floor) {
                self.texts.push(NewText {
                    digest: digest.clone(),
                    text: masked.text.clone(),
                    bytes: read.read.bytes,
                    masking: mask::VERSION,
                    redactions: Redaction::encode(&masked.redactions),
                });
            }
            if indexed && let Some(entry) = read.index() {
                self.index.push((read.path.clone(), entry));
            }
            if self
                .started_ns
                .is_some_and(|started| read.read.stat.modified_ns > started)
            {
                self.flags.push((read.path.clone(), FileFlag::AfterStart));
                self.after_start
                    .push((read.path.clone(), read.read.stat.modified_ns));
            }
            if read.read.unsettled {
                self.flags.push((read.path.clone(), FileFlag::Unsettled));
            }
        }
    }

    fn add_chain(&mut self, chain: &Chain) {
        let before = self.texts.len();
        self.add_reads(&chain.reads, false);
        let chain_texts: Vec<NewText> = self.texts.drain(before..).collect();
        self.chain_texts.extend(chain_texts);
        for read in &chain.reads {
            if let Some(digest) = &read.read.digest {
                self.attached.push((read.path.clone(), digest.clone()));
            }
        }
    }

    fn write(&self, tx: &observer_store::Transaction<'_>, now: Timestamp) -> Result<(), Error> {
        for (sha256, listing, texts, kinds) in &self.trees {
            trees::insert(tx, sha256, listing, texts, kinds)?;
        }
        for text in &self.texts {
            texts::store(tx, text, self.id)?;
        }
        for text in &self.chain_texts {
            texts::store_over(tx, text, self.id)?;
        }
        for (path, digest) in &self.attached {
            texts::attach(tx, self.id, path, digest)?;
        }
        index::save(tx, &self.index, now)?;
        for (path, flag) in &self.flags {
            snapshots::flag(tx, self.id, path, *flag)?;
        }
        self.reloaded(tx)?;
        Ok(())
    }

    /// Flags the files a live start or restart of their resource reloaded: one that came after
    /// they were last modified.
    fn reloaded(&self, tx: &observer_store::Transaction<'_>) -> Result<(), Error> {
        if self.after_start.is_empty() {
            return Ok(());
        }
        let reloads: Vec<(String, i128)> = live::list(tx, self.id, u32::MAX)?
            .into_iter()
            .filter(|change| change.change != LiveChangeKind::Stopped)
            .map(|change| (change.name, change.at.as_nanosecond()))
            .collect();
        if reloads.is_empty() {
            return Ok(());
        }
        let Some(snapshot) = snapshots::get(tx, self.id)? else {
            return Ok(());
        };
        let names = resources::names(tx, snapshot.holder())?;
        for (path, modified_ns) in &self.after_start {
            let Some((_, resource)) = names.iter().find(|(root, _)| {
                path.strip_prefix(root.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            }) else {
                continue;
            };
            // Resource names are kept in lower case; the bridge reports them as written.
            let reloaded = reloads
                .iter()
                .any(|(name, at)| name.eq_ignore_ascii_case(resource) && *at > i128::from(*modified_ns));
            if reloaded {
                snapshots::flag(tx, self.id, path, FileFlag::ReloadedLive)?;
            }
        }
        Ok(())
    }
}

async fn below_floor(data_dir: &std::path::Path) -> bool {
    let data_dir = data_dir.to_owned();
    tokio::task::spawn_blocking(move || {
        data_dir
            .ancestors()
            .find_map(|dir| observer_platform::available_space(dir).ok())
            .is_some_and(|free| free < DISK_FLOOR)
    })
    .await
    .unwrap_or(false)
}
