//! FXServer's builds: which one runs, the build waiting for the next scheduled restart, and
//! what the schedule's restart does with it.

use std::path::Path;

use jiff::Timestamp;
use observer_artifacts::{Health, Platform};
use observer_domain::artifacts::{ArtifactNotice, NoticeKind, NoticeReason};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_store::Transaction;
use observer_store::artifacts::{self as stored, ArtifactState};
use serde_json::json;

use crate::audit::Record;
use crate::{Error, Servers};

/// What a scheduled restart did with the pending build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Promotion {
    Applied {
        build: u32,
        from: Option<u32>,
    },
    /// Deleted or broken since it was chosen: the restart goes ahead on the selected build.
    Dropped {
        build: u32,
    },
}

/// Makes the pending build the selected one, when it is still installed and valid, or drops it
/// with a notice. It runs in the transaction of the schedule's restart, before the launch is
/// handed over, so that the restart starts the build it applied.
pub(crate) fn promote_pending(
    tx: &Transaction<'_>,
    root: &Path,
    now: Timestamp,
) -> Result<Option<Promotion>, Error> {
    let state = stored::state(tx, Servers::DEFAULT)?;
    let Some(pending) = state.pending.clone() else {
        return Ok(None);
    };
    let folder = root.join(pending.build.to_string());
    let (_, health) = observer_artifacts::inspect(&folder, pending.build, Platform::host());
    let system = |action| Record::operator(ActorKind::System, action).target(Servers::DEFAULT);
    let mut next = ArtifactState {
        pending: None,
        ..state.clone()
    };
    let promotion = if health == Health::Valid {
        next.selected = Some(pending.build);
        system(AuditAction::PendingBuildApplied)
            .details(json!({ "build": pending.build, "from": state.selected, "at": now }))
            .write(tx, now)?;
        Promotion::Applied {
            build: pending.build,
            from: state.selected,
        }
    } else {
        let reason = if std::fs::symlink_metadata(&folder).is_err() {
            NoticeReason::NotInstalled
        } else {
            NoticeReason::Broken
        };
        next.notice = Some(ArtifactNotice {
            kind: NoticeKind::PendingDropped,
            build: pending.build,
            other: None,
            reason,
            at: now,
        });
        system(AuditAction::PendingBuildDropped)
            .outcome(Outcome::Failure)
            .details(json!({ "build": pending.build, "reason": reason }))
            .write(tx, now)?;
        Promotion::Dropped { build: pending.build }
    };
    stored::save_state(tx, Servers::DEFAULT, &next, now)?;
    Ok(Some(promotion))
}
