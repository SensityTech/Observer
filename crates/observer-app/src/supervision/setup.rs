//! The server setup: checks of an existing server's folder and cfg, read where they are, and the
//! save that follows them. The setup never writes into that folder; the owner fixes what blocks.
//! Observer writes there only SFTP uploads, the fixes of the server tools the owner runs or turns
//! on, and the cfg the owner saves from its page.

use std::io;
use std::time::Duration;

use observer_domain::convars::CfgConvars;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::launch::{LaunchChange, OneSync, StoredLaunch};
use observer_domain::server_data::{
    CfgCheck, CfgFinding, CfgReport, CfgText, CfgTextQuery, Discovery, EndpointProtocol, FileListing,
    FilesQuery, FindingKind, FolderCheck, FolderListing, FolderQuery, FolderReport, LicenceState, Place,
    PlaceKind, Severity,
};
use observer_fxserver::cfg::Context;
use observer_fxserver::folders::{self, Search};
use observer_fxserver::layout::{Flavour, Layout};
use observer_fxserver::{server_data, server_files};

use super::{Supervision, read_launch};
use crate::servers::follow_rules;
use crate::{Caller, Error};

/// How long the search for servers reads the host's folders, their cfgs read after.
const SEARCH_BUDGET: Duration = Duration::from_secs(5);
/// Folders down from the home folder, where `Documents\Servers\live\server-data` is.
const HOME_DEPTH: usize = 4;
const DRIVE_DEPTH: usize = 3;

impl Supervision {
    /// Checks the folder of an existing server. It changes nothing.
    pub async fn check_folder(&self, check: FolderCheck) -> Result<FolderReport, Error> {
        let observer = self.inner.data_dir.clone();
        blocking(move || server_data::check_folder(&check.data_path, &observer)).await
    }

    /// The folders of a folder of the host, for the setup's path field and its folder browser. It
    /// changes nothing.
    pub async fn list_folders(&self, query: FolderQuery) -> Result<FolderListing, Error> {
        let _turn = self
            .inner
            .folder_reads
            .acquire()
            .await
            .map_err(io::Error::other)?;
        let observer = self.inner.data_dir.clone();
        blocking(move || folders::list(&query, &observer)).await
    }

    /// The entries of one folder of a server folder, for the setup's file browser. It changes
    /// nothing.
    pub async fn list_files(&self, query: FilesQuery) -> Result<FileListing, Error> {
        let _turn = self
            .inner
            .folder_reads
            .acquire()
            .await
            .map_err(io::Error::other)?;
        let observer = self.inner.data_dir.clone();
        blocking(move || server_files::list(&query.data_path, &query.path, &observer)).await
    }

    /// A cfg of a server folder with its secrets left out, for the setup to show. It changes
    /// nothing.
    pub async fn cfg_text(&self, query: CfgTextQuery) -> Result<CfgText, Error> {
        let observer = self.inner.data_dir.clone();
        blocking(move || server_files::cfg_text(&query.data_path, &query.cfg_file, &observer)).await
    }

    /// Searches the home folder and the local drives for servers, and names the places the owner
    /// may browse from. It changes nothing.
    pub async fn discover(&self) -> Result<Discovery, Error> {
        let _turn = self
            .inner
            .folder_reads
            .acquire()
            .await
            .map_err(io::Error::other)?;
        let home = std::env::home_dir();
        let drives = observer_platform::local_roots();
        let search = Search {
            roots: home
                .iter()
                .map(|home| (home.clone(), HOME_DEPTH))
                .chain(drives.iter().map(|drive| (drive.clone(), DRIVE_DEPTH)))
                .collect(),
            observer: self.inner.data_dir.clone(),
            own_pids: self.inner.supervisor.status().borrow().pid.into_iter().collect(),
            budget: SEARCH_BUDGET,
        };
        let places = drives
            .iter()
            .map(|drive| (PlaceKind::Drive, drive))
            .chain(home.iter().map(|home| (PlaceKind::Home, home)))
            .filter_map(|(kind, path)| {
                Some(Place {
                    kind,
                    path: path.to_str()?.to_owned(),
                })
            })
            .collect();
        let found = blocking(move || folders::discover(&search)).await?;
        Ok(Discovery {
            places,
            servers: found.servers,
            truncated: found.truncated,
        })
    }

