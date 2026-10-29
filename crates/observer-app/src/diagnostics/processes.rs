//! The processes FXServer's tree starts, as Observer records and audits them: shells and programs
//! FXServer never starts by itself go to the audit log, at most once per program every ten minutes
//! of a run with the starts in between counted.

use std::collections::{HashMap, HashSet};

use jiff::{SignedDuration, Timestamp};
use observer_domain::diagnostics::ProcessKind;
use observer_fxserver::console::mask_command_line;
use observer_platform::ProcessInfo;
use serde_json::{Value, json};

/// The most of a command line a row keeps.
pub(super) const MAX_COMMAND_LINE: usize = 4096;
/// The most of it an audit entry keeps, in characters.
const AUDITED_COMMAND_LINE: usize = 1024;
/// A program's starts are audited at most this often in a run.
const AUDIT_EVERY: SignedDuration = SignedDuration::from_mins(10);
/// Distinct programs and command lines recorded per run; past them starts are only counted.
pub(super) const MAX_PROGRAMS: usize = 200;
/// Past the rows, the distinct programs and command lines a run's audit entries still name.
const MAX_AUDITED_PAST_ROWS: usize = 1000;

/// The main process as its children name their parent.
pub(super) const FXSERVER: &str = if cfg!(windows) { "FXServer.exe" } else { "FXServer" };

/// A command line as a row and the audit log keep it: secrets masked, control characters gone,
/// at most 4 KiB. The flag says whether it was cut.
pub(super) fn masked_command_line(line: &str) -> (String, bool) {
    let mut masked = mask_command_line(line);
    if masked.len() <= MAX_COMMAND_LINE {
        return (masked, false);
    }
    let mut end = MAX_COMMAND_LINE;
    while !masked.is_char_boundary(end) {
        end -= 1;
    }
    masked.truncate(end);
    (masked, true)
}

pub(super) fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// One run of FXServer, as its processes are recorded.
pub(super) struct Run {
    pub started: Timestamp,
    pub main_pid: Option<u32>,
    /// The processes running now: their row, unless the cap left them out, and their image.
    pub running: HashMap<u32, (Option<i64>, Option<String>)>,
    programs: HashSet<(Option<String>, Option<String>)>,
    /// Starts not recorded past the cap.
    pub overflow: u64,
    audits: HashMap<String, Audited>,
    /// The programs and command lines past the cap that got an audit entry of their own.
    audited_past_rows: HashSet<(Option<String>, Option<String>)>,
    past_rows_full: bool,
    /// Starts past the rows and their audit entries whose command line no entry kept.
    pub unrecorded: u64,
}

struct Audited {
    last_entry: Timestamp,
    /// Starts since the last entry.
    unaudited: u32,
    /// The latest start's details, for the entry that sums the run up.
    latest: Value,
}

/// What a start the cap left without a row gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PastRows {
    /// The first of its program and command line: an audit entry of its own.
    New,
    /// One already audited.
    Known,
    /// The first past the run's entries for them: from now on only counted.
    Full,
    /// Past them.
    Counted,
}

/// Whether a start goes to the audit log now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Audit {
    /// The program's first start in the run.
    First,
    /// The first in ten minutes, with the starts since the previous entry, this one included.
    Repeated(u32),
    /// Counted, for a later entry.
    Held,
}

impl Run {
    pub(super) fn new(started: Timestamp, main_pid: Option<u32>) -> Self {
        Self {
            started,
            main_pid,
            running: HashMap::new(),
            programs: HashSet::new(),
            overflow: 0,
            audits: HashMap::new(),
            audited_past_rows: HashSet::new(),
            past_rows_full: false,
            unrecorded: 0,
        }
    }

    /// Whether a start of this program and command line gets a row: the cap counts distinct ones.
    pub(super) fn admits(&mut self, image: Option<&str>, command_line: Option<&str>) -> bool {
        let program = (image.map(str::to_owned), command_line.map(str::to_owned));
        if self.programs.contains(&program) {
            return true;
        }
        if self.programs.len() >= MAX_PROGRAMS {
            self.overflow += 1;
            return false;
        }
        self.programs.insert(program);
        true
    }

