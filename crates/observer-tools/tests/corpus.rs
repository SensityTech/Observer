//! Runs the duplicate archetypes tool, read only, on the server folder `OBSERVER_SERVER_CORPUS`
//! names, with its cfg (`OBSERVER_SERVER_CFG`, `server.cfg` by default), and prints what it found.
//! Real servers hold paid assets and the owner's cfg: this test only reads them where they are,
//! prints counts and archetype names, never a line of the cfg, and does nothing without the
//! variable.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use observer_domain::tools::{Differences, EscrowPolicy, GroupAction, LeaveReason, ToolSettings};
use observer_gamefiles::hash::name_hash;
use observer_tools::archetypes::{self, Analysis, KeeperCheck};
use observer_tools::resources::{ServerIndex, Starts};

const FOLDER: &str = "OBSERVER_SERVER_CORPUS";
const CFG: &str = "OBSERVER_SERVER_CFG";
/// The warnings the owner reported, each to be accounted for.
const EXAMPLES: [&str; 8] = [
    "bzzz_camp_food_marshmallow",
    "bzzz_camp_stick_marshmallow",
    "bz_prop_gift",
    "bzzz_foodpack_croissant001",
    "bzzz_food_dessert_a",
    "bzzz_food_xmas_gingerbread_a",
    "bzzz_plants_weed_purple_bud",
    "w_at_ar_nutscrackercarbineriflesupp",
];

#[test]
fn a_real_server_is_scanned_read_only_and_every_plan_holds() {
    let Some(folder) = std::env::var_os(FOLDER).map(PathBuf::from) else {
        eprintln!("{FOLDER} is not set: no server to scan");
        return;
    };
    let cfg = std::env::var(CFG).unwrap_or_else(|_| "server.cfg".to_owned());
    let cancel = AtomicBool::new(false);

    let began = Instant::now();
    let scan = observer_tools::resources::scan_cfg(&folder, &cfg, None).expect("the cfg reads");
    let starts = Starts::from_scan(&scan, None);
    let cfg_ms = began.elapsed().as_millis();
    let index = ServerIndex::build(&folder, &starts, &cancel).expect("the folder indexes");
    // A second build, with the folder in the system's cache.
    let warm = Instant::now();
    let index_again = ServerIndex::build(&folder, &starts, &cancel).expect("the folder indexes again");
    let warm_ms = warm.elapsed().as_millis();
    assert_eq!(index_again.started(), index.started(), "two builds agree");

    println!(
        "cfg scan: {cfg_ms} ms, {} resource commands",
        scan.resource_commands.len()
    );
    println!("index: {} ms cold-ish, {warm_ms} ms warm", index.walk_ms());
    let stream_files: usize = index
        .resources()
        .iter()
        .map(|resource| resource.stream.len())
        .sum();
    println!("stream files listed: {stream_files}");

    let default = archetypes::analyse(&index, &ToolSettings::default(), &cancel).expect("the analysis runs");
    print_analysis("default settings", &default);
    print_examples(&default);
    check_plan(&index, &ToolSettings::default(), &default);

    let open = ToolSettings {
        differences: Differences::KeepLatest,
        escrowed: EscrowPolicy::Allow,
        ..ToolSettings::default()
    };
    let wide = archetypes::analyse(&index, &open, &cancel).expect("the analysis runs");
    print_analysis("keepLatest, escrow allowed", &wide);
    print_examples(&wide);
    check_plan(&index, &open, &wide);

    assert!(default.report.counts.resources > 0, "no resource found");
}

fn print_analysis(title: &str, analysis: &Analysis) {
    let report = &analysis.report;
    let counts = &report.counts;
    println!("\n== {title}");
    println!(
        "resources {} | started {} | client {} | escrowed {}",
        counts.resources, counts.started, counts.client, counts.escrowed
    );
    println!(
        "ytyp loaded {} shadowed {} unreadable {} | model-info loaded {} unreadable {}",
        counts.ytyp_loaded,
        counts.ytyp_shadowed,
        counts.ytyp_unreadable,
        counts.model_info_loaded,
        counts.model_info_unreadable
    );
    println!(
        "registrations {} | groups {} | fixed {} | files changed {} | removals {}",
        counts.registrations, counts.groups, counts.groups_fixed, counts.files_changed, counts.removals
    );
    let mut by_reason: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_difference: BTreeMap<String, usize> = BTreeMap::new();
    let mut alone: BTreeMap<&str, usize> = BTreeMap::new();
    for group in &report.groups {
        let difference = serde_json::to_value(&group.difference).expect("serialises")["kind"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        *by_difference.entry(difference).or_default() += 1;
        if let GroupAction::Leave { reasons } = &group.action {
            let mut kinds: Vec<&str> = reasons.iter().map(reason).collect();
            kinds.dedup();
            for kind in &kinds {
                *by_reason.entry(kind).or_default() += 1;
            }
            if let [only] = kinds.as_slice() {
                *alone.entry(only).or_default() += 1;
            }
        }
    }
    println!("groups by difference: {by_difference:?}");
    println!("groups left, by reason (a group counts once per reason): {by_reason:?}");
    println!("groups left for one reason only: {alone:?}");
    let mut notes: BTreeMap<String, u32> = BTreeMap::new();
    for note in &report.notes {
        *notes.entry(format!("{:?}", note.kind)).or_default() += note.count;
    }
    println!("notes ({} in all), by kind: {notes:?}", report.notes_total);
    println!(
        "timings: walk {} ms, parse {} ms, total {} ms",
        report.timings.walk_ms, report.timings.parse_ms, report.timings.total_ms
    );
    let mut per_resource: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for edit in &analysis.edits {
        let entry = per_resource.entry(&edit.resource).or_default();
        entry.0 += 1;
        entry.1 += edit.removals.len();
    }
    println!("edits by resource (files, removals): {per_resource:?}");
}

fn reason(reason: &LeaveReason) -> &'static str {
    match reason {
        LeaveReason::DefinitionsDiffer => "definitionsDiffer",
        LeaveReason::SeveralResources => "severalResources",
        LeaveReason::Escrowed => "escrowed",
        LeaveReason::Interior => "interior",
        LeaveReason::MixedSources => "mixedSources",
        LeaveReason::Contested { .. } => "contested",
        LeaveReason::ManifestDependent { .. } => "manifestDependent",
        LeaveReason::NotWritable { .. } => "notWritable",
        LeaveReason::Excluded { .. } => "excluded",
    }
}

fn print_examples(analysis: &Analysis) {
    println!("\nthe reported warnings:");
    for name in EXAMPLES {
        let hash = name_hash(name);
        let Some(group) = analysis.report.groups.iter().find(|group| group.hash == hash) else {
            println!("  {name} ({hash:08X}): no group");
            continue;
        };
        let copies: Vec<String> = group
            .copies
            .iter()
            .map(|copy| format!("{} {:?}", copy.path, copy.source))
            .collect();
        println!(
            "  {name} ({hash:08X}): {:?}, same file {}, {:?}\n    copies: {copies:#?}",
            group.difference, group.same_file, group.action
        );
    }
}

/// Every removal of the plan relies on a copy that holds on the folder as it is.
fn check_plan(index: &ServerIndex, settings: &ToolSettings, analysis: &Analysis) {
    for edit in &analysis.edits {
        for removal in &edit.removals {
            assert_eq!(
                archetypes::check_keeper(index, settings, &edit.path, removal),
                KeeperCheck::Holds,
                "{} entry {}",
                edit.path,
                removal.index
            );
        }
    }
}
