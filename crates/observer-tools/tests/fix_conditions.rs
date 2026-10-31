//! Each condition a duplicate must meet to be fixed, on synthetic servers: a group that misses one
//! is left with that reason, and nothing is taken out of it.

// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::sync::atomic::AtomicBool;

use common::{MANIFEST, Server, fixed, group, reasons, weapons, ymf};
use observer_domain::tools::{
    Contest, CopySource, Difference, Differences, EscrowPolicy, GroupAction, LeaveReason, NotWritable,
    ToolNoteKind, ToolSettings,
};
use observer_gamefiles::hash::name_hash;
use observer_gamefiles::testing::{ArchetypeSpec, YtypBuilder};
use observer_gamefiles::ytyp::ArchetypeKind;
use observer_tools::archetypes::{self, KeeperCheck, KeeperGone};
use observer_tools::edit::Format;
use observer_tools::resources::Error;
use sha2::{Digest, Sha256};

/// A server whose cfg starts the `[props]` category, holding the resource `props`.
fn props() -> Server {
    let server = Server::new();
    server.resource("[props]/props", "");
    server.cfg("ensure [props]\n");
    server
}

fn sha256(path: &std::path::Path) -> [u8; 32] {
    Sha256::digest(fs::read(path).unwrap()).into()
}

#[test]
fn the_same_archetype_twice_in_one_file_is_fixed_keeping_the_later_entry() {
    let server = props();
    let file = server.ytyp(
        "[props]/props/stream/camp.ytyp",
        &["marshmallow", "stick", "marshmallow"],
    );

    let analysis = server.analyse(&ToolSettings::default());

    let marshmallow = group(&analysis, "marshmallow");
    assert!(marshmallow.same_file);
    assert_eq!(marshmallow.difference, Difference::Identical);
    assert_eq!(
        marshmallow.action,
        GroupAction::Fix {
            keep: 1,
            remove: vec![0]
        }
    );
    assert_eq!(analysis.report.counts.groups, 1);
    let [edit] = analysis.edits.as_slice() else {
        panic!("one edit: {:#?}", analysis.edits)
    };
    assert_eq!(edit.path, "resources/[props]/props/stream/camp.ytyp");
    assert_eq!(edit.resource, "props");
    assert_eq!(edit.format, Format::Ytyp);
    assert_eq!(edit.sha256, sha256(&file));
    let [removal] = edit.removals.as_slice() else {
        panic!("one removal: {:#?}", edit.removals)
    };
    assert_eq!((removal.index, removal.hash), (0, name_hash("marshmallow")));
    assert_eq!(removal.keeper.path, edit.path);
    assert_eq!(removal.keeper.index, 2);
    assert_eq!(removal.keeper.sha256, edit.sha256);
}

#[test]
fn identical_copies_in_two_files_of_one_resource_are_fixed_keeping_the_later_file() {
    let server = props();
    // The client registers `cache:/props/gift.ytyp` before `cache:/props/gift2.ytyp`, whatever
    // folders hold them.
    server.ytyp("[props]/props/stream/z/gift.ytyp", &["gift"]);
    server.ytyp("[props]/props/stream/a/gift2.ytyp", &["gift", "gift2"]);

    let analysis = server.analyse(&ToolSettings::default());

    let gift = group(&analysis, "gift");
    assert!(!gift.same_file);
    assert_eq!(gift.copies[0].path, "resources/[props]/props/stream/z/gift.ytyp");
    assert_eq!(gift.copies[1].path, "resources/[props]/props/stream/a/gift2.ytyp");
    assert_eq!(
        gift.action,
        GroupAction::Fix {
            keep: 1,
            remove: vec![0]
        }
    );
    assert_eq!(
        gift.name.as_deref(),
        Some("gift"),
        "named after the streamed file"
    );
    let [edit] = analysis.edits.as_slice() else {
        panic!("one edit: {:#?}", analysis.edits)
    };
    assert_eq!(edit.path, "resources/[props]/props/stream/z/gift.ytyp");
    assert_eq!(
        edit.removals[0].keeper.path,
        "resources/[props]/props/stream/a/gift2.ytyp"
    );
    assert_eq!(edit.removals[0].keeper.index, 0);
}

