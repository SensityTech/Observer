//! What the panel reads of the history: the starts, two of them compared, one resource across
//! them, and a file's diff or text. Contents are read from masked bodies only, and their reads go
//! to the audit log once an hour per account, file and pair of snapshots.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{AuditAction, AuditEntry};
use observer_domain::changes::{
    ArgumentChange, ArgumentChangeKind, Attribution, CfgChange, CfgFileChange, ChangeCounts, ChangesLive,
    ChangesSettingsView, Comparison, EnvironmentChange, EnvironmentFact, FileChange, FileChangeKind,
    FileDiff, FileFlag, FileKind, FileText, GameChange, LineCounts, LiveChange, NumberChange, PackageChange,
    ResourceChange, ResourceChangeKind, ResourceDetail, ResourceVersion, SettingChange, SnapshotKind,
    SnapshotState, StartList, glob_covers, hidden,
};
use observer_domain::error::{ApiError, ErrorCode};
use observer_fxserver::bridge::{self, BridgeEvent, NumberMoved, StartChange, StartCounts, StartHead};
use observer_fxserver::changes::diff::{self, DiffOptions};
use observer_fxserver::changes::mask::{self, Masked, Redaction};
use observer_fxserver::changes::tree::TreeEntry;
use observer_store::Connection;
use observer_store::changes::resources::{self, ResourceRow};
use observer_store::changes::snapshots::{self, Snapshot};
use observer_store::changes::{live, texts, trees, upkeep};
use observer_store::{StoreError, audit, diagnostics};
use serde_json::{Value, json};

use super::compare::{self, Pairs};
use super::plan::ChainFile;
use super::{Changes, SERVER};
use crate::audit::Record;
use crate::{Caller, Error};

/// Starts listed in one page.
const STARTS_PAGE: u32 = 50;
/// Text files whose lines one comparison counts, at most, and their stored bytes.
const LINE_FILES: usize = 400;
const LINE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ATTRIBUTIONS: u32 = 500;
const MAX_LIVE: u32 = 200;
const MAX_HISTORY: u32 = 200;
/// A content read of one file and pair of snapshots is audited once in this window per account.
const AUDIT_WINDOW: SignedDuration = SignedDuration::from_hours(1);
const ENVIRONMENT: [AuditAction; 6] = [
    AuditAction::BuildSelected,
    AuditAction::PendingBuildApplied,
    AuditAction::BuildRolledBack,
    AuditAction::ProbationPassed,
    AuditAction::ProbationFailed,
    AuditAction::LaunchSettingsChanged,
];
const WRITES: [AuditAction; 3] = [
    AuditAction::FileWritten,
    AuditAction::FileDeleted,
    AuditAction::FileRenamed,
];
/// Launch settings every viewer sees before and after; the owner also sees the cfg file and the
/// data folder.
const SHOWN_SETTINGS: [&str; 3] = ["onesync", "autoStart", "txadminEvents"];
const OWNER_SETTINGS: [&str; 2] = ["cfgFile", "dataPath"];

/// The later side of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparedTo {
    Start(i64),
    /// The latest check of the disk.
    Disk,
}

/// A snapshot and the rows that list its resources.
struct Side {
    snapshot: Snapshot,
    rows: Vec<ResourceRow>,
}

fn side(conn: &Connection, snapshot: Snapshot) -> Result<Side, StoreError> {
    let rows = resources::rows(conn, snapshot.holder())?;
    Ok(Side { snapshot, rows })
}

/// A row's entries: its tree's, or those its pending listing knows so far.
fn entries(conn: &Connection, row: &ResourceRow) -> Result<Option<Vec<TreeEntry>>, StoreError> {
    let listing = match (&row.tree, &row.pending_listing) {
        (Some(tree), _) => trees::listing(conn, tree)?,
        (None, Some(pending)) => Some(pending.clone()),
        (None, None) => None,
    };
    Ok(listing.and_then(|listing| serde_json::from_slice(&listing).ok()))
}

/// A stored text as the masking left it, or `None` when its body is not kept.
fn masked(conn: &Connection, digest: &str) -> Result<Option<Masked>, StoreError> {
    Ok(texts::get(conn, digest)?.and_then(|text| {
        Some(Masked {
            text: text.text?,
            redactions: Redaction::decode(&text.redactions),
        })
    }))
}

fn kept(conn: &Connection, digest: &str) -> Result<bool, StoreError> {
    Ok(texts::get(conn, digest)?.is_some_and(|text| text.text.is_some()))
}

/// The two snapshots a request names: `to` a start or the latest check, `from` a start or, when
/// none is named, the start before `to`.
fn snapshots_of(
    conn: &Connection,
    from: Option<i64>,
    to: Option<ComparedTo>,
) -> Result<(Snapshot, Snapshot), Error> {
    let found = |id: i64| -> Result<Snapshot, Error> {
        snapshots::get(conn, id)?
            .filter(|snapshot| snapshot.server == SERVER)
            .ok_or_else(|| ErrorCode::StartNotFound.into())
    };
    let to = match to {
        Some(ComparedTo::Start(id)) => found(id)?,
        Some(ComparedTo::Disk) => snapshots::latest_check(conn, SERVER)?.ok_or(ErrorCode::StartNotFound)?,
        None => snapshots::starts(conn, SERVER, None, 1)?
            .pop()
            .ok_or(ErrorCode::StartNotFound)?,
    };
    let from = if let Some(id) = from {
        found(id)?
    } else if to.kind == SnapshotKind::Start {
        snapshots::reference(conn, SERVER, to.id)?.ok_or(ErrorCode::StartNotFound)?
    } else {
        // The disk compares with the start that ran when it was checked.
        snapshots::ran_at(conn, SERVER, to.taken_at)?.ok_or(ErrorCode::StartNotFound)?
    };
    Ok((from, to))
}

/// What a comparison found before the state database's attributions are added.
struct Compared {
    counts: ChangeCounts,
    resources: Vec<ResourceChange>,
    cfg: CfgChange,
    live: Vec<LiveChange>,
    changed_while_starting: u32,
}

