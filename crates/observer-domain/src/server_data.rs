//! The server setup: what Observer finds in the folder of an existing server and in its cfg before
//! it saves launch settings for them. The setup only reads that folder; the reports name each
//! problem with its file and line, and the owner fixes the cfg.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::launch::{LaunchProblem, OneSync};

/// `POST /api/servers/default/setup/folder`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct FolderCheck {
    pub data_path: String,
}

/// What a server folder holds, or why FXServer cannot run from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderReport {
    /// Null when FXServer can run from the folder.
    pub problem: Option<FolderProblem>,
    /// Why the path rules refuse it, with `invalidPath`.
    pub path_problem: Option<LaunchProblem>,
    /// A folder nearby that passes every check, offered in place of a refused one.
    pub suggestion: Option<String>,
    /// Null for a refused folder.
    pub resources: Option<ResourceSummary>,
    /// Relative to the folder with `/`, the detected one first, at most 50.
    pub cfg_files: Vec<String>,
    pub detected_cfg: Option<String>,
}

/// Why FXServer cannot run from a folder. The checks run in this order and the first problem wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FolderProblem {
    /// The path rules refuse it: `pathProblem` says why.
    InvalidPath,
    Missing,
    NotAFolder,
    /// It cannot be listed.
    Unreadable,
    /// It is Observer's data folder or lies inside it.
    InsideObserver,
    /// It is the `resources` folder of a server rather than the folder that holds it.
    IsResources,
    NoResources,
    EmptyResources,
}

/// `resources/` as FXServer walks it: a folder named `[x]` is a category, any other a resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceSummary {
    pub resources: u32,
    pub categories: u32,
    /// At most 20 resources with neither `fxmanifest.lua` nor `__resource.lua`.
    pub without_manifest: Vec<String>,
    /// The walk stopped at 250 categories or 5,000 entries: the counts are lower bounds.
    pub truncated: bool,
}

/// `POST /api/servers/default/setup/folders`: the folders in `path` whose name starts with
/// `prefix`, case aside.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct FolderQuery {
    pub path: String,
    #[serde(default)]
    pub prefix: String,
}

/// The folders of one folder of the host, each with what it looks like to the setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderListing {
    /// The folder listed, as asked.
    pub path: String,
    /// Null at the root of a drive or of the file system.
    pub parent: Option<String>,
    /// Why the folder cannot be listed: `invalidPath`, `missing`, `notAFolder` or `unreadable`.
    pub problem: Option<FolderProblem>,
    pub path_problem: Option<LaunchProblem>,
    /// By name, case aside. Hidden and system folders are left out unless the prefix asks for
    /// them with a leading dot.
    pub entries: Vec<FolderEntry>,
    /// More folders matched than the 200 listed.
    pub truncated: bool,
}

/// A folder as the setup sees it from outside: what it is, and for a folder of the host that holds
/// servers, which ones. Looking into folders stops after a second and a half: the folders past it
/// come without `folders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderEntry {
    pub name: String,
    /// The full path, as the folder check takes it.
    pub path: String,
    pub kind: FolderKind,
    /// A link or a junction to a folder elsewhere.
    pub link: bool,
    /// For a server folder, the resources counted.
    pub resources: Option<u32>,
    /// For a server folder, the cfg detected.
    pub cfg: Option<String>,
    /// The folders inside, counted up to 200; null when it was not looked into.
    pub folders: Option<u32>,
    /// The server folders and txAdmin data folders right inside, at most 5, with nothing inside.
    pub inside: Vec<FolderEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FolderKind {
    Folder,
    /// It holds a `resources` folder with at least one resource.
    Server,
    /// txAdmin's data: its `admins.json`, or a `default` profile with its `config.json`.
    TxAdminData,
    /// Observer cannot list it.
    Unreadable,
}

/// `POST /api/servers/default/setup/discover`: where the owner may look, and the servers Observer
/// found in the usual places. The search reads names, the walk of each server's resources, its cfg
/// as the cfg check does, and from txAdmin's data the server folder each profile names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Discovery {
    pub places: Vec<Place>,
    pub servers: Vec<FoundServer>,
    /// The search stopped at its time or entry limit, or at 24 servers: there may be more.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Place {
    pub kind: PlaceKind,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PlaceKind {
    /// A local drive on Windows, `/` elsewhere.
    Drive,
    Home,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FoundServer {
    pub path: String,
    pub resources: u32,
    /// The walk stopped at one of its limits: `resources` is a floor.
    pub resources_truncated: bool,
    pub cfg: Option<String>,
    /// As the detected cfg sets it.
    pub onesync: Option<OneSync>,
    /// A profile of txAdmin's data names this folder.
    pub tx_admin: bool,
    /// The port of the cfg's first `endpoint_add_tcp`.
    pub port: Option<u16>,
    /// A program other than Observer's FXServer listens on that port now.
    pub port_holder: Option<PortHolder>,
}

/// Who listens on a server's port, as the system's tables say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PortHolder {
    /// The file name of its executable, when the system says whose the socket is.
    pub program: Option<String>,
    /// The program is FXServer.
    pub fx_server: bool,
}