#[test]
fn copies_that_differ_are_left_unless_the_latest_may_be_kept() {
    let server = props();
    let near = ArchetypeSpec {
        lod_dist: 60.0,
        ..ArchetypeSpec::base("donut")
    };
    let other = ArchetypeSpec {
        asset_name: name_hash("donut_b"),
        ..ArchetypeSpec::base("cookie")
    };
    server.ytyp_with(
        "[props]/props/stream/food_a.ytyp",
        vec![ArchetypeSpec::base("donut"), ArchetypeSpec::base("cookie")],
    );
    server.ytyp_with("[props]/props/stream/food_b.ytyp", vec![near, other]);

    let strict = server.analyse(&ToolSettings::default());
    let donut = group(&strict, "donut");
    assert_eq!(
        donut.difference,
        Difference::Minor {
            fields: vec!["lodDist".to_owned()]
        }
    );
    assert_eq!(reasons(donut), [LeaveReason::DefinitionsDiffer]);
    let cookie = group(&strict, "cookie");
    assert_eq!(
        cookie.difference,
        Difference::Major {
            fields: vec!["assetName".to_owned()]
        }
    );
    assert_eq!(reasons(cookie), [LeaveReason::DefinitionsDiffer]);
    assert_eq!(strict.edits, Vec::new());

    let latest = server.analyse(&ToolSettings {
        differences: Differences::KeepLatest,
        ..ToolSettings::default()
    });
    assert!(fixed(group(&latest, "donut")));
    assert!(fixed(group(&latest, "cookie")));
    let [edit] = latest.edits.as_slice() else {
        panic!("one edit: {:#?}", latest.edits)
    };
    assert_eq!(
        edit.path, "resources/[props]/props/stream/food_a.ytyp",
        "the later copies stay"
    );
    assert_eq!(edit.removals.len(), 2);
}

#[test]
fn copies_in_two_resources_are_left() {
    let server = Server::new();
    server.resource("[props]/props_a", "");
    server.resource("[props]/props_b", "");
    server.ytyp("[props]/props_a/stream/a.ytyp", &["bench"]);
    server.ytyp("[props]/props_b/stream/b.ytyp", &["bench"]);
    server.cfg("ensure [props]\n");

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(
        reasons(group(&analysis, "bench")),
        [LeaveReason::SeveralResources]
    );
    assert_eq!(analysis.edits, Vec::new());
}

#[test]
fn an_escrowed_resource_is_left_unless_escrow_is_allowed() {
    let server = props();
    server.write("resources/[props]/props/.fxap", b"FXAP\x01\0\0\0\0\0");
    server.ytyp("[props]/props/stream/crate.ytyp", &["crate", "crate"]);

    let skipped = server.analyse(&ToolSettings::default());
    let crate_group = group(&skipped, "crate");
    assert!(crate_group.copies.iter().all(|copy| copy.escrowed));
    assert_eq!(reasons(crate_group), [LeaveReason::Escrowed]);
    assert_eq!(skipped.report.counts.escrowed, 1);

    let allowed = server.analyse(&ToolSettings {
        escrowed: EscrowPolicy::Allow,
        ..ToolSettings::default()
    });
    assert!(fixed(group(&allowed, "crate")));
    assert_eq!(allowed.edits.len(), 1);
}

#[test]
fn an_interior_is_never_taken_out() {
    let server = props();
    server.ytyp_with(
        "[props]/props/stream/shell.ytyp",
        vec![ArchetypeSpec::mlo("shell_int")],
    );
    server.ytyp_with(
        "[props]/props/stream/shell_int.ytyp",
        vec![ArchetypeSpec::mlo("shell_int")],
    );

    let analysis = server.analyse(&ToolSettings::default());

    let shell = group(&analysis, "shell_int");
    assert!(matches!(
        shell.copies[0].source,
        CopySource::Ytyp {
            archetype: observer_domain::tools::ArchetypeType::Mlo,
            ..
        }
    ));
    assert_eq!(reasons(shell), [LeaveReason::Interior]);
}

#[test]
fn a_ytyp_copy_meeting_a_model_info_one_is_left() {
    let server = Server::new();
    server.resource(
        "[weapons]/arsenal",
        "files { 'meta/weaponarchetypes.meta' }\ndata_file 'WEAPON_METADATA_FILE' 'meta/weaponarchetypes.meta'\n",
    );
    server.write(
        "resources/[weapons]/arsenal/meta/weaponarchetypes.meta",
        weapons(&[("w_sb_mag1", "")]),
    );
    server.ytyp("[weapons]/arsenal/stream/arsenal.ytyp", &["w_sb_mag1"]);
    server.cfg("ensure [weapons]\n");

    let analysis = server.analyse(&ToolSettings {
        differences: Differences::KeepLatest,
        ..ToolSettings::default()
    });

    let mag = group(&analysis, "w_sb_mag1");
    assert_eq!(
        mag.difference,
        Difference::Major {
            fields: vec!["source".to_owned()]
        }
    );
    assert_eq!(reasons(mag), [LeaveReason::MixedSources]);
    assert_eq!(mag.name.as_deref(), Some("w_sb_mag1"));
}

