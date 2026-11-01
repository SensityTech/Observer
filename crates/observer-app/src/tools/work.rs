//! What a run does on its blocking thread: read the cfg, index the folder, heal, analyse, apply or
//! undo, and gather what the run's record, the audit log and the console say of it. Everything
//! here reads and writes files, so it runs on a blocking thread, checking the cancel flag between
//! steps and between files.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use observer_domain::server_data::Game;
use observer_domain::tools::{ArchetypeReport, GroupAction, ToolRunOutcome, ToolRunSummary, ToolSettings};
use observer_fxserver::cfg::Scan;
use observer_tools::archetypes::{self, Analysis};
use observer_tools::edit::plan_digest;
use observer_tools::fix::{
    self, Backups, CloseReason, FileResult, FolderId, Journal, Outcome, Report, RunId, Selection,
    ServerFolder,
};
use observer_tools::resources::{self, ServerIndex, Starts};

use super::journal::StoreJournal;
use crate::OwnPaths;

/// The server folder a run works on, and how it learns which resources start.
#[derive(Debug, Clone)]
pub enum Target {
    /// The cfg `cfg_file` of the folder, read the way a launch reads it, with the system resources
    /// of the build at `system_resources` when the build is known.
    Cfg {
        folder: PathBuf,
        cfg_file: String,
        system_resources: Option<PathBuf>,
    },
    /// The scan a launch already made of the folder's cfg.
    Scanned {
        folder: PathBuf,
        scan: Box<Scan>,
        system_resources: Option<PathBuf>,
    },
    /// No cfg: every resource FXServer accepts is taken as started, in name order. A guess, which
    /// the report says.
    AllResources { folder: PathBuf },
}

impl Target {
    /// The server folder, as the launch settings or the command line give it.
    pub fn folder(&self) -> &Path {
        match self {
            Self::Cfg { folder, .. } | Self::Scanned { folder, .. } | Self::AllResources { folder } => folder,
        }
    }

    /// Which resources start, and the game the cfg sets. Reads the cfg for [`Target::Cfg`].
    fn starts(&self) -> io::Result<(Starts, Option<Game>)> {
        match self {
            Self::Cfg {
                folder,
                cfg_file,
                system_resources,
            } => {
                let scan = resources::scan_cfg(folder, cfg_file, system_resources.clone())?;
                let game = scan.facts.game;
                Ok((Starts::from_scan(&scan, system_resources.as_deref()), game))
            }
            Self::Scanned {
                scan,
                system_resources,
                ..
            } => Ok((
                Starts::from_scan(scan, system_resources.as_deref()),
                scan.facts.game,
            )),
            Self::AllResources { .. } => Ok((Starts::AllResources, None)),
        }
    }
}

/// What a run knows before it starts: which resources start, the game, Observer's own paths in
/// the folder and the folder's identity.
#[derive(Debug, Clone)]
pub(super) struct Prepared {
    pub starts: Starts,
    pub game: Option<Game>,
    pub refused: Vec<Vec<String>>,
    pub folder: FolderId,
}

impl Prepared {
    /// Whether the tools run on this server: GTA V, which a cfg setting no game runs too.
    pub fn supported(&self) -> bool {
        self.game != Some(Game::Rdr3)
    }
}

/// Reads what a run needs before it starts. Blocking.
pub(super) fn prepare(target: &Target, own: &OwnPaths) -> io::Result<Prepared> {
    let (starts, game) = target.starts()?;
    let folder = ServerFolder::open(target.folder()).map_err(io::Error::other)?;
    Ok(Prepared {
        starts,
        game,
        refused: own.inside(target.folder()),
        folder: folder.id().clone(),
    })
}

/// The report of a run that writes nothing, from a fresh index: the command line's report.
/// Blocking.
pub fn report(
    target: &Target,
    settings: &ToolSettings,
    own: &OwnPaths,
    cancel: &AtomicBool,
) -> Result<Analysis, ReportError> {
    let (starts, _) = target.starts().map_err(ReportError::Cfg)?;
    let index =
        ServerIndex::build(target.folder(), &starts, cancel)?.with_refused(own.inside(target.folder()));
    Ok(archetypes::analyse(&index, settings, cancel)?)
}

