//! `observer tools` as it ships: the built binary on a synthetic server folder. A report writes
//! nothing, needs no database it can read, and exits with 2 while duplicates are left; a fix takes
//! Observer's lock, and an undo puts back what it took out, in the folder it was taken out of; the
//! settings, excluded resources included, change from the command line; RedM is refused even with
//! every resource taken as started.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use observer_domain::launch::{LaunchSettings, OneSync, StoredLaunch};
use observer_store::{Database, launch};
use serde_json::Value;
use tempfile::TempDir;

const CAMP: &str = "resources/props/stream/camp.ytyp";

/// An Observer configuration and a server folder whose `props` resource holds a marshmallow twice
/// in `camp.ytyp`, with no database yet.
struct Host {
    dir: TempDir,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("observer.toml"), "data_dir = \"data\"\n").unwrap();
        let host = Self { dir };
        host.write(
            "resources/props/fxmanifest.lua",
            b"fx_version 'cerulean'\ngame 'gta5'\n",
        );
        host.write(
            CAMP,
            &observer_gamefiles::testing::ytyp("camp", &["marshmallow", "stick", "marshmallow"]),
        );
        host.write("server.cfg", b"ensure props\n");
        host
    }

    fn folder(&self) -> PathBuf {
        self.dir.path().join("server-data")
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let full = self.folder().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, bytes).unwrap();
    }

    fn read(&self, path: &str) -> Vec<u8> {
        fs::read(self.folder().join(path)).unwrap()
    }

    fn observer(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_observer"))
            .arg("--config")
            .arg(self.dir.path().join("observer.toml"))
            .args(args)
            .output()
            .unwrap()
    }

    /// The database a first start of Observer makes, with the launch settings naming the folder.
    fn database(&self) {
        let data = self.dir.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let folder = self.folder().to_string_lossy().into_owned();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let db = Database::open(data.join("observer.sqlite3"), data.join("backups"))
                .await
                .unwrap();
            let stored = StoredLaunch {
                settings: LaunchSettings {
                    data_path: folder,
                    cfg_file: "server.cfg".to_owned(),
                    onesync: OneSync::On,
                    auto_start: false,
                    txadmin_events: false,
                    extra_arguments: Vec::new(),
                    hang_detection: true,
                    hang_timeout_seconds: 360,
                },
                version: 1,
                legacy_build_path: None,
            };
            db.write(move |tx| launch::save(tx, "default", &stored, jiff::Timestamp::now()))
                .await
                .unwrap();
            db.close().await.unwrap();
        });
    }

    /// Holds Observer's lock, as a running Observer does.
    fn lock(&self) -> File {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.path().join("data/observer.lock"))
            .unwrap();
        file.try_lock().unwrap();
        file
    }

    /// A second server folder beside the first, with the same `props` resource.
    fn copy(&self) -> PathBuf {
        let copy = self.dir.path().join("copy");
        for path in ["resources/props/fxmanifest.lua", CAMP, "server.cfg"] {
            let to = copy.join(path);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(self.folder().join(path), to).unwrap();
        }
        copy
    }

    /// Sets the schema the database says it has, as an Observer of another version left it.
    fn schema(&self, version: u32) {
        let path = self.dir.path().join("data/observer.sqlite3");
        let mut bytes = fs::read(&path).unwrap();
        // SQLite keeps the user version, which the migrations count in, at byte 60 of its header.
        bytes[60..64].copy_from_slice(&version.to_be_bytes());
        fs::write(&path, bytes).unwrap();
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The run an output's undo line names.
fn undo_run(output: &Output) -> String {
    let words = text(output);
    let line = words
        .lines()
        .find(|line| line.starts_with("to put back what run "))
        .unwrap_or_else(|| panic!("{words}"));
    line["to put back what run ".len()..]
        .split(' ')
        .next()
        .unwrap()
        .to_owned()
}

fn archetypes(path: &Path) -> usize {
    observer_gamefiles::ytyp::read(&fs::read(path).unwrap())
        .unwrap()
        .archetypes
        .len()
}

#[test]
fn a_report_writes_nothing_and_exits_with_2_while_duplicates_are_left() {
    let host = Host::new();
    let camp = host.read(CAMP);
    let folder = host.folder();
    let folder = folder.to_str().unwrap();

    let json = host.observer(&["tools", "run", "duplicate-archetypes", "--data", folder, "--json"]);
    let words = host.observer(&["tools", "run", "duplicate-archetypes", "--data", folder]);

    assert_eq!(json.status.code(), Some(2), "{}", text(&json));
    let report: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["counts"]["groups"], 1);
    assert_eq!(report["counts"]["groupsFixed"], 1);
    assert_eq!(report["groups"][0]["action"]["kind"], "fix");
    assert_eq!(words.status.code(), Some(2), "{}", text(&words));
    let words = text(&words);
    assert!(
        words.contains("1 registered more than once: 1 fixable in 1 file, 0 left"),
        "{words}"
    );
    assert!(words.contains(&format!("    keep    {CAMP} #2")), "{words}");
    assert!(words.contains(&format!("    remove  {CAMP} #0")), "{words}");
    assert!(words.contains("nothing was written"), "{words}");
    assert_eq!(host.read(CAMP), camp);
    assert!(!host.dir.path().join("data/tools").exists());
}