#[test]
fn a_ytyp_whose_stem_another_resource_holds_is_left_even_when_that_resource_does_not_start() {
    let server = props();
    server.ytyp("[props]/props/stream/chairs.ytyp", &["chair", "chair"]);
    server.resource("spare", "");
    server.ytyp("spare/stream/deep/Chairs.ytyp", &["sofa"]);

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(
        reasons(group(&analysis, "chair")),
        [LeaveReason::Contested {
            file: "resources/[props]/props/stream/chairs.ytyp".to_owned(),
            by: Contest::SameName {
                resource: "spare".to_owned(),
                path: "resources/spare/stream/deep/Chairs.ytyp".to_owned(),
            },
        }]
    );
}

#[test]
fn a_ytyp_with_a_same_named_sibling_in_its_resource_is_left() {
    let server = props();
    server.ytyp("[props]/props/stream/chairs.ytyp", &["chair", "chair"]);
    server.ytyp("[props]/props/stream/old/chairs.ytyp", &["chair"]);

    let analysis = server.analyse(&ToolSettings::default());

    let chair = group(&analysis, "chair");
    assert_eq!(chair.copies.len(), 2, "the sibling is never sent");
    assert_eq!(
        reasons(chair),
        [LeaveReason::Contested {
            file: "resources/[props]/props/stream/chairs.ytyp".to_owned(),
            by: Contest::Sibling {
                path: "resources/[props]/props/stream/old/chairs.ytyp".to_owned(),
            },
        }]
    );
}

#[test]
fn a_ytyp_another_resource_requests_is_left() {
    let server = props();
    server.ytyp("[props]/props/stream/chairs.ytyp", &["chair", "chair"]);
    server.resource(
        "[props]/map",
        "data_file 'DLC_ITYP_REQUEST' 'stream/CHAIRS.ytyp'\n",
    );

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(
        reasons(group(&analysis, "chair")),
        [LeaveReason::Contested {
            file: "resources/[props]/props/stream/chairs.ytyp".to_owned(),
            by: Contest::Requested {
                resource: "map".to_owned()
            },
        }]
    );
}

#[test]
fn a_ytyp_another_resources_manifest_names_is_left() {
    let server = props();
    server.ytyp("[props]/props/stream/chairs.ytyp", &["chair", "chair"]);
    server.resource("[props]/map", "");
    server.write("resources/[props]/map/stream/_manifest.ymf", ymf(&["chairs"]));

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(
        reasons(group(&analysis, "chair")),
        [LeaveReason::Contested {
            file: "resources/[props]/props/stream/chairs.ytyp".to_owned(),
            by: Contest::Manifest {
                resource: "map".to_owned(),
                path: "resources/[props]/map/stream/_manifest.ymf".to_owned(),
            },
        }]
    );
}

#[test]
fn copies_in_two_files_its_own_manifest_lists_are_left_and_a_copy_inside_one_file_is_fixed() {
    let server = props();
    server.ytyp(
        "[props]/props/stream/interior_props.ytyp",
        &["lamp", "rug", "rug"],
    );
    server.ytyp("[props]/props/stream/street_props.ytyp", &["lamp"]);
    server.write(
        "resources/[props]/props/stream/_manifest.ymf",
        ymf(&["interior_props"]),
    );

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(
        reasons(group(&analysis, "lamp")),
        [LeaveReason::ManifestDependent {
            file: "resources/[props]/props/stream/interior_props.ytyp".to_owned(),
            manifest: "resources/[props]/props/stream/_manifest.ymf".to_owned(),
        }]
    );
    assert!(
        fixed(group(&analysis, "rug")),
        "one file's own duplicates do not depend on maps"
    );
}

