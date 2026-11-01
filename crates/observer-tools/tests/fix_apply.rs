//! Applying a plan on synthetic servers: the duplicates go and the originals are kept, the cache's
//! hard links keep the old bytes, a second fix of a changed file adds to its record, and every way a
//! write can be refused or interrupted leaves the files and the journal agreeing.

// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::sync::atomic::AtomicBool;

use common::fixes::{Fixes, archetypes, hashes, items, names_in, outcomes, sha256};
use common::{Server, weapons};
use observer_domain::tools::{Differences, ToolSettings};
use observer_gamefiles::testing::ArchetypeSpec;
use observer_tools::edit::Format;
use observer_tools::fix::{
    self, CloseReason, Failure, Journal, Outcome, Pending, Recorded, Selection, ServerFolder, Skip,
};

const CAMP: &str = "resources/[props]/props/stream/camp.ytyp";
const GIFT: &str = "resources/[props]/props/stream/gift.ytyp";
const STREAM: &str = "resources/[props]/props/stream";

/// A server whose `props` resource has an archetype twice in `camp.ytyp`, and one in both
/// `gift.ytyp` and `gift2.ytyp`: the plan takes the first marshmallow out of `camp.ytyp` and the
/// gift out of `gift.ytyp`.
fn props() -> Server {
    let server = Server::new();
    server.resource("[props]/props", "");
    server.cfg("ensure [props]\n");
    server.ytyp(
        "[props]/props/stream/camp.ytyp",
        &["marshmallow", "stick", "marshmallow"],
    );
    server.ytyp("[props]/props/stream/gift.ytyp", &["gift"]);
    server.ytyp("[props]/props/stream/gift2.ytyp", &["gift", "bow"]);
    server
}

fn written(removed: &[usize]) -> Outcome {
    Outcome::Written {
        removed: removed.to_vec(),
    }
}

fn skipped(why: Skip) -> Outcome {
    Outcome::Skipped { why }
}

#[test]
fn a_plan_applied_leaves_no_duplicate_and_keeps_each_original() {
    let server = props();
    let camp = fs::read(server.root().join(CAMP)).unwrap();
    let gift = fs::read(server.root().join(GIFT)).unwrap();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    assert_eq!(analysis.edits.len(), 2);

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert_eq!(
        outcomes(&report),
        [(CAMP, &written(&[0])), (GIFT, &written(&[0]))]
    );
    assert_eq!(report.written().count(), 2);
    assert!(report.reconciled.is_empty() && !report.cancelled && !report.storage_full);
    assert_eq!(server.analyse(&ToolSettings::default()).report.counts.groups, 0);
    assert_eq!(
        archetypes(&server.root().join(CAMP)),
        hashes(&["stick", "marshmallow"])
    );
    assert_eq!(archetypes(&server.root().join(GIFT)), Vec::<u32>::new());
    for (path, original) in [(CAMP, &camp), (GIFT, &gift)] {
        let record = fixes.journal.record(path).unwrap();
        assert_eq!(record.original_sha256, sha256(original));
        assert_eq!(
            record.patched_sha256,
            Some(sha256(&fs::read(server.root().join(path)).unwrap()))
        );
        assert!(record.pending.is_none());
        assert_eq!(record.removals.len(), 1);
        assert_eq!(record.removals[0].run, 1);
        assert_eq!(&fixes.backups.get(&record.original_sha256).unwrap(), original);
    }
    assert_eq!(fixes.journal.uploads, Vec::<(String, String)>::new());
    assert_eq!(
        names_in(&server.root().join(STREAM)),
        ["camp.ytyp", "gift.ytyp", "gift2.ytyp"]
    );
}

#[test]
fn a_stream_file_hard_linked_into_the_cache_is_replaced_and_the_link_keeps_the_old_bytes() {
    let server = props();
    let cache = server.root().join("cache/files/props/stream_cache");
    fs::create_dir_all(&cache).unwrap();
    fs::hard_link(server.root().join(CAMP), cache.join("camp.ytyp")).unwrap();
    let original = fs::read(server.root().join(CAMP)).unwrap();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert_eq!(report.files[0].outcome, written(&[0]));
    assert_eq!(
        archetypes(&server.root().join(CAMP)),
        hashes(&["stick", "marshmallow"])
    );
    assert_eq!(fs::read(cache.join("camp.ytyp")).unwrap(), original);
}