/// `POST /api/servers/default/setup/cfg`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CfgCheck {
    pub data_path: String,
    /// Relative to the data folder.
    pub cfg_file: String,
    /// The OneSync FXServer will run with, which decides how many players the cfg may allow.
    pub onesync: OneSync,
}

/// What a cfg and the files it runs say, and what in them works against Observer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgReport {
    /// The data folder's own problem: with one, the cfg is not read.
    pub folder: Option<FolderProblem>,
    /// What keeps the cfg itself from being read: with one, the files, facts and findings are
    /// empty.
    pub problem: Option<CfgFileProblem>,
    /// Why the path rules refuse the cfg, with `invalidPath`.
    pub path_problem: Option<LaunchProblem>,
    /// Each file met, in the order the scan met them, the cfg first.
    pub files: Vec<CfgFile>,
    pub facts: Option<CfgFacts>,
    pub findings: Vec<CfgFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CfgFileProblem {
    /// The path rules refuse it: `pathProblem` says why.
    InvalidPath,
    Missing,
    NotAFile,
    Unreadable,
    /// Over 1 MiB.
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CfgFile {
    /// Relative to the data folder with `/`, or the `exec` target as written when it is not
    /// inside it.
    pub path: String,
    pub state: CfgFileState,
}

/// Whether a file was read, and if not why. Only `read` files are checked; FXServer still runs
/// the others it can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CfgFileState {
    Read,
    /// Not there, or it cannot be read.
    Missing,
    /// It resolves outside the data folder, through `..`, an absolute path or a link.
    Outside,
    /// `exec @resource/...`.
    Resource,
    /// Past 5 `exec`s deep.
    TooDeep,
    /// Read already: FXServer runs it again.
    Repeated,
    /// Over 1 MiB.
    TooLarge,
    /// Past the 32 files one check reads.
    TooMany,
}

/// What the cfg sets, for the summary. The last setter wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgFacts {
    /// Null when the cfg does not set it: FXServer then runs GTA V.
    pub game: Option<Game>,
    /// As written, at most 16.
    pub endpoints: Vec<Endpoint>,
    pub max_clients: Option<u32>,
    /// The cfg's own value, which Observer's overrides.
    pub onesync: Option<OneSync>,
    /// `sv_hostname` without its control characters, at most 128 characters.
    pub hostname: Option<String>,
    pub licence: LicenceState,
    /// `sv_enforceGameBuild` as the chain sets it, the last setter winning.
    pub game_build: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Endpoint {
    pub protocol: EndpointProtocol,
    pub address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EndpointProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Game {
    Gta5,
    Rdr3,
}

/// The licence key as an offline check sees it. The key itself never leaves the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LicenceState {
    Set,
    Missing,
    /// Empty, or `changeme`.
    Placeholder,
    /// Neither a `cfxk_` key nor 32 word characters.
    Malformed,
    /// Not in the files read, but a file that was not read may set it.
    Unknown,
}

/// One problem, where it is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CfgFinding {
    pub kind: FindingKind,
    /// The kind's, but for `noEndpoint`, a warning only when a file that was not read may hold the
    /// endpoints.
    pub severity: Severity,
    /// Relative to the data folder with `/`: the line's file, the resource folder for
    /// `resourceNamedObserver`, the cfg for what the whole chain lacks.
    pub file: String,
    pub line: Option<u32>,
    pub params: FindingParams,
}

/// Only names Observer knows and the cfg's own words that hold no secret.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export, optional_fields)]
pub struct FindingParams {
    /// A resource, a convar or a command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<EndpointProtocol>,
    /// An endpoint as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// An `exec` target as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The value of `sv_maxclients` or `gamename` as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub licence: Option<LicenceState>,
}

