//! The binary's half of an update: the swap of the executable once Observer has stopped, the start
//! of the new one, its probation until it has started, and the return to the previous one when it
//! does not. Under systemd, Observer exits with [`RESTART_CODE`] and the unit starts it again; on
//! Linux otherwise, it executes the new executable in its own place; on Windows, it starts the new
//! one in its console and watches it until it has started.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use jiff::Timestamp;
use observer_app::Takeover;
use observer_app::updates::handover::{self, Handover, HandoverState};
use observer_app::updates::swap;
use observer_domain::updates::{RollbackReason, UpdateOutcome, UpdateResult};
use observer_store::Database;
use tracing::{error, info, warn};

use crate::config::DataPaths;

/// What Observer exits with for systemd to start it again, the new executable then: `EX_TEMPFAIL`.
/// The printed unit counts it as a success that restarts; an older unit restarts on any failure.
pub const RESTART_CODE: u8 = 75;
/// How long the previous Observer watches the new one on Windows, at most.
#[cfg(windows)]
const WATCH_LIMIT: std::time::Duration = std::time::Duration::from_mins(15);

/// What a previous Observer left, as this one starts.
pub enum Arrival {
    /// No install was under way.
    Plain,
    /// This is the new version, on probation until [`Probation::confirm`].
    Probation(Probation),
    /// This is the previous version, back after the new one did not start. Its database is the
    /// one from before the swap when the new version had changed its schema.
    Returned(Returned),
    /// The new version already started once and never finished: the previous one is put back,
    /// and starts in this one's place.
    RolledBack(Relaunch),
}

pub struct Probation {
    data_dir: PathBuf,
    handover: Handover,
}

pub struct Returned {
    outcome: UpdateOutcome,
    /// FXServer ran before the install: it starts again.
    pub fxserver: bool,
}

/// What starts once this Observer has exited.
#[derive(Debug)]
pub struct Relaunch {
    executable: PathBuf,
    /// The new version, which this Observer watches until it has started.
    watch: Option<PathBuf>,
}

/// Settles what a previous Observer left in the data folder, before the database is opened: the
/// return of the previous version needs the database as it was before the swap.
pub async fn arrive(paths: &DataPaths, executable: &Path) -> anyhow::Result<Arrival> {
    let data_dir = &paths.root;
    let Some(mut handover) = handover::read(data_dir).context("cannot read updates/handover.json")? else {
        swap::sweep(executable);
        return Ok(Arrival::Plain);
    };
    match handover.state {
        HandoverState::Swapped if handover.attempts == 0 => {
            handover.attempts = 1;
            handover::write(data_dir, &handover)?;
            info!(from = %handover.from, to = %handover.to, "starting the new release of Observer");
            Ok(Arrival::Probation(Probation {
                data_dir: data_dir.clone(),
                handover,
            }))
        }
        HandoverState::Swapped => {
            error!(to = %handover.to, "the new release did not finish starting the last time; going back to {}", handover.from);
            let relaunch = roll_back(data_dir, handover, RollbackReason::Crashed)?;
            Ok(Arrival::RolledBack(relaunch))
        }
        HandoverState::RolledBack { reason, at } => {
            if let Some(backup) = &handover.database_backup {
                restore_if_migrated(paths, backup).await?;
            }
            let outcome = UpdateOutcome {
                from: handover.from.clone(),
                to: handover.to.clone(),
                at,
                result: UpdateResult::RolledBack { reason },
            };
            handover::clear(data_dir)?;
            swap::sweep(executable);
            warn!(to = %handover.to, ?reason, "Observer {} is back: the new release did not start", handover.from);
            Ok(Arrival::Returned(Returned {
                outcome,
                fxserver: handover.fxserver,
            }))
        }
    }
}

impl Probation {
    /// FXServer ran before the install: the new version starts it again.
    pub fn fxserver(&self) -> bool {
        self.handover.fxserver
    }

    /// The new version has started: it stays, and the previous executable is kept beside it.
    /// Says how the install ended, once it is settled.
    pub async fn confirm(self, db: &Database, clock: &observer_app::Clock) -> Option<UpdateOutcome> {
        let outcome = UpdateOutcome {
            from: self.handover.from.clone(),
            to: self.handover.to.clone(),
            at: clock.now(),
            result: UpdateResult::Installed,
        };
        if let Err(error) = handover::clear(&self.data_dir) {
            error!(%error, "cannot settle the update of Observer: it would be undone at the next start");
            return None;
        }
        info!(from = %outcome.from, to = %outcome.to, "the new release of Observer started");
        observer_app::updates::record_outcome(db, clock, &self.data_dir, &outcome).await;
        Some(outcome)
    }

    /// The new version failed to start: the previous one is put back, and starts in its place.
    pub fn roll_back(self) -> anyhow::Result<Relaunch> {
        roll_back(&self.data_dir, self.handover, RollbackReason::Failed)
    }
}

impl Returned {
    pub async fn record(&self, db: &Database, clock: &observer_app::Clock, data_dir: &Path) {
        observer_app::updates::record_outcome(db, clock, data_dir, &self.outcome).await;
    }
}

