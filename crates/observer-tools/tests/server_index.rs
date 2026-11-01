//! The index of a server folder on synthetic servers: which resources start and in which order,
//! what FXServer's walk finds, the order stream files are listed in, which data files the client
//! loads.

// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use common::{MANIFEST, Server, weapons};
use observer_domain::tools::ToolNoteKind;
use observer_tools::resources::{Error, ServerIndex, Starts, Status};

fn notes(index: &ServerIndex, kind: ToolNoteKind) -> Vec<(Option<String>, Option<String>, u32)> {
    index
        .notes()
        .0
        .iter()
        .filter(|note| note.kind == kind)
        .map(|note| (note.resource.clone(), note.detail.clone(), note.count))
        .collect()
}

fn resource<'a>(index: &'a ServerIndex, name: &str) -> &'a observer_tools::resources::Resource {
    index
        .resources()
        .iter()
        .find(|resource| resource.name == name && resource.status != Status::Duplicate)
        .unwrap_or_else(|| panic!("no resource {name}"))
}

#[test]
fn dependencies_start_before_the_resource_that_names_them() {
    let server = Server::new();
    server.resource("app", "dependency 'lib'\ndependencies { 'core', 'ui' }\n");
    server.resource("lib", "");
    server.resource("core", "dependency 'base'\n");
    server.resource("base", "");
    server.resource("ui", "");
    server.cfg("ensure app\nensure lib\nensure base\n");

    assert_eq!(server.started(), ["lib", "base", "core", "ui", "app"]);
}

#[test]
fn a_provided_name_resolves_to_the_resource_that_provides_it() {
    let server = Server::new();
    server.resource("shop", "dependency 'mysql-async'\n");
    server.resource("oxmysql", "provide 'mysql-async'\n");
    server.cfg("ensure shop\n");

    assert_eq!(server.started(), ["oxmysql", "shop"]);
}

#[test]
fn a_missing_dependency_keeps_the_resource_and_its_dependants_from_starting() {
    let server = Server::new();
    server.resource("phone", "dependency 'screenshot-basic'\n");
    server.resource("apps", "dependency 'phone'\n");
    server.resource("radio", "");
    server.cfg("ensure apps\nensure radio\n");

    let index = server.index();

    assert_eq!(server.started(), ["radio"]);
    assert_eq!(
        notes(&index, ToolNoteKind::MissingDependency),
        [(Some("phone".to_owned()), Some("screenshot-basic".to_owned()), 1)]
    );
    assert_eq!(
        notes(&index, ToolNoteKind::DependencyNotStarted),
        [(Some("apps".to_owned()), Some("phone".to_owned()), 1)]
    );
}

#[test]
fn a_constraint_on_the_server_is_taken_as_met_and_noted_once_for_every_resource() {
    let server = Server::new();
    server.resource("[maps]/harbour", "dependency '/assetpacks'\n");
    server.resource(
        "[maps]/hills",
        "dependencies { '/assetpacks', '/gameBuild:2545' }\n",
    );
    server.cfg("ensure [maps]\n");

    let index = server.index();

    assert_eq!(server.started(), ["harbour", "hills"]);
    assert_eq!(
        notes(&index, ToolNoteKind::ConstraintNotChecked),
        [
            (Some("harbour".to_owned()), Some("/assetpacks".to_owned()), 2),
            (Some("hills".to_owned()), Some("/gameBuild:2545".to_owned()), 1),
        ]
    );
}

#[test]
fn stop_stops_the_resource_and_its_dependants() {
    let server = Server::new();
    server.resource("garage", "dependency 'vehicles'\n");
    server.resource("vehicles", "");
    server.resource("weather", "");
    server.cfg("ensure garage\nensure weather\nstop vehicles\n");

    assert_eq!(server.started(), ["weather"]);
}

