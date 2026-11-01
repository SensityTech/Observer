//! `observer tools`: the server tools by hand. A report reads the server folder and writes nothing,
//! the database included, whether Observer runs or not. A fix and an undo write into the server
//! folder, so they take Observer's lock: while Observer runs, they belong to its Tools page, which
//! runs them with FXServer stopped. The settings change while Observer runs too: they carry a
//! version. The folder and the cfg are those of the launch settings unless the command names
//! others.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, anyhow, bail};
use clap::{ArgGroup, Args, Subcommand, ValueEnum};
use observer_app::{Clock, CommitGate, OwnPaths, Servers, Target, Tools, ToolsSetup};
use observer_domain::error::ErrorCode;
use observer_domain::launch::StoredLaunch;
use observer_domain::tools::{
    ArchetypeCopy, ArchetypeGroup, ArchetypeReport, Contest, CopySource, Difference, Differences,
    EscrowPolicy, GroupAction, LeaveReason, NotWritable, ToolMode, ToolNoteKind, ToolRunOutcome,
    ToolRunSummary, ToolSettings, ToolUndoScope,
};
use observer_fxserver::layout::{Flavour, Layout};
use observer_store::{Database, artifacts, launch};
use observer_tools::fix::Outcome;
use serde_json::json;

use crate::config::{Config, DataPaths};
use crate::lock;

/// What `observer tools run` exits with when duplicates are left once it ends: every group a
/// report finds, since it fixes none, and the groups a fix did not fix. An error, or a fix that
/// stopped before the end, exits with 1.
const GROUPS_LEFT: u8 = 2;

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    /// Scan the server folder and report what the tool finds; with --fix, apply what it can fix.
    /// Exits with 2 while duplicates are left, 1 on an error.
    Run(RunArgs),
    /// Put back what fixes took out, in the folder of the launch settings. Stop Observer first.
    Undo(UndoArgs),
    /// Show what a tool does at every start, or change it. Works while Observer runs.
    Config(ConfigArgs),
}

/// The tools there are.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ToolName {
    /// Archetypes the clients register more than once: the `Duplicate Archetype` warnings.
    DuplicateArchetypes,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    tool: ToolName,
    /// Apply the fixes. Takes Observer's lock: stop Observer first, or fix from its Tools page.
    #[arg(long)]
    fix: bool,
    /// Print the report as JSON; with --fix, the run and what became of each file.
    #[arg(long)]
    json: bool,
    /// The server folder; the launch settings' by default.
    #[arg(long, value_name = "DIR")]
    data: Option<PathBuf>,
    /// The cfg FXServer runs, relative to the server folder; the launch settings' by default, or
    /// server.cfg with --data.
    #[arg(long, value_name = "FILE", conflicts_with = "all_resources")]
    cfg: Option<String>,
    /// Read no cfg: take every resource as started, in name order.
    #[arg(long)]
    all_resources: bool,
    /// Which duplicates may be fixed, for this run only; the saved setting by default.
    #[arg(long, value_enum)]
    differences: Option<DifferencesArg>,
    /// Whether resources protected by the asset escrow may be changed, for this run only; the
    /// saved setting by default.
    #[arg(long, value_enum)]
    escrowed: Option<EscrowedArg>,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("scope").required(true).args(["run", "all"])))]
