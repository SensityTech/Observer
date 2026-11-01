//! Applies, heals and undoes the duplicate archetypes fixes on the server folder
//! `OBSERVER_SERVER_CORPUS` names, with its cfg (`OBSERVER_SERVER_CFG`, `server.cfg` by default), and
//! checks that every file of the folder ends byte for byte as it began. It writes into that folder,
//! so it runs only on a scratch copy: the folder must hold a file named `observer-scratch-copy`, and
//! neither it nor any folder above it may hold an FXServer install, where real servers live. It
//! prints counts and archetype hashes only, and does nothing without the variable.

// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use common::fixes::{MemoryJournal, sha256};
use observer_domain::tools::{GroupAction, ToolSettings};
use observer_tools::archetypes::{self, Analysis};
use observer_tools::fix::{self, Backups, CloseReason, Outcome, Selection, ServerFolder};
use observer_tools::resources::{ServerIndex, Starts};

const FOLDER: &str = "OBSERVER_SERVER_CORPUS";
const CFG: &str = "OBSERVER_SERVER_CFG";
/// The file that marks a folder as a scratch copy this test may write into.
const MARKER: &str = "observer-scratch-copy";
/// What an FXServer install holds at its root, on Windows and on Linux.
const INSTALL: [&str; 3] = ["FXServer.exe", "citizen", "run.sh"];

/// Why the test refuses to write into `folder`, if it does.
fn refusal(folder: &Path) -> Option<String> {
    let Ok(folder) = fs::canonicalize(folder) else {
        return Some(format!("{} cannot be read", folder.display()));
    };
    if !folder.join(MARKER).is_file() {
        return Some(format!(
            "{} holds no {MARKER} file: only a scratch copy is written into",
            folder.display()
        ));
    }
    folder
        .ancestors()
        .find(|dir| INSTALL.iter().any(|name| dir.join(name).exists()))
        .map(|install| {
            format!(
                "{} lies in the FXServer install {}: only a scratch copy is written into",
                folder.display(),
                install.display()
            )
        })
}

#[test]
fn the_guard_refuses_a_folder_that_is_not_a_scratch_copy() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("copy");
    fs::create_dir_all(copy.join("resources")).unwrap();
    assert!(refusal(&copy).is_some(), "no marker");
    fs::write(copy.join(MARKER), "").unwrap();
    assert_eq!(refusal(&copy), None);

    let install = dir.path().join("FXServer");
    fs::create_dir_all(install.join("resources/[maps]")).unwrap();
    fs::write(install.join("FXServer.exe"), "").unwrap();
    fs::write(install.join(MARKER), "").unwrap();
    fs::write(install.join("resources/[maps]").join(MARKER), "").unwrap();
    assert!(refusal(&install).is_some(), "an install");
    assert!(
        refusal(&install.join("resources/[maps]")).is_some(),
        "inside an install"
    );
    assert!(refusal(&dir.path().join("missing")).is_some());
}

#[test]
fn fixes_on_a_scratch_copy_of_a_real_server_are_applied_healed_and_undone() {
    let Some(folder) = std::env::var_os(FOLDER).map(PathBuf::from) else {
        eprintln!("{FOLDER} is not set: no server to fix");
        return;
    };
    if let Some(refused) = refusal(&folder) {
        eprintln!("refused: {refused}");
        return;
    }
    let cfg = std::env::var(CFG).unwrap_or_else(|_| "server.cfg".to_owned());
    let settings = ToolSettings::default();
    let cancel = AtomicBool::new(false);
    let scan = observer_tools::resources::scan_cfg(&folder, &cfg, None).expect("the cfg reads");
    let starts = Starts::from_scan(&scan, None);
    let index = || ServerIndex::build(&folder, &starts, &cancel).expect("the folder indexes");
    let data = tempfile::tempdir().unwrap();
    let backups = Backups::new(data.path().join("tools").join("backups"));
    let mut journal = MemoryJournal::default();
    let before = snapshot(&folder);

    let fixed = apply(&folder, &index, &settings, &mut journal, &backups);

    let after = archetypes::analyse(&index(), &settings, &cancel).expect("the analysis runs");
    print_plan("after the fix", &after);
    let left: Vec<String> = after
        .report
        .groups
        .iter()
        .filter(|group| fixed.contains(&group.hash))
        .map(|group| format!("{:08X}", group.hash))
        .collect();
    assert!(left.is_empty(), "groups fixed but still there: {left:?}");
    assert!(after.edits.is_empty(), "a second plan: {:#?}", after.edits);

    let began = Instant::now();
    let open = ServerFolder::open(&folder).unwrap();
    let healed = fix::heal(&index(), &settings, &mut journal, &open, &backups, 2, &cancel).unwrap();
    println!(
        "heal: {} files touched, {} left as they were, in {} ms",
        healed.files.len(),
        healed.unchanged,
        began.elapsed().as_millis()
    );
    assert!(healed.files.is_empty(), "{healed:#?}");

    let began = Instant::now();
    let undone = fix::undo(&mut journal, &open, &backups, &Selection::All, 3, &cancel).unwrap();
    drop(open);
    println!(
        "undo: {} files written back in {} ms",
        undone.written().count(),
        began.elapsed().as_millis()
    );
    for file in &undone.files {
        let expected = Outcome::Closed {
            reason: CloseReason::Undone,
        };
        assert_eq!(file.outcome, expected, "{}", file.path);
    }
    check_unchanged(&folder, &before, &backups);
}