/// Compares two sides: resources, their files where they changed, lines within a budget.
fn compared(
    conn: &Connection,
    from: &Side,
    to: &Side,
    hidden_globs: &[String],
    with_files: bool,
) -> Result<Compared, StoreError> {
    let (resources, unchanged) = changed_resources(conn, from, to, hidden_globs, with_files)?;
    let mut counts = compare::count(&resources, unchanged);
    // A side still listing has read no cfg yet: its chain is not known to differ.
    let listed = from.snapshot.listed_at.is_some() && to.snapshot.listed_at.is_some();
    let cfg = if listed {
        cfg_change(conn, &from.snapshot, &to.snapshot)?.0
    } else {
        CfgChange::default()
    };
    counts.environment = environment_facts(&from.snapshot, &to.snapshot, &cfg);
    let (live, changed_while_starting) = live_of(conn, &to.snapshot)?;
    Ok(Compared {
        counts,
        resources,
        cfg,
        live,
        changed_while_starting,
    })
}

/// The resources that changed, ranked, and how many did not.
fn changed_resources(
    conn: &Connection,
    from: &Side,
    to: &Side,
    hidden_globs: &[String],
    with_files: bool,
) -> Result<(Vec<ResourceChange>, u32), StoreError> {
    let mut pairs = compare::pair(&from.rows, &to.rows);
    let mut read = |before: bool, index: usize| {
        let row = if before {
            &from.rows[index]
        } else {
            &to.rows[index]
        };
        with_files.then(|| entries(conn, row).ok().flatten()).flatten()
    };
    compare::renames(&mut pairs, &from.rows, &to.rows, &mut read);
    let settled = compare::settled(&to.snapshot);
    let mut budget = (LINE_FILES, LINE_BYTES);
    let mut resources = Vec::new();
    let mut unchanged = 0;
    let Pairs {
        paired,
        removed,
        added,
    } = pairs;
    for (a, b, how) in paired {
        let (before, after) = (&from.rows[a], &to.rows[b]);
        let did_not_start = compare::started(&from.snapshot, &before.name) == Some(true)
            && compare::started(&to.snapshot, &after.name) == Some(false);
        let waiting = before.tree.is_none() || after.tree.is_none();
        let sides = if waiting || (with_files && before.tree != after.tree) {
            entries(conn, before)?.zip(entries(conn, after)?)
        } else {
            None
        };
        let (kind, changed) = compare::classify(
            before,
            after,
            how,
            sides.as_ref().map(|(x, y)| (x.as_slice(), y.as_slice())),
            did_not_start,
            settled,
        );
        if kind == ResourceChangeKind::Unchanged {
            unchanged += 1;
            continue;
        }
        let mut change = compare::resource_change(Some(before), Some(after), kind, changed);
        if with_files && let Some((x, y)) = &sides {
            let files = compare::files(Some((&before.path, x)), Some((&after.path, y)), hidden_globs);
            change.files = compare::file_counts(&files);
            change.lines = lines(conn, &files, x, y, &mut budget)?;
        }
        resources.push(change);
    }
    // A side still listing has no rows yet, and one whose listing left resources out does not
    // say the others are gone.
    let missing = |side: &Side, otherwise: ResourceChangeKind| {
        let snapshot = &side.snapshot;
        if snapshot.state == SnapshotState::Listing {
            ResourceChangeKind::Checking
        } else if compare::shortened(snapshot)
            || snapshot.listed_at.is_none()
            || snapshot.state == SnapshotState::Failed
        {
            // Cut short or failed before its listing ended: it has no rows to say anything by.
            ResourceChangeKind::Unknown
        } else {
            otherwise
        }
    };
    let (gone, new) = (
        missing(to, ResourceChangeKind::Removed),
        missing(from, ResourceChangeKind::Added),
    );
    for before in removed.into_iter().map(|index| &from.rows[index]) {
        let mut change = compare::resource_change(Some(before), None, gone, true);
        if with_files && let Some(x) = entries(conn, before)? {
            change.files =
                compare::file_counts(&compare::files(Some((&before.path, &x)), None, hidden_globs));
        }
        resources.push(change);
    }
    for after in added.into_iter().map(|index| &to.rows[index]) {
        let mut change = compare::resource_change(None, Some(after), new, true);
        if with_files && let Some(y) = entries(conn, after)? {
            change.files = compare::file_counts(&compare::files(None, Some((&after.path, &y)), hidden_globs));
        }
        resources.push(change);
    }
    resources.sort_by(|x, y| (compare::rank(x.change), &x.path).cmp(&(compare::rank(y.change), &y.path)));
    Ok((resources, unchanged))
}

/// Whether the builds of two snapshots compare: a check of the disk runs nothing, so it has none.
fn builds(from: &Snapshot, to: &Snapshot) -> bool {
    from.kind == SnapshotKind::Start && to.kind == SnapshotKind::Start
}

/// What changed besides the resources.
fn environment_facts(from: &Snapshot, to: &Snapshot, cfg: &CfgChange) -> Vec<EnvironmentFact> {
    [
        (EnvironmentFact::Build, builds(from, to) && from.build != to.build),
        (
            EnvironmentFact::GameBuild,
            builds(from, to) && from.game_build != to.game_build,
        ),
        (EnvironmentFact::Launch, from.launch != to.launch),
        (
            EnvironmentFact::Cfg,
            cfg.changed > 0 || !cfg.started.is_empty() || !cfg.stopped.is_empty(),
        ),
    ]
    .into_iter()
    .filter(|(_, changed)| *changed)
    .map(|(fact, _)| fact)
    .collect()
}