pub struct UndoArgs {
    tool: ToolName,
    /// Put back what this run took out, in every file it changed.
    #[arg(long, value_name = "ID")]
    run: Option<i64>,
    /// Put back everything: every changed file gets its original again.
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    tool: ToolName,
    /// What the tool does at every start of the server.
    #[arg(long, value_enum)]
    at_start: Option<ModeArg>,
    /// Which duplicates may be fixed.
    #[arg(long, value_enum)]
    differences: Option<DifferencesArg>,
    /// Whether resources protected by the asset escrow may be changed.
    #[arg(long, value_enum)]
    escrowed: Option<EscrowedArg>,
    /// Never change this resource: its name as FXServer knows it, case included. Repeat it for
    /// several.
    #[arg(long, value_name = "NAME")]
    exclude: Vec<String>,
    /// Take this resource off the excluded ones. Repeat it for several.
    #[arg(long, value_name = "NAME")]
    include: Vec<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    /// Nothing but heal what earlier fixes need put back.
    Off,
    /// Scan and say what changed in the console; write nothing but heals.
    Report,
    /// Scan and apply the fixes.
    Fix,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DifferencesArg {
    /// Only copies identical in every field.
    Identical,
    /// Copies that differ too, keeping the last registered: unverified in game.
    // The name the panel's API and the database use is taken too.
    #[value(alias = "keepLatest")]
    KeepLatest,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EscrowedArg {
    /// Report them only: their author ships the fix.
    Skip,
    /// Fix their files the escrow does not encrypt too.
    Allow,
}

impl From<ModeArg> for ToolMode {
    fn from(mode: ModeArg) -> Self {
        match mode {
            ModeArg::Off => Self::Off,
            ModeArg::Report => Self::Report,
            ModeArg::Fix => Self::Fix,
        }
    }
}

impl From<DifferencesArg> for Differences {
    fn from(differences: DifferencesArg) -> Self {
        match differences {
            DifferencesArg::Identical => Self::Identical,
            DifferencesArg::KeepLatest => Self::KeepLatest,
        }
    }
}

impl From<EscrowedArg> for EscrowPolicy {
    fn from(escrowed: EscrowedArg) -> Self {
        match escrowed {
            EscrowedArg::Skip => Self::Skip,
            EscrowedArg::Allow => Self::Allow,
        }
    }
}

impl From<ToolMode> for ModeArg {
    fn from(mode: ToolMode) -> Self {
        match mode {
            ToolMode::Off => Self::Off,
            ToolMode::Report => Self::Report,
            ToolMode::Fix => Self::Fix,
        }
    }
}

impl From<Differences> for DifferencesArg {
    fn from(differences: Differences) -> Self {
        match differences {
            Differences::Identical => Self::Identical,
            Differences::KeepLatest => Self::KeepLatest,
        }
    }
}

impl From<EscrowPolicy> for EscrowedArg {
    fn from(escrowed: EscrowPolicy) -> Self {
        match escrowed {
            EscrowPolicy::Skip => Self::Skip,
            EscrowPolicy::Allow => Self::Allow,
        }
    }
}

pub async fn command(config: &Config, command: ToolsCommand) -> anyhow::Result<ExitCode> {
    match command {
        ToolsCommand::Run(args) if args.fix => fix(config, &args).await,
        ToolsCommand::Run(args) => report(config, &args).await,
        ToolsCommand::Undo(args) => undo(config, &args).await.map(|()| ExitCode::SUCCESS),
        ToolsCommand::Config(args) => configure(config, &args).await.map(|()| ExitCode::SUCCESS),
    }
}

/// `observer tools run`: a report, which takes no lock and never writes.
async fn report(config: &Config, args: &RunArgs) -> anyhow::Result<ExitCode> {
    let paths = config.paths();
    let db = if paths.database.exists() {
        Some(open(&paths).await?)
    } else {
        None
    };
    let tools = db.as_ref().map(|db| service(config, &paths, db.clone()));
    let stored = match &db {
        Some(db) => stored(db).await?,
        None => None,
    };
    let saved = match &tools {
        Some(tools) => tools.settings().await?.0,
        None => ToolSettings::default(),
    };
    let target = target(args, stored.as_ref(), &paths)?;
    let settings = overridden(saved, args);
    let own = OwnPaths::new(&paths.root, &config.path);
    let scanned = {
        let target = target.clone();
        tokio::task::spawn_blocking(move || {
            observer_app::report(&target, &settings, &own, &AtomicBool::new(false))
        })
        .await?
    };
    let analysis = scanned.context("cannot scan the server folder")?;
    refuse_unsupported(&analysis.report)?;
    let open = match &tools {
        Some(tools) => tools.open_changes(target.folder().to_path_buf()).await.ok(),
        None => None,
    };
    if let Some(db) = db {
        db.close().await?;
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&analysis.report)?);
    } else {
        print!("{}", describe(&analysis.report, &Done::Report { open }));
    }
    Ok(exit_code(analysis.report.counts.groups > 0))
}

