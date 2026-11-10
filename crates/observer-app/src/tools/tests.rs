//! The server tools end to end, over a real database and a synthetic server folder in a temporary
//! directory: the panel's scan, fix and undo with their gates, the settings, the work before each
//! start for every mode and cause, and the settling at Observer's start.

use std::collections::BTreeSet;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use observer_domain::launch::{LaunchSettings, OneSync, StoredLaunch};
use observer_domain::permissions::Permissions;
use observer_domain::server::{LaunchReadiness, ServerState, ServerStatus};
use observer_domain::tools::{
    Differences, ToolChangeState, ToolCloseReason, ToolFileFormat, ToolKeeper, ToolMode, ToolRemoval,
    ToolRunMode, ToolRunOutcome, ToolSettings, ToolSettingsChange, ToolTrigger, ToolUndoRequest,
    ToolUndoScope,
};
use observer_fxserver::launch::ServerLaunch;
use observer_fxserver::layout::{Flavour, Layout};
use observer_fxserver::pre_launch::{PreLaunch, PreLaunchRequest};
use observer_fxserver::supervisor::Cause;
use observer_gamefiles::hash::name_hash;
use observer_store::tools::{self as stored, BeginWrite, NewRun};
use observer_store::{Database, StoreError, launch};
use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::watch;

use super::*;
use crate::{Client, FeaturesSetup};

const CAMP: &str = "resources/props/stream/camp.ytyp";
const GIFT: &str = "resources/props/stream/gift.ytyp";
const KEEPER: &str = "resources/props/stream/gift2.ytyp";
const TENT: &str = "resources/props/stream/tent.ytyp";

/// Observer's database, a server folder whose `props` resource holds a marshmallow twice in
/// `camp.ytyp` and a gift in both `gift.ytyp` and `gift2.ytyp`, the marshmallow's model beside
/// them, its launch settings saved, and the tools over them with FXServer stopped.
struct Lab {
    dir: TempDir,
    db: Database,
    status: watch::Sender<ServerStatus>,
    tools: Tools,
    hook: ToolsHook,
}

impl Lab {
    async fn new() -> Self {
        Self::with_cfg("ensure props\n").await
    }