#[test]
fn restart_moves_the_resource_last_and_leaves_its_dependants_stopped() {
    let server = Server::new();
    server.resource("garage", "dependency 'vehicles'\n");
    server.resource("vehicles", "");
    server.resource("weather", "");
    server.cfg("ensure garage\nensure weather\nrestart vehicles\nrestart [cars]\nrestart sleeping\n");
    server.resource("sleeping", "");

    assert_eq!(server.started(), ["weather", "vehicles"]);
}

#[test]
fn a_category_starts_its_resources_in_byte_order_of_their_names_at_any_depth() {
    let server = Server::new();
    server.resource("[cars]/beta", "");
    server.resource("[cars]/Alpha", "");
    server.resource("[cars]/[sport]/gamma", "");
    server.resource("[boats]/delta", "");
    server.cfg("ensure [cars]\nstart [sport]\nensure [boats]\n");

    assert_eq!(server.started(), ["Alpha", "beta", "gamma", "delta"]);
}

#[test]
fn ensure_leaves_a_resource_already_started_in_its_place() {
    let server = Server::new();
    server.resource("one", "");
    server.resource("two", "");
    server.cfg("ensure one\nensure two\nensure one\nstart two\n");

    assert_eq!(server.started(), ["one", "two"]);
}

#[test]
fn a_manifest_fxserver_refuses_never_starts() {
    let server = Server::new();
    server.write("resources/no_game/fxmanifest.lua", "fx_version 'cerulean'\n");
    server.write(
        "resources/both/fxmanifest.lua",
        "fx_version 'cerulean'\ngames { 'gta5', 'common' }\n",
    );
    server.write(
        "resources/old_version/fxmanifest.lua",
        "fx_version 'zebra'\ngame 'gta5'\n",
    );
    server.write(
        "resources/broken/fxmanifest.lua",
        "fx_version 'cerulean'\ngame 'gta5'\nfiles { 'a.lua'\n",
    );
    server.write(
        "resources/legacy/__resource.lua",
        "resource_manifest_version '44febabe-d386-4d18-afbe-5e627f4af937'\n",
    );
    server.write(
        "resources/redm_too/fxmanifest.lua",
        "fx_version 'adamant'\ngames { 'gta5', 'rdr3' }\n",
    );
    server.cfg(
        "ensure no_game\nensure both\nensure old_version\nensure broken\nensure legacy\nensure redm_too\n",
    );

    let index = server.index();

    assert_eq!(server.started(), ["legacy", "redm_too"]);
    assert_eq!(resource(&index, "broken").status, Status::Refused);
    assert_eq!(resource(&index, "no_game").status, Status::Stopped);
    let refused: Vec<Option<String>> = notes(&index, ToolNoteKind::ManifestRefused)
        .into_iter()
        .map(|(resource, _, _)| resource)
        .collect();
    assert_eq!(
        refused,
        [
            Some("no_game".to_owned()),
            Some("both".to_owned()),
            Some("old_version".to_owned())
        ]
    );
    assert_eq!(
        notes(&index, ToolNoteKind::MissingResource),
        [(None, Some("broken".to_owned()), 1)],
        "FXServer does not load a manifest it cannot run"
    );
}

#[test]
fn a_server_only_resource_starts_and_reaches_no_client() {
    let server = Server::new();
    server.resource("db", "server_only 'yes'\n");
    server.resource("hud", "");
    server.cfg("ensure db\nensure hud\n");

    let index = server.index();

    assert_eq!(server.started(), ["db", "hud"]);
    let client: Vec<&str> = index
        .client_order()
        .map(|at| index.resources()[at].name.as_str())
        .collect();
    assert_eq!(client, ["hud"]);
}

#[test]
fn without_a_cfg_every_resource_fxserver_accepts_starts_in_name_order() {
    let server = Server::new();
    server.resource("zeta", "dependency 'omega'\n");
    server.resource("omega", "");
    server.resource("[x]/alpha", "");
    server.write("resources/bare/readme.txt", "no manifest");

    let index = ServerIndex::build(server.root(), &Starts::AllResources, &AtomicBool::new(false)).unwrap();

    let started: Vec<&str> = index
        .started()
        .iter()
        .map(|&at| index.resources()[at].name.as_str())
        .collect();
    assert_eq!(started, ["alpha", "omega", "zeta"]);
    assert_eq!(resource(&index, "bare").status, Status::NoManifest);
    assert_eq!(notes(&index, ToolNoteKind::AllResources).len(), 1);
}