/// Why a report could not be made.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("the cfg cannot be read: {0}")]
    Cfg(#[source] io::Error),
    #[error(transparent)]
    Index(#[from] resources::Error),
}

/// What a run does once it has started.
#[derive(Debug, Clone)]
pub(super) enum Steps {
    /// Analyses, writing nothing: neither a heal nor a settling of interrupted writes.
    Report,
    /// Heals the open changes, if any, and nothing else.
    Heal,
    /// Heals, then analyses.
    HealAndReport,
    /// Heals, analyses again, and applies the plan: with a digest, only when the plan is that one.
    Fix(Option<[u8; 32]>),
    /// Puts back the entries the selection takes.
    Undo(Selection),
}

impl Steps {
    fn writes(&self) -> bool {
        !matches!(self, Self::Report)
    }
}

/// A run's work, handed to its blocking thread.
pub(super) struct Job {
    pub folder: PathBuf,
    pub prepared: Prepared,
    pub settings: ToolSettings,
    pub backups: Backups,
    pub journal: StoreJournal,
    pub run: RunId,
    pub cancel: Arc<AtomicBool>,
    /// The open changes of the folder when the run started.
    pub open: u32,
}

/// What a run did, for its record, the audit log and the console.
#[derive(Debug, Default)]
pub(super) struct Finished {
    pub outcome: Option<ToolRunOutcome>,
    pub summary: ToolRunSummary,
    pub plan: Option<[u8; 32]>,
    pub report: Option<ArchetypeReport>,
    /// The files a heal gave entries back to.
    pub healed: Vec<FileResult>,
    /// The files the plan was applied to, or not.
    pub applied: Vec<FileResult>,
    /// The files an undo gave entries back to, or not.
    pub undone: Vec<FileResult>,
    /// The records settled before the run: writes an earlier run was interrupted in.
    pub reconciled: Vec<FileResult>,
    /// The fix confirmed another plan than the fresh one: nothing was applied.
    pub plan_changed: bool,
}

impl Finished {
    /// The run's outcome; done unless something ended it early.
    pub fn outcome(&self) -> ToolRunOutcome {
        self.outcome.unwrap_or(ToolRunOutcome::Done)
    }

    /// Whether the run changed a file of the server folder.
    pub fn wrote(&self) -> bool {
        self.summary.files_changed > 0 || self.summary.files_restored > 0
    }

    /// The paths of the files the plan was written to.
    pub fn changed_paths(&self) -> Vec<String> {
        written(&self.applied)
    }

    pub fn healed_paths(&self) -> Vec<String> {
        written(&self.healed)
    }

    pub fn undone_paths(&self) -> Vec<String> {
        written(&self.undone)
    }

    fn fail(&mut self, problem: String) {
        self.outcome = Some(ToolRunOutcome::Failed);
        self.summary.problem = Some(problem);
    }

    fn cancelled(&mut self) {
        self.outcome.get_or_insert(ToolRunOutcome::Cancelled);
    }

    /// Takes in what a heal, an apply or an undo did.
    fn took(&mut self, report: &Report) {
        self.reconciled.extend(report.reconciled.iter().cloned());
        if report.cancelled {
            self.cancelled();
        }
        if report.storage_full && self.outcome.is_none() {
            self.fail("a volume is full or would fall below its reserve: the run stopped there".to_owned());
        }
    }
}

/// Runs `steps` on the job's folder. Every failure ends in the run's outcome, never in an error:
/// the run is recorded whatever happened. Blocking.
pub(super) fn run(mut job: Job, steps: &Steps) -> Finished {
    let began = Instant::now();
    let mut finished = Finished::default();
    if let Err(problem) = run_steps(&mut job, steps, &mut finished) {
        match problem {
            Stop::Cancelled => finished.cancelled(),
            Stop::Failed(problem) => finished.fail(problem),
        }
    }
    if job.cancel.load(Ordering::Relaxed) {
        finished.cancelled();
    }
    finished.summary.elapsed_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    finished.summary.open_changes = if steps.writes() {
        open_now(&mut job).unwrap_or(job.open)
    } else {
        job.open
    };
    finished
}

/// What ended a run before its last step.
enum Stop {
    Cancelled,
    Failed(String),
}

impl From<resources::Error> for Stop {
    fn from(error: resources::Error) -> Self {
        match error {
            resources::Error::Cancelled => Self::Cancelled,
            error @ resources::Error::Folder { .. } => Self::Failed(error.to_string()),
        }
    }
}

impl From<fix::Error> for Stop {
    fn from(error: fix::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

fn run_steps(job: &mut Job, steps: &Steps, finished: &mut Finished) -> Result<(), Stop> {
    if !steps.writes() {
        let index = index(job)?;
        analysed(finished, archetypes::analyse(&index, &job.settings, &job.cancel)?);
        return Ok(());
    }
    let folder = ServerFolder::open(&job.folder)?.with_refused(job.prepared.refused.clone());
    if let Steps::Undo(selection) = steps {
        let report = fix::undo(
            &mut job.journal,
            &folder,
            &job.backups,
            selection,
            job.run,
            &job.cancel,
        )?;
        finished.took(&report);
        finished.summary.files_restored = count(written(&report.files).len());
        finished.undone = report.files;
        return Ok(());
    }
    let index = index(job)?;
    if job.open > 0 {
        let report = fix::heal(
            &index,
            &job.settings,
            &mut job.journal,
            &folder,
            &job.backups,
            job.run,
            &job.cancel,
        )?;
        finished.took(&report);
        finished.summary.files_restored = count(written(&report.files).len());
        finished.healed = report.files;
    }
    if matches!(steps, Steps::Heal) || !job.prepared.supported() || job.cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    let analysis = archetypes::analyse(&index, &job.settings, &job.cancel)?;
    let Steps::Fix(confirmed) = steps else {
        analysed(finished, analysis);
        return Ok(());
    };
    let digest = plan_digest(&analysis.edits);
    if confirmed.is_some_and(|confirmed| confirmed != digest) {
        finished.plan_changed = true;
        finished.summary.problem =
            Some("the plan changed since it was shown: nothing was applied".to_owned());
        analysed(finished, analysis);
        return Ok(());
    }
    job.journal.learn(&analysis.report);
    let report = fix::apply(
        &analysis.edits,
        &index,
        &mut job.journal,
        &folder,
        &job.backups,
        job.run,
        &job.cancel,
    )?;
    finished.took(&report);
    let written: HashSet<&str> = report
        .files
        .iter()
        .filter(|file| matches!(file.outcome, Outcome::Written { .. }))
        .map(|file| file.path.as_str())
        .collect();
    finished.summary.files_changed = count(written.len());
    finished.summary.removals = count(
        analysis
            .edits
            .iter()
            .filter(|edit| written.contains(edit.path.as_str()))
            .map(|edit| edit.removals.len())
            .sum(),
    );
    finished.summary.fixed = count(
        analysis
            .report
            .groups
            .iter()
            .filter(|group| match &group.action {
                GroupAction::Fix { remove, .. } => remove.iter().all(|&at| {
                    usize::try_from(at)
                        .ok()
                        .and_then(|at| group.copies.get(at))
                        .is_some_and(|copy| written.contains(copy.path.as_str()))
                }),
                GroupAction::Leave { .. } => false,
            })
            .count(),
    );
    analysed(finished, analysis);
    finished.applied = report.files;
    Ok(())
}

fn index(job: &Job) -> Result<ServerIndex, Stop> {
    Ok(
        ServerIndex::build(&job.folder, &job.prepared.starts, &job.cancel)?
            .with_refused(job.prepared.refused.clone()),
    )
}

/// Takes in an analysis: its report, its plan's digest and its counts.
fn analysed(finished: &mut Finished, analysis: Analysis) {
    let counts = analysis.report.counts;
    finished.summary.groups = counts.groups;
    finished.summary.fixable = counts.groups_fixed;
    finished.summary.left = counts.groups.saturating_sub(counts.groups_fixed);
    finished.plan = Some(plan_digest(&analysis.edits));
    finished.report = Some(analysis.report);
}

fn open_now(job: &mut Job) -> Option<u32> {
    let folder = job.prepared.folder.clone();
    job.journal.open(&folder).ok().map(|records| count(records.len()))
}

/// The files a heal, an apply or an undo wrote.
fn written(files: &[FileResult]) -> Vec<String> {
    files
        .iter()
        .filter(|file| {
            matches!(
                file.outcome,
                Outcome::Written { .. }
                    | Outcome::Closed {
                        reason: CloseReason::Healed | CloseReason::Undone
                    }
            )
        })
        .map(|file| file.path.clone())
        .collect()
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