    async fn with_cfg(cfg: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2026-11-01T09:00:00Z".parse().unwrap());
        let folder = dir.path().join("server-data");
        props(&folder, cfg);
        let stored = StoredLaunch {
            settings: LaunchSettings {
                data_path: folder.to_string_lossy().into_owned(),
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
        let now = clock.now();
        db.write(move |tx| launch::save(tx, Servers::DEFAULT, &stored, now))
            .await
            .unwrap();
        let (status, watched) = watch::channel(ServerStatus {
            state: ServerState::Stopped,
            cause: None,
            since: now,
            started_at: None,
            generation: 0,
            problems: Vec::new(),
            build: None,
            players: None,
            restart_at: None,
            launch: LaunchReadiness::Ready,
            name: None,
            port: None,
            start: None,
        });
        let tools = Tools::new(ToolsSetup {
            db: db.clone(),
            clock,
            data_dir: dir.path().join("data"),
            artifacts: dir.path().join("data/artifacts"),
            status: Some(watched),
            gate: CommitGate::default(),
            own_paths: OwnPaths::of(vec![dir.path().join("data")]),
        });
        let hook = ToolsHook::default();
        hook.bind(tools.clone());
        Self {
            dir,
            db,
            status,
            tools,
            hook,
        }
    }

    fn folder(&self) -> PathBuf {
        self.dir.path().join("server-data")
    }

    fn read(&self, path: &str) -> Vec<u8> {
        fs::read(self.folder().join(path)).unwrap()
    }

    /// The archetype hashes of a `.ytyp` of the folder, in list order.
    fn archetypes(&self, path: &str) -> Vec<u32> {
        observer_gamefiles::ytyp::read(&self.read(path))
            .unwrap()
            .archetypes
            .iter()
            .map(|archetype| archetype.name)
            .collect()
    }

    fn set_state(&self, state: ServerState) {
        self.status.send_modify(|status| status.state = state);
    }

    async fn configure(&self, at_start: ToolMode) {
        let (settings, version) = self.tools.settings().await.unwrap();
        let change = ToolSettingsChange {
            settings: ToolSettings { at_start, ..settings },
            version,
        };
        self.tools.change_settings(&owner(), change).await.unwrap();
    }

    /// Scans, then fixes the plan the scan showed.
    async fn fix(&self) -> ToolRun {
        let scanned = self.tools.scan(&owner()).await.unwrap();
        self.tools
            .fix(&owner(), scanned.plan.as_deref().unwrap())
            .await
            .unwrap()
    }

    /// Has the owner turn the tools off, and the work before a start follow their switch.
    async fn turn_off(&mut self) {
        let features = Features::load(FeaturesSetup {
            db: self.db.clone(),
            clock: Clock::manual("2026-11-01T10:00:00Z".parse().unwrap()),
            host_off: BTreeSet::new(),
            sftp: None,
            data_dir: self.dir.path().join("data"),
        })
        .await
        .unwrap();
        features.set(&owner(), Feature::Tools, false).await.unwrap();
        self.hook = ToolsHook::switched(features);
        self.hook.bind(self.tools.clone());
    }

    /// The work before a start, as the supervisor asks for it, with its lines.
    async fn start(&self, cause: Cause) -> Vec<String> {
        self.start_with(cause, Arc::new(AtomicBool::new(false))).await
    }

    async fn start_with(&self, cause: Cause, cancel: Arc<AtomicBool>) -> Vec<String> {
        let folder = self.folder();
        let scan = observer_tools::resources::scan_cfg(&folder, "server.cfg", None).unwrap();
        let build = self.dir.path().join("data/artifacts/35245");
        let request = PreLaunchRequest {
            launch: ServerLaunch {
                build: build.clone(),
                settings: LaunchSettings {
                    data_path: folder.to_string_lossy().into_owned(),
                    cfg_file: "server.cfg".to_owned(),
                    onesync: OneSync::On,
                    auto_start: false,
                    txadmin_events: false,
                    extra_arguments: Vec::new(),
                    hang_detection: true,
                    hang_timeout_seconds: 360,
                },
            },
            layout: Layout::new(build, Flavour::HOST),
            scan,
            cause,
            generation: 1,
            cancel,
        };
        self.hook.run(request).await.unwrap()
    }

    /// The audit entries of `action`, oldest first: their actor kind and details.
    async fn audited(&self, action: AuditAction) -> Vec<(String, Value)> {
        let name = action.as_str();
        self.db
            .read(move |conn| -> Result<_, StoreError> {
                let mut statement =
                    conn.prepare("SELECT actor_kind, details FROM audit_log WHERE action = ?1 ORDER BY id")?;
                let rows = statement.query_map([name], |row| {
                    let details: String = row.get(1)?;
                    Ok((row.get(0)?, serde_json::from_str(&details).unwrap()))
                })?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
            .unwrap()
    }

    async fn runs(&self) -> Vec<ToolRun> {
        self.db
            .read(|conn| stored::runs(conn, Servers::DEFAULT, TOOL, 100))
            .await
            .unwrap()
    }
}

/// The `props` resource in `folder`, and the cfg `cfg`.
fn props(folder: &Path, cfg: &str) {
    write(
        folder,
        "resources/props/fxmanifest.lua",
        "fx_version 'cerulean'\ngame 'gta5'\n",
    );
    ytyp(folder, CAMP, &["marshmallow", "stick", "marshmallow"]);
    ytyp(folder, GIFT, &["gift"]);
    ytyp(folder, KEEPER, &["gift", "bow"]);
    // A model streamed under its name spells it in the report.
    write(folder, "resources/props/stream/marshmallow.ydr", b"RSC7");
    write(folder, "server.cfg", cfg);
}

fn write(folder: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let full = folder.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, bytes).unwrap();
}

/// A `.ytyp` defining a prop for each name, every prop of a name the same.
fn ytyp(folder: &Path, path: &str, names: &[&str]) {
    let stem = Path::new(path)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    write(folder, path, observer_gamefiles::testing::ytyp(&stem, names));
}

fn hashes(names: &[&str]) -> Vec<u32> {
    names.iter().map(|name| name_hash(name)).collect()
}

/// The owner, signed in from the host.
fn owner() -> Caller {
    Caller {
        session_id: 1,
        user_id: 1,
        username: "ada".to_owned(),
        is_owner: true,
        permissions: Permissions::NONE,
        must_change_password: false,
        reauthenticated_at: None,
        client: Client {
            ip: IpAddr::from([127, 0, 0, 1]),
            user_agent: None,
        },
    }
}

fn code(error: &Error) -> Option<ErrorCode> {
    error.code()
}

#[tokio::test]
async fn a_scan_records_its_report_and_plan_and_writes_nothing_even_while_fxserver_runs() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    lab.set_state(ServerState::Online);

    let run = lab.tools.scan(&owner()).await.unwrap();

    assert_eq!(run.mode, ToolRunMode::Report);
    assert_eq!(run.trigger, ToolTrigger::Panel);
    assert_eq!(run.actor.as_deref(), Some("ada"));
    assert_eq!(run.outcome, Some(ToolRunOutcome::Done));
    let summary = run.summary.unwrap();
    assert_eq!((summary.groups, summary.fixable, summary.left), (2, 2, 0));
    assert_eq!(summary.files_changed, 0);
    assert_eq!(run.plan.as_ref().map(String::len), Some(64));
    assert_eq!(run.report.unwrap().counts.files_changed, 2);
    assert_eq!(lab.read(CAMP), camp);
    assert_eq!(lab.audited(AuditAction::ToolFilesChanged).await, Vec::new());
}

#[tokio::test]
async fn a_confirmed_fix_writes_the_plan_keeps_the_originals_and_is_audited() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);