/// Swaps the verified executable in, once Observer has stopped, and leaves the handover for the
/// next start. The previous executable starts again when the swap fails.
pub fn hand_over(takeover: &Takeover, paths: &DataPaths, executable: &Path) -> Relaunch {
    let mut handover = Handover {
        from: takeover.from.clone(),
        to: takeover.to.clone(),
        executable: executable.to_path_buf(),
        previous: swap::previous_of(executable),
        database_backup: Some(takeover.database_backup.clone()),
        fxserver: takeover.fxserver,
        at: Timestamp::now(),
        attempts: 0,
        state: HandoverState::Swapped,
    };
    match swap::swap(&takeover.staged, &takeover.sha256, executable) {
        Ok(previous) => {
            handover.previous = previous;
            if let Err(error) = handover::write(&paths.root, &handover) {
                error!(%error, "cannot leave the handover to the new release; putting the previous one back");
                if let Err(error) = swap::restore(executable, &handover.previous) {
                    error!(%error, "cannot put the previous executable back");
                }
                return Relaunch {
                    executable: executable.to_path_buf(),
                    watch: None,
                };
            }
            info!(to = %takeover.to, "the new release of Observer is in place");
            Relaunch {
                executable: executable.to_path_buf(),
                watch: Some(paths.root.clone()),
            }
        }
        Err(error) => {
            error!(%error, "cannot put the new release of Observer in place; starting this one again");
            handover.state = HandoverState::RolledBack {
                reason: RollbackReason::Swap,
                at: Timestamp::now(),
            };
            handover.database_backup = None;
            if let Err(error) = handover::write(&paths.root, &handover) {
                warn!(%error, "cannot keep why the update did not happen");
            }
            Relaunch {
                executable: executable.to_path_buf(),
                watch: None,
            }
        }
    }
}

fn roll_back(data_dir: &Path, mut handover: Handover, reason: RollbackReason) -> anyhow::Result<Relaunch> {
    swap::restore(&handover.executable, &handover.previous)
        .with_context(|| format!("cannot put {} back", handover.previous.display()))?;
    handover.state = HandoverState::RolledBack {
        reason,
        at: Timestamp::now(),
    };
    handover::write(data_dir, &handover)?;
    Ok(Relaunch {
        executable: handover.executable,
        watch: None,
    })
}

/// Puts the database back as it was before the swap when the new version changed its schema,
/// which this version could not read.
async fn restore_if_migrated(paths: &DataPaths, backup: &Path) -> anyhow::Result<()> {
    let Ok(inspection) = Database::inspect(&paths.database).await else {
        return Ok(());
    };
    if inspection.schema <= inspection.latest {
        return Ok(());
    }
    let suffix = format!("update-{}", Timestamp::now().strftime("%Y%m%dT%H%M%SZ"));
    let aside = observer_store::restore_backup(&paths.database, backup, &suffix)
        .with_context(|| format!("cannot restore {}", backup.display()))?;
    warn!(
        backup = %backup.display(),
        set_aside = %aside.display(),
        "the new release had changed the database's schema: the database from before the update is back"
    );
    Ok(())
}

/// Starts what `relaunch` names, the way this host starts Observer again.
pub fn relaunch(relaunch: Relaunch) -> ExitCode {
    if std::env::var_os("NOTIFY_SOCKET").is_some() {
        return ExitCode::from(RESTART_CODE);
    }
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut command = std::process::Command::new(&relaunch.executable);
    command.args(&args);
    #[cfg(feature = "test-catalog")]
    command.env_remove(crate::run::TEST_VERSION);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = relaunch.watch;
        let error = command.exec();
        eprintln!("error: cannot start {}: {error}", relaunch.executable.display());
        ExitCode::FAILURE
    }
    #[cfg(windows)]
    {
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                eprintln!("error: cannot start {}: {error}", relaunch.executable.display());
                return ExitCode::FAILURE;
            }
        };
        if let Some(data_dir) = relaunch.watch {
            watch(child, &data_dir, &relaunch.executable, command);
        }
        ExitCode::SUCCESS
    }
}

/// Waits until the new version has started, then lets it be. Should it end before, the previous
/// version, this one, is put back and started in its place.
#[cfg(windows)]
fn watch(
    mut child: std::process::Child,
    data_dir: &Path,
    executable: &Path,
    mut command: std::process::Command,
) {
    let deadline = std::time::Instant::now() + WATCH_LIMIT;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Ok(exited) = child.try_wait() else {
            return;
        };
        let handover = handover::read(data_dir).ok().flatten();
        let swapped = handover
            .as_ref()
            .is_some_and(|handover| handover.state == HandoverState::Swapped);
        match exited {
            None if swapped => {}
            None => return,
            Some(_) if !swapped => return,
            Some(status) => {
                eprintln!(
                    "the new release of Observer ended as it started ({status}); going back to this one"
                );
                let Some(handover) = handover else {
                    return;
                };
                match roll_back(data_dir, handover, RollbackReason::Crashed) {
                    Ok(_) => {
                        if let Err(error) = command.spawn() {
                            eprintln!("error: cannot start {}: {error}", executable.display());
                        }
                    }
                    Err(error) => eprintln!("error: cannot go back to this release: {error:#}"),
                }
                return;
            }
        }
    }
}