#[test]
fn a_second_fix_of_a_changed_file_adds_to_its_record_and_each_run_is_undone_alone() {
    let server = Server::new();
    server.resource("[props]/props", "");
    server.cfg("ensure [props]\n");
    let far = ArchetypeSpec {
        lod_dist: 200.0,
        ..ArchetypeSpec::base("bow")
    };
    let path = server.ytyp_with(
        "[props]/props/stream/camp.ytyp",
        vec![
            ArchetypeSpec::base("gift"),
            ArchetypeSpec::base("bow"),
            ArchetypeSpec::base("gift"),
            far,
        ],
    );
    let original = fs::read(&path).unwrap();
    let mut fixes = Fixes::new();

    // Identical copies only: the first gift goes, the bows differ.
    let first = server.analyse(&ToolSettings::default());
    assert_eq!(
        fixes.apply(&server, &first.edits, 1).files[0].outcome,
        written(&[0])
    );
    assert_eq!(archetypes(&path), hashes(&["bow", "gift", "bow"]));

    // Keeping the latest copy: the near bow, first in the changed file, is the original's second.
    let latest = ToolSettings {
        differences: Differences::KeepLatest,
        ..ToolSettings::default()
    };
    let second = server.analyse(&latest);
    assert_eq!(second.edits[0].removals[0].index, 0);
    assert_eq!(
        fixes.apply(&server, &second.edits, 2).files[0].outcome,
        written(&[0, 1])
    );
    assert_eq!(archetypes(&path), hashes(&["gift", "bow"]));
    assert_eq!(server.analyse(&latest).report.counts.groups, 0);
    let runs: Vec<(usize, i64)> = fixes
        .journal
        .record(CAMP)
        .unwrap()
        .removals
        .iter()
        .map(|recorded| (recorded.removal.index, recorded.run))
        .collect();
    assert_eq!(runs, [(0, 1), (1, 2)]);

    // The first run undone alone: the first gift is back, the near bow stays out.
    let undone = fixes.undo(&server, &Selection::Run(1), 3);
    assert_eq!(undone.files[0].outcome, written(&[1]));
    assert_eq!(undone.files[0].restored.len(), 1);
    assert_eq!(
        (undone.files[0].restored[0].index, undone.files[0].restored[0].run),
        (0, 1)
    );
    assert_eq!(archetypes(&path), hashes(&["gift", "gift", "bow"]));

    // Then the second: the original, byte for byte, and the record closed.
    let undone = fixes.undo(&server, &Selection::Run(2), 4);
    assert_eq!(
        undone.files[0].outcome,
        Outcome::Closed {
            reason: CloseReason::Undone
        }
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fixes.journal.closed(CAMP), [CloseReason::Undone]);
    assert_eq!(fixes.journal.open_paths(), Vec::<String>::new());
}

#[test]
fn a_file_changed_between_the_plan_and_the_write_is_skipped() {
    let server = props();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    server.ytyp(
        "[props]/props/stream/camp.ytyp",
        &["marshmallow", "stick", "marshmallow", "box"],
    );
    let changed = fs::read(server.root().join(CAMP)).unwrap();

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert_eq!(
        outcomes(&report),
        [(CAMP, &skipped(Skip::Changed)), (GIFT, &written(&[0]))]
    );
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), changed);
    assert!(fixes.journal.record(CAMP).is_none());
    assert_eq!(fixes.journal.closed(CAMP), Vec::<CloseReason>::new());
}

#[test]
fn a_file_replaced_between_its_read_and_the_rename_is_never_overwritten() {
    let server = props();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    let root = server.root().to_path_buf();
    // Another writer commits its own file under the name while the write is under way.
    fixes.journal.on_begin = Some(Box::new(move |path, _| {
        let theirs = root.join("theirs.partial");
        fs::write(&theirs, b"theirs").unwrap();
        fs::rename(&theirs, root.join(path)).unwrap();
    }));

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert_eq!(
        outcomes(&report),
        [(CAMP, &skipped(Skip::Changed)), (GIFT, &skipped(Skip::Changed))]
    );
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), b"theirs");
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), b"theirs");
    assert_eq!(fixes.journal.closed(CAMP), [CloseReason::Abandoned]);
    assert_eq!(fixes.journal.open_paths(), Vec::<String>::new());
    assert_eq!(fixes.journal.uploads, Vec::<(String, String)>::new());
    assert_eq!(
        names_in(&server.root().join(STREAM)),
        ["camp.ytyp", "gift.ytyp", "gift2.ytyp"]
    );
}