    let run = lab.fix().await;

    assert_eq!(run.mode, ToolRunMode::Fix);
    let summary = run.summary.unwrap();
    assert_eq!(
        (
            summary.fixed,
            summary.files_changed,
            summary.removals,
            summary.open_changes
        ),
        (2, 2, 2, 2)
    );
    assert_eq!(lab.archetypes(CAMP), hashes(&["stick", "marshmallow"]));
    assert_eq!(lab.archetypes(GIFT), Vec::<u32>::new());
    let audited = lab.audited(AuditAction::ToolFilesChanged).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].0, "user");
    assert_eq!(audited[0].1["count"], 2);
    assert_eq!(audited[0].1["trigger"], "panel");
    assert_eq!(audited[0].1["run"], run.id);
    let view = lab.tools.view().await.unwrap();
    let tool = &view.tools[0];
    assert_eq!(tool.open.len(), 2);
    assert!(
        tool.open
            .iter()
            .all(|change| change.state == ToolChangeState::Applied)
    );
    let camp_change = tool.open.iter().find(|change| change.path == CAMP).unwrap();
    assert_eq!(camp_change.format, ToolFileFormat::Ytyp);
    assert_eq!(camp_change.removals[0].name.as_deref(), Some("marshmallow"));
    assert_eq!(camp_change.folder, lab.folder().to_string_lossy());
    assert!(tool.backup_bytes >= camp.len() as u64);
    assert_eq!(tool.last_run.as_ref().map(|last| last.id), Some(run.id));
    assert!(tool.last_run.as_ref().unwrap().report.is_some());
    let backups = lab.dir.path().join("data/tools/backups");
    assert_eq!(
        fs::read(backups.join(observer_domain::tools::sha256_hex(&sha256(&camp)))).unwrap(),
        camp
    );
}

#[tokio::test]
async fn fix_and_undo_are_refused_unless_fxserver_is_stopped() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    let plan = lab.tools.scan(&owner()).await.unwrap().plan.unwrap();
    let undo = ToolUndoRequest {
        scope: ToolUndoScope::All,
    };

    for state in [ServerState::Online, ServerState::Starting, ServerState::Crashed] {
        lab.set_state(state);
        let fixed = lab.tools.fix(&owner(), &plan).await.unwrap_err();
        let undone = lab.tools.undo(&owner(), undo.clone()).await.unwrap_err();
        assert_eq!(code(&fixed), Some(ErrorCode::ServerNotStopped), "{state:?}");
        assert_eq!(code(&undone), Some(ErrorCode::ServerNotStopped), "{state:?}");
    }
    lab.set_state(ServerState::CrashLoop);
    assert!(lab.tools.fix(&owner(), &plan).await.is_ok());

    assert_ne!(lab.read(CAMP), camp);
}

#[tokio::test]
async fn a_fix_of_a_plan_that_changed_since_it_was_shown_writes_nothing_and_hands_back_the_fresh_run() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    let shown = lab.tools.scan(&owner()).await.unwrap();
    ytyp(&lab.folder(), TENT, &["tent", "tent"]);

    let refused = lab
        .tools
        .fix(&owner(), shown.plan.as_deref().unwrap())
        .await
        .unwrap_err();

    let Error::Refused(refusal) = refused else {
        panic!("not a refusal: {refused}");
    };
    assert_eq!(refusal.code, ErrorCode::PlanChanged);
    let fresh: ToolRun = serde_json::from_value(refusal.params["run"].clone()).unwrap();
    assert_ne!(fresh.plan, shown.plan);
    assert_eq!(fresh.summary.as_ref().unwrap().fixable, 3);
    assert_eq!(fresh.summary.as_ref().unwrap().files_changed, 0);
    assert_eq!(fresh.report.unwrap().counts.files_changed, 3);
    assert_eq!(lab.read(CAMP), camp);
    assert_eq!(lab.audited(AuditAction::ToolFilesChanged).await, Vec::new());

    let fixed = lab
        .tools
        .fix(&owner(), fresh.plan.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(fixed.summary.unwrap().files_changed, 3);
    assert_eq!(
        code(&lab.tools.fix(&owner(), "not a digest").await.unwrap_err()),
        Some(ErrorCode::InvalidRequest)
    );
}

