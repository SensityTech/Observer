//! Observer's optional features: what each one runs and reaches while it is on, and the view of
//! their switches. Supervision, the console and its logs, accounts, the audit log, the restart
//! schedule and FXServer's builds are always there; any of these can be turned off, and then none
//! of its code runs: no port, no task, no route.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::live::Topic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Feature {
    /// File access over SFTP, on the port `[sftp] listen` of `observer.toml` names.
    Sftp,
    /// What changed in the server folder from one start of FXServer to the next.
    Changes,
    /// The server tools, such as the duplicate archetypes fix, run by hand or at every start.
    Tools,
    /// The processes, connections, crashes and hangs of FXServer's tree.
    Diagnostics,
    /// The statistics: a sample every five seconds, and the charts drawn from them.
    Metrics,
    /// Players' sessions, play time and identifiers.
    Players,
    /// The statistics of player drops, drawn from the players' sessions.
    Drops,
}

/// When a change of a switch takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Applies {
    Now,
    /// Its routes at once, its work from the next start of FXServer, which is when it runs.
    NextServerStart,
    /// At the next start of Observer, which also restarts FXServer.
    ObserverRestart,
}

/// Why a feature does not run although the owner left it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FeatureHold {
    /// `observer.toml` turns it off, which the panel cannot undo.
    Host,
    /// It needs a setting `observer.toml` does not give: SFTP's `listen`.
    NotConfigured,
    /// It works on what the feature in `requires` collects, which does not run.
    Requires,
}

/// What a feature does in the background, for the panel to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FeatureTask {
    /// Accepts SFTP connections on its port.
    SftpServer,
    /// Audits closed windows of failed SFTP logins.
    SftpUpkeep,
    /// Removes the temporary uploads a crash left in the server folder.
    UploadsSweep,
    /// Lists and hashes the server folder; a check does it on request too.
    Snapshots,
    /// The retention and the budget of the stored texts.
    ChangesUpkeep,
    /// What the tools' settings ask for before a start: a report or a fix.
    ToolsAtStart,
    /// While a fix of the tools is still applied: puts back what it took out once the copy it
    /// kept is gone.
    ToolsHeal,
    /// The processes FXServer's tree starts and ends.
    ProcessRecords,
    /// Where the tree's processes connect and listen.
    Connections,
    /// Reads the dumps of a crash.
    CrashAnalysis,
    /// Writes a dump of a hung server before it is killed.
    HangSnapshot,
    /// The retention of the records.
    DiagnosticsUpkeep,
    /// The process, host and FXServer measures.
    Sampler,
    /// Rolls the samples up and applies their retention.
    MetricsUpkeep,
    /// Players' sessions from the bridge and `/players.json`, and their retention.
    PlayersFollower,
    /// The drop statistics, from the sessions that ended.
    DropsWorker,
}

/// When a task of a feature runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Cadence {
    Every {
        seconds: u32,
    },
    /// All along, waiting for what comes: a connection, an event of the server.
    Continuous,
    ObserverStart,
    /// At every start of FXServer.
    ServerStart,
    /// After a crash of FXServer.
    Crash,
    /// When FXServer hangs.
    Hang,
}

/// A task of a feature and when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct FeatureTaskRun {
    pub task: FeatureTask,
    pub cadence: Cadence,
}

/// What a feature does with the server folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServerFolder {
    Untouched,
    Reads,
    Writes,
}

impl Feature {
    pub const ALL: [Self; 7] = [
        Self::Sftp,
        Self::Changes,
        Self::Tools,
        Self::Diagnostics,
        Self::Metrics,
        Self::Players,
        Self::Drops,
    ];