#[test]
fn a_file_rewritten_in_place_between_its_read_and_the_rename_is_never_overwritten() {
    let server = props();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    let root = server.root().to_path_buf();
    // Another writer rewrites the file through its name while the write is under way: the file
    // keeps its identity, and only its bytes changed.
    fixes.journal.on_begin = Some(Box::new(move |path, _| {
        fs::write(root.join(path), b"theirs").unwrap();
    }));

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert_eq!(
        outcomes(&report),
        [(CAMP, &skipped(Skip::Changed)), (GIFT, &skipped(Skip::Changed))]
    );
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), b"theirs");
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), b"theirs");
    assert_eq!(fixes.journal.closed(CAMP), [CloseReason::Abandoned]);
    assert_eq!(fixes.journal.open_paths(), Vec::<String>::new());
    assert_eq!(fixes.journal.uploads, Vec::<(String, String)>::new());
    assert_eq!(
        names_in(&server.root().join(STREAM)),
        ["camp.ytyp", "gift.ytyp", "gift2.ytyp"]
    );
}

#[test]
fn a_failed_write_whose_file_holds_the_new_bytes_is_recorded_as_written_not_abandoned() {
    let server = props();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    let original = fs::read(server.root().join(CAMP)).unwrap();
    let patched = observer_gamefiles::ytyp::remove_archetypes(&original, &[0]).unwrap();
    let root = server.root().to_path_buf();
    let landed = patched.clone();
    // The new bytes are in place while the commit fails, as when the rename lands and the folder's
    // flush after it fails: here a new file under the name refuses the commit.
    fixes.journal.on_begin = Some(Box::new(move |path, pending| {
        if path == CAMP {
            assert_eq!(sha256(&landed), pending.expected_sha256);
            let file = root.join("landed.partial");
            fs::write(&file, &landed).unwrap();
            fs::rename(&file, root.join(path)).unwrap();
        }
    }));

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert_eq!(
        outcomes(&report),
        [(CAMP, &written(&[0])), (GIFT, &written(&[0]))]
    );
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), patched);
    let record = fixes.journal.record(CAMP).unwrap();
    assert_eq!(record.original_sha256, sha256(&original));
    assert_eq!(record.patched_sha256, Some(sha256(&patched)));
    assert!(record.pending.is_none());
    assert_eq!(fixes.journal.closed(CAMP), Vec::<CloseReason>::new());
    assert_eq!(fixes.journal.uploads, Vec::<(String, String)>::new());

    // The record knows the removal: an undo puts it back.
    fixes.journal.on_begin = None;
    let undone = fixes.undo(&server, &Selection::Paths(vec![CAMP.to_owned()]), 2);
    assert_eq!(
        undone.files[0].outcome,
        Outcome::Closed {
            reason: CloseReason::Undone
        }
    );
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), original);
}

/// Records the first write of the plan's first edit, which would write `bytes`, as the journal does
/// before the temporary file exists, with its backup kept.
fn pending_write(fixes: &mut Fixes, server: &Server, bytes: &[u8]) -> Pending {
    let analysis = server.analyse(&ToolSettings::default());
    let edit = &analysis.edits[0];
    let original = fs::read(server.root().join(&edit.path)).unwrap();
    let original_sha256 = fixes.backups.put(&original, "server", &edit.path).unwrap();
    let pending = Pending {
        temporary: fix::temporary_name().unwrap(),
        expected_sha256: sha256(bytes),
        removals: edit
            .removals
            .iter()
            .map(|removal| Recorded {
                removal: removal.clone(),
                run: 9,
            })
            .collect(),
    };
    let folder = Fixes::folder(server);
    fixes
        .journal
        .begin(
            9,
            folder.id(),
            &edit.path,
            &edit.resource,
            edit.format,
            original_sha256,
            &pending,
            None,
        )
        .unwrap();
    pending
}

#[test]
fn a_write_interrupted_before_its_rename_is_abandoned_at_the_next_start_and_its_file_removed() {
    let server = props();
    let mut fixes = Fixes::new();
    let original = fs::read(server.root().join(CAMP)).unwrap();
    let patched = observer_gamefiles::ytyp::remove_archetypes(&original, &[0]).unwrap();
    let pending = pending_write(&mut fixes, &server, &patched);
    // Observer stopped while writing the temporary file.
    let temporary = server.root().join(STREAM).join(&pending.temporary);
    fs::write(&temporary, &patched[..patched.len() / 2]).unwrap();

    let settled = fixes.reconcile(&server, 10);

    assert_eq!(settled.len(), 1);
    assert_eq!(
        settled[0].outcome,
        Outcome::Closed {
            reason: CloseReason::Abandoned
        }
    );
    assert!(!temporary.exists());
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), original);
    assert_eq!(fixes.journal.closed(CAMP), [CloseReason::Abandoned]);
    assert_eq!(fixes.journal.uploads, Vec::<(String, String)>::new());
}