/// What a finding means for the setup: `blocking` keeps it from saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Severity {
    Blocking,
    Warning,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FindingKind {
    /// Neither `endpoint_add_tcp` nor `endpoint_add_udp`.
    NoEndpoint,
    /// Not one `ip:port` argument.
    EndpointInvalid,
    /// On the panel's port, at an address that overlaps the panel's.
    EndpointOnPanel,
    /// A TCP address without its UDP twin, or the reverse: players cannot connect.
    EndpointUnpaired,
    EndpointDuplicate,
    /// In txAdmin's range, 40120 to 40150: a txAdmin running on the host takes it.
    EndpointReserved,
    /// Observer could not bind it on this host while FXServer was stopped.
    PortInUse,
    /// Starts `monitor`, `txadmin` or `txadminclient`.
    StartsMonitor,
    /// Sets `txAdminServerMode`: FXServer starts `monitor` by itself.
    SetsServerMode,
    SetsMonitorMode,
    /// Sets a convar Observer sets at each launch: Observer's value wins.
    SetsObserverConvar,
    /// `+set`, `+setr`, `+sets` or `+exec` inside a cfg.
    PlusCommand,
    /// FXServer stops at start without a key.
    LicenceMissing,
    /// A placeholder or a malformed key, as `licence` says.
    LicenceUnusable,
    /// `sv_maxclients` is not 1 to 2048.
    MaxClientsInvalid,
    /// Over 32 players while OneSync is off.
    MaxClientsNeedOneSync,
    /// `gamename` is neither `gta5` nor `rdr3`.
    GameInvalid,
    /// `gamename` set twice to different values.
    GameTwice,
    /// Started, but no resource, category or system resource of the selected build has its name.
    ResourceMissing,
    /// More missing resources past the first 20: `count` of them.
    ResourceMissingMore,
    /// A resource folder named `observer`, the name of Observer's bridge.
    ResourceNamedObserver,
    ExecMissing,
    ExecOutside,
    ExecResource,
    ExecTooDeep,
    /// A file run again: FXServer runs it again, and a loop never ends.
    ExecRepeated,
    /// txAdmin commented lines out; Observer leaves them as they are.
    EditedByTxAdmin,
}

impl FindingKind {
    pub const ALL: [Self; 27] = [
        Self::NoEndpoint,
        Self::EndpointInvalid,
        Self::EndpointOnPanel,
        Self::EndpointUnpaired,
        Self::EndpointDuplicate,
        Self::EndpointReserved,
        Self::PortInUse,
        Self::StartsMonitor,
        Self::SetsServerMode,
        Self::SetsMonitorMode,
        Self::SetsObserverConvar,
        Self::PlusCommand,
        Self::LicenceMissing,
        Self::LicenceUnusable,
        Self::MaxClientsInvalid,
        Self::MaxClientsNeedOneSync,
        Self::GameInvalid,
        Self::GameTwice,
        Self::ResourceMissing,
        Self::ResourceMissingMore,
        Self::ResourceNamedObserver,
        Self::ExecMissing,
        Self::ExecOutside,
        Self::ExecResource,
        Self::ExecTooDeep,
        Self::ExecRepeated,
        Self::EditedByTxAdmin,
    ];