#[test]
fn a_root_cache_file_counts_as_a_manifest_of_the_resource() {
    let server = props();
    server.ytyp("[props]/props/stream/a.ytyp", &["lamp"]);
    server.ytyp("[props]/props/stream/b.ytyp", &["lamp"]);
    server.write("resources/[props]/props/props_cache_y.dat", ymf(&["b"]));

    let analysis = server.analyse(&ToolSettings::default());

    assert!(matches!(
        reasons(group(&analysis, "lamp")).as_slice(),
        [LeaveReason::ManifestDependent { file, .. }] if file.ends_with("/b.ytyp")
    ));
}

#[test]
fn a_file_the_change_is_not_proven_on_is_left() {
    let server = props();
    let bytes = YtypBuilder::new("odd")
        .archetype(ArchetypeSpec::base("odd"))
        .archetype(ArchetypeSpec::base("odd"))
        .declared_size(ArchetypeKind::Base, 152)
        .archetypes_per_block(1)
        .build();
    server.write("resources/[props]/props/stream/odd.ytyp", bytes);

    // The copies of a misdescribed structure may not digest: whatever they hold, the file stays.
    let analysis = server.analyse(&ToolSettings {
        differences: Differences::KeepLatest,
        ..ToolSettings::default()
    });

    assert!(matches!(
        reasons(group(&analysis, "odd")).as_slice(),
        [LeaveReason::NotWritable {
            why: NotWritable::Layout { .. },
            ..
        }]
    ));
}

#[test]
fn a_read_only_file_is_left() {
    let server = props();
    let file = server.ytyp("[props]/props/stream/locked.ytyp", &["lock", "lock"]);
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&file, permissions.clone()).unwrap();

    let analysis = server.analyse(&ToolSettings::default());

    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&file, permissions).unwrap();
    assert_eq!(
        reasons(group(&analysis, "lock")),
        [LeaveReason::NotWritable {
            file: "resources/[props]/props/stream/locked.ytyp".to_owned(),
            why: NotWritable::ReadOnly,
        }]
    );
}

#[test]
fn an_excluded_resource_is_left() {
    let server = props();
    server.ytyp("[props]/props/stream/kept.ytyp", &["kept", "kept"]);

    let analysis = server.analyse(&ToolSettings {
        excluded: vec!["props".to_owned()],
        ..ToolSettings::default()
    });

    assert_eq!(
        reasons(group(&analysis, "kept")),
        [LeaveReason::Excluded {
            resource: "props".to_owned()
        }]
    );
}

#[test]
fn observers_own_paths_are_never_changed() {
    let server = props();
    server.ytyp("[props]/props/stream/own.ytyp", &["own", "own"]);
    let index = server
        .index()
        .with_refused(vec![vec!["resources".to_owned(), "[props]".to_owned()]]);

    let analysis = archetypes::analyse(&index, &ToolSettings::default(), &AtomicBool::new(false)).unwrap();

    assert!(matches!(
        reasons(group(&analysis, "own")).as_slice(),
        [LeaveReason::NotWritable {
            why: NotWritable::Refused,
            ..
        }]
    ));
}

#[test]
fn nothing_is_taken_out_of_a_group_that_cannot_be_fully_fixed() {
    let server = props();
    // Two identical copies in one file, and a third elsewhere that differs: taking out the first
    // would silence one warning and leave another, for nothing.
    server.ytyp(
        "[props]/props/stream/camp_props.ytyp",
        &["marshmallow", "marshmallow"],
    );
    server.ytyp_with(
        "[props]/props/stream/camp_pack.ytyp",
        vec![ArchetypeSpec {
            flags: 32,
            ..ArchetypeSpec::base("marshmallow")
        }],
    );

    let analysis = server.analyse(&ToolSettings::default());

    let marshmallow = group(&analysis, "marshmallow");
    assert_eq!(marshmallow.copies.len(), 3);
    assert_eq!(reasons(marshmallow), [LeaveReason::DefinitionsDiffer]);
    assert_eq!(analysis.edits, Vec::new());
    assert_eq!(analysis.report.counts.removals, 0);
}

#[test]
fn every_failed_condition_is_reported_not_only_the_first() {
    let server = Server::new();
    server.resource("[props]/props_a", "");
    server.resource("[props]/props_b", "");
    server.write("resources/[props]/props_b/.fxap", b"FXAP\x01");
    server.ytyp_with("[props]/props_a/stream/a.ytyp", vec![ArchetypeSpec::mlo("hall")]);
    server.ytyp_with(
        "[props]/props_b/stream/b.ytyp",
        vec![ArchetypeSpec {
            lod_dist: 10.0,
            ..ArchetypeSpec::mlo("hall")
        }],
    );
    server.cfg("ensure [props]\n");

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(
        reasons(group(&analysis, "hall")),
        [
            LeaveReason::DefinitionsDiffer,
            LeaveReason::SeveralResources,
            LeaveReason::Escrowed,
            LeaveReason::Interior,
        ]
    );
}