/// `observer tools run --fix`: Observer's lock, then the fresh plan applied.
async fn fix(config: &Config, args: &RunArgs) -> anyhow::Result<ExitCode> {
    let paths = config.paths();
    let _lock = observer_lock(&paths)?;
    let db = open(&paths).await?;
    let tools = service(config, &paths, db.clone());
    let stored = stored(&db).await?;
    let target = target(args, stored.as_ref(), &paths)?;
    let settings = overridden(tools.settings().await?.0, args);
    let ran = tools.fix_from_cli(target, settings).await;
    db.close().await?;
    let ran = ran.map_err(refusal)?;
    let summary = ran.run.summary.clone().unwrap_or_default();
    if args.json {
        let files: Vec<_> = ran.files().collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "run": ran.run, "files": files }))?
        );
    } else {
        let written: Vec<String> = ran
            .files()
            .map(|file| format!("{}: {}", file.path, observer_app::outcome_text(&file.outcome)))
            .collect();
        let fixed: Vec<String> = ran
            .applied
            .iter()
            .filter(|file| matches!(file.outcome, Outcome::Written { .. }))
            .map(|file| file.path.clone())
            .collect();
        match &ran.run.report {
            Some(report) => print!(
                "{}",
                describe(
                    report,
                    &Done::Fix {
                        run: ran.run.id,
                        fixed: &fixed,
                        written: &written,
                        open: summary.open_changes,
                        backups: paths.root.join("tools").join("backups"),
                    }
                )
            ),
            None => println!("nothing was fixed"),
        }
    }
    ended(ran.run.outcome, &summary)?;
    Ok(exit_code(summary.groups > summary.fixed))
}

/// `observer tools undo`: Observer's lock, then the removals put back.
async fn undo(config: &Config, args: &UndoArgs) -> anyhow::Result<()> {
    let paths = config.paths();
    let _lock = observer_lock(&paths)?;
    let db = open(&paths).await?;
    let tools = service(config, &paths, db.clone());
    let scope = match args.run {
        Some(run) if !args.all => ToolUndoScope::Run(run),
        _ => ToolUndoScope::All,
    };
    let ran = tools.undo_from_cli(scope).await;
    db.close().await?;
    let ran = ran.map_err(refusal)?;
    if ran.files().next().is_none() {
        println!("nothing to put back");
    }
    for file in ran.files() {
        println!("{}: {}", file.path, observer_app::outcome_text(&file.outcome));
    }
    let summary = ran.run.summary.clone().unwrap_or_default();
    println!(
        "{} back; {} still open",
        files(u64::from(summary.files_restored)),
        files(u64::from(summary.open_changes))
    );
    ended(ran.run.outcome, &summary)
}

/// `observer tools config`: the settings, changed when an option says so. Works while Observer
/// runs: the settings carry a version, and a running Observer reads them at every start.
async fn configure(config: &Config, args: &ConfigArgs) -> anyhow::Result<()> {
    if let Some(name) = args.exclude.iter().find(|name| args.include.contains(name)) {
        bail!("{name} cannot be excluded and included at once");
    }
    let paths = config.paths();
    let db = open(&paths).await?;
    let tools = service(config, &paths, db.clone());
    let settings: anyhow::Result<ToolSettings> = async {
        let (saved, version) = tools.settings().await?;
        let asked = configured(saved.clone(), args);
        if asked == saved {
            return Ok(saved);
        }
        let (settings, _) = tools.configure_from_cli(asked, version).await.map_err(refusal)?;
        Ok(settings)
    }
    .await;
    db.close().await?;
    let settings = settings?;
    println!("at start: {}", spelled(&ModeArg::from(settings.at_start)));
    println!(
        "differences: {}",
        spelled(&DifferencesArg::from(settings.differences))
    );
    println!("escrowed: {}", spelled(&EscrowedArg::from(settings.escrowed)));
    if settings.excluded.is_empty() {
        println!("excluded: none");
    } else {
        println!("excluded: {}", settings.excluded.join(", "));
    }
    Ok(())
}

/// The settings with what the options change: a name excluded is added once, after the others; a
/// name included is taken off.
fn configured(mut settings: ToolSettings, args: &ConfigArgs) -> ToolSettings {
    if let Some(mode) = args.at_start {
        settings.at_start = mode.into();
    }
    if let Some(differences) = args.differences {
        settings.differences = differences.into();
    }
    if let Some(escrowed) = args.escrowed {
        settings.escrowed = escrowed.into();
    }
    settings.excluded.retain(|name| !args.include.contains(name));
    for name in &args.exclude {
        if !settings.excluded.contains(name) {
            settings.excluded.push(name.clone());
        }
    }
    settings
}