#[test]
fn a_fix_takes_observer_s_lock_and_an_undo_puts_back_what_it_took_out() {
    let host = Host::new();
    let camp = host.read(CAMP);
    host.database();

    let held = host.lock();
    let refused = host.observer(&["tools", "run", "duplicate-archetypes", "--fix"]);
    drop(held);
    let fixed = host.observer(&["tools", "run", "duplicate-archetypes", "--fix"]);
    let after_fix = archetypes(&host.folder().join(CAMP));
    let again = host.observer(&["tools", "run", "duplicate-archetypes"]);
    let undone = host.observer(&["tools", "undo", "duplicate-archetypes", "--all"]);

    assert_eq!(refused.status.code(), Some(1));
    assert!(
        text(&refused).contains("Observer is running"),
        "{}",
        text(&refused)
    );
    assert_eq!(fixed.status.code(), Some(0), "{}", text(&fixed));
    let words = text(&fixed);
    assert!(words.contains("1 fixed in 1 file, 0 left"), "{words}");
    assert!(words.contains("fixed (unnamed) "), "{words}");
    assert!(
        words.contains(&format!("{CAMP}: written without the entries at #0")),
        "{words}"
    );
    assert!(
        words.contains("observer tools undo duplicate-archetypes --run "),
        "{words}"
    );
    assert_eq!(after_fix, 2);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again));
    assert!(
        text(&again).contains("1 file holds an earlier fix"),
        "{}",
        text(&again)
    );
    assert_eq!(undone.status.code(), Some(0), "{}", text(&undone));
    assert!(
        text(&undone).contains("1 file back; 0 files still open"),
        "{}",
        text(&undone)
    );
    assert!(
        text(&undone).contains("undone: its original is back"),
        "{}",
        text(&undone)
    );
    assert_eq!(host.read(CAMP), camp);
}

#[test]
fn the_settings_are_shown_and_changed_from_the_command_line() {
    let host = Host::new();
    host.database();

    let changed = host.observer(&[
        "tools",
        "config",
        "duplicate-archetypes",
        "--at-start",
        "report",
        "--differences",
        "keep-latest",
    ]);
    let shown = host.observer(&["tools", "config", "duplicate-archetypes"]);

    assert_eq!(changed.status.code(), Some(0), "{}", text(&changed));
    for output in [&changed, &shown] {
        let words = text(output);
        assert!(words.contains("at start: report"), "{words}");
        assert!(words.contains("differences: keep-latest"), "{words}");
        assert!(words.contains("escrowed: skip"), "{words}");
    }
}

#[test]
fn an_excluded_resource_is_left_by_the_report_until_it_is_included_again() {
    let host = Host::new();
    host.database();

    let excluded = host.observer(&[
        "tools",
        "config",
        "duplicate-archetypes",
        "--exclude",
        "props",
        "--exclude",
        "maps",
    ]);
    let report = host.observer(&["tools", "run", "duplicate-archetypes"]);
    let refused = host.observer(&["tools", "config", "duplicate-archetypes", "--exclude", "a/b"]);
    let included = host.observer(&["tools", "config", "duplicate-archetypes", "--include", "props"]);

    assert_eq!(excluded.status.code(), Some(0), "{}", text(&excluded));
    assert!(
        text(&excluded).contains("excluded: props, maps"),
        "{}",
        text(&excluded)
    );
    assert_eq!(report.status.code(), Some(2), "{}", text(&report));
    let words = text(&report);
    assert!(
        words.contains("1 registered more than once: 0 fixable in 0 files, 1 left"),
        "{words}"
    );
    assert!(words.contains("props is excluded"), "{words}");
    assert_eq!(refused.status.code(), Some(1), "{}", text(&refused));
    assert!(text(&refused).contains("refused"), "{}", text(&refused));
    assert_eq!(included.status.code(), Some(0), "{}", text(&included));
    assert!(
        text(&included).lines().any(|line| line == "excluded: maps"),
        "{}",
        text(&included)
    );
}

