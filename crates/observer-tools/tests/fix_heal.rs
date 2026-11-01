//! Healing and undoing fixes on synthetic servers: a removal is put back as soon as the copy kept no
//! longer stands in for it, and only that one; a file someone else changed is left to them; an undo
//! puts back exactly what it is asked to, byte for byte.

// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

mod common;

use std::fs;

use common::fixes::{Fixes, archetypes, hashes, outcomes};
use common::{Server, ymf};
use observer_domain::tools::ToolSettings;
use observer_tools::fix::{CloseReason, Outcome, Report, Selection, Skip};

const CAMP: &str = "resources/[props]/props/stream/camp.ytyp";
const GIFT: &str = "resources/[props]/props/stream/gift.ytyp";

/// A server whose `props` resource defines the gift in `gift.ytyp` and `gift2.ytyp`, and holds a
/// `camp.ytyp` with `extra` in it: the plan takes the gift out of `gift.ytyp`, keeping
/// `gift2.ytyp`'s.
fn gifts(camp: &[&str]) -> Server {
    let server = Server::new();
    server.resource("[props]/props", "");
    server.cfg("ensure [props]\n");
    server.ytyp("[props]/props/stream/camp.ytyp", camp);
    server.ytyp("[props]/props/stream/gift.ytyp", &["gift"]);
    server.ytyp("[props]/props/stream/gift2.ytyp", &["gift", "bow"]);
    server
}

/// The server with the plan applied as run 1.
fn fixed(server: &Server) -> Fixes {
    let mut fixes = Fixes::new();
    let report = fixes.apply(server, &server.analyse(&ToolSettings::default()).edits, 1);
    assert!(report.written().count() > 0, "{report:#?}");
    fixes
}

fn closed(reason: CloseReason) -> Outcome {
    Outcome::Closed { reason }
}

/// Why the one entry put back in the report's one file was put back.
fn why(report: &Report) -> String {
    let [file] = report.files.as_slice() else {
        panic!("one file: {report:#?}")
    };
    let [restored] = file.restored.as_slice() else {
        panic!("one entry put back: {file:#?}")
    };
    restored.why.clone().unwrap()
}

#[test]
fn a_removal_whose_copy_kept_no_longer_defines_the_archetype_is_put_back() {
    let server = gifts(&["stick"]);
    let original = fs::read(server.root().join(GIFT)).unwrap();
    let mut fixes = fixed(&server);
    // FXServer links the changed file into its cache at its next start.
    let cache = server.root().join("cache/files/props/stream_cache");
    fs::create_dir_all(&cache).unwrap();
    fs::hard_link(server.root().join(GIFT), cache.join("gift.ytyp")).unwrap();
    let patched = fs::read(server.root().join(GIFT)).unwrap();
    server.ytyp("[props]/props/stream/gift2.ytyp", &["bow"]);

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert_eq!(outcomes(&report), [(GIFT, &closed(CloseReason::Healed))]);
    assert_eq!(why(&report), "the copy kept no longer defines the archetype");
    assert_eq!(report.files[0].restored[0].run, 1);
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), original);
    assert_eq!(fs::read(cache.join("gift.ytyp")).unwrap(), patched);
    assert_eq!(fixes.journal.closed(GIFT), [CloseReason::Healed]);
    assert_eq!(fixes.journal.open_paths(), Vec::<String>::new());
    assert_eq!(server.analyse(&ToolSettings::default()).report.counts.groups, 0);
}

#[test]
fn only_the_removals_whose_copy_kept_is_gone_are_put_back() {
    let server = Server::new();
    server.resource("[props]/props", "");
    server.cfg("ensure [props]\n");
    // The gift is kept in gift2.ytyp, the stick in camp.ytyp itself.
    server.ytyp("[props]/props/stream/camp.ytyp", &["gift", "stick", "stick"]);
    server.ytyp("[props]/props/stream/gift2.ytyp", &["gift"]);
    let mut fixes = fixed(&server);
    assert_eq!(archetypes(&server.root().join(CAMP)), hashes(&["stick"]));
    server.ytyp("[props]/props/stream/gift2.ytyp", &["bow"]);

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert_eq!(
        outcomes(&report),
        [(CAMP, &Outcome::Written { removed: vec![1] })]
    );
    assert_eq!(report.files[0].restored[0].index, 0);
    assert_eq!(archetypes(&server.root().join(CAMP)), hashes(&["gift", "stick"]));
    let record = fixes.journal.record(CAMP).unwrap();
    assert_eq!(record.removals.len(), 1);
    assert_eq!(record.removals[0].removal.index, 1);
    assert_eq!(server.analyse(&ToolSettings::default()).report.counts.groups, 0);
}

#[test]
fn a_removal_whose_copy_kept_another_file_may_replace_is_put_back() {
    let server = gifts(&["stick"]);
    let original = fs::read(server.root().join(GIFT)).unwrap();
    let mut fixes = fixed(&server);
    server.resource("spare", "");
    server.ytyp("spare/stream/gift2.ytyp", &["gift"]);

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert_eq!(outcomes(&report), [(GIFT, &closed(CloseReason::Healed))]);
    assert_eq!(
        why(&report),
        "another file may be loaded in place of the copy kept"
    );
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), original);
}