/// What started and stopped while a start's run was up, and how many of its files changed while
/// the server was starting.
fn live_of(conn: &Connection, to: &Snapshot) -> Result<(Vec<LiveChange>, u32), StoreError> {
    if to.kind != SnapshotKind::Start {
        return Ok((Vec::new(), 0));
    }
    let live = live::list(conn, to.id, MAX_LIVE)?
        .into_iter()
        .map(|change| LiveChange {
            at: change.at,
            name: change.name,
            change: change.change,
            by: change.by,
            command: change.command,
        })
        .collect();
    let after_start = snapshots::flags(conn, to.id)?
        .into_iter()
        .filter(|(_, flag)| *flag == FileFlag::AfterStart)
        .map(|(path, _)| path)
        .collect::<HashSet<_>>()
        .len();
    Ok((live, u32::try_from(after_start).unwrap_or(u32::MAX)))
}

/// Lines added and removed in the modified text files, while the comparison's budget lasts.
fn lines(
    conn: &Connection,
    files: &[FileChange],
    before: &[TreeEntry],
    after: &[TreeEntry],
    budget: &mut (usize, u64),
) -> Result<Option<LineCounts>, StoreError> {
    let mut total = LineCounts::default();
    for file in files {
        let Some((old, new)) = compare::texts_of(file, before, after) else {
            continue;
        };
        let (Some(old), Some(new)) = (masked(conn, &old)?, masked(conn, &new)?) else {
            continue;
        };
        let bytes = (old.text.len() + new.text.len()) as u64;
        if budget.0 == 0 || budget.1 < bytes {
            return Ok(None);
        }
        budget.0 -= 1;
        budget.1 -= bytes;
        let (added, removed) = diff::counts(Some(&old), Some(&new));
        total = compare::sum_lines([total, LineCounts { added, removed }].into_iter());
    }
    Ok(Some(total))
}

fn chain_files(snapshot: &Snapshot) -> Vec<ChainFile> {
    snapshot
        .cfg
        .get("files")
        .and_then(|files| serde_json::from_value(files.clone()).ok())
        .unwrap_or_default()
}

fn chain_started(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .cfg
        .get("started")
        .and_then(|started| serde_json::from_value(started.clone()).ok())
        .unwrap_or_default()
}