#[test]
fn a_fix_of_another_folder_is_undone_there_with_the_command_it_prints() {
    let host = Host::new();
    host.database();
    let camp = host.read(CAMP);
    let copy = host.copy();
    let data = copy.to_str().unwrap();

    let fixed = host.observer(&["tools", "run", "duplicate-archetypes", "--fix", "--data", data]);
    let run = undo_run(&fixed);
    let by_run = host.observer(&["tools", "undo", "duplicate-archetypes", "--run", &run]);
    let after_run = fs::read(copy.join(CAMP)).unwrap();
    let again = host.observer(&["tools", "run", "duplicate-archetypes", "--fix", "--data", data]);
    let all = host.observer(&["tools", "undo", "duplicate-archetypes", "--all", "--data", data]);

    assert_eq!(fixed.status.code(), Some(0), "{}", text(&fixed));
    assert!(
        text(&fixed).contains(&format!(
            "observer tools undo duplicate-archetypes --run {run} --data {data}"
        )),
        "{}",
        text(&fixed)
    );
    assert_eq!(by_run.status.code(), Some(0), "{}", text(&by_run));
    assert!(text(&by_run).contains("1 file back"), "{}", text(&by_run));
    assert_eq!(after_run, camp);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again));
    assert_eq!(all.status.code(), Some(0), "{}", text(&all));
    assert!(text(&all).contains("1 file back"), "{}", text(&all));
    assert_eq!(fs::read(copy.join(CAMP)).unwrap(), camp);
    assert_eq!(host.read(CAMP), camp, "the launch settings' folder is left alone");
}

#[test]
fn a_fix_refuses_escrowed_resources_for_one_run_while_the_saved_setting_skips_them() {
    let host = Host::new();
    host.database();
    let camp = host.read(CAMP);

    let refused = host.observer(&[
        "tools",
        "run",
        "duplicate-archetypes",
        "--fix",
        "--escrowed",
        "allow",
    ]);

    assert_eq!(refused.status.code(), Some(1), "{}", text(&refused));
    assert!(
        text(&refused).contains("observer tools config duplicate-archetypes --escrowed allow"),
        "{}",
        text(&refused)
    );
    assert_eq!(host.read(CAMP), camp);
}

#[test]
fn a_report_reads_past_a_database_this_build_cannot_read() {
    let host = Host::new();
    host.database();
    host.schema(1);
    let folder = host.folder();

    let report = host.observer(&[
        "tools",
        "run",
        "duplicate-archetypes",
        "--data",
        folder.to_str().unwrap(),
    ]);

    assert_eq!(report.status.code(), Some(2), "{}", text(&report));
    let words = text(&report);
    assert!(words.contains("1 registered more than once"), "{words}");
    assert!(words.contains("the default settings"), "{words}");
}

#[test]
fn every_resource_taken_as_started_is_still_refused_on_redm() {
    let host = Host::new();
    host.write("server.cfg", b"set gamename rdr3\nensure props\n");
    let folder = host.folder();
    let folder = folder.to_str().unwrap();
    let bare = Host::new();
    fs::remove_file(bare.folder().join("server.cfg")).unwrap();
    let bare_folder = bare.folder();

    let refused = host.observer(&[
        "tools",
        "run",
        "duplicate-archetypes",
        "--data",
        folder,
        "--all-resources",
    ]);
    let assumed = bare.observer(&[
        "tools",
        "run",
        "duplicate-archetypes",
        "--data",
        bare_folder.to_str().unwrap(),
        "--all-resources",
    ]);

    assert_eq!(refused.status.code(), Some(1), "{}", text(&refused));
    assert!(text(&refused).contains("RedM"), "{}", text(&refused));
    assert_eq!(assumed.status.code(), Some(2), "{}", text(&assumed));
    assert!(
        text(&assumed).contains("taken to run GTA V"),
        "{}",
        text(&assumed)
    );
}