/// A setting's value as the options spell it.
fn spelled(value: &impl ValueEnum) -> String {
    value
        .to_possible_value()
        .map(|value| value.get_name().to_owned())
        .unwrap_or_default()
}

/// Why a run that may have written did not end as it should, as the error the command exits with
/// once it said what the run did.
fn ended(outcome: Option<ToolRunOutcome>, summary: &ToolRunSummary) -> anyhow::Result<()> {
    if outcome == Some(ToolRunOutcome::Done) {
        return Ok(());
    }
    let problem = summary.problem.as_deref().unwrap_or("it did not end");
    bail!("the run stopped before the end: {problem}")
}

/// Observer's lock, or why the command must go through the panel instead.
fn observer_lock(paths: &DataPaths) -> anyhow::Result<lock::DataDirLock> {
    lock::acquire(&paths.root).context(
        "Observer is running on this data folder: fix and undo from its Tools page, which runs them \
         with FXServer stopped, or stop Observer first",
    )
}

/// The database alongside a running Observer, as the other commands open it.
async fn open(paths: &DataPaths) -> anyhow::Result<Database> {
    Database::open_existing(&paths.database, &paths.backups)
        .await
        .with_context(|| format!("cannot open {}", paths.database.display()))
}

fn service(config: &Config, paths: &DataPaths, db: Database) -> Tools {
    Tools::new(ToolsSetup {
        db,
        clock: Clock::system(),
        data_dir: paths.root.clone(),
        artifacts: paths.artifacts.clone(),
        status: None,
        gate: CommitGate::default(),
        own_paths: OwnPaths::new(&paths.root, &config.path),
    })
}

/// The launch settings, and the selected build.
async fn stored(db: &Database) -> anyhow::Result<Option<(StoredLaunch, Option<u32>)>> {
    Ok(db
        .read(|conn| -> Result<_, observer_store::StoreError> {
            let stored = launch::get(conn, Servers::DEFAULT)?;
            let selected = artifacts::state(conn, Servers::DEFAULT)?.selected;
            Ok(stored.map(|stored| (stored, selected)))
        })
        .await?)
}

/// The folder and the cfg the options name, or the launch settings'.
fn target(
    args: &RunArgs,
    stored: Option<&(StoredLaunch, Option<u32>)>,
    paths: &DataPaths,
) -> anyhow::Result<Target> {
    let folder = match (&args.data, stored) {
        (Some(data), _) => {
            std::path::absolute(data).with_context(|| format!("cannot resolve {}", data.display()))?
        }
        (None, Some((launch, _))) => PathBuf::from(&launch.settings.data_path),
        (None, None) => {
            bail!("no server folder: pass --data, or save the launch settings in the panel first")
        }
    };
    if args.all_resources {
        return Ok(Target::AllResources { folder });
    }
    let cfg_file = match (&args.cfg, &args.data, stored) {
        (Some(cfg), _, _) => cfg.clone(),
        (None, None, Some((launch, _))) => launch.settings.cfg_file.clone(),
        _ => "server.cfg".to_owned(),
    };
    let system_resources = stored
        .and_then(|(_, selected)| *selected)
        .map(|build| Layout::new(paths.artifacts.join(build.to_string()), Flavour::HOST).system_resources());
    Ok(Target::Cfg {
        folder,
        cfg_file,
        system_resources,
    })
}

fn overridden(mut settings: ToolSettings, args: &RunArgs) -> ToolSettings {
    if let Some(differences) = args.differences {
        settings.differences = differences.into();
    }
    if let Some(escrowed) = args.escrowed {
        settings.escrowed = escrowed.into();
    }
    settings
}

fn refuse_unsupported(report: &ArchetypeReport) -> anyhow::Result<()> {
    if report
        .notes
        .iter()
        .any(|note| note.kind == ToolNoteKind::UnsupportedGame)
    {
        bail!("this tool works on GTA V servers only: the cfg runs RedM");
    }
    Ok(())
}