#[test]
fn a_removal_into_another_file_is_put_back_once_its_resource_gains_a_manifest_listing_the_copy_kept() {
    let server = gifts(&["stick"]);
    let original = fs::read(server.root().join(GIFT)).unwrap();
    let mut fixes = fixed(&server);
    // A map update added since: the client may now load gift2.ytyp only with the maps that need it.
    server.write("resources/[props]/props/stream/_manifest.ymf", ymf(&["gift2"]));

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert_eq!(outcomes(&report), [(GIFT, &closed(CloseReason::Healed))]);
    assert_eq!(
        why(&report),
        "the manifest resources/[props]/props/stream/_manifest.ymf may load the copies only with the maps \
         that need them"
    );
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), original);
}

#[test]
fn a_removal_whose_copy_kept_is_now_escrowed_is_put_back() {
    let server = gifts(&["stick"]);
    let original = fs::read(server.root().join(GIFT)).unwrap();
    let mut fixes = fixed(&server);
    server.write("resources/[props]/props/.fxap", b"FXAP\x01");

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert_eq!(outcomes(&report), [(GIFT, &closed(CloseReason::Healed))]);
    assert_eq!(why(&report), "the resource of the copy kept is now escrowed");
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), original);
}

#[test]
fn a_heal_writes_nothing_while_every_copy_kept_stands() {
    let server = gifts(&["stick", "stick"]);
    let mut fixes = fixed(&server);
    let camp = fs::read(server.root().join(CAMP)).unwrap();
    let gift = fs::read(server.root().join(GIFT)).unwrap();

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert!(report.files.is_empty(), "{report:#?}");
    assert_eq!(report.unchanged, 2);
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), camp);
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), gift);
    assert_eq!(fixes.journal.open_records().len(), 2);
}

#[test]
fn a_heal_closes_the_records_of_files_someone_else_changed_and_writes_nothing() {
    let server = gifts(&["stick", "stick"]);
    let camp = fs::read(server.root().join(CAMP)).unwrap();
    let mut fixes = fixed(&server);
    // The owner put the camp's original back by hand, and uploaded a new gift.ytyp.
    fs::write(server.root().join(CAMP), &camp).unwrap();
    server.ytyp("[props]/props/stream/gift.ytyp", &["gift", "ribbon"]);
    let uploaded = fs::read(server.root().join(GIFT)).unwrap();

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert_eq!(
        outcomes(&report),
        [
            (CAMP, &closed(CloseReason::Restored)),
            (GIFT, &closed(CloseReason::Stale))
        ]
    );
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), camp);
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), uploaded);
    assert_eq!(fixes.journal.open_paths(), Vec::<String>::new());
}

#[test]
fn a_heal_closes_the_record_of_a_file_that_is_gone() {
    let server = gifts(&["stick"]);
    let mut fixes = fixed(&server);
    fs::remove_file(server.root().join(GIFT)).unwrap();

    let report = fixes.heal(&server, &ToolSettings::default(), 2);

    assert_eq!(outcomes(&report), [(GIFT, &closed(CloseReason::Stale))]);
    assert!(!server.root().join(GIFT).exists());
}

#[test]
fn undo_all_puts_every_file_back_byte_for_byte() {
    let server = gifts(&["stick", "stick"]);
    let camp = fs::read(server.root().join(CAMP)).unwrap();
    let gift = fs::read(server.root().join(GIFT)).unwrap();
    let mut fixes = fixed(&server);

    let report = fixes.undo(&server, &Selection::All, 2);

    assert_eq!(
        outcomes(&report),
        [
            (CAMP, &closed(CloseReason::Undone)),
            (GIFT, &closed(CloseReason::Undone))
        ]
    );
    assert_eq!(report.written().count(), 2);
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), camp);
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), gift);
    assert_eq!(fixes.journal.open_paths(), Vec::<String>::new());
    assert_eq!(fixes.journal.uploads, Vec::<(String, String)>::new());
}

#[test]
fn undo_of_some_files_leaves_the_others_and_of_an_unknown_run_does_nothing() {
    let server = gifts(&["stick", "stick"]);
    let gift = fs::read(server.root().join(GIFT)).unwrap();
    let mut fixes = fixed(&server);
    let camp = fs::read(server.root().join(CAMP)).unwrap();

    let nothing = fixes.undo(&server, &Selection::Run(7), 2);
    assert!(nothing.files.is_empty(), "{nothing:#?}");

    let report = fixes.undo(&server, &Selection::Paths(vec![GIFT.to_owned()]), 3);

    assert_eq!(outcomes(&report), [(GIFT, &closed(CloseReason::Undone))]);
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), gift);
    assert_eq!(fs::read(server.root().join(CAMP)).unwrap(), camp);
    assert!(fixes.journal.record(CAMP).is_some());
}

#[test]
fn undo_leaves_a_file_that_no_longer_holds_what_observer_wrote() {
    let server = gifts(&["stick", "stick"]);
    let mut fixes = fixed(&server);
    server.ytyp("[props]/props/stream/gift.ytyp", &["ribbon"]);
    let uploaded = fs::read(server.root().join(GIFT)).unwrap();

    let report = fixes.undo(&server, &Selection::All, 2);

    assert_eq!(
        outcomes(&report),
        [
            (CAMP, &closed(CloseReason::Undone)),
            (GIFT, &Outcome::Skipped { why: Skip::Changed })
        ]
    );
    assert_eq!(fs::read(server.root().join(GIFT)).unwrap(), uploaded);
    assert!(
        fixes.journal.record(GIFT).is_some(),
        "a heal closes it, not an undo"
    );
}