    /// Checks the cfg of an existing server and every file it runs. Its endpoints are bound to see
    /// whether they are free only while FXServer, which would hold them, is stopped. It changes
    /// nothing.
    pub async fn check_cfg(&self, check: CfgCheck) -> Result<CfgReport, Error> {
        let context = self.cfg_context(check.onesync).await?;
        let observer = self.inner.data_dir.clone();
        blocking(move || server_data::check_cfg(&check.data_path, &check.cfg_file, &observer, &context)).await
    }

    /// What the saved launch's cfg sets, which wins over the launch's own `+set`; `None` until the
    /// launch settings are saved. It changes nothing.
    pub async fn cfg_convars(&self) -> Result<Option<CfgConvars>, Error> {
        let (stored, _) = read_launch(&self.inner.db).await?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        blocking(move || Some(server_data::cfg_convars(&stored.settings))).await
    }

    /// Saves launch settings from the setup: the rules first, then the folder and its cfg read
    /// again, whatever the panel was told before, and only then the save, as the Server page's.
    pub async fn save_setup(&self, caller: &Caller, change: LaunchChange) -> Result<StoredLaunch, Error> {
        follow_rules(&change.settings)?;
        let check = CfgCheck {
            data_path: change.settings.data_path.clone(),
            cfg_file: change.settings.cfg_file.clone(),
            onesync: change.settings.onesync,
        };
        let report = self.check_cfg(check).await?;
        let blocked = ApiError::new(ErrorCode::SetupBlocked);
        if let Some(problem) = report.folder {
            return Err(blocked
                .with("folder", serde_json::to_value(problem).unwrap_or_default())
                .into());
        }
        if let Some(problem) = report.problem {
            return Err(blocked
                .with("cfg", serde_json::to_value(problem).unwrap_or_default())
                .into());
        }
        let mut blocking_kinds: Vec<FindingKind> = Vec::new();
        for finding in &report.findings {
            if finding.severity == Severity::Blocking && !blocking_kinds.contains(&finding.kind) {
                blocking_kinds.push(finding.kind);
            }
        }
        if !blocking_kinds.is_empty() {
            let kinds = serde_json::to_value(blocking_kinds).unwrap_or_default();
            return Err(blocked.with("findings", kinds).into());
        }
        self.inner
            .servers
            .save_from_setup(caller, change.settings, change.version)
            .await
    }

    pub(super) async fn cfg_context(&self, onesync: OneSync) -> Result<Context, Error> {
        let (_, state) = read_launch(&self.inner.db).await?;
        let system_resources = state.selected.map(|build| {
            Layout::new(self.inner.artifacts.join(build.to_string()), Flavour::HOST).system_resources()
        });
        Ok(Context {
            panel: Some(self.inner.panel),
            onesync,
            probe_ports: self.inner.supervisor.status().borrow().pid.is_none(),
            system_resources,
            follow_outside: false,
            convars: None,
        })
    }
}

pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Error> {
    Ok(tokio::task::spawn_blocking(work)
        .await
        .map_err(io::Error::other)?)
}

