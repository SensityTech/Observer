//! What the work before a start says in the console: one summary line, and at most ten lines of
//! detail, the last of which points to the Tools page when there is more. A report, or a fix that
//! wrote nothing, says nothing when its counts and its plan are those of the last report; a heal
//! says nothing when it put nothing back.

use observer_domain::tools::{ToolMode, ToolRun, ToolRunOutcome, ToolRunSummary};
use observer_tools::fix::{CloseReason, Failure, FileResult, Outcome, Skip};

use super::work::Finished;

/// The most lines of detail besides the summary.
const DETAIL_LINES: usize = 10;

/// The lines for the console of a start's run in `mode`, given the last report before it.
pub(super) fn start(finished: &Finished, mode: ToolMode, previous: Option<&ToolRun>) -> Vec<String> {
    let mut parts = Vec::new();
    let mut details = Vec::new();
    let restored: usize = finished.healed.iter().map(|file| file.restored.len()).sum();
    let healed = finished.healed_paths();
    if !healed.is_empty() {
        parts.push(format!(
            "put back {} whose copy kept is gone, in {}",
            plural(restored, "archetype"),
            plural(healed.len(), "file")
        ));
        for file in finished.healed.iter().filter(|file| !file.restored.is_empty()) {
            let why = file
                .restored
                .iter()
                .find_map(|restored| restored.why.clone())
                .unwrap_or_default();
            details.push(format!(
                "put back {} in {}: {why}",
                plural(file.restored.len(), "archetype"),
                file.path
            ));
        }
    }
    let summary = &finished.summary;
    let same = previous.is_some_and(|previous| {
        previous.plan == finished.plan.as_ref().map(observer_domain::tools::sha256_hex)
            && previous.summary.as_ref().map(counts) == Some(counts(summary))
    });
    match mode {
        _ if finished.report.is_none() => {}
        ToolMode::Off => {}
        ToolMode::Report => {
            if !same {
                parts.push(format!(
                    "{}, {} by a fix in {}, {} left",
                    plural(as_usize(summary.groups), "duplicate archetype"),
                    as_usize(summary.fixable),
                    plural(planned_files(finished), "file"),
                    as_usize(summary.left)
                ));
            }
        }
        ToolMode::Fix => {
            let left = as_usize(summary.groups.saturating_sub(summary.fixed));
            if summary.files_changed > 0 {
                parts.push(format!(
                    "fixed {} by taking {} out of {}; {left} left",
                    plural(as_usize(summary.fixed), "duplicate archetype"),
                    plural(as_usize(summary.removals), "copy"),
                    plural(as_usize(summary.files_changed), "file"),
                ));
            } else if summary.fixable > 0 {
                parts.push(format!(
                    "fixed none of the {} a fix takes out; {left} left",
                    plural(as_usize(summary.fixable), "duplicate archetype")
                ));
            } else if !same {
                parts.push(format!(
                    "nothing to fix; {} left",
                    plural(left, "duplicate archetype")
                ));
            }
            details.extend(finished.applied.iter().filter_map(applied));
        }
    }
    match (finished.outcome(), &summary.problem) {
        (ToolRunOutcome::Failed, problem) => parts.push(format!(
            "failed: {}",
            problem.as_deref().unwrap_or("for a reason the log gives")
        )),
        (ToolRunOutcome::Cancelled | ToolRunOutcome::TimedOut, _) => {
            parts.push("stopped before the end".to_owned());
        }
        (ToolRunOutcome::Done, Some(problem)) => parts.push(problem.clone()),
        (ToolRunOutcome::Done, None) => {}
    }
    if parts.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!("duplicate archetypes: {}", parts.join("; "))];
    if details.len() > DETAIL_LINES {
        let others = details.len() - (DETAIL_LINES - 1);
        details.truncate(DETAIL_LINES - 1);
        details.push(format!("and {others} more on the Tools page"));
    }
    lines.extend(details);
    lines
}

/// What became of one file of the plan, when it is worth a line.
fn applied(file: &FileResult) -> Option<String> {
    match &file.outcome {
        Outcome::Skipped {
            why: Skip::NotTried | Skip::NothingToDo,
        }
        | Outcome::Closed { .. }
        | Outcome::Kept => None,
        outcome => Some(format!("{}: {}", file.path, outcome_text(outcome))),
    }
}

/// What became of a file, in words.
pub fn outcome_text(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Written { removed } if removed.is_empty() => "written back as it was".to_owned(),
        Outcome::Written { removed } => format!(
            "written without the entries at {}",
            removed
                .iter()
                .map(|index| format!("#{index}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Outcome::Closed { reason } => match reason {
            CloseReason::Undone => "undone: its original is back".to_owned(),
            CloseReason::Healed => "healed: its original is back".to_owned(),
            CloseReason::Stale => "changed by someone else since: no longer Observer's to change".to_owned(),
            CloseReason::Restored => "holds its original again".to_owned(),
            CloseReason::Abandoned => "its first write never landed".to_owned(),
        },
        Outcome::Kept => "an interrupted write that never landed: it holds what it held".to_owned(),
        Outcome::Skipped { why } => format!(
            "left as it is: {}",
            match why {
                Skip::Changed => "it changed since the scan",
                Skip::Missing => "it is gone",
                Skip::Refused => "Observer never writes there",
                Skip::Unresolved => "an earlier write to it is still pending",
                Skip::NothingToDo => "nothing to change",
                Skip::NotTried => "the run stopped before it",
            }
        ),
        Outcome::Failed { why } => match why {
            Failure::StorageFull { error } => format!("not written, a volume is full: {error}"),
            Failure::Io { error } => format!("not written: {error}"),
            Failure::Patch { error } => format!("not written, the new bytes did not check: {error}"),
            Failure::Backup { error } => format!("not written, its backup failed: {error}"),
        },
    }
}

/// How many files the report's plan changes.
fn planned_files(finished: &Finished) -> usize {
    finished
        .report
        .as_ref()
        .map_or(0, |report| as_usize(report.counts.files_changed))
}

/// A summary's counts, without its timing: what tells two reports apart.
fn counts(summary: &ToolRunSummary) -> (u32, u32, u32) {
    (summary.groups, summary.fixable, summary.left)
}

fn as_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn plural(count: usize, noun: &str) -> String {
    match (count, noun) {
        (1, _) => format!("1 {noun}"),
        (_, "copy") => format!("{count} copies"),
        _ => format!("{count} {noun}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nouns_take_their_plural_past_one() {
        assert_eq!(plural(1, "file"), "1 file");
        assert_eq!(plural(0, "file"), "0 files");
        assert_eq!(plural(3, "copy"), "3 copies");
        assert_eq!(plural(2, "duplicate archetype"), "2 duplicate archetypes");
    }
}