/// A refusal of the service in words, other failures as they are.
fn refusal(error: observer_app::Error) -> anyhow::Error {
    match error.code() {
        Some(ErrorCode::ToolNotAvailable) => {
            anyhow!("this tool works on GTA V servers only: the cfg runs RedM")
        }
        Some(ErrorCode::LaunchNotConfigured) => {
            anyhow!("no launch settings yet: save them in the panel first, or pass --data")
        }
        Some(ErrorCode::Conflict) => anyhow!("the settings changed meanwhile: run the command again"),
        Some(ErrorCode::InvalidRequest) => anyhow!(
            "these settings are refused: an excluded name is empty, longer than 255 bytes or holds / \
             or \\, or more than 1000 are excluded"
        ),
        _ => error.into(),
    }
}

/// `1 file`, `2 files`.
fn files(count: u64) -> String {
    if count == 1 {
        "1 file".to_owned()
    } else {
        format!("{count} files")
    }
}

/// `1 file holds`, `2 files hold`.
fn holding(count: u32) -> String {
    if count == 1 {
        "1 file holds".to_owned()
    } else {
        format!("{count} files hold")
    }
}

fn exit_code(groups_left: bool) -> ExitCode {
    if groups_left {
        ExitCode::from(GROUPS_LEFT)
    } else {
        ExitCode::SUCCESS
    }
}

/// What a run did besides its report.
enum Done<'a> {
    Report {
        /// The changes open in the folder, when the database says.
        open: Option<u32>,
    },
    Fix {
        /// The run's number, which an undo takes.
        run: i64,
        /// The files the plan was written to.
        fixed: &'a [String],
        /// Each file looked at, with what became of it.
        written: &'a [String],
        open: u32,
        backups: PathBuf,
    },
}

/// A report in words: its counts, then each group fixed or left with why, then what was written.
fn describe(report: &ArchetypeReport, done: &Done<'_>) -> String {
    let counts = &report.counts;
    let groups = usize::try_from(counts.groups).unwrap_or(usize::MAX);
    let mut text = String::new();
    let _ = writeln!(text, "duplicate archetypes in {}", report.folder);
    let _ = writeln!(
        text,
        "  {} resources, {} started, {} of them escrowed; read {} .ytyp and {} model-info files",
        counts.resources, counts.started, counts.escrowed, counts.ytyp_loaded, counts.model_info_loaded
    );
    match done {
        Done::Report { open } => {
            let fixable = usize::try_from(counts.groups_fixed).unwrap_or(usize::MAX);
            let _ = writeln!(
                text,
                "  {groups} registered more than once: {fixable} fixable in {}, {} left",
                files(u64::from(counts.files_changed)),
                groups.saturating_sub(fixable)
            );
            if let Some(open) = open.filter(|open| *open > 0) {
                let _ = writeln!(
                    text,
                    "  {} an earlier fix: the next start, or the next fix, puts back any entry whose \
                     copy kept is gone",
                    holding(open)
                );
            }
        }
        Done::Fix { fixed, open, .. } => {
            let fixed_groups = report
                .groups
                .iter()
                .filter(|group| fixed_group(group, fixed))
                .count();
            let _ = writeln!(
                text,
                "  {groups} registered more than once: {fixed_groups} fixed in {}, {} left",
                files(fixed.len() as u64),
                groups.saturating_sub(fixed_groups)
            );
            if *open > 0 {
                let _ = writeln!(text, "  {} a fix, kept to heal or undo", holding(*open));
            }
        }
    }
    if report.notes_total > 0 {
        let _ = writeln!(
            text,
            "  {} notes on what the prediction could not check: --json lists them",
            report.notes_total
        );
    }
    for group in &report.groups {
        text.push('\n');
        describe_group(&mut text, group, done);
    }
    if report.groups.len() < groups {
        let _ = writeln!(
            text,
            "\n{} more groups: --json lists the first {}",
            groups - report.groups.len(),
            report.groups.len()
        );
    }
    match done {
        Done::Fix {
            run,
            fixed,
            written,
            backups,
            ..
        } if !written.is_empty() => {
            text.push_str("\nwritten:\n");
            for line in *written {
                let _ = writeln!(text, "  {line}");
            }
            let _ = writeln!(text, "the originals are kept in {}", backups.display());
            if !fixed.is_empty() {
                let _ = writeln!(
                    text,
                    "to put back what run {run} took out: observer tools undo duplicate-archetypes --run {run}"
                );
            }
        }
        Done::Report { .. } | Done::Fix { .. } => text.push_str("\nnothing was written\n"),
    }
    text
}