    /// The name on the wire, in the store and in `observer.toml`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sftp => "sftp",
            Self::Changes => "changes",
            Self::Tools => "tools",
            Self::Diagnostics => "diagnostics",
            Self::Metrics => "metrics",
            Self::Players => "players",
            Self::Drops => "drops",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|feature| feature.as_str() == name)
    }

    /// The feature whose records this one works on, which must run for this one to.
    pub const fn requires(self) -> Option<Self> {
        match self {
            Self::Drops => Some(Self::Players),
            _ => None,
        }
    }

    /// When turning it on takes effect. The tools' hook reads the switch before every start;
    /// the others follow FXServer from Observer's start, and a port opens only when the host
    /// restarts Observer.
    pub const fn turn_on(self) -> Applies {
        match self {
            Self::Tools => Applies::NextServerStart,
            _ => Applies::ObserverRestart,
        }
    }

    /// When turning it off takes effect: at once, for every feature.
    pub const fn turn_off(self) -> Applies {
        Applies::Now
    }

    pub const fn tasks(self) -> &'static [FeatureTaskRun] {
        use Cadence as C;
        use FeatureTask as T;
        const fn run(task: FeatureTask, cadence: Cadence) -> FeatureTaskRun {
            FeatureTaskRun { task, cadence }
        }
        const HOURLY: Cadence = C::Every { seconds: 3600 };
        const EACH_MINUTE: Cadence = C::Every { seconds: 60 };
        const SFTP: &[FeatureTaskRun] = &[
            run(T::SftpServer, C::Continuous),
            run(T::SftpUpkeep, EACH_MINUTE),
            run(T::UploadsSweep, C::ObserverStart),
        ];
        const CHANGES: &[FeatureTaskRun] =
            &[run(T::Snapshots, C::ServerStart), run(T::ChangesUpkeep, HOURLY)];
        const TOOLS: &[FeatureTaskRun] = &[
            run(T::ToolsAtStart, C::ServerStart),
            run(T::ToolsHeal, C::ServerStart),
        ];
        const DIAGNOSTICS: &[FeatureTaskRun] = &[
            run(T::ProcessRecords, C::Continuous),
            run(T::Connections, C::Every { seconds: 2 }),
            run(T::CrashAnalysis, C::Crash),
            run(T::HangSnapshot, C::Hang),
            run(T::DiagnosticsUpkeep, HOURLY),
        ];
        const METRICS: &[FeatureTaskRun] = &[
            run(T::Sampler, C::Every { seconds: 5 }),
            run(T::MetricsUpkeep, EACH_MINUTE),
        ];
        const PLAYERS: &[FeatureTaskRun] = &[run(T::PlayersFollower, C::Continuous)];
        const DROPS: &[FeatureTaskRun] = &[run(T::DropsWorker, EACH_MINUTE)];
        match self {
            Self::Sftp => SFTP,
            Self::Changes => CHANGES,
            Self::Tools => TOOLS,
            Self::Diagnostics => DIAGNOSTICS,
            Self::Metrics => METRICS,
            Self::Players => PLAYERS,
            Self::Drops => DROPS,
        }
    }

    /// What goes on while the feature is off, and only while it has something to finish.
    pub const fn while_off(self) -> &'static [FeatureTaskRun] {
        match self {
            // A fix took entries out of a file because another copy kept them; once that copy
            // is gone, the entries must come back or the game misses them.
            Self::Tools => &[FeatureTaskRun {
                task: FeatureTask::ToolsHeal,
                cadence: Cadence::ServerStart,
            }],
            _ => &[],
        }
    }

    /// The files and folders it keeps, under Observer's data folder.
    pub const fn files(self) -> &'static [&'static str] {
        match self {
            Self::Sftp => &[],
            Self::Changes => &["changes.sqlite3"],
            Self::Tools => &["tools/backups"],
            Self::Diagnostics => &["crashes"],
            Self::Metrics | Self::Players | Self::Drops => &["metrics.sqlite3"],
        }
    }

    pub const fn server_folder(self) -> ServerFolder {
        match self {
            Self::Sftp | Self::Tools => ServerFolder::Writes,
            Self::Changes => ServerFolder::Reads,
            Self::Diagnostics | Self::Metrics | Self::Players | Self::Drops => ServerFolder::Untouched,
        }
    }

    /// The API paths it answers, relative to `/api`: each route under one of them is its.
    pub const fn routes(self) -> &'static [&'static str] {
        match self {
            Self::Sftp => &["/files", "/account/files", "/servers/default/files"],
            Self::Changes => &["/servers/default/changes"],
            Self::Tools => &["/tools"],
            Self::Diagnostics => &["/servers/default/crashes"],
            Self::Metrics => &["/metrics"],
            Self::Players => &["/servers/default/players"],
            Self::Drops => &["/servers/default/drops"],
        }
    }

    pub const fn topics(self) -> &'static [Topic] {
        match self {
            Self::Changes => &[Topic::ServerChanges],
            Self::Diagnostics => &[Topic::ServerDiagnostics, Topic::ServerAlerts],
            Self::Metrics => &[Topic::MetricsLive],
            Self::Sftp | Self::Tools | Self::Players | Self::Drops => &[],
        }
    }

    /// The feature an API route belongs to, from its path relative to `/api`.
    pub fn of_route(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|feature| {
            feature.routes().iter().any(|prefix| {
                path.strip_prefix(prefix)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            })
        })
    }

    pub fn of_topic(topic: Topic) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|feature| feature.topics().contains(&topic))
    }
}