#[tokio::test]
async fn an_undo_gives_every_file_its_original_back_and_is_audited() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    let gift = lab.read(GIFT);
    let fixed = lab.fix().await;

    let run = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::Run(fixed.id),
            },
        )
        .await
        .unwrap();

    assert_eq!(run.mode, ToolRunMode::Undo);
    let summary = run.summary.unwrap();
    assert_eq!((summary.files_restored, summary.open_changes), (2, 0));
    assert_eq!(lab.read(CAMP), camp);
    assert_eq!(lab.read(GIFT), gift);
    let audited = lab.audited(AuditAction::ToolFilesRestored).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].1["reason"], "undo");
    assert_eq!(audited[0].1["count"], 2);
    let view = lab.tools.view().await.unwrap();
    assert_eq!(view.tools[0].open, Vec::new());
    assert!(
        view.tools[0]
            .closed
            .iter()
            .all(|change| change.close_reason == Some(ToolCloseReason::Undone))
    );
}

#[tokio::test]
async fn an_undo_by_change_takes_only_changes_open_in_the_folder() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    lab.fix().await;
    let view = lab.tools.view().await.unwrap();
    let camp_change = view.tools[0]
        .open
        .iter()
        .find(|change| change.path == CAMP)
        .unwrap()
        .id;

    let unknown = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::Changes(vec![camp_change, 9999]),
            },
        )
        .await
        .unwrap_err();
    let undone = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::Changes(vec![camp_change]),
            },
        )
        .await
        .unwrap();

    assert_eq!(code(&unknown), Some(ErrorCode::NotFound));
    assert_eq!(undone.summary.unwrap().open_changes, 1);
    assert_eq!(lab.read(CAMP), camp);
    assert_eq!(lab.archetypes(GIFT), Vec::<u32>::new());
}

/// A copy of the server folder beside it, fixed from the command line, as `--fix --data` does.
async fn fixed_copy(lab: &Lab) -> (PathBuf, ToolRun) {
    let copy = lab.dir.path().join("copy");
    props(&copy, "ensure props\n");
    let target = Target::Cfg {
        folder: copy.clone(),
        cfg_file: "server.cfg".to_owned(),
        system_resources: None,
    };
    let fixed = lab
        .tools
        .fix_from_cli(target, ToolSettings::default())
        .await
        .unwrap()
        .run;
    (copy, fixed)
}

#[tokio::test]
async fn a_change_of_another_folder_is_undone_there_by_file_or_by_run_and_left_by_an_undo_of_everything() {
    let lab = Lab::new().await;
    let (copy, fixed) = fixed_copy(&lab).await;
    let camp = lab.read(CAMP);
    let gift = lab.read(GIFT);
    lab.fix().await;
    let view = lab.tools.view().await.unwrap();
    let tool = &view.tools[0];
    let theirs = |path: &str| {
        tool.open
            .iter()
            .find(|change| change.path == path && change.folder == copy.to_string_lossy())
            .unwrap()
            .id
    };
    let ours = tool
        .open
        .iter()
        .find(|change| change.folder != copy.to_string_lossy())
        .unwrap()
        .id;
    let mut elsewhere = vec![theirs(CAMP), theirs(GIFT)];
    elsewhere.sort_unstable();

    let mixed = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::Changes(vec![theirs(CAMP), ours]),
            },
        )
        .await
        .unwrap_err();
    let all = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::All,
            },
        )
        .await
        .unwrap();
    let by_file = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::Changes(vec![theirs(CAMP)]),
            },
        )
        .await
        .unwrap();
    let by_run = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::Run(fixed.id),
            },
        )
        .await
        .unwrap();

    let mut listed = tool.elsewhere.clone();
    listed.sort_unstable();
    assert_eq!(listed, elsewhere);
    assert_eq!(tool.unreachable, Vec::<i64>::new());
    assert_eq!(all.summary.unwrap().files_restored, 2);
    assert_eq!((lab.read(CAMP), lab.read(GIFT)), (camp.clone(), gift.clone()));
    assert_eq!(
        code(&mixed),
        Some(ErrorCode::InvalidRequest),
        "the changes of two folders are undone apart"
    );
    assert_eq!(by_file.summary.unwrap().files_restored, 1);
    assert_eq!(fs::read(copy.join(CAMP)).unwrap(), camp);
    assert_eq!(by_run.summary.unwrap().files_restored, 1);
    assert_eq!(fs::read(copy.join(GIFT)).unwrap(), gift);
    assert_eq!(lab.tools.view().await.unwrap().tools[0].open, Vec::new());
}