/// The cfg exec chain compared, and every path it holds on either side.
fn cfg_change(
    conn: &Connection,
    from: &Snapshot,
    to: &Snapshot,
) -> Result<(CfgChange, HashSet<String>), StoreError> {
    let (before, after) = (chain_files(from), chain_files(to));
    let chain: HashSet<String> = before
        .iter()
        .chain(&after)
        .map(|file| file.path.clone())
        .collect();
    let mut files = Vec::new();
    for path in &chain {
        let was = before
            .iter()
            .find(|file| &file.path == path)
            .and_then(|file| file.digest.clone());
        let now = after
            .iter()
            .find(|file| &file.path == path)
            .and_then(|file| file.digest.clone());
        let change = match (&was, &now) {
            (Some(a), Some(b)) if a == b => continue,
            (None, None) => continue,
            (Some(_), Some(_)) => FileChangeKind::Modified,
            (None, Some(_)) => FileChangeKind::Added,
            (Some(_), None) => FileChangeKind::Removed,
        };
        let old = was
            .as_deref()
            .map(|digest| masked(conn, digest))
            .transpose()?
            .flatten();
        let new = now
            .as_deref()
            .map(|digest| masked(conn, digest))
            .transpose()?
            .flatten();
        let lines = (old.is_some() || new.is_some()).then(|| {
            let (added, removed) = diff::counts(old.as_ref(), new.as_ref());
            LineCounts { added, removed }
        });
        files.push(CfgFileChange {
            path: path.clone(),
            change,
            lines,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let (started_before, started_after) = (chain_started(from), chain_started(to));
    let cfg = CfgChange {
        changed: u32::try_from(files.len()).unwrap_or(u32::MAX),
        lines: compare::sum_lines(files.iter().filter_map(|file| file.lines)),
        started: started_after
            .iter()
            .filter(|name| !started_before.contains(name))
            .cloned()
            .collect(),
        stopped: started_before
            .iter()
            .filter(|name| !started_after.contains(name))
            .cloned()
            .collect(),
        files,
    };
    Ok((cfg, chain))
}

/// What an attribution tells: a change of the launch settings shows others than the owner the
/// settings the comparison shows them, never the cfg file, the data folder or the arguments' names.
fn shown_details(action: AuditAction, details: Value, owner: bool) -> Value {
    if owner || action != AuditAction::LaunchSettingsChanged {
        return details;
    }
    match details {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(name, _)| SHOWN_SETTINGS.contains(&name.as_str()))
                .collect(),
        ),
        _ => Value::Null,
    }
}

/// The launch settings compared: what every viewer sees, and for the owner the cfg file, the data
/// folder and which argument's value changed.
fn environment(from: &Snapshot, to: &Snapshot, owner: bool) -> EnvironmentChange {
    let number = |a: Option<u32>, b: Option<u32>| (a != b).then_some(NumberChange { from: a, to: b });
    let mut settings = Vec::new();
    let shown = SHOWN_SETTINGS
        .iter()
        .chain(if owner { OWNER_SETTINGS.iter() } else { [].iter() });
    for setting in shown {
        let (a, b) = (
            from.launch.get(setting).cloned().unwrap_or(Value::Null),
            to.launch.get(setting).cloned().unwrap_or(Value::Null),
        );
        if a != b {
            settings.push(SettingChange {
                setting: (*setting).to_owned(),
                from: a,
                to: b,
            });
        }
    }
    let arguments_of = |snapshot: &Snapshot| -> Vec<(String, Value)> {
        snapshot
            .launch
            .get("arguments")
            .and_then(Value::as_array)
            .map(|arguments| {
                arguments
                    .iter()
                    .filter_map(|argument| {
                        Some((
                            argument.get("name")?.as_str()?.to_owned(),
                            argument.get("tag")?.clone(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (before, after) = (arguments_of(from), arguments_of(to));
    let mut arguments = Vec::new();
    for (name, tag) in &after {
        match before.iter().find(|(other, _)| other == name) {
            None => arguments.push(ArgumentChange {
                name: name.clone(),
                change: ArgumentChangeKind::Set,
            }),
            Some((_, was)) if was != tag && owner => arguments.push(ArgumentChange {
                name: name.clone(),
                change: ArgumentChangeKind::Changed,
            }),
            Some(_) => {}
        }
    }
    for (name, _) in &before {
        if !after.iter().any(|(other, _)| other == name) {
            arguments.push(ArgumentChange {
                name: name.clone(),
                change: ArgumentChangeKind::Removed,
            });
        }
    }
    EnvironmentChange {
        build: number(from.build, to.build).filter(|_| builds(from, to)),
        game_build: number(from.game_build, to.game_build).filter(|_| builds(from, to)),
        game: (from.game != to.game).then_some(GameChange {
            from: from.game,
            to: to.game,
        }),
        settings,
        arguments,
        cfg: CfgChange::default(),
        attributions: Vec::new(),
    }
}

/// Who wrote each path over SFTP between two instants, the last write winning: a hint, since a
/// copy keeps the time it likes.
fn last_writers(entries: &[AuditEntry]) -> HashMap<String, (String, Timestamp)> {
    let mut writers = HashMap::new();
    for entry in entries {
        let (Some(target), Some(by)) = (&entry.target, &entry.actor_name) else {
            continue;
        };
        writers.insert(target.clone(), (by.clone(), entry.at));
    }
    writers
}

impl Changes {
    /// The starts, newest first, with what changed against the one before each, and the latest
    /// check of the disk.
    pub async fn starts(&self, before: Option<i64>) -> Result<StartList, Error> {
        let crashed = self.crashed_runs().await?;
        let hidden_globs = self.settings().hidden;
        self.inner
            .shared
            .store
            .read(move |conn| -> Result<StartList, Error> {
                let mut starts = snapshots::starts(conn, SERVER, before, STARTS_PAGE + 1)?;
                let next = (starts.len() > STARTS_PAGE as usize).then(|| starts[STARTS_PAGE as usize - 1].id);
                starts.truncate(STARTS_PAGE as usize);
                let mut sides: HashMap<i64, Side> = HashMap::new();
                let mut summaries = Vec::with_capacity(starts.len());
                for snapshot in &starts {
                    let earlier = snapshots::reference(conn, SERVER, snapshot.id)?;
                    let counts = match earlier {
                        Some(earlier) if !snapshot.baseline && snapshot.listed_at.is_some() => {
                            for side_of in [&earlier, snapshot] {
                                if let Entry::Vacant(vacant) = sides.entry(side_of.id) {
                                    vacant.insert(side(conn, side_of.clone())?);
                                }
                            }
                            let (a, b) = (&sides[&earlier.id], &sides[&snapshot.id]);
                            Some(compared(conn, a, b, &hidden_globs, false)?.counts)
                        }
                        _ => None,
                    };
                    let ran = snapshot.run_started.zip(snapshot.generation);
                    summaries.push(compare::summary(
                        snapshot,
                        counts,
                        ran.is_some_and(|run| crashed.contains(&run)),
                    ));
                }
                let check =
                    snapshots::latest_check(conn, SERVER)?.map(|check| compare::summary(&check, None, false));
                Ok(StartList {
                    starts: summaries,
                    next,
                    check,
                })
            })
            .await
    }

    /// The latest start with what changed against the one before, and the latest check.
    pub(crate) async fn latest(&self) -> Result<ChangesLive, Error> {
        let crashed = self.crashed_runs().await?;
        let hidden_globs = self.settings().hidden;
        self.inner
            .shared
            .store
            .read(move |conn| -> Result<ChangesLive, Error> {
                let latest = snapshots::starts(conn, SERVER, None, 1)?.pop();
                let earlier = match &latest {
                    Some(latest) => snapshots::reference(conn, SERVER, latest.id)?,
                    None => None,
                };
                let latest = latest
                    .map(|latest| -> Result<_, StoreError> {
                        let counts = match &earlier {
                            Some(earlier) if !latest.baseline && latest.listed_at.is_some() => {
                                let (a, b) = (side(conn, earlier.clone())?, side(conn, latest.clone())?);
                                Some(compared(conn, &a, &b, &hidden_globs, false)?.counts)
                            }
                            _ => None,
                        };
                        let ran = latest.run_started.zip(latest.generation);
                        Ok(compare::summary(
                            &latest,
                            counts,
                            ran.is_some_and(|run| crashed.contains(&run)),
                        ))
                    })
                    .transpose()?;
                let check =
                    snapshots::latest_check(conn, SERVER)?.map(|check| compare::summary(&check, None, false));
                Ok(ChangesLive { latest, check })
            })
            .await
    }

    /// `observer:startChanges` for the start `id`: what changed since the start before it, or a
    /// baseline. Resources still waiting for their files are counted and left out.
    pub(crate) async fn announcement(&self, id: i64) -> Result<BridgeEvent, Error> {
        let hidden_globs = self.settings().hidden;
        self.inner
            .shared
            .store
            .read(move |conn| -> Result<BridgeEvent, Error> {
                let to = snapshots::get(conn, id)?.ok_or(ErrorCode::StartNotFound)?;
                let millis = |at: Timestamp| at.as_millisecond();
                let earlier = snapshots::reference(conn, SERVER, id)?;
                let Some(from) = earlier.filter(|_| !to.baseline) else {
                    let head = StartHead {
                        at: millis(to.taken_at),
                        previous_at: None,
                        baseline: true,
                        build: None,
                        game_build: None,
                        counts: StartCounts::default(),
                    };
                    return Ok(bridge::start_changes(head, []));
                };
                let (a, b) = (side(conn, from.clone())?, side(conn, to.clone())?);
                let (resources, _) = changed_resources(conn, &a, &b, &hidden_globs, false)?;
                let moved = |before: Option<u32>, after: Option<u32>| {
                    (before != after).then_some(NumberMoved {
                        from: before,
                        to: after,
                    })
                };
                let mut counts = StartCounts::default();
                let mut listed = Vec::new();
                for resource in resources {
                    let change = match resource.change {
                        ResourceChangeKind::Added => StartChange::Added,
                        ResourceChangeKind::Removed => StartChange::Removed,
                        ResourceChangeKind::Updated => StartChange::Updated,
                        ResourceChangeKind::SameVersion => StartChange::SameVersion,
                        ResourceChangeKind::Renamed => StartChange::Renamed,
                        ResourceChangeKind::Moved => StartChange::Moved,
                        ResourceChangeKind::DidNotStart => StartChange::DidNotStart,
                        ResourceChangeKind::Checking | ResourceChangeKind::Unknown => {
                            counts.pending += 1;
                            continue;
                        }
                        ResourceChangeKind::Unchanged => continue,
                    };
                    let counter = match change {
                        StartChange::Added => &mut counts.added,
                        StartChange::Removed => &mut counts.removed,
                        StartChange::Updated => &mut counts.updated,
                        StartChange::SameVersion => &mut counts.same_version,
                        StartChange::Renamed => &mut counts.renamed,
                        StartChange::Moved => &mut counts.moved,
                        StartChange::DidNotStart => &mut counts.did_not_start,
                    };
                    *counter += 1;
                    listed.push((resource.name, change, resource.from_version, resource.to_version));
                }
                let order = |change: StartChange| match change {
                    StartChange::Added => 0,
                    StartChange::Updated => 1,
                    StartChange::SameVersion => 2,
                    _ => 3,
                };
                listed.sort_by_key(|(_, change, _, _)| order(*change));
                let head = StartHead {
                    at: millis(to.taken_at),
                    previous_at: Some(millis(from.taken_at)),
                    baseline: false,
                    build: moved(from.build, to.build),
                    game_build: moved(from.game_build, to.game_build),
                    counts,
                };
                Ok(bridge::start_changes(head, listed))
            })
            .await
    }

    async fn crashed_runs(&self) -> Result<HashSet<(Timestamp, u32)>, Error> {
        let since = self.inner.shared.clock.now() - SignedDuration::from_hours(24 * 400);
        Ok(self
            .inner
            .shared
            .db
            .read(move |conn| diagnostics::crashed_runs(conn, since))
            .await?
            .into_iter()
            .collect())
    }

    /// Two snapshots compared: the later a start or the latest check of the disk, the earlier the
    /// start before it unless named. What the cfg exec chain holds is the owner's.
    pub async fn compare(
        &self,
        caller: &Caller,
        from: Option<i64>,
        to: Option<ComparedTo>,
    ) -> Result<Comparison, Error> {
        let hidden_globs = self.settings().hidden;
        let crashed = self.crashed_runs().await?;
        let (from, to, compared) = self
            .inner
            .shared
            .store
            .read(move |conn| -> Result<_, Error> {
                let (from, to) = snapshots_of(conn, from, to)?;
                let (a, b) = (side(conn, from.clone())?, side(conn, to.clone())?);
                let compared = compared(conn, &a, &b, &hidden_globs, true)?;
                Ok((from, to, compared))
            })
            .await?;
        let (start, end) = (from.taken_at.min(to.taken_at), from.taken_at.max(to.taken_at));
        let (environment_entries, file_entries) = self
            .inner
            .shared
            .db
            .read(move |conn| -> Result<_, StoreError> {
                Ok((
                    audit::of_actions(conn, &ENVIRONMENT, start, end, MAX_ATTRIBUTIONS)?,
                    audit::of_actions(conn, &WRITES, start, end, 10_000)?,
                ))
            })
            .await?;
        let mut environment = environment(&from, &to, caller.is_owner);
        environment.attributions = environment_entries
            .into_iter()
            .map(|entry| Attribution {
                details: shown_details(entry.action, entry.details, caller.is_owner),
                action: entry.action,
                by: entry.actor_name,
                at: entry.at,
            })
            .collect();
        let mut cfg = compared.cfg;
        if !caller.is_owner {
            cfg.files.clear();
        }
        environment.cfg = cfg;
        let written = last_writers(&file_entries);
        let mut resources = compared.resources;
        for resource in &mut resources {
            let prefix = format!("{}/", resource.path);
            resource.by = compare::distinct(
                written
                    .iter()
                    .filter(|(path, _)| path.starts_with(&prefix))
                    .map(|(_, (by, _))| by.clone()),
            );
            resource.by.sort();
        }
        let crashed_run = |snapshot: &Snapshot| {
            snapshot
                .run_started
                .zip(snapshot.generation)
                .is_some_and(|run| crashed.contains(&run))
        };
        Ok(Comparison {
            from: compare::summary(&from, None, crashed_run(&from)),
            to: compare::summary(&to, Some(compared.counts.clone()), crashed_run(&to)),
            counts: compared.counts,
            environment,
            resources,
            live: compared.live,
            changed_while_starting: compared.changed_while_starting,
        })
    }

    /// One resource between two snapshots, found by its path on either side, with every file.
    pub async fn resource(
        &self,
        caller: &Caller,
        from: Option<i64>,
        to: Option<ComparedTo>,
        path: String,
    ) -> Result<ResourceDetail, Error> {
        let hidden_globs = self.settings().hidden;
        let owner = caller.is_owner;
        let (mut detail, start, end) = self
            .inner
            .shared
            .store
            .read(move |conn| resource_detail(conn, from, to, &path, &hidden_globs, owner))
            .await?;
        let file_entries = self
            .inner
            .shared
            .db
            .read(move |conn| audit::of_actions(conn, &WRITES, start, end, 10_000))
            .await?;
        let written = last_writers(&file_entries);
        for file in &mut detail.files {
            if let Some((by, at)) = written.get(&file.path) {
                file.written_by = Some(by.clone());
                file.written_at = Some(*at);
            }
        }
        detail.change.by = compare::distinct(detail.files.iter().filter_map(|file| file.written_by.clone()));
        Ok(detail)
    }

    /// The versions of a resource over the kept starts, newest first.
    pub async fn history(&self, name: String) -> Result<Vec<ResourceVersion>, Error> {
        let name = name.to_ascii_lowercase();
        self.inner
            .shared
            .store
            .read(move |conn| -> Result<Vec<ResourceVersion>, Error> {
                let starts = snapshots::starts(conn, SERVER, None, MAX_HISTORY)?;
                let mut versions: Vec<ResourceVersion> = Vec::new();
                let mut last_tree: Option<Option<String>> = None;
                for snapshot in starts {
                    let rows = resources::rows(conn, snapshot.holder())?;
                    let Some(row) = rows.into_iter().find(|row| row.name == name) else {
                        last_tree = None;
                        continue;
                    };
                    let same = last_tree.as_ref() == Some(&row.tree) && row.tree.is_some();
                    match versions.last_mut() {
                        Some(version) if same => version.since = compare::summary(&snapshot, None, false),
                        _ => versions.push(ResourceVersion {
                            since: compare::summary(&snapshot, None, false),
                            until: snapshot.id,
                            version: row.version().map(str::to_owned),
                            path: row.path.clone(),
                        }),
                    }
                    last_tree = Some(row.tree);
                }
                Ok(versions)
            })
            .await
    }

    /// A file's diff between two snapshots, masked: `path` in `to`, and `from_path` in `from` when
    /// the file or its resource moved. The owner alone reads the cfg exec chain; hidden files
    /// never; each read is audited once an hour per account, file and pair.
    pub async fn diff(
        &self,
        caller: &Caller,
        from: Option<i64>,
        to: Option<ComparedTo>,
        (path, from_path): (String, Option<String>),
        options: DiffOptions,
    ) -> Result<FileDiff, Error> {
        let hidden_globs = self.settings().hidden;
        for path in std::iter::once(&path).chain(&from_path) {
            if hidden(path, &hidden_globs) {
                return Err(ApiError::new(ErrorCode::ContentHidden)
                    .with("glob", compare::hidden_by(path, &hidden_globs))
                    .into());
            }
        }
        let owner = caller.is_owner;
        let lookup = path.clone();
        let asked_from = from_path.clone();
        let earlier = from_path.unwrap_or_else(|| path.clone());
        let (from_id, to_id, resource, chain, file, diff) = self
            .inner
            .shared
            .store
            .read(move |conn| -> Result<_, Error> {
                let (from, to) = snapshots_of(conn, from, to)?;
                let (before, after) = (located(conn, &from, &earlier)?, located(conn, &to, &lookup)?);
                if before.is_none() && after.is_none() {
                    return Err(ErrorCode::NotFound.into());
                }
                // A file the chain runs on either side, or ran in any start kept, is the owner's.
                let ran = snapshots::chain_paths(conn, SERVER)?;
                let in_chain = [&before, &after]
                    .into_iter()
                    .flatten()
                    .any(|located| located.chain)
                    || ran.contains(&lookup)
                    || ran.contains(&earlier);
                if in_chain && !owner {
                    return Err(ErrorCode::OwnerOnly.into());
                }
                let resource = before
                    .as_ref()
                    .or(after.as_ref())
                    .and_then(|located| located.resource.clone());
                let old = before.as_ref().map(|located| &located.entry);
                let new = after.as_ref().map(|located| &located.entry);
                let mut file = compare::files(
                    old.map(|entry| ("", std::slice::from_ref(entry))),
                    new.map(|entry| ("", std::slice::from_ref(entry))),
                    &hidden_globs,
                )
                .pop()
                .ok_or(ErrorCode::NotFound)?;
                let text = |entry: Option<&TreeEntry>| -> Result<Option<Masked>, StoreError> {
                    match entry
                        .filter(|entry| entry.kind == FileKind::Text)
                        .and_then(TreeEntry::digest)
                    {
                        Some(digest) => masked(conn, digest),
                        None => Ok(None),
                    }
                };
                let (was, now) = (text(old)?, text(new)?);
                let texts = old.is_none_or(|entry| entry.kind == FileKind::Text)
                    && new.is_none_or(|entry| entry.kind == FileKind::Text);
                file.kept = texts && (old.is_none() || was.is_some()) && (new.is_none() || now.is_some());
                if texts && !file.kept {
                    return Err(ErrorCode::ContentNotKept.into());
                }
                let diff = file.kept.then(|| diff::diff(was.as_ref(), now.as_ref(), options));
                if let Some(diff) = &diff {
                    file.lines = Some(LineCounts {
                        added: diff.added,
                        removed: diff.removed,
                    });
                }
                if to.kind == SnapshotKind::Start {
                    file.flags = snapshots::flags(conn, to.id)?
                        .into_iter()
                        .filter(|(flagged, _)| *flagged == file.path)
                        .map(|(_, flag)| flag)
                        .collect();
                }
                Ok((from.id, to.id, resource, in_chain, file, diff))
            })
            .await?;
        if diff.is_some() {
            self.audit_read(
                caller,
                &path,
                chain,
                json!({
                    "from": from_id,
                    "to": to_id,
                    "fromPath": asked_from.filter(|_| !chain),
                    "resource": resource,
                    "view": "diff",
                }),
            )
            .await?;
        }
        Ok(FileDiff { file, diff })
    }

    /// A whole file of one snapshot, masked, a page of it.
    pub async fn file(
        &self,
        caller: &Caller,
        snapshot: i64,
        path: String,
        page: usize,
    ) -> Result<FileText, Error> {
        let hidden_globs = self.settings().hidden;
        if hidden(&path, &hidden_globs) {
            return Err(ApiError::new(ErrorCode::ContentHidden)
                .with("glob", compare::hidden_by(&path, &hidden_globs))
                .into());
        }
        let owner = caller.is_owner;
        let lookup = path.clone();
        let (resource, chain, text) = self
            .inner
            .shared
            .store
            .read(move |conn| -> Result<_, Error> {
                let found = snapshots::get(conn, snapshot)?
                    .filter(|found| found.server == SERVER)
                    .ok_or(ErrorCode::StartNotFound)?;
                let located = located(conn, &found, &lookup)?.ok_or(ErrorCode::NotFound)?;
                // A file the chain ran in any start kept is the owner's in every version of it.
                let chain = located.chain || snapshots::chain_paths(conn, SERVER)?.contains(&lookup);
                if chain && !owner {
                    return Err(ErrorCode::OwnerOnly.into());
                }
                let digest = located
                    .entry
                    .digest()
                    .filter(|_| located.entry.kind == FileKind::Text)
                    .ok_or(ErrorCode::ContentNotKept)?
                    .to_owned();
                let text = masked(conn, &digest)?.ok_or(ErrorCode::ContentNotKept)?;
                Ok((located.resource, chain, diff::page(&text, page)))
            })
            .await?;
        self.audit_read(
            caller,
            &path,
            chain,
            json!({ "from": snapshot, "to": snapshot, "resource": resource, "view": "file" }),
        )
        .await?;
        Ok(FileText { path, snapshot, text })
    }

    /// Audits a read of contents, unless the same account read the same file of the same pair of
    /// snapshots within the hour: the audit log is append-only, so repeats are left out. A file of
    /// the cfg exec chain, outside any resource, is named by a keyed tag: its path may say where
    /// the host keeps its secrets, and the audit log is read by more than the owner.
    async fn audit_read(
        &self,
        caller: &Caller,
        path: &str,
        chain: bool,
        details: Value,
    ) -> Result<(), Error> {
        let now = self.inner.shared.clock.now();
        let since = now - AUDIT_WINDOW;
        let target = if chain {
            format!("cfg:{:04x}", mask::tag(path, &self.inner.shared.keys.tag))
        } else {
            path.to_owned()
        };
        let record = Record::by(caller, AuditAction::ChangeContentsViewed)
            .target(&target)
            .details(details.clone());
        let (user, path) = (caller.username.clone(), target);
        self.inner
            .shared
            .db
            .write(move |tx| -> Result<(), Error> {
                let recent = audit::of_actions(tx, &[AuditAction::ChangeContentsViewed], since, now, 10_000)?;
                let repeated = recent.iter().any(|entry| {
                    entry.actor_name.as_deref() == Some(user.as_str())
                        && entry.target.as_deref() == Some(path.as_str())
                        && entry.details.get("from") == details.get("from")
                        && entry.details.get("to") == details.get("to")
                        && entry.details.get("fromPath") == details.get("fromPath")
                });
                if !repeated {
                    record.write(tx, now)?;
                }
                Ok(())
            })
            .await
    }

    /// The owner's settings with how many files of the latest start each glob covers.
    pub async fn settings_view(&self) -> Result<ChangesSettingsView, Error> {
        let settings = self.settings();
        let (hidden_globs, excluded_globs) = (settings.hidden.clone(), settings.excluded.clone());
        let (hidden_matches, excluded_matches, stored) = self
            .inner
            .shared
            .store
            .read(move |conn| -> Result<_, Error> {
                let latest = snapshots::starts(conn, SERVER, None, 1)?.pop();
                let mut paths = Vec::new();
                if let Some(latest) = latest {
                    for row in resources::rows(conn, latest.holder())? {
                        paths.push(row.path.clone());
                        if let Some(entries) = entries(conn, &row)? {
                            paths.extend(entries.into_iter().map(|entry| entry.path));
                        }
                    }
                }
                let matches = |globs: &[String]| -> Vec<u32> {
                    globs
                        .iter()
                        .map(|glob| {
                            u32::try_from(paths.iter().filter(|path| glob_covers(glob, path)).count())
                                .unwrap_or(u32::MAX)
                        })
                        .collect()
                };
                Ok((
                    matches(&hidden_globs),
                    matches(&excluded_globs),
                    upkeep::stored(conn)?,
                ))
            })
            .await?;
        Ok(ChangesSettingsView {
            settings,
            hidden_matches,
            excluded_matches,
            stored,
        })
    }
}

/// One resource between two snapshots, found by its path on either side, with the instants between
/// which its files' writers are looked for.
fn resource_detail(
    conn: &Connection,
    from: Option<i64>,
    to: Option<ComparedTo>,
    path: &str,
    hidden_globs: &[String],
    owner: bool,
) -> Result<(ResourceDetail, Timestamp, Timestamp), Error> {
    let (from, to) = snapshots_of(conn, from, to)?;
    let (a, b) = (side(conn, from.clone())?, side(conn, to.clone())?);
    let mut pairs = compare::pair(&a.rows, &b.rows);
    let mut read = |before: bool, index: usize| {
        let row = if before { &a.rows[index] } else { &b.rows[index] };
        entries(conn, row).ok().flatten()
    };
    compare::renames(&mut pairs, &a.rows, &b.rows, &mut read);
    let found = pairs
        .paired
        .iter()
        .map(|(x, y, how)| (Some(*x), Some(*y), Some(*how)))
        .chain(pairs.removed.iter().map(|x| (Some(*x), None, None)))
        .chain(pairs.added.iter().map(|y| (None, Some(*y), None)))
        .find(|(x, y, _)| {
            x.is_some_and(|x| a.rows[x].path == path) || y.is_some_and(|y| b.rows[y].path == path)
        })
        .ok_or(ErrorCode::NotFound)?;
    let before = found.0.map(|x| &a.rows[x]);
    let after = found.1.map(|y| &b.rows[y]);
    let old = before.map(|row| entries(conn, row)).transpose()?.flatten();
    let new = after.map(|row| entries(conn, row)).transpose()?.flatten();
    let (kind, changed) = match (before, after, found.2) {
        (Some(x), Some(y), Some(how)) => {
            let did_not_start = compare::started(&from, &x.name) == Some(true)
                && compare::started(&to, &y.name) == Some(false);
            compare::classify(
                x,
                y,
                how,
                old.as_deref().zip(new.as_deref()),
                did_not_start,
                compare::settled(&to),
            )
        }
        (None, _, _) => (ResourceChangeKind::Added, true),
        _ => (ResourceChangeKind::Removed, true),
    };
    let mut files = compare::files(
        before
            .zip(old.as_deref())
            .map(|(row, entries)| (row.path.as_str(), entries)),
        after
            .zip(new.as_deref())
            .map(|(row, entries)| (row.path.as_str(), entries)),
        hidden_globs,
    );
    if !owner {
        let (_, chain) = cfg_change(conn, &from, &to)?;
        files.retain(|file| !chain.contains(&file.path));
    }
    enrich(
        conn,
        &mut files,
        &to,
        old.as_deref().unwrap_or_default(),
        new.as_deref().unwrap_or_default(),
    )?;
    let mut change = compare::resource_change(before, after, kind, changed);
    change.files = compare::file_counts(&files);
    let shown = after.or(before).map(|row| row.name.clone()).unwrap_or_default();
    let duplicates = b
        .rows
        .iter()
        .filter(|row| row.name == shown && Some(&row.path) != after.map(|row| &row.path))
        .map(|row| row.path.clone())
        .collect();
    let detail = ResourceDetail {
        change,
        from_manifest: before.and_then(|row| row.manifest.clone()),
        to_manifest: after.and_then(|row| row.manifest.clone()),
        from_git: before.and_then(|row| row.git_head.clone()),
        to_git: after.and_then(|row| row.git_head.clone()),
        packages: packages(before, after),
        files,
        duplicates,
    };
    Ok((
        detail,
        from.taken_at.min(to.taken_at),
        from.taken_at.max(to.taken_at),
    ))
}

/// Each file's flags in the later snapshot, whether both its texts are kept, and the lines of the
/// modified texts within the budget.
fn enrich(
    conn: &Connection,
    files: &mut [FileChange],
    to: &Snapshot,
    old: &[TreeEntry],
    new: &[TreeEntry],
) -> Result<(), StoreError> {
    let flags: HashMap<String, Vec<FileFlag>> = if to.kind == SnapshotKind::Start {
        snapshots::flags(conn, to.id)?
            .into_iter()
            .fold(HashMap::new(), |mut flags, (path, flag)| {
                flags.entry(path).or_insert_with(Vec::new).push(flag);
                flags
            })
    } else {
        HashMap::new()
    };
    let digest_of = |entries: &[TreeEntry], path: &str| {
        entries
            .iter()
            .find(|entry| entry.path == path)
            .and_then(|entry| entry.digest().map(str::to_owned))
    };
    let mut budget = (LINE_FILES, LINE_BYTES);
    for file in files {
        file.flags = compare::flags_of(&flags, &file.path);
        if file.kind != FileKind::Text {
            continue;
        }
        let was = digest_of(old, file.from_path.as_deref().unwrap_or(&file.path));
        let now = digest_of(new, &file.path);
        let was_kept = was
            .as_deref()
            .map_or(Ok(file.change == FileChangeKind::Added), |digest| {
                kept(conn, digest)
            })?;
        let now_kept = now
            .as_deref()
            .map_or(Ok(file.change == FileChangeKind::Removed), |digest| {
                kept(conn, digest)
            })?;
        file.kept = was_kept && now_kept;
        if file.change == FileChangeKind::Modified {
            file.lines = lines(conn, std::slice::from_ref(file), old, new, &mut budget)?;
        }
    }
    Ok(())
}

/// A file found in a snapshot: in a resource's tree, or among the cfg exec chain's files.
struct Located {
    entry: TreeEntry,
    resource: Option<String>,
    chain: bool,
}

fn located(conn: &Connection, snapshot: &Snapshot, path: &str) -> Result<Option<Located>, StoreError> {
    let chain = chain_files(snapshot);
    let in_chain = chain.iter().any(|file| file.path == path);
    for row in resources::rows(conn, snapshot.holder())? {
        if !path.starts_with(&format!("{}/", row.path)) {
            continue;
        }
        if let Some(entry) =
            entries(conn, &row)?.and_then(|entries| entries.into_iter().find(|entry| entry.path == path))
        {
            return Ok(Some(Located {
                entry,
                resource: Some(row.shown_name),
                chain: in_chain,
            }));
        }
    }
    if let Some(file) = chain.into_iter().find(|file| file.path == path) {
        let digest = texts::attached(conn, snapshot.id, path)?.or(file.digest);
        return Ok(digest.map(|digest| Located {
            entry: TreeEntry {
                path: path.to_owned(),
                size: 0,
                key: digest,
                kind: FileKind::Text,
                sha256: None,
            },
            resource: None,
            chain: true,
        }));
    }
    Ok(None)
}

fn packages(before: Option<&ResourceRow>, after: Option<&ResourceRow>) -> Vec<PackageChange> {
    let list = |row: Option<&ResourceRow>| row.map(|row| row.packages.clone()).unwrap_or_default();
    let (old, new) = (list(before), list(after));
    let names: Vec<String> = compare::distinct(old.iter().chain(&new).map(|package| package.name.clone()));
    let mut changes: Vec<PackageChange> = names
        .into_iter()
        .filter_map(|name| {
            let from = old
                .iter()
                .find(|package| package.name == name)
                .and_then(|package| package.version.clone());
            let to = new
                .iter()
                .find(|package| package.name == name)
                .and_then(|package| package.version.clone());
            let present = (
                old.iter().any(|package| package.name == name),
                new.iter().any(|package| package.name == name),
            );
            (from != to || present.0 != present.1).then_some(PackageChange { name, from, to })
        })
        .collect();
    changes.sort_by(|a, b| a.name.cmp(&b.name));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn others_than_the_owner_see_only_the_shown_launch_settings_in_an_attribution() {
        let details = json!({
            "onesync": { "before": "on", "after": "legacy" },
            "cfgFile": true,
            "extraArguments": { "set": ["sv_licenseKey"], "removed": [] },
        });

        let shown = shown_details(AuditAction::LaunchSettingsChanged, details.clone(), false);
        let owner = shown_details(AuditAction::LaunchSettingsChanged, details.clone(), true);
        let build = shown_details(AuditAction::BuildSelected, details.clone(), false);

        assert_eq!(shown, json!({ "onesync": { "before": "on", "after": "legacy" } }));
        assert_eq!(owner, details);
        assert_eq!(build, details);
    }
}