#[test]
fn an_interrupted_write_to_a_changed_file_leaves_its_record_as_it_was() {
    let server = props();
    let mut fixes = Fixes::new();
    let original = fs::read(server.root().join(CAMP)).unwrap();
    fixes.apply(&server, &server.analyse(&ToolSettings::default()).edits, 1);
    let before = fixes.journal.record(CAMP).unwrap().clone();
    // An undo that never got to its rename.
    let pending = undo_pending(&mut fixes, &server, &original);
    fs::write(server.root().join(STREAM).join(&pending.temporary), b"half").unwrap();

    let settled = fixes.reconcile(&server, 10);

    assert_eq!(settled[0].outcome, Outcome::Kept);
    assert_eq!(fixes.journal.record(CAMP).unwrap(), &before);
    assert_eq!(
        names_in(&server.root().join(STREAM)),
        ["camp.ytyp", "gift.ytyp", "gift2.ytyp"]
    );

    // The same undo, whose rename landed this time but whose record was never closed.
    let pending = undo_pending(&mut fixes, &server, &original);
    fs::write(server.root().join(CAMP), &original).unwrap();
    let settled = fixes.reconcile(&server, 11);
    assert_eq!(
        settled[0].outcome,
        Outcome::Closed {
            reason: CloseReason::Restored
        }
    );
    assert!(!server.root().join(STREAM).join(&pending.temporary).exists());
    assert!(fixes.journal.record(CAMP).is_none());
}

/// Records an undo of the camp's record back to `original`, as the journal does before the rename.
fn undo_pending(fixes: &mut Fixes, server: &Server, original: &[u8]) -> Pending {
    let record = fixes.journal.record(CAMP).unwrap().clone();
    let pending = Pending {
        temporary: fix::temporary_name().unwrap(),
        expected_sha256: sha256(original),
        removals: Vec::new(),
    };
    let folder = Fixes::folder(server);
    fixes
        .journal
        .begin(
            9,
            folder.id(),
            CAMP,
            &record.resource,
            Format::Ytyp,
            record.original_sha256,
            &pending,
            Some(record.id),
        )
        .unwrap();
    pending
}

#[test]
fn a_rename_that_landed_before_the_journal_heard_of_it_is_applied_at_the_next_start() {
    let server = props();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    fixes.journal.fail_applied = true;

    let failed = fixes.try_apply(&server, &analysis.edits, 1);

    assert!(matches!(failed, Err(fix::Error::Journal(_))), "{failed:?}");
    let record = fixes.journal.record(CAMP).unwrap().clone();
    let pending = record.pending.clone().unwrap();
    assert_eq!(
        sha256(&fs::read(server.root().join(CAMP)).unwrap()),
        pending.expected_sha256
    );
    assert!(
        fixes.journal.record(GIFT).is_none(),
        "the run stopped at the journal's failure"
    );

    fixes.journal.fail_applied = false;
    let settled = fixes.reconcile(&server, 2);

    assert_eq!(settled[0].outcome, written(&[0]));
    let record = fixes.journal.record(CAMP).unwrap();
    assert_eq!(record.patched_sha256, Some(pending.expected_sha256));
    assert!(record.pending.is_none());
    assert_eq!(fixes.journal.uploads, Vec::<(String, String)>::new());
    let left = server.analyse(&ToolSettings::default());
    assert_eq!(left.edits.len(), 1);
    assert_eq!(left.edits[0].path, GIFT);
}

#[test]
fn a_full_volume_stops_the_run_before_anything_is_written() {
    let server = props();
    let camp = fs::read(server.root().join(CAMP)).unwrap();
    let mut fixes = Fixes::new();
    fixes.backups = fixes.backups.clone().with_reserve(u64::MAX);
    let analysis = server.analyse(&ToolSettings::default());

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert!(report.storage_full);
    assert!(matches!(
        report.files[0].outcome,
        Outcome::Failed {
            why: Failure::StorageFull { .. }
        }
    ));
    assert_eq!(report.files[1].outcome, skipped(Skip::NotTried));
    assert_eq!(report.written().count(), 0);
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), camp);
    assert!(fixes.journal.stored.is_empty());
    assert_eq!(fixes.backup_names(), Vec::<String>::new());
}