#[tokio::test]
async fn a_change_whose_folder_is_no_longer_there_is_listed_out_of_reach_and_no_undo_writes_elsewhere() {
    let lab = Lab::new().await;
    let (copy, fixed) = fixed_copy(&lab).await;
    let camp = lab.read(CAMP);
    fs::rename(&copy, lab.dir.path().join("moved")).unwrap();
    // Another folder at the same path is not the one the change was made in.
    props(&copy, "ensure props\n");

    let view = lab.tools.view().await.unwrap();
    let by_run = lab
        .tools
        .undo(
            &owner(),
            ToolUndoRequest {
                scope: ToolUndoScope::Run(fixed.id),
            },
        )
        .await
        .unwrap_err();

    let tool = &view.tools[0];
    assert_eq!(tool.open.len(), 2);
    assert_eq!(tool.elsewhere.len(), 2);
    assert_eq!(tool.unreachable.len(), 2);
    assert_eq!(code(&by_run), Some(ErrorCode::NotFound));
    assert_eq!(fs::read(copy.join(CAMP)).unwrap(), camp);
    assert_eq!(lab.read(CAMP), camp);
}

#[tokio::test]
async fn settings_are_saved_over_the_version_read_and_audited_with_each_change() {
    let lab = Lab::new().await;
    let keep_latest = ToolSettings {
        differences: Differences::KeepLatest,
        excluded: vec!["maps".to_owned()],
        ..ToolSettings::default()
    };

    let first = lab
        .tools
        .change_settings(
            &owner(),
            ToolSettingsChange {
                settings: keep_latest.clone(),
                version: None,
            },
        )
        .await
        .unwrap();
    let stale = lab
        .tools
        .change_settings(
            &owner(),
            ToolSettingsChange {
                settings: ToolSettings::default(),
                version: None,
            },
        )
        .await
        .unwrap_err();
    let same = lab
        .tools
        .change_settings(
            &owner(),
            ToolSettingsChange {
                settings: keep_latest.clone(),
                version: Some(1),
            },
        )
        .await
        .unwrap();
    let refused = lab
        .tools
        .change_settings(
            &owner(),
            ToolSettingsChange {
                settings: ToolSettings {
                    excluded: vec![String::new()],
                    ..ToolSettings::default()
                },
                version: Some(1),
            },
        )
        .await
        .unwrap_err();

    assert_eq!(first.tools[0].settings, keep_latest);
    assert_eq!(first.tools[0].version, Some(1));
    assert_eq!(code(&stale), Some(ErrorCode::Conflict));
    assert_eq!(same.tools[0].version, Some(1));
    assert_eq!(code(&refused), Some(ErrorCode::InvalidRequest));
    let audited = lab.audited(AuditAction::ToolSettingsChanged).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].1["differences"]["before"], "identical");
    assert_eq!(audited[0].1["differences"]["after"], "keepLatest");
    assert_eq!(audited[0].1["excluded"]["after"][0], "maps");
    assert!(audited[0].1.get("atStart").is_none());
}

#[tokio::test]
async fn the_work_before_a_start_heals_a_removal_whose_copy_kept_is_gone_whatever_the_mode() {
    let lab = Lab::new().await;
    let gift = lab.read(GIFT);
    lab.fix().await;
    fs::remove_file(lab.folder().join(KEEPER)).unwrap();
    ytyp(&lab.folder(), TENT, &["tent", "tent"]);

    let lines = lab.start(Cause::Requested { by: "ada".to_owned() }).await;

    assert_eq!(lab.read(GIFT), gift);
    assert_eq!(lab.archetypes(CAMP), hashes(&["stick", "marshmallow"]));
    assert_eq!(
        lab.archetypes(TENT),
        hashes(&["tent", "tent"]),
        "mode off fixes nothing new"
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("duplicate archetypes: put back 1 archetype"),
        "{lines:?}"
    );
    assert!(lines[1].contains(GIFT), "{lines:?}");
    let audited = lab.audited(AuditAction::ToolFilesRestored).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].0, "system");
    assert_eq!(audited[0].1["reason"], "heal");
    assert_eq!(audited[0].1["trigger"], "start");
    let runs = lab.runs().await;
    assert_eq!(runs[0].mode, ToolRunMode::Heal);
    assert_eq!(runs[0].trigger, ToolTrigger::Start);
    assert_eq!(runs[0].actor, None);
    let view = lab.tools.view().await.unwrap();
    assert_eq!(view.tools[0].open.len(), 1);
    assert_eq!(
        view.tools[0].closed[0].close_reason,
        Some(ToolCloseReason::Healed)
    );
}

