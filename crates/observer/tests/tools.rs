//! `observer tools` as it ships: the built binary on a synthetic server folder. A report writes
//! nothing and exits with 2 while duplicates are left; a fix takes Observer's lock, and an undo
//! puts back what it took out.
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
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
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
        assert!(words.contains("differences: keepLatest"), "{words}");
        assert!(words.contains("escrowed: skip"), "{words}");
    }
}