#[test]
fn the_walk_finds_resources_as_fxserver_does() {
    let server = Server::new();
    server.resource("radio", "");
    server.resource("[music]/radio", "");
    server.resource("[music]/.hidden", "");
    server.resource("txAdmin", "");
    server.resource("chat", "");
    server.resource("[music]/[jazz]/sax", "");
    server.cfg("ensure radio\nensure sax\nensure ghost\nensure [rock]\n");

    let index = server.index();

    let found: Vec<(&str, &str, Status)> = index
        .resources()
        .iter()
        .map(|resource| (resource.name.as_str(), resource.path.as_str(), resource.status))
        .collect();
    assert_eq!(
        found,
        [
            ("radio", "resources/radio", Status::Started),
            ("radio", "resources/[music]/radio", Status::Duplicate),
            ("sax", "resources/[music]/[jazz]/sax", Status::Started),
        ]
    );
    assert_eq!(resource(&index, "sax").categories, ["[music]", "[jazz]"]);
    assert_eq!(notes(&index, ToolNoteKind::DuplicateResourceName).len(), 1);
    let missing: Vec<Option<String>> = notes(&index, ToolNoteKind::MissingResource)
        .into_iter()
        .map(|(_, detail, _)| detail)
        .collect();
    assert_eq!(missing, [Some("ghost".to_owned()), Some("[rock]".to_owned())]);
}

/// The folder's entries in the order the system lists them, as FXServer reads them.
fn listed(folder: &Path) -> Vec<String> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn stream_files_come_in_the_system_order_files_before_folders_and_the_first_of_a_name_is_sent() {
    let server = Server::new();
    server.resource("props", "");
    server.cfg("ensure props\n");
    for path in [
        "zz/shared.ytyp",
        "aa/shared.ytyp",
        "aa/deeper/shared.ytyp",
        "top.ytyp",
        "_under/shared.ytyp",
        ".git/shared.ytyp",
        "mid/top.ytyp",
        "mid/top.ytyp.stream_raw",
    ] {
        server.write(&format!("resources/props/stream/{path}"), b"RSC7");
    }

    let index = server.index();

    let stream = server.root().join("resources/props/stream");
    let mut expected = Vec::new();
    let mut folders = vec![(stream.clone(), String::new())];
    // Depth first, each folder's files before its subfolders, as `IterateRecursively` walks.
    while let Some((folder, prefix)) = folders.pop() {
        let mut inner = Vec::new();
        for name in listed(&folder) {
            let path = folder.join(&name);
            if path.is_dir() {
                if !name.starts_with('.') {
                    inner.push((path, format!("{prefix}{name}/")));
                }
            } else {
                expected.push(format!("resources/props/stream/{prefix}{name}"));
            }
        }
        folders.extend(inner.into_iter().rev());
    }
    let props = resource(&index, "props");
    let paths: Vec<&str> = props.stream.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, expected);
    assert_eq!(
        paths[0], "resources/props/stream/top.ytyp",
        "a folder's files come first"
    );
    let sent: Vec<&str> = props
        .stream
        .iter()
        .filter(|file| file.sent)
        .map(|file| file.path.as_str())
        .collect();
    let first_shared = expected
        .iter()
        .find(|path| path.ends_with("/shared.ytyp"))
        .unwrap();
    assert_eq!(
        sent,
        [
            "resources/props/stream/top.ytyp",
            first_shared.as_str(),
            // `top.ytyp` in `mid/` loses to the first one; the raw file is never sent.
        ]
    );
}