fn describe_group(text: &mut String, group: &ArchetypeGroup, done: &Done<'_>) {
    let name = group.name.as_deref().unwrap_or("(unnamed)");
    let difference = match &group.difference {
        Difference::Identical => "identical".to_owned(),
        Difference::Minor { fields } => format!("differ in draw distances or bounds: {}", fields.join(", ")),
        Difference::Major { fields } => format!("differ in {}", fields.join(", ")),
    };
    match &group.action {
        GroupAction::Fix { keep, remove } => {
            let label = match done {
                Done::Report { .. } => "fixable",
                Done::Fix { fixed, .. } if fixed_group(group, fixed) => "fixed",
                Done::Fix { .. } => "not fixed",
            };
            let _ = writeln!(text, "{label} {name} {:08X}, {difference}", group.hash);
            if let Some(copy) = copy_at(group, *keep) {
                let _ = writeln!(text, "    keep    {}", describe_copy(copy));
            }
            for &at in remove {
                if let Some(copy) = copy_at(group, at) {
                    let _ = writeln!(text, "    remove  {}", describe_copy(copy));
                }
            }
        }
        GroupAction::Leave { reasons } => {
            let reasons: Vec<String> = reasons.iter().map(describe_reason).collect();
            let _ = writeln!(
                text,
                "left {name} {:08X}, {difference}: {}",
                group.hash,
                reasons.join("; ")
            );
            for copy in &group.copies {
                let _ = writeln!(text, "    {}", describe_copy(copy));
            }
        }
    }
}

/// Whether every copy a group's fix takes out is in a file the run wrote.
fn fixed_group(group: &ArchetypeGroup, fixed: &[String]) -> bool {
    match &group.action {
        GroupAction::Fix { remove, .. } => remove
            .iter()
            .all(|&at| copy_at(group, at).is_some_and(|copy| fixed.contains(&copy.path))),
        GroupAction::Leave { .. } => false,
    }
}

fn copy_at(group: &ArchetypeGroup, at: u32) -> Option<&ArchetypeCopy> {
    usize::try_from(at).ok().and_then(|at| group.copies.get(at))
}

fn describe_copy(copy: &ArchetypeCopy) -> String {
    let place = match copy.source {
        CopySource::Ytyp { index, .. } => format!("#{index}"),
        CopySource::ModelInfo { ordinal, line, .. } => format!("item {ordinal}, line {line}"),
    };
    let escrowed = if copy.escrowed { " (escrowed)" } else { "" };
    format!("{} {place}{escrowed}", copy.path)
}

fn describe_reason(reason: &LeaveReason) -> String {
    match reason {
        LeaveReason::DefinitionsDiffer => "the copies differ".to_owned(),
        LeaveReason::SeveralResources => "the copies are in several resources".to_owned(),
        LeaveReason::Escrowed => "a copy's resource is escrowed".to_owned(),
        LeaveReason::Interior => "a copy defines an interior".to_owned(),
        LeaveReason::MixedSources => "a .ytyp copy meets a model-info one".to_owned(),
        LeaveReason::Contested { file, by } => format!("{file} {}", describe_contest(by)),
        LeaveReason::ManifestDependent { file, manifest } => format!("{file} is listed by {manifest}"),
        LeaveReason::NotWritable { file, why } => format!(
            "{file} cannot be changed: {}",
            match why {
                NotWritable::Layout { detail } =>
                    format!("its layout is not one the change is proven on ({detail})"),
                NotWritable::ReadOnly => "it is read-only".to_owned(),
                NotWritable::Linked => "it is reached through a link".to_owned(),
                NotWritable::Refused => "Observer never changes it".to_owned(),
            }
        ),
        LeaveReason::Excluded { resource } => format!("{resource} is excluded"),
    }
}