    /// What a start the cap left without a row gets: an audit entry at once for the first of its
    /// program and command line, so that a run flooded with harmless starts still names the one
    /// that mattered, up to 1,000 a run. Past them a flood can hide one, and the run says so.
    fn past_rows(&mut self, image: Option<&str>, command_line: Option<&str>) -> PastRows {
        let program = (image.map(str::to_owned), command_line.map(str::to_owned));
        if self.audited_past_rows.contains(&program) {
            PastRows::Known
        } else if self.audited_past_rows.len() < MAX_AUDITED_PAST_ROWS {
            self.audited_past_rows.insert(program);
            PastRows::New
        } else if std::mem::replace(&mut self.past_rows_full, true) {
            PastRows::Counted
        } else {
            PastRows::Full
        }
    }

    /// The audit entry a start of a program FXServer never starts by itself gets now, if any, and
    /// whether it is the start that filled the run's entries for starts past the rows. Counts the
    /// starts whose command line no entry keeps.
    pub(super) fn unexpected_start(
        &mut self,
        admitted: bool,
        (image, command_line): (Option<&str>, Option<&str>),
        program: &str,
        details: Value,
        now: Timestamp,
    ) -> (Option<Value>, bool) {
        let past_rows = (!admitted).then(|| self.past_rows(image, command_line));
        let entry = if past_rows == Some(PastRows::New) {
            // No row holds it: the audit log does.
            Some(details)
        } else {
            self.audit_entry(program, details, now)
        };
        if matches!(past_rows, Some(PastRows::Full | PastRows::Counted)) && entry.is_none() {
            self.unrecorded += 1;
        }
        (entry, past_rows == Some(PastRows::Full))
    }

    /// The parent's file name: a process of the tree, or FXServer itself.
    pub(super) fn parent_name(&self, parent: Option<u32>) -> Option<String> {
        let parent = parent?;
        if Some(parent) == self.main_pid {
            return Some(FXSERVER.to_owned());
        }
        let (_, image) = self.running.get(&parent)?;
        image.as_deref().map(|image| file_name(image).to_owned())
    }

    /// The parent's image: a process of the tree, or FXServer itself by its name.
    pub(super) fn parent_image(&self, parent: Option<u32>) -> Option<String> {
        let parent = parent?;
        if Some(parent) == self.main_pid {
            return Some(FXSERVER.to_owned());
        }
        self.running.get(&parent).and_then(|(_, image)| image.clone())
    }

    /// The audit entry a start of an unexpected `program` gets now, if any: its details, with the
    /// starts since the previous entry when it is not the first.
    pub(super) fn audit_entry(&mut self, program: &str, details: Value, now: Timestamp) -> Option<Value> {
        match self.audit(program, details.clone(), now) {
            Audit::First => Some(details),
            Audit::Repeated(repeated) => {
                let mut details = details;
                details["repeated"] = json!(repeated);
                Some(details)
            }
            Audit::Held => None,
        }
    }

    /// Decides the audit of a start of `program` at `now`, and keeps its details for the run's end.
    pub(super) fn audit(&mut self, program: &str, details: Value, now: Timestamp) -> Audit {
        match self.audits.get_mut(program) {
            None => {
                self.audits.insert(
                    program.to_owned(),
                    Audited {
                        last_entry: now,
                        unaudited: 0,
                        latest: details,
                    },
                );
                Audit::First
            }
            Some(audited) => {
                audited.latest = details;
                if audited.last_entry.duration_until(now) >= AUDIT_EVERY {
                    let repeated = audited.unaudited + 1;
                    audited.last_entry = now;
                    audited.unaudited = 0;
                    Audit::Repeated(repeated)
                } else {
                    audited.unaudited += 1;
                    Audit::Held
                }
            }
        }
    }

    /// The entries the run still owes: each program's latest start with the starts not audited.
    pub(super) fn leftovers(&mut self) -> Vec<Value> {
        self.audits
            .values_mut()
            .filter(|audited| audited.unaudited > 0)
            .map(|audited| {
                let mut details = audited.latest.clone();
                details["repeated"] = json!(audited.unaudited);
                audited.unaudited = 0;
                details
            })
            .collect()
    }
}

/// What the audit log says of a start. No path beyond the image, the command line cut to 1,024
/// characters.
pub(super) fn audit_details(
    process: &ProcessInfo,
    kind: ProcessKind,
    command_line: Option<&str>,
    parent: Option<&str>,
    generation: u32,
) -> Value {
    let image = process
        .image
        .as_deref()
        .map(|image| image.to_string_lossy().into_owned());
    let command_line: Option<String> =
        command_line.map(|line| line.chars().take(AUDITED_COMMAND_LINE).collect());
    json!({
        "kind": kind,
        "name": image.as_deref().map(file_name),
        "image": image,
        "commandLine": command_line,
        "pid": process.pid,
        "parent": { "pid": process.parent, "name": parent },
        "generation": generation,
    })
}