#[test]
fn data_files_load_only_what_the_resource_ships() {
    let server = Server::new();
    server.resource(
        "shipped",
        "files { 'meta/*.meta' }\ndata_file 'WEAPON_METADATA_FILE' 'meta/*.meta'\n",
    );
    server.resource("unshipped", "data_file 'WEAPON_METADATA_FILE' 'meta/*.meta'\n");
    server.resource(
        "literal",
        "files { 'meta/a.meta' }\ndata_file 'weapon_metadata_file' 'meta/a.meta'\ndata_file 'WEAPON_METADATA_FILE' 'meta/missing.meta'\n",
    );
    for resource in ["shipped", "unshipped", "literal"] {
        server.write(
            &format!("resources/{resource}/meta/a.meta"),
            weapons(&[("w_a", "")]),
        );
        server.write(
            &format!("resources/{resource}/meta/b.meta"),
            weapons(&[("w_b", "")]),
        );
    }
    server.cfg("ensure shipped\nensure unshipped\nensure literal\n");

    let index = server.index();

    let loaded = |name: &str| -> Vec<String> {
        resource(&index, name)
            .model_info
            .iter()
            .map(|file| file.path.clone())
            .collect()
    };
    assert_eq!(
        loaded("shipped"),
        ["resources/shipped/meta/a.meta", "resources/shipped/meta/b.meta"]
    );
    assert_eq!(loaded("unshipped"), Vec::<String>::new());
    assert_eq!(loaded("literal"), ["resources/literal/meta/a.meta"]);
    assert_eq!(
        notes(&index, ToolNoteKind::DataFileMissing),
        [(
            Some("literal".to_owned()),
            Some("meta/missing.meta".to_owned()),
            1
        )]
    );
}

#[test]
fn a_data_file_of_a_resource_that_does_not_start_is_not_loaded() {
    let server = Server::new();
    server.resource(
        "idle",
        "files { 'a.meta' }\ndata_file 'WEAPON_METADATA_FILE' 'a.meta'\n",
    );
    server.write("resources/idle/a.meta", weapons(&[("w_a", "")]));
    server.cfg("");

    assert_eq!(resource(&server.index(), "idle").model_info, Vec::new());
}

#[test]
fn ityp_requests_keep_the_lower_cased_stem_of_each_path() {
    let server = Server::new();
    server.resource(
        "map",
        "files { 'stream/**.ytyp' }\ndata_file 'DLC_ITYP_REQUEST' 'stream/Sub/Harbour_Props.ytyp'\ndata_file 'DLC_ITYP_REQUEST' 'stream/**.ytyp'\n",
    );
    server.write("resources/map/stream/a/one.ytyp", b"RSC7");
    server.write("resources/map/stream/two.ytyp", b"RSC7");
    server.cfg("");

    let index = server.index();

    assert_eq!(
        resource(&index, "map").ytyp_requests,
        ["harbour_props", "one", "two"]
    );
}

#[test]
fn archives_in_stream_are_noted() {
    let server = Server::new();
    server.resource("island", "");
    server.write("resources/island/stream/island.rpf", b"RPF7");
    server.write("resources/island/stream/island_metadata.rpf", b"RPF7");
    server.cfg("ensure island\n");

    let index = server.index();

    assert_eq!(
        notes(&index, ToolNoteKind::RpfInStream),
        [(Some("island".to_owned()), None, 2)]
    );
}

#[test]
fn a_build_stops_when_cancelled() {
    let server = Server::new();
    server.resource("one", "");
    server.write("resources/one/stream/a.ytyp", b"RSC7");

    let built = ServerIndex::build(server.root(), &Starts::AllResources, &AtomicBool::new(true));

    assert!(matches!(built, Err(Error::Cancelled)), "{built:?}");
}

#[test]
fn the_manifest_header_alone_starts_a_resource() {
    let server = Server::new();
    server.write("resources/plain/fxmanifest.lua", MANIFEST);
    server.cfg("ensure plain\n");

    assert_eq!(server.started(), ["plain"]);
}