#[test]
fn a_cancelled_run_writes_nothing() {
    let server = props();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    let folder = Fixes::folder(&server);

    let report = fix::apply(
        &analysis.edits,
        &server.index(),
        &mut fixes.journal,
        &folder,
        &fixes.backups,
        1,
        &AtomicBool::new(true),
    )
    .unwrap();

    assert!(report.cancelled);
    assert!(
        report
            .files
            .iter()
            .all(|file| file.outcome == skipped(Skip::NotTried))
    );
    assert!(fixes.journal.stored.is_empty());
}

#[test]
fn observers_own_paths_are_never_written_whatever_the_plan_says() {
    let server = props();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());
    let folder = ServerFolder::open(server.root())
        .unwrap()
        .with_refused(vec![vec!["resources".to_owned(), "[props]".to_owned()]]);

    let report = fix::apply(
        &analysis.edits,
        &server.index(),
        &mut fixes.journal,
        &folder,
        &fixes.backups,
        1,
        &AtomicBool::new(false),
    )
    .unwrap();

    assert!(
        report
            .files
            .iter()
            .all(|file| file.outcome == skipped(Skip::Refused))
    );
    assert!(fixes.journal.stored.is_empty());
}

#[test]
fn a_backup_that_no_longer_holds_the_original_is_never_restored() {
    let server = props();
    let mut fixes = Fixes::new();
    fixes.apply(&server, &server.analyse(&ToolSettings::default()).edits, 1);
    let gift = fixes.journal.record(GIFT).unwrap().clone();
    let patched = fs::read(server.root().join(GIFT)).unwrap();
    fs::write(fixes.backups.file(&gift.original_sha256), b"damaged").unwrap();

    let report = fixes.undo(&server, &Selection::All, 2);

    assert_eq!(
        outcomes(&report),
        [
            (
                CAMP,
                &Outcome::Closed {
                    reason: CloseReason::Undone
                }
            ),
            (
                GIFT,
                &Outcome::Failed {
                    why: Failure::Backup {
                        error: format!(
                            "the backup {} no longer holds the bytes it is named after, so it is not used",
                            fixes
                                .backups
                                .file(&gift.original_sha256)
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                        )
                    }
                }
            ),
        ]
    );
    assert!(report.files[1].restored.is_empty(), "{:#?}", report.files[1]);
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), patched);
    assert_eq!(fixes.journal.record(GIFT).unwrap(), &gift);

    // A heal refuses it too, and says so.
    server.ytyp("[props]/props/stream/gift2.ytyp", &["bow"]);
    let healed = fixes.heal(&server, &ToolSettings::default(), 3);
    assert!(matches!(
        healed.files[0].outcome,
        Outcome::Failed {
            why: Failure::Backup { .. }
        }
    ));
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), patched);
}

#[test]
fn model_info_items_are_taken_out_and_put_back_too() {
    let server = Server::new();
    server.resource(
        "[weapons]/xmas",
        "files { 'meta/**/weaponarchetypes.meta' }\ndata_file 'WEAPON_METADATA_FILE' 'meta/**/weaponarchetypes.meta'\n",
    );
    let first = server.write(
        "resources/[weapons]/xmas/meta/pack_01/weaponarchetypes.meta",
        weapons(&[("w_supp", ""), ("w_grip", ""), ("w_supp", "")]),
    );
    server.write(
        "resources/[weapons]/xmas/meta/pack_02/weaponarchetypes.meta",
        weapons(&[("w_grip", "")]),
    );
    server.cfg("ensure [weapons]\n");
    let original = fs::read(&first).unwrap();
    let mut fixes = Fixes::new();
    let analysis = server.analyse(&ToolSettings::default());

    let report = fixes.apply(&server, &analysis.edits, 1);

    assert_eq!(report.files[0].outcome, written(&[0, 1]));
    assert_eq!(items(&first), hashes(&["w_supp"]));
    assert_eq!(server.analyse(&ToolSettings::default()).report.counts.groups, 0);

    let undone = fixes.undo(&server, &Selection::All, 2);

    assert_eq!(
        undone.files[0].outcome,
        Outcome::Closed {
            reason: CloseReason::Undone
        }
    );
    assert_eq!(undone.files[0].restored.len(), 2);
    assert_eq!(fs::read(&first).unwrap(), original);
}