/// The console's word of a run whose starts are now only counted.
pub(super) fn records_full_note() -> String {
    format!(
        "FXServer's tree started more than {} different programs and command lines in this run: the next ones are only counted",
        MAX_PROGRAMS + MAX_AUDITED_PAST_ROWS
    )
}

/// The console's word of a shell started in the tree.
pub(super) fn alert_note(name: &str, pid: u32, parent: Option<&str>, parent_pid: Option<u32>) -> String {
    let parent = match (parent, parent_pid) {
        (Some(name), Some(pid)) => format!(", parent {name} {pid}"),
        (None, Some(pid)) => format!(", parent {pid}"),
        _ => String::new(),
    };
    format!("security alert: FXServer's tree started {name} (pid {pid}{parent})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> Timestamp {
        "2026-10-29T10:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_mins(minutes)
    }

    #[test]
    fn a_program_is_audited_at_once_then_once_in_ten_minutes_with_the_starts_between() {
        let mut run = Run::new(at(0), Some(3312));

        let decisions: Vec<Audit> = [0, 1, 2, 9, 10, 11]
            .into_iter()
            .map(|minute| run.audit("cmd.exe", json!({ "pid": minute }), at(minute)))
            .collect();

        assert_eq!(
            decisions,
            [
                Audit::First,
                Audit::Held,
                Audit::Held,
                Audit::Held,
                Audit::Repeated(4),
                Audit::Held
            ]
        );
        let leftovers = run.leftovers();
        assert_eq!(leftovers, [json!({ "pid": 11, "repeated": 1 })]);
        assert!(run.leftovers().is_empty(), "owed once");
    }

    #[test]
    fn past_two_hundred_programs_a_run_only_counts_starts() {
        let mut run = Run::new(at(0), None);

        for index in 0..MAX_PROGRAMS {
            assert!(run.admits(Some("cmd.exe"), Some(&format!("cmd /c {index}"))));
        }

        assert!(
            run.admits(Some("cmd.exe"), Some("cmd /c 0")),
            "a repeat still counts on its row"
        );
        assert!(!run.admits(Some("cmd.exe"), Some("cmd /c new")));
        assert_eq!(run.overflow, 1);
    }

    #[test]
    fn past_the_rows_each_new_program_and_command_line_is_audited_up_to_a_thousand() {
        let mut run = Run::new(at(0), None);
        let cmd = |line: &str| (Some("cmd.exe"), Some(line.to_owned()));

        let (image, line) = cmd("cmd /c payload");
        assert_eq!(run.past_rows(image, line.as_deref()), PastRows::New);
        assert_eq!(run.past_rows(image, line.as_deref()), PastRows::Known, "once");
        for index in 1..MAX_AUDITED_PAST_ROWS {
            let (image, line) = cmd(&format!("cmd /c {index}"));
            assert_eq!(run.past_rows(image, line.as_deref()), PastRows::New);
        }
        let (image, line) = cmd("cmd /c one more");
        assert_eq!(run.past_rows(image, line.as_deref()), PastRows::Full);
        assert_eq!(
            run.past_rows(image, line.as_deref()),
            PastRows::Counted,
            "only once full"
        );
        let (image, line) = cmd("cmd /c payload");
        assert_eq!(
            run.past_rows(image, line.as_deref()),
            PastRows::Known,
            "still known"
        );
    }

    #[test]
    fn a_command_line_is_masked_and_cut_to_4_kib() {
        let (masked, truncated) = masked_command_line("mysql -ufivem -phunter2 qbcore");
        assert_eq!(masked, "mysql -ufivem -p[REDACTED] qbcore");
        assert!(!truncated);

        let (long, truncated) = masked_command_line(&format!("cmd /c {}", "é".repeat(3000)));
        assert!(truncated);
        assert!(long.len() <= MAX_COMMAND_LINE);
    }

    #[test]
    fn the_console_names_the_program_and_its_parent() {
        assert_eq!(
            alert_note("powershell.exe", 4120, Some("FXServer.exe"), Some(3312)),
            "security alert: FXServer's tree started powershell.exe (pid 4120, parent FXServer.exe 3312)"
        );
    }
}