/// `GET /api/features`, and the answer to a change: every feature, in a fixed order.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct FeatureList {
    pub features: Vec<FeatureView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FeatureView {
    pub feature: Feature,
    /// The owner's choice, or the default when they never made one: on.
    pub enabled: bool,
    /// Whether it runs in this Observer now: turning it off stops it at once.
    pub running: bool,
    pub held: Option<FeatureHold>,
    pub requires: Option<Feature>,
    /// When turning it on takes effect.
    pub turn_on: Applies,
    pub turn_off: Applies,
    /// When the owner last turned it on or off.
    pub changed_at: Option<Timestamp>,
    pub reach: FeatureReach,
}

/// What a feature opens and runs while it is on.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FeatureReach {
    /// The addresses it listens on, as `observer.toml` gives them.
    pub listens: Vec<String>,
    pub tasks: Vec<FeatureTaskRun>,
    pub while_off: Vec<FeatureTaskRun>,
    pub files: Vec<FeatureFile>,
    pub server_folder: ServerFolder,
    /// API path prefixes, relative to `/api`.
    pub routes: Vec<String>,
    pub topics: Vec<Topic>,
}

/// A file or a folder a feature keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct FeatureFile {
    /// Under Observer's data folder, with `/` between folders.
    pub path: String,
    /// What it holds on disk, its journal included; `None` when it is not there yet.
    pub bytes: Option<u64>,
}

impl FeatureReach {
    /// What `feature` reaches, with the size on disk `size` gives each of its files.
    pub fn of(feature: Feature, listens: Vec<String>, size: impl Fn(&str) -> Option<u64>) -> Self {
        Self {
            listens,
            tasks: feature.tasks().to_vec(),
            while_off: feature.while_off().to_vec(),
            files: feature
                .files()
                .iter()
                .map(|&path| FeatureFile {
                    path: path.to_owned(),
                    bytes: size(path),
                })
                .collect(),
            server_folder: feature.server_folder(),
            routes: feature.routes().iter().map(|&route| route.to_owned()).collect(),
            topics: feature.topics().to_vec(),
        }
    }
}

/// `PUT /api/features/{feature}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct FeatureSwitch {
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_names_match_the_wire_names() {
        for feature in Feature::ALL {
            let wire = serde_json::to_value(feature).unwrap();
            assert_eq!(wire, feature.as_str());
            assert_eq!(Feature::from_name(feature.as_str()), Some(feature));
        }
    }

    #[test]
    fn a_route_belongs_to_the_feature_whose_prefix_it_extends_by_whole_segments() {
        assert_eq!(Feature::of_route("/tools"), Some(Feature::Tools));
        assert_eq!(
            Feature::of_route("/tools/duplicate-archetypes/fix"),
            Some(Feature::Tools)
        );
        assert_eq!(Feature::of_route("/account/files/keys/{id}"), Some(Feature::Sftp));
        assert_eq!(
            Feature::of_route("/servers/default/drops/log.csv"),
            Some(Feature::Drops)
        );
        assert_eq!(Feature::of_route("/toolsets"), None);
        assert_eq!(Feature::of_route("/account/sessions"), None);
        assert_eq!(Feature::of_route("/servers/default/launch"), None);
    }

    #[test]
    fn every_topic_belongs_to_one_feature_at_most_and_core_topics_to_none() {
        for topic in Topic::ALL {
            let owners = Feature::ALL
                .into_iter()
                .filter(|feature| feature.topics().contains(&topic))
                .count();
            assert!(owners <= 1, "{topic:?}");
        }
        for core in [Topic::Status, Topic::Audit, Topic::ServerStatus, Topic::Console] {
            assert_eq!(Feature::of_topic(core), None);
        }
    }

    #[test]
    fn a_feature_requires_only_one_that_requires_nothing() {
        for feature in Feature::ALL {
            if let Some(required) = feature.requires() {
                assert_ne!(required, feature);
                assert_eq!(required.requires(), None, "{feature:?}");
            }
        }
    }
}