#[tokio::test]
async fn a_crash_recovery_heals_but_never_fixes_even_in_mode_fix() {
    let lab = Lab::new().await;
    let gift = lab.read(GIFT);
    lab.fix().await;
    lab.configure(ToolMode::Fix).await;
    fs::remove_file(lab.folder().join(KEEPER)).unwrap();
    ytyp(&lab.folder(), TENT, &["tent", "tent"]);

    let lines = lab.start(Cause::Recovery).await;

    assert_eq!(lab.read(GIFT), gift);
    assert_eq!(lab.archetypes(TENT), hashes(&["tent", "tent"]));
    assert!(
        lines[0].starts_with("duplicate archetypes: put back"),
        "{lines:?}"
    );
    assert_eq!(lab.runs().await[0].mode, ToolRunMode::Heal);

    let lines = lab.start(Cause::Restart { by: "ada".to_owned() }).await;

    assert_eq!(lab.archetypes(TENT), hashes(&["tent"]));
    assert!(
        lines[0].starts_with("duplicate archetypes: fixed 1 duplicate archetype"),
        "{lines:?}"
    );
    assert!(lines.iter().any(|line| line.contains(TENT)), "{lines:?}");
    let audited = lab.audited(AuditAction::ToolFilesChanged).await;
    assert_eq!(audited.last().unwrap().1["trigger"], "start");
    assert_eq!(audited.last().unwrap().0, "system");

    let restart = || Cause::Restart { by: "ada".to_owned() };
    let after = lab.start(restart()).await;
    let quiet = lab.start(restart()).await;
    assert_eq!(
        after,
        ["duplicate archetypes: nothing to fix; 0 duplicate archetypes left"]
    );
    assert_eq!(quiet, Vec::<String>::new());
}

#[tokio::test]
async fn nothing_runs_and_nothing_is_recorded_before_a_start_in_mode_off_without_open_changes() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);

    let lines = lab.start(Cause::Requested { by: "ada".to_owned() }).await;

    assert_eq!(lines, Vec::<String>::new());
    assert_eq!(lab.runs().await, Vec::new());
    assert_eq!(lab.read(CAMP), camp);
}

#[tokio::test]
async fn turned_off_the_tools_do_nothing_before_a_start_whatever_their_settings_say() {
    let mut lab = Lab::new().await;
    let camp = lab.read(CAMP);
    lab.configure(ToolMode::Fix).await;
    lab.turn_off().await;

    let lines = lab.start(Cause::Requested { by: "ada".to_owned() }).await;

    assert_eq!(lines, Vec::<String>::new());
    assert_eq!(lab.runs().await, Vec::new());
    assert_eq!(lab.read(CAMP), camp);
}

#[tokio::test]
async fn turned_off_the_tools_still_heal_a_fix_left_applied_and_fix_nothing_new() {
    let mut lab = Lab::new().await;
    let gift = lab.read(GIFT);
    lab.fix().await;
    lab.configure(ToolMode::Fix).await;
    lab.turn_off().await;
    fs::remove_file(lab.folder().join(KEEPER)).unwrap();
    ytyp(&lab.folder(), TENT, &["tent", "tent"]);

    let lines = lab.start(Cause::Requested { by: "ada".to_owned() }).await;

    assert_eq!(lab.read(GIFT), gift);
    assert_eq!(lab.archetypes(TENT), hashes(&["tent", "tent"]));
    assert!(
        lines[0].starts_with("duplicate archetypes: put back 1 archetype"),
        "{lines:?}"
    );
    assert_eq!(lab.runs().await[0].mode, ToolRunMode::Heal);
}

#[tokio::test]
async fn a_report_before_a_start_speaks_only_when_its_counts_or_plan_changed() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    lab.configure(ToolMode::Report).await;
    let requested = || Cause::Requested { by: "ada".to_owned() };

    let first = lab.start(requested()).await;
    let second = lab.start(requested()).await;
    ytyp(&lab.folder(), TENT, &["tent", "tent"]);
    let third = lab.start(requested()).await;

    assert_eq!(
        first,
        ["duplicate archetypes: 2 duplicate archetypes, 2 by a fix in 2 files, 0 left"]
    );
    assert_eq!(second, Vec::<String>::new());
    assert_eq!(
        third,
        ["duplicate archetypes: 3 duplicate archetypes, 3 by a fix in 3 files, 0 left"]
    );
    assert_eq!(lab.read(CAMP), camp);
    let runs = lab.runs().await;
    assert_eq!(runs.len(), 3);
    assert!(runs.iter().all(|run| run.mode == ToolRunMode::Report));
}

#[tokio::test]
async fn work_told_to_end_before_it_began_writes_nothing() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    lab.configure(ToolMode::Fix).await;

    let lines = lab
        .start_with(
            Cause::Requested { by: "ada".to_owned() },
            Arc::new(AtomicBool::new(true)),
        )
        .await;

    assert_eq!(lines, Vec::<String>::new());
    assert_eq!(lab.read(CAMP), camp);
}