/// What the console says of a finding of the cfg as FXServer starts: `server.cfg:2 starts
/// txAdmin's monitor resource`. Its words come from the finding, which holds no secret.
pub(super) fn console_note(finding: &CfgFinding) -> String {
    let at = match finding.line {
        Some(line) => format!("{}:{line}", finding.file),
        None => finding.file.clone(),
    };
    let params = &finding.params;
    let text = |value: &Option<String>| value.clone().unwrap_or_default();
    let count = params.count.unwrap_or_default();
    let (protocol, twin) = match params.protocol {
        Some(EndpointProtocol::Udp) => ("UDP", "TCP"),
        _ => ("TCP", "UDP"),
    };
    let endpoint = match &params.address {
        Some(address) => format!("{protocol} endpoint {address}"),
        None => format!("a {protocol} endpoint"),
    };
    let what = match finding.kind {
        FindingKind::NoEndpoint => "adds no endpoint: players cannot connect".to_owned(),
        FindingKind::EndpointInvalid => format!("adds {endpoint}, which is not one ip:port address"),
        FindingKind::EndpointOnPanel => format!("adds {endpoint} on the panel's port"),
        FindingKind::EndpointUnpaired => {
            format!("adds {endpoint} without its {twin} twin: players cannot connect")
        }
        FindingKind::EndpointDuplicate => format!("adds {endpoint} again"),
        FindingKind::EndpointReserved => format!("adds {endpoint} among txAdmin's ports, 40120 to 40150"),
        FindingKind::PortInUse => format!(
            "adds {endpoint} on a port FXServer cannot listen on: another program holds it, or the system reserves it"
        ),
        FindingKind::StartsMonitor => format!("starts txAdmin's {} resource", text(&params.name)),
        FindingKind::SetsServerMode => {
            "sets txAdminServerMode: FXServer starts txAdmin's monitor resource by itself".to_owned()
        }
        FindingKind::SetsMonitorMode => "sets monitorMode, which only txAdmin reads".to_owned(),
        FindingKind::SetsObserverConvar => {
            format!(
                "sets {}, which Observer sets at each launch: Observer's value wins",
                text(&params.name)
            )
        }
        FindingKind::PlusCommand => {
            format!(
                "writes {}, which FXServer only reads on its command line",
                text(&params.name)
            )
        }
        FindingKind::LicenceMissing => "sets no licence key: FXServer stops at start".to_owned(),
        FindingKind::LicenceUnusable => match params.licence {
            Some(LicenceState::Placeholder) => "sets a placeholder for the licence key".to_owned(),
            _ => "sets a licence key that is not one Cfx.re gives".to_owned(),
        },
        FindingKind::MaxClientsInvalid => {
            format!("sets sv_maxclients to {}, outside 1 to 2048", text(&params.value))
        }
        FindingKind::MaxClientsNeedOneSync => format!("allows {count} players, which needs OneSync on"),
        FindingKind::GameInvalid => {
            format!("sets gamename to {}, neither gta5 nor rdr3", text(&params.value))
        }
        FindingKind::GameTwice => format!("sets gamename again, to {}", text(&params.value)),
        FindingKind::ResourceMissing => format!(
            "starts {}, which is no resource of the server folder or the build",
            text(&params.name)
        ),
        FindingKind::ResourceMissingMore => format!("starts {count} more resources that are missing"),
        FindingKind::ExecMissing => format!("runs {}, which cannot be read", text(&params.target)),
        FindingKind::ExecOutside => format!("runs {}, outside the server folder", text(&params.target)),
        FindingKind::ExecResource => format!("runs {} from a resource", text(&params.target)),
        FindingKind::ExecTooDeep => format!("runs {}, more than 5 files deep", text(&params.target)),
        FindingKind::ExecRepeated => format!("runs {} again", text(&params.target)),
        FindingKind::EditedByTxAdmin => format!("holds {count} lines txAdmin commented out"),
    };
    format!("{at} {what}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
    use std::path::Path;

    use jiff::Timestamp;
    use observer_domain::audit::{ActorKind, AuditAction, AuditFilter};
    use observer_domain::launch::{ExtraArgument, LaunchSettings, StoredLaunch};
    use observer_domain::server_data::{FindingParams, FolderProblem};
    use observer_store::Database;
    use observer_store::artifacts::{self, ArtifactState};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::supervision::{Running, Setup};
    use crate::{Accounts, AuditLog, Client, Clock, PasswordCost, Servers};

    /// Where the test's panel listens. Nothing binds it: the checks only compare with it.
    const PANEL: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 41975);
    /// A test fixture, not a real key.
    const KEY: &str = "sv_licenseKey cfxk_ObserverLabFixture_k3y";

    fn folder(path: &Path) -> String {
        path.to_str().unwrap().to_owned()
    }

    struct Lab {
        dir: TempDir,
        supervision: Supervision,
        audit: AuditLog,
        owner: Caller,
        _running: Running,
    }

    impl Lab {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
                .await
                .unwrap();
            let clock = Clock::manual("2025-01-06T09:00:00Z".parse().unwrap());
            let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
                .await
                .unwrap();
            let client = Client::new(Ipv4Addr::LOCALHOST.into(), Some("test"));
            let token = accounts
                .create_setup_token(ActorKind::System)
                .await
                .unwrap()
                .unwrap()
                .token;
            let signed_in = accounts
                .complete_setup(&token, "ada", "correct horse battery staple".to_owned(), &client)
                .await
                .unwrap();
            let owner = accounts
                .authenticate(&signed_in.token, &signed_in.key, client)
                .await
                .unwrap();
            let observer = dir.path().join("observer-data");
            fs::create_dir_all(&observer).unwrap();
            let setup = Setup {
                servers: Servers::new(db.clone(), clock.clone()),
                db: db.clone(),
                clock: clock.clone(),
                accounts,
                data_dir: observer.clone(),
                panel: PANEL,
                artifacts: observer.join("artifacts"),
                console_logs: observer.join("logs/console"),
                log: None,
            };
            let (supervision, running) = Supervision::start(setup, |_| {}).await.unwrap();
            Self {
                audit: AuditLog::new(db, clock),
                dir,
                supervision,
                owner,
                _running: running,
            }
        }

        /// A server folder at `name` holding `chat` and `cfg` as its `server.cfg`.
        fn server(&self, name: &str, cfg: &str) -> String {
            let data = self.dir.path().join(name);
            let chat = data.join("resources/chat");
            fs::create_dir_all(&chat).unwrap();
            fs::write(chat.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
            fs::write(data.join("server.cfg"), cfg).unwrap();
            folder(&data)
        }

        async fn check(&self, data_path: &str) -> CfgReport {
            let check = CfgCheck {
                data_path: data_path.to_owned(),
                cfg_file: "server.cfg".to_owned(),
                onesync: OneSync::On,
            };
            self.supervision.check_cfg(check).await.unwrap()
        }

        async fn save(&self, data_path: &str, version: Option<i64>) -> Result<StoredLaunch, Error> {
            let change = LaunchChange {
                settings: settings(data_path),
                version,
            };
            self.supervision.save_setup(&self.owner, change).await
        }
    }

    fn settings(data_path: &str) -> LaunchSettings {
        LaunchSettings {
            data_path: data_path.to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::On,
            auto_start: false,
            txadmin_events: false,
            extra_arguments: Vec::new(),
            hang_detection: true,
            hang_timeout_seconds: 360,
        }
    }

    /// A loopback address whose port was free a moment ago: the checks bind what the cfg adds.
    fn free_address() -> String {
        let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        free.local_addr().unwrap().to_string()
    }

    fn endpoints(address: &str) -> String {
        format!("endpoint_add_tcp \"{address}\"\nendpoint_add_udp \"{address}\"\n{KEY}\n")
    }

    fn refusal(result: Result<StoredLaunch, Error>) -> ApiError {
        match result {
            Err(Error::Refused(refusal)) => refusal,
            other => panic!("not refused: {other:?}"),
        }
    }

    fn places(report: &CfgReport, kind: FindingKind) -> Vec<(String, Option<u32>)> {
        report
            .findings
            .iter()
            .filter(|finding| finding.kind == kind)
            .map(|finding| (finding.file.clone(), finding.line))
            .collect()
    }

    #[tokio::test]
    async fn the_setup_saves_a_server_that_starts_and_says_so_in_the_audit_log() {
        let lab = Lab::new().await;
        let live = lab.server("live", &endpoints(&free_address()));

        let saved = lab.save(&live, None).await.unwrap();
        let again = lab.save(&live, Some(1)).await.unwrap();

        assert_eq!((saved.version, again.version), (1, 1));
        assert_eq!(saved.settings, settings(&live));
        let filter = AuditFilter {
            action: Some(AuditAction::LaunchSettingsChanged),
            ..AuditFilter::default()
        };
        let entries = lab.audit.page(filter).await.unwrap().entries;
        assert_eq!(entries.len(), 1, "the same settings again change nothing");
        assert_eq!(entries[0].actor_name.as_deref(), Some("ada"));
        assert_eq!(entries[0].details["setup"], json!(true));
        assert_eq!(
            entries[0].details["onesync"],
            json!({ "before": null, "after": "on" })
        );
    }

    #[tokio::test]
    async fn the_setup_refuses_a_folder_or_a_cfg_that_keeps_fxserver_from_starting() {
        let lab = Lab::new().await;
        let empty = lab.dir.path().join("empty");
        fs::create_dir_all(empty.join("resources")).unwrap();
        let unreachable = lab.server("unreachable", &format!("{KEY}\n"));
        let no_cfg = lab.server("no-cfg", "");
        fs::remove_file(lab.dir.path().join("no-cfg/server.cfg")).unwrap();
        let mut invalid = settings("server-data");
        invalid.extra_arguments.push(ExtraArgument {
            name: "rcon_password".to_owned(),
            value: "Hunter2-rcon".to_owned(),
        });

        let folder_refused = refusal(lab.save(&folder(&empty), None).await);
        let cfg_refused = refusal(lab.save(&no_cfg, None).await);
        let finding_refused = refusal(lab.save(&unreachable, None).await);
        let rules_refused = refusal(
            lab.supervision
                .save_setup(
                    &lab.owner,
                    LaunchChange {
                        settings: invalid,
                        version: None,
                    },
                )
                .await,
        );

        assert_eq!(folder_refused.code, ErrorCode::SetupBlocked);
        assert_eq!(folder_refused.params["folder"], "emptyResources");
        assert_eq!(cfg_refused.params["cfg"], "missing");
        assert_eq!(finding_refused.params["findings"], json!(["noEndpoint"]));
        assert_eq!(rules_refused.code, ErrorCode::InvalidLaunchSettings);
        assert_eq!(
            lab.supervision
                .inner
                .servers
                .launch_settings(Servers::DEFAULT)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_save_from_settings_changed_since_is_a_conflict() {
        let lab = Lab::new().await;
        let live = lab.server("live", &endpoints(&free_address()));
        let other = lab.server("other", &endpoints(&free_address()));
        lab.save(&live, None).await.unwrap();
        lab.supervision
            .inner
            .servers
            .change_launch_settings(&lab.owner, Servers::DEFAULT, settings(&other), Some(1))
            .await
            .unwrap();

        let stale = refusal(lab.save(&live, Some(1)).await);

        assert_eq!(stale.code, ErrorCode::Conflict);
    }

    #[tokio::test]
    async fn a_server_folder_inside_observers_own_is_refused() {
        let lab = Lab::new().await;
        let inside = lab.server("observer-data/server-data", &endpoints(&free_address()));

        let report = lab
            .supervision
            .check_folder(FolderCheck {
                data_path: inside.clone(),
            })
            .await
            .unwrap();
        let refused = refusal(lab.save(&inside, None).await);

        assert_eq!(report.problem, Some(FolderProblem::InsideObserver));
        assert_eq!(refused.params["folder"], "insideObserver");
    }

    #[tokio::test]
    async fn an_endpoint_on_the_panels_port_blocks_the_save() {
        let lab = Lab::new().await;
        let live = lab.server("live", &endpoints(&PANEL.to_string()));

        let report = lab.check(&live).await;
        let refused = refusal(lab.save(&live, None).await);

        assert_eq!(
            places(&report, FindingKind::EndpointOnPanel),
            [("server.cfg".to_owned(), Some(1))]
        );
        assert_eq!(refused.params["findings"], json!(["endpointOnPanel"]));
    }

    #[tokio::test]
    async fn a_port_another_program_holds_is_reported_while_fxserver_is_stopped() {
        let lab = Lab::new().await;
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = held.local_addr().unwrap().to_string();
        let live = lab.server("live", &endpoints(&address));

        let report = lab.check(&live).await;
        let saved = lab.save(&live, None).await;

        assert_eq!(
            report
                .findings
                .iter()
                .find(|finding| finding.kind == FindingKind::PortInUse)
                .map(|finding| (finding.line, finding.severity, finding.params.clone())),
            Some((
                Some(1),
                Severity::Warning,
                FindingParams {
                    protocol: Some(EndpointProtocol::Tcp),
                    address: Some(address),
                    ..FindingParams::default()
                }
            ))
        );
        assert!(saved.is_ok(), "a busy port only warns");
    }

    #[tokio::test]
    async fn the_selected_builds_system_resources_count_as_resources() {
        let lab = Lab::new().await;
        let live = lab.server(
            "live",
            &format!("{}ensure spawnmanager\n", endpoints(&free_address())),
        );
        let before = lab.check(&live).await;
        let build = lab.dir.path().join("observer-data/artifacts/35245");
        let layout = Layout::new(&build, Flavour::HOST);
        fs::create_dir_all(layout.system_resources().join("spawnmanager")).unwrap();
        let state = ArtifactState {
            selected: Some(35245),
            ..ArtifactState::default()
        };
        lab.supervision
            .inner
            .db
            .write(move |tx| artifacts::save_state(tx, Servers::DEFAULT, &state, Timestamp::now()))
            .await
            .unwrap();

        let after = lab.check(&live).await;

        assert_eq!(
            places(&before, FindingKind::ResourceMissing),
            [("server.cfg".to_owned(), Some(4))]
        );
        assert_eq!(places(&after, FindingKind::ResourceMissing), []);
    }

    #[test]
    fn a_finding_reads_as_a_console_note_where_it_is_written() {
        let note = |kind, line, params| {
            console_note(&CfgFinding {
                kind,
                severity: kind.severity(),
                file: "server.cfg".to_owned(),
                line,
                params,
            })
        };

        assert_eq!(
            note(
                FindingKind::StartsMonitor,
                Some(2),
                FindingParams {
                    name: Some("monitor".to_owned()),
                    ..FindingParams::default()
                }
            ),
            "server.cfg:2 starts txAdmin's monitor resource"
        );
        assert_eq!(
            note(FindingKind::LicenceMissing, None, FindingParams::default()),
            "server.cfg sets no licence key: FXServer stops at start"
        );
        assert_eq!(
            note(
                FindingKind::EndpointUnpaired,
                Some(1),
                FindingParams {
                    protocol: Some(EndpointProtocol::Tcp),
                    address: Some("0.0.0.0:30140".to_owned()),
                    ..FindingParams::default()
                }
            ),
            "server.cfg:1 adds TCP endpoint 0.0.0.0:30140 without its UDP twin: players cannot connect"
        );
        assert_eq!(
            note(
                FindingKind::PortInUse,
                Some(2),
                FindingParams {
                    protocol: Some(EndpointProtocol::Udp),
                    address: Some("0.0.0.0:30140".to_owned()),
                    ..FindingParams::default()
                }
            ),
            "server.cfg:2 adds UDP endpoint 0.0.0.0:30140 on a port FXServer cannot listen on: \
             another program holds it, or the system reserves it",
            "binding it may be denied with no other program in sight"
        );
    }
}