/// Analyses the folder and applies the plan as run 1, checking that every file was written; answers
/// the hashes of the groups fixed.
fn apply(
    folder: &Path,
    index: &dyn Fn() -> ServerIndex,
    settings: &ToolSettings,
    journal: &mut MemoryJournal,
    backups: &Backups,
) -> HashSet<u32> {
    let cancel = AtomicBool::new(false);
    let analysis = archetypes::analyse(&index(), settings, &cancel).expect("the analysis runs");
    print_plan("before", &analysis);
    assert!(!analysis.edits.is_empty(), "nothing to fix in this copy");
    let began = Instant::now();
    let open = ServerFolder::open(folder).unwrap();
    let applied = fix::apply(&analysis.edits, &index(), journal, &open, backups, 1, &cancel).unwrap();
    println!(
        "apply: {} files written in {} ms",
        applied.written().count(),
        began.elapsed().as_millis()
    );
    for file in &applied.files {
        assert!(
            matches!(file.outcome, Outcome::Written { .. }),
            "{}: {:?}",
            file.path,
            file.outcome
        );
    }
    analysis
        .report
        .groups
        .iter()
        .filter(|group| matches!(group.action, GroupAction::Fix { .. }))
        .map(|group| group.hash)
        .collect()
}

/// Checks that every file of the folder is byte for byte what `before` says.
fn check_unchanged(folder: &Path, before: &BTreeMap<PathBuf, [u8; 32]>, backups: &Backups) {
    let now = snapshot(folder);
    assert_eq!(now.len(), before.len(), "files appeared or went");
    let differing: Vec<&PathBuf> = before
        .iter()
        .filter(|(path, sha)| now.get(*path) != Some(sha))
        .map(|(path, _)| path)
        .collect();
    assert!(
        differing.is_empty(),
        "files not back to their original: {differing:?}"
    );
    let kept = fs::read_dir(backups.folder())
        .unwrap()
        .filter(|entry| entry.as_ref().is_ok_and(|entry| entry.file_name().len() == 64))
        .count();
    println!(
        "every one of the {} files is byte for byte as it began; {kept} backups kept",
        before.len()
    );
}

fn print_plan(title: &str, analysis: &Analysis) {
    let counts = &analysis.report.counts;
    println!(
        "{title}: ytyp loaded {} | model-info loaded {} | groups {} | fixed {} | files changed {} | removals {}",
        counts.ytyp_loaded,
        counts.model_info_loaded,
        counts.groups,
        counts.groups_fixed,
        counts.files_changed,
        counts.removals
    );
    let mut per_resource: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for edit in &analysis.edits {
        let entry = per_resource.entry(&edit.resource).or_default();
        entry.0 += 1;
        entry.1 += edit.removals.len();
    }
    println!("  edits by resource (files, removals): {per_resource:?}");
}

/// Every file under `folder`, by relative path, with its SHA-256.
fn snapshot(folder: &Path) -> BTreeMap<PathBuf, [u8; 32]> {
    let mut files = BTreeMap::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(folder).unwrap().to_path_buf();
                files.insert(relative, sha256(&fs::read(&path).unwrap()));
            }
        }
    }
    files
}