#[test]
fn identical_model_info_items_are_fixed_in_one_file_and_across_the_files_of_a_resource() {
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
    server.write(
        "resources/[weapons]/xmas/meta/pack_02/weaponarchetypes.backup.meta",
        weapons(&[("w_grip", "")]),
    );
    server.cfg("ensure [weapons]\n");

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(
        analysis.report.counts.model_info_loaded, 2,
        "the backup is not loaded"
    );
    let supp = group(&analysis, "w_supp");
    assert!(supp.same_file);
    assert_eq!(
        supp.action,
        GroupAction::Fix {
            keep: 1,
            remove: vec![0]
        }
    );
    assert!(matches!(
        supp.copies[1].source,
        CopySource::ModelInfo {
            ordinal: 2,
            line: 12,
            ..
        }
    ));
    let grip = group(&analysis, "w_grip");
    assert!(!grip.same_file);
    assert!(fixed(grip));
    let [edit] = analysis.edits.as_slice() else {
        panic!("one edit: {:#?}", analysis.edits)
    };
    assert_eq!(edit.format, Format::ModelInfo);
    assert_eq!(edit.sha256, sha256(&first));
    let removed: Vec<(usize, &str)> = edit
        .removals
        .iter()
        .map(|removal| (removal.index, removal.keeper.path.as_str()))
        .collect();
    assert_eq!(
        removed,
        [
            (0, "resources/[weapons]/xmas/meta/pack_01/weaponarchetypes.meta"),
            (1, "resources/[weapons]/xmas/meta/pack_02/weaponarchetypes.meta"),
        ]
    );
}

#[test]
fn model_info_items_that_differ_name_their_fields() {
    let server = Server::new();
    server.resource(
        "[weapons]/arms",
        "files { 'meta/*.meta' }\ndata_file 'WEAPON_METADATA_FILE' 'meta/*.meta'\n",
    );
    server.write(
        "resources/[weapons]/arms/meta/a.meta",
        weapons(&[("w_pistol", "      <lodDist>100</lodDist>\n")]),
    );
    server.write(
        "resources/[weapons]/arms/meta/b.meta",
        weapons(&[("w_pistol", "      <lodDist>200</lodDist>\n")]),
    );
    server.cfg("ensure arms\n");

    let analysis = server.analyse(&ToolSettings::default());

    let pistol = group(&analysis, "w_pistol");
    assert_eq!(
        pistol.difference,
        Difference::Major {
            fields: vec!["lodDist".to_owned()]
        }
    );
    assert_eq!(reasons(pistol), [LeaveReason::DefinitionsDiffer]);
}

#[test]
fn a_file_another_slot_owner_hides_is_listed_as_shadowed() {
    let server = Server::new();
    server.resource("[props]/abc", "");
    server.resource("[props]/abc_extra", "");
    server.ytyp("[props]/abc/stream/shared.ytyp", &["one"]);
    server.ytyp("[props]/abc_extra/stream/Shared.ytyp", &["two"]);
    server.cfg("ensure [props]\n");

    let analysis = server.analyse(&ToolSettings::default());

    let report = &analysis.report;
    assert_eq!((report.counts.ytyp_loaded, report.counts.ytyp_shadowed), (1, 1));
    assert_eq!(
        report.shadowed[0].path,
        "resources/[props]/abc/stream/shared.ytyp"
    );
    assert_eq!(
        report.shadowed[0].owner_path,
        "resources/[props]/abc_extra/stream/Shared.ytyp"
    );
}

#[test]
fn an_unreadable_ytyp_is_listed_with_what_it_looks_like() {
    let server = props();
    server.write(
        "resources/[props]/props/stream/sealed.ytyp",
        b"FXAP\x01\0\0\0\0\0garbage",
    );

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(analysis.report.counts.ytyp_unreadable, 1);
    assert_eq!(
        analysis.report.unreadable[0].problem,
        observer_domain::tools::FileProblem::Format {
            found: observer_domain::tools::FileFormat::Escrowed
        }
    );
}