fn describe_contest(contest: &Contest) -> String {
    match contest {
        Contest::SameName { resource, path } => format!("has a namesake in {resource}: {path}"),
        Contest::Sibling { path } => format!("has a namesake in its own resource: {path}"),
        Contest::Requested { resource } => format!("is requested by {resource}"),
        Contest::Manifest { resource, path } => format!("is named by {path} of {resource}"),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(subcommand)]
        command: ToolsCommand,
    }

    #[test]
    fn the_options_read_as_the_tools_page_names_them() {
        let cli = Cli::try_parse_from([
            "tools",
            "run",
            "duplicate-archetypes",
            "--fix",
            "--differences",
            "keep-latest",
            "--escrowed",
            "allow",
        ])
        .unwrap();
        let ToolsCommand::Run(args) = cli.command else {
            panic!("not a run");
        };
        assert!(args.fix);
        let settings = overridden(ToolSettings::default(), &args);
        assert_eq!(settings.differences, Differences::KeepLatest);
        assert_eq!(settings.escrowed, EscrowPolicy::Allow);
        assert_eq!(settings.at_start, ToolMode::Off);
    }

    #[test]
    fn a_cfg_and_every_resource_cannot_be_asked_together_nor_an_undo_without_its_scope() {
        assert!(
            Cli::try_parse_from([
                "tools",
                "run",
                "duplicate-archetypes",
                "--cfg",
                "server.cfg",
                "--all-resources"
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["tools", "undo", "duplicate-archetypes"]).is_err());
        assert!(
            Cli::try_parse_from(["tools", "undo", "duplicate-archetypes", "--run", "4", "--all"]).is_err()
        );
        assert!(Cli::try_parse_from(["tools", "undo", "duplicate-archetypes", "--all"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "tools",
                "config",
                "duplicate-archetypes",
                "--differences",
                "keepLatest"
            ])
            .is_ok(),
            "the name the panel's API uses is taken too"
        );
    }

    #[test]
    fn config_adds_each_excluded_name_once_and_takes_the_included_ones_off() {
        let cli = Cli::try_parse_from([
            "tools",
            "config",
            "duplicate-archetypes",
            "--exclude",
            "maps",
            "--exclude",
            "cars",
            "--include",
            "props",
            "--differences",
            "keep-latest",
        ])
        .unwrap();
        let ToolsCommand::Config(args) = cli.command else {
            panic!("not a config");
        };
        let saved = ToolSettings {
            excluded: vec!["props".to_owned(), "maps".to_owned()],
            ..ToolSettings::default()
        };

        let settings = configured(saved, &args);

        assert_eq!(settings.excluded, ["maps", "cars"]);
        assert_eq!(settings.differences, Differences::KeepLatest);
        assert_eq!(settings.at_start, ToolMode::Off);
        assert_eq!(
            spelled(&DifferencesArg::from(settings.differences)),
            "keep-latest",
            "config prints what it takes"
        );
    }

    #[test]
    fn a_run_that_did_not_end_is_an_error_that_says_why() {
        let failed = ToolRunSummary {
            problem: Some("a volume is full".to_owned()),
            ..ToolRunSummary::default()
        };
        let done = ToolRunSummary::default();

        assert!(ended(Some(ToolRunOutcome::Done), &done).is_ok());
        let error = ended(Some(ToolRunOutcome::Failed), &failed).unwrap_err();
        assert!(error.to_string().contains("a volume is full"), "{error}");
        assert!(ended(Some(ToolRunOutcome::Cancelled), &done).is_err());
        assert!(ended(None, &done).is_err());
    }

    #[test]
    fn the_folder_and_cfg_come_from_the_options_before_the_launch_settings() {
        let paths = DataPaths::new(PathBuf::from("data"));
        let parse = |line: &[&str]| {
            let cli = Cli::try_parse_from(line).unwrap();
            let ToolsCommand::Run(args) = cli.command else {
                panic!("not a run");
            };
            args
        };
        let named = parse(&["tools", "run", "duplicate-archetypes", "--data", "/srv/other"]);
        let all = parse(&["tools", "run", "duplicate-archetypes", "--all-resources"]);
        let bare = parse(&["tools", "run", "duplicate-archetypes"]);

        let Target::Cfg { cfg_file, .. } = target(&named, None, &paths).unwrap() else {
            panic!("not a cfg");
        };
        assert_eq!(cfg_file, "server.cfg");
        assert!(target(&bare, None, &paths).is_err());
        assert!(target(&all, None, &paths).is_err());
    }
}