#[tokio::test]
async fn a_fix_asked_as_observer_stops_ends_before_writing_and_is_recorded_as_cancelled() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    let plan = lab.tools.scan(&owner()).await.unwrap().plan.unwrap();
    lab.tools.close().await;

    let run = lab.tools.fix(&owner(), &plan).await.unwrap();

    assert_eq!(run.outcome, Some(ToolRunOutcome::Cancelled));
    assert_eq!(run.summary.unwrap().files_changed, 0);
    assert_eq!(lab.read(CAMP), camp);
}

#[tokio::test]
async fn a_run_under_way_hears_that_observer_stops_without_anyone_waiting_for_it() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    let plan = lab.tools.scan(&owner()).await.unwrap().plan.unwrap();
    // As a fix from the panel holds the writer, and a start queued behind it waits.
    let writing = Arc::clone(&lab.tools.inner.writer).lock_owned().await;

    lab.tools.stopping();

    let closing = tokio::time::timeout(std::time::Duration::from_millis(50), lab.tools.close()).await;
    assert!(closing.is_err(), "the close waits for the run to end");
    drop(writing);
    let run = lab.tools.fix(&owner(), &plan).await.unwrap();
    assert_eq!(run.outcome, Some(ToolRunOutcome::Cancelled));
    assert_eq!(lab.read(CAMP), camp);
}

#[tokio::test]
async fn the_tool_is_not_available_on_a_redm_server() {
    let lab = Lab::with_cfg("set gamename rdr3\nensure props\n").await;
    lab.configure(ToolMode::Fix).await;

    let scanned = lab.tools.scan(&owner()).await.unwrap_err();
    let fixed = lab.tools.fix(&owner(), &"0".repeat(64)).await.unwrap_err();
    let lines = lab.start(Cause::Requested { by: "ada".to_owned() }).await;

    for refused in [scanned, fixed] {
        let Error::Refused(refusal) = refused else {
            panic!("not a refusal");
        };
        assert_eq!(refusal.code, ErrorCode::ToolNotAvailable);
        assert_eq!(refusal.params["game"], "rdr3");
    }
    assert_eq!(lines, Vec::<String>::new());
    assert_eq!(lab.runs().await, Vec::new());
}

#[tokio::test]
async fn observer_s_start_settles_an_interrupted_write_and_ends_unfinished_runs() {
    let lab = Lab::new().await;
    let camp = lab.read(CAMP);
    let folder = ServerFolder::open(&lab.folder()).unwrap();
    let identity = folder.id().clone();
    let temporary = observer_tools::fix::temporary_name().unwrap();
    write(
        &lab.folder(),
        &format!("resources/props/stream/{temporary}"),
        b"half",
    );
    let root = lab.folder().to_string_lossy().into_owned();
    let started = "2026-11-01T08:00:00Z".parse().unwrap();
    let temporary_name = temporary.clone();
    let original = sha256(&camp);
    let interrupted = lab
        .db
        .write(move |tx| -> Result<i64, StoreError> {
            let run = stored::insert_run(
                tx,
                &NewRun {
                    server_id: Servers::DEFAULT,
                    tool: TOOL,
                    trigger: ToolTrigger::Panel,
                    actor: Some("ada"),
                    mode: ToolRunMode::Fix,
                    started_at: started,
                },
            )?;
            stored::begin(
                tx,
                &BeginWrite {
                    server_id: Servers::DEFAULT,
                    tool: TOOL,
                    run,
                    folder_path: &root,
                    folder_id: &identity.id,
                    path: CAMP,
                    resource: "props",
                    format: ToolFileFormat::Ytyp,
                    original_sha256: original,
                    temporary: &temporary_name,
                    sha256: [9; 32],
                    removals: &[],
                    existing: None,
                    at: started,
                },
            )?;
            Ok(run)
        })
        .await
        .unwrap();

    lab.tools
        .recover("2026-11-01T09:00:00Z".parse().unwrap())
        .await
        .unwrap();

    let runs = lab.runs().await;
    let ended = runs.iter().find(|run| run.id == interrupted).unwrap();
    assert_eq!(ended.outcome, Some(ToolRunOutcome::Failed));
    assert_eq!(runs[0].mode, ToolRunMode::Heal);
    assert_eq!(runs[0].outcome, Some(ToolRunOutcome::Done));
    assert!(
        !lab.folder()
            .join("resources/props/stream")
            .join(&temporary)
            .exists()
    );
    assert_eq!(lab.read(CAMP), camp);
    let view = lab.tools.view().await.unwrap();
    assert_eq!(view.tools[0].open, Vec::new());
    assert_eq!(
        view.tools[0].closed[0].close_reason,
        Some(ToolCloseReason::Abandoned)
    );
}