    /// Blocking is what keeps FXServer from starting under Observer; the rest the owner may
    /// leave as it is.
    pub const fn severity(self) -> Severity {
        match self {
            Self::NoEndpoint | Self::EndpointInvalid | Self::EndpointOnPanel => Severity::Blocking,
            Self::EndpointUnpaired
            | Self::EndpointDuplicate
            | Self::EndpointReserved
            | Self::PortInUse
            | Self::StartsMonitor
            | Self::SetsServerMode
            | Self::SetsMonitorMode
            | Self::PlusCommand
            | Self::LicenceMissing
            | Self::LicenceUnusable
            | Self::MaxClientsInvalid
            | Self::MaxClientsNeedOneSync
            | Self::GameInvalid
            | Self::GameTwice
            | Self::ResourceMissing
            | Self::ResourceMissingMore
            | Self::ResourceNamedObserver
            | Self::ExecMissing
            | Self::ExecRepeated => Severity::Warning,
            Self::SetsObserverConvar
            | Self::ExecOutside
            | Self::ExecResource
            | Self::ExecTooDeep
            | Self::EditedByTxAdmin => Severity::Note,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_severity_of_each_kind_is_frozen() {
        let table: Vec<_> = FindingKind::ALL
            .into_iter()
            .map(|kind| (serde_json::to_value(kind).unwrap(), kind.severity()))
            .collect();

        let (blocking, warning, note) = (Severity::Blocking, Severity::Warning, Severity::Note);
        assert_eq!(
            table,
            [
                (json!("noEndpoint"), blocking),
                (json!("endpointInvalid"), blocking),
                (json!("endpointOnPanel"), blocking),
                (json!("endpointUnpaired"), warning),
                (json!("endpointDuplicate"), warning),
                (json!("endpointReserved"), warning),
                (json!("portInUse"), warning),
                (json!("startsMonitor"), warning),
                (json!("setsServerMode"), warning),
                (json!("setsMonitorMode"), warning),
                (json!("setsObserverConvar"), note),
                (json!("plusCommand"), warning),
                (json!("licenceMissing"), warning),
                (json!("licenceUnusable"), warning),
                (json!("maxClientsInvalid"), warning),
                (json!("maxClientsNeedOneSync"), warning),
                (json!("gameInvalid"), warning),
                (json!("gameTwice"), warning),
                (json!("resourceMissing"), warning),
                (json!("resourceMissingMore"), warning),
                (json!("resourceNamedObserver"), warning),
                (json!("execMissing"), warning),
                (json!("execOutside"), note),
                (json!("execResource"), note),
                (json!("execTooDeep"), note),
                (json!("execRepeated"), warning),
                (json!("editedByTxAdmin"), note),
            ]
        );
    }

    #[test]
    fn requests_are_camel_case_and_refuse_unknown_fields() {
        let folder = json!({ "dataPath": "/srv/fivem/server-data" });
        let cfg = json!({
            "dataPath": "/srv/fivem/server-data",
            "cfgFile": "server.cfg",
            "onesync": "legacy",
        });

        assert_eq!(
            serde_json::from_value::<FolderCheck>(folder.clone()).unwrap(),
            FolderCheck {
                data_path: "/srv/fivem/server-data".to_owned()
            }
        );
        assert_eq!(
            serde_json::from_value::<CfgCheck>(cfg.clone()).unwrap(),
            CfgCheck {
                data_path: "/srv/fivem/server-data".to_owned(),
                cfg_file: "server.cfg".to_owned(),
                onesync: OneSync::Legacy,
            }
        );
        let mut folder = folder;
        folder["cfgFile"] = json!("server.cfg");
        let mut cfg = cfg;
        cfg["autoStart"] = json!(true);
        assert!(serde_json::from_value::<FolderCheck>(folder).is_err());
        assert!(serde_json::from_value::<CfgCheck>(cfg).is_err());
    }

    #[test]
    fn reports_are_camel_case_and_leave_unset_params_out() {
        let folder = FolderReport {
            problem: Some(FolderProblem::IsResources),
            path_problem: None,
            suggestion: Some("/srv/fivem/server-data".to_owned()),
            resources: None,
            cfg_files: Vec::new(),
            detected_cfg: None,
        };
        let finding = CfgFinding {
            kind: FindingKind::EndpointOnPanel,
            severity: Severity::Blocking,
            file: "configs/net.cfg".to_owned(),
            line: Some(3),
            params: FindingParams {
                protocol: Some(EndpointProtocol::Tcp),
                address: Some("0.0.0.0:40125".to_owned()),
                ..FindingParams::default()
            },
        };
        let facts = CfgFacts {
            game: Some(Game::Rdr3),
            endpoints: vec![Endpoint {
                protocol: EndpointProtocol::Udp,
                address: "0.0.0.0:30140".to_owned(),
            }],
            max_clients: Some(48),
            onesync: Some(OneSync::On),
            hostname: Some("Lab".to_owned()),
            licence: LicenceState::Placeholder,
            game_build: Some(3095),
        };

        assert_eq!(
            serde_json::to_value(folder).unwrap(),
            json!({
                "problem": "isResources",
                "pathProblem": null,
                "suggestion": "/srv/fivem/server-data",
                "resources": null,
                "cfgFiles": [],
                "detectedCfg": null,
            })
        );
        assert_eq!(
            serde_json::to_value(finding).unwrap(),
            json!({
                "kind": "endpointOnPanel",
                "severity": "blocking",
                "file": "configs/net.cfg",
                "line": 3,
                "params": { "protocol": "tcp", "address": "0.0.0.0:40125" },
            })
        );
        assert_eq!(
            serde_json::to_value(facts).unwrap(),
            json!({
                "game": "rdr3",
                "endpoints": [{ "protocol": "udp", "address": "0.0.0.0:30140" }],
                "maxClients": 48,
                "onesync": "on",
                "hostname": "Lab",
                "licence": "placeholder",
                "gameBuild": 3095,
            })
        );
        assert_eq!(serde_json::to_value(Game::Gta5).unwrap(), "gta5");
        assert_eq!(serde_json::to_value(CfgFileState::TooDeep).unwrap(), "tooDeep");
    }
}