#[test]
fn a_removal_holds_while_its_keeper_stands_and_is_undone_once_it_goes() {
    let server = props();
    server.ytyp("[props]/props/stream/gift.ytyp", &["gift"]);
    let keeper = server.ytyp("[props]/props/stream/gift2.ytyp", &["gift"]);
    let settings = ToolSettings::default();
    let analysis = server.analyse(&settings);
    let edit = &analysis.edits[0];
    let removal = &edit.removals[0];
    let check = || archetypes::check_keeper(&server.index(), &settings, &edit.path, removal);

    assert_eq!(check(), KeeperCheck::Holds);

    // Rewritten with the archetype still in it: it holds.
    server.ytyp("[props]/props/stream/gift2.ytyp", &["bow", "gift"]);
    assert_eq!(check(), KeeperCheck::Holds);
    // Rewritten without it: the removal must be undone.
    server.ytyp("[props]/props/stream/gift2.ytyp", &["bow"]);
    assert_eq!(check(), KeeperCheck::Gone(KeeperGone::NotDefined));
    server.ytyp("[props]/props/stream/gift2.ytyp", &["gift"]);

    server.resource("spare", "");
    server.ytyp("spare/stream/gift2.ytyp", &["gift"]);
    assert!(matches!(check(), KeeperCheck::Gone(KeeperGone::Contested(_))));
    fs::remove_dir_all(server.root().join("resources/spare")).unwrap();

    server.write("resources/[props]/props/.fxap", b"FXAP\x01");
    assert_eq!(check(), KeeperCheck::Gone(KeeperGone::Escrowed));
    fs::remove_file(server.root().join("resources/[props]/props/.fxap")).unwrap();
    let excluded = ToolSettings {
        excluded: vec!["props".to_owned()],
        ..ToolSettings::default()
    };
    assert_eq!(
        archetypes::check_keeper(&server.index(), &excluded, &edit.path, removal),
        KeeperCheck::Gone(KeeperGone::Excluded)
    );

    fs::remove_file(&keeper).unwrap();
    assert_eq!(check(), KeeperCheck::Gone(KeeperGone::Missing));
}

#[test]
fn a_model_info_keeper_no_longer_loaded_is_gone() {
    let server = Server::new();
    server.resource(
        "[weapons]/arms",
        "files { 'meta/*.meta' }\ndata_file 'WEAPON_METADATA_FILE' 'meta/*.meta'\n",
    );
    server.write("resources/[weapons]/arms/meta/a.meta", weapons(&[("w_grip", "")]));
    server.write("resources/[weapons]/arms/meta/b.meta", weapons(&[("w_grip", "")]));
    server.cfg("ensure arms\n");
    let settings = ToolSettings::default();
    let analysis = server.analyse(&settings);
    let edit = &analysis.edits[0];
    let removal = &edit.removals[0];
    assert_eq!(edit.path, "resources/[weapons]/arms/meta/a.meta");
    assert_eq!(
        archetypes::check_keeper(&server.index(), &settings, &edit.path, removal),
        KeeperCheck::Holds
    );

    server.write(
        "resources/[weapons]/arms/fxmanifest.lua",
        format!("{MANIFEST}files {{ 'meta/a.meta' }}\ndata_file 'WEAPON_METADATA_FILE' 'meta/a.meta'\n"),
    );

    assert_eq!(
        archetypes::check_keeper(&server.index(), &settings, &edit.path, removal),
        KeeperCheck::Gone(KeeperGone::NotLoaded)
    );
}

#[test]
fn a_redm_server_is_not_scanned() {
    let server = props();
    server.ytyp("[props]/props/stream/camp.ytyp", &["camp", "camp"]);
    server.cfg("set gamename rdr3\nensure [props]\n");

    let analysis = server.analyse(&ToolSettings::default());

    assert_eq!(analysis.report.groups, Vec::new());
    assert_eq!(analysis.report.counts.resources, 0);
    assert_eq!(analysis.report.notes[0].kind, ToolNoteKind::UnsupportedGame);
}

#[test]
fn an_analysis_stops_when_cancelled() {
    let server = props();
    server.ytyp("[props]/props/stream/camp.ytyp", &["camp", "camp"]);
    let index = server.index();

    let cancelled = archetypes::analyse(&index, &ToolSettings::default(), &AtomicBool::new(true));

    assert!(matches!(cancelled, Err(Error::Cancelled)), "{cancelled:?}");
}