#[tokio::test]
async fn the_files_runs_an_earlier_observer_left_unfinished_wrote_are_audited_at_the_next_start() {
    let lab = Lab::new().await;
    let identity = ServerFolder::open(&lab.folder()).unwrap().id().clone();
    let root = lab.folder().to_string_lossy().into_owned();
    let started: Timestamp = "2026-11-01T08:00:00Z".parse().unwrap();
    let camp = sha256(&lab.read(CAMP));
    let gift = sha256(&lab.read(GIFT));
    let (first, second) = (
        observer_tools::fix::temporary_name().unwrap(),
        observer_tools::fix::temporary_name().unwrap(),
    );
    let (fixing, undoing) = lab
        .db
        .write(move |tx| -> Result<(i64, i64), StoreError> {
            let run = |mode, trigger, actor| {
                stored::insert_run(
                    tx,
                    &NewRun {
                        server_id: Servers::DEFAULT,
                        tool: TOOL,
                        trigger,
                        actor,
                        mode,
                        started_at: started,
                    },
                )
            };
            let fixing = run(ToolRunMode::Fix, ToolTrigger::Panel, Some("ada"))?;
            let undoing = run(ToolRunMode::Undo, ToolTrigger::Cli, None)?;
            let removal = ToolRemoval {
                index: 0,
                hash: name_hash("marshmallow"),
                name: None,
                keeper: ToolKeeper {
                    path: CAMP.to_owned(),
                    format: ToolFileFormat::Ytyp,
                    index: 2,
                    sha256: "ab".repeat(32),
                },
                run: fixing,
            };
            let begin = |run, path, temporary: &str, sha256, removals: &[ToolRemoval], existing| {
                stored::begin(
                    tx,
                    &BeginWrite {
                        server_id: Servers::DEFAULT,
                        tool: TOOL,
                        run,
                        folder_path: &root,
                        folder_id: &identity.id,
                        path,
                        resource: "props",
                        format: ToolFileFormat::Ytyp,
                        original_sha256: [7; 32],
                        temporary,
                        sha256,
                        removals,
                        existing,
                        at: started,
                    },
                )
                .map(Option::unwrap)
            };
            // The fix's first write landed and was recorded, its second landed before it could be.
            let landed = begin(fixing, CAMP, &first, camp, std::slice::from_ref(&removal), None)?;
            stored::applied(tx, landed, started)?;
            begin(fixing, GIFT, &second, gift, std::slice::from_ref(&removal), None)?;
            // The undo wrote an original back and closed its change.
            let earlier = begin(fixing, TENT, "a", [8; 32], &[removal], None)?;
            stored::applied(tx, earlier, started)?;
            begin(undoing, TENT, "b", [7; 32], &[], Some(earlier))?;
            stored::applied(tx, earlier, started)?;
            stored::close(tx, earlier, undoing, ToolCloseReason::Undone, started)?;
            Ok((fixing, undoing))
        })
        .await
        .unwrap();

    lab.tools
        .recover("2026-11-01T09:00:00Z".parse().unwrap())
        .await
        .unwrap();

    let changed = lab.audited(AuditAction::ToolFilesChanged).await;
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert_eq!(changed[0].0, "system");
    let details = &changed[0].1;
    assert_eq!(details["run"], fixing);
    assert_eq!(details["trigger"], "panel");
    assert_eq!(details["actor"], "ada");
    assert_eq!(details["interrupted"], true);
    assert_eq!(details["count"], 2);
    assert_eq!(details["files"], serde_json::json!([CAMP, GIFT]));
    let restored = lab.audited(AuditAction::ToolFilesRestored).await;
    assert_eq!(restored.len(), 1, "{restored:?}");
    assert_eq!(restored[0].1["run"], undoing);
    assert_eq!(restored[0].1["reason"], "undo");
    assert_eq!(restored[0].1["files"], serde_json::json!([TENT]));
    let runs = lab.runs().await;
    assert!(
        runs.iter()
            .filter(|run| run.id == fixing || run.id == undoing)
            .all(|run| run.outcome == Some(ToolRunOutcome::Failed))
    );
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

#[tokio::test]
async fn sftp_commits_pass_beside_each_other_and_wait_while_a_tool_writes() {
    let gate = CommitGate::default();
    let wait = std::time::Duration::from_millis(50);

    let first = gate.shared().await;
    let second = tokio::time::timeout(wait, gate.shared()).await;
    assert!(second.is_ok(), "two commits pass together");
    assert!(tokio::time::timeout(wait, gate.exclusive()).await.is_err());
    drop((first, second));
    let writing = gate.exclusive().await;
    assert!(tokio::time::timeout(wait, gate.shared()).await.is_err());
    drop(writing);
    assert!(tokio::time::timeout(wait, gate.shared()).await.is_ok());
}
