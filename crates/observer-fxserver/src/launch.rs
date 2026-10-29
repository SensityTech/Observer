//! Turns a build and launch settings into the exact process to start. The settings follow the
//! rules of `observer_domain::launch`, which refuse anything that would hand FXServer a command
//! Observer did not mean to give it; what is left here is the file system and the command line
//! itself.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use observer_domain::launch::{self as rules, LaunchIssue, LaunchProblem, LaunchSettings, Platform};
use observer_domain::server::SpawnFailure;
use observer_platform::Launch;
use serde::Serialize;

use crate::command;
use crate::layout::{Flavour, Layout};

/// The convars Observer sets on every launch, through argv or stdin.
const OBSERVER_CONVARS: [&str; 5] = [
    "onesync",
    "observer_txadmin_events",
    "sv_prometheusBasicAuthUser",
    "sv_prometheusBasicAuthPassword",
    "sv_playersToken",
];

/// How deep `exec` lines are followed when the cfg is scanned for warnings.
const EXEC_DEPTH: usize = 5;

/// What the cfg does that works against Observer. The cfg is the user's file: Observer reads it
/// and says so, it never rewrites it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CfgWarning {
    /// Starts txAdmin's `monitor` resource, which expects txAdmin to run the server.
    StartsMonitor { file: PathBuf, line: usize },
    /// Sets `txAdminServerMode`, which makes FXServer start `monitor` by itself.
    SetsServerMode { file: PathBuf, line: usize },
    /// Sets a convar Observer sets too; Observer's value wins.
    SetsObserverConvar {
        name: String,
        file: PathBuf,
        line: usize,
    },
    /// A file named by `exec` could not be read, so it was not checked.
    Unreadable { file: PathBuf },
}

/// What a spawn starts: a build folder, and how FXServer runs from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLaunch {
    /// The folder of `FXServer.exe` on Windows, the one holding `alpine` on Linux.
    pub build: PathBuf,
    pub settings: LaunchSettings,
}

#[derive(Debug)]
pub struct Prepared {
    pub launch: Launch,
    pub layout: Layout,
    pub warnings: Vec<CfgWarning>,
    /// The first `endpoint_add_tcp` of the cfg, made reachable from this host.
    pub endpoint: Option<SocketAddr>,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("the launch settings are refused: {}", describe(.0))]
    Invalid(Vec<LaunchIssue>),
    #[error("the build folder {} is refused: {}", path.display(), name(serde_json::to_value(problem)))]
    BuildFolder { path: PathBuf, problem: LaunchProblem },
    #[error("cannot resolve {}: {source}", path.display())]
    Resolve {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not an FXServer build: {} is missing", build.display(), program.display())]
    NotABuild { build: PathBuf, program: PathBuf },
    #[error("the server data folder {} does not exist", .0.display())]
    MissingData(PathBuf),
    #[error("cannot read the server cfg {}: {source}", path.display())]
    Cfg {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl LaunchError {
    /// Which step failed, without the folders the message names.
    pub fn failure(&self) -> SpawnFailure {
        match self {
            Self::Invalid(_) | Self::BuildFolder { .. } | Self::Resolve { .. } => {
                SpawnFailure::InvalidSettings
            }
            Self::NotABuild { .. } => SpawnFailure::NotABuild,
            Self::MissingData(_) => SpawnFailure::MissingData,
            Self::Cfg { .. } => SpawnFailure::CfgUnreadable,
        }
    }
}

fn name(value: serde_json::Result<serde_json::Value>) -> String {
    match value {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// `dataPath notAbsolute, argumentName 2 reserved`: the issues as the API names them.
fn describe(issues: &[LaunchIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            let field = name(serde_json::to_value(issue.field));
            let problem = name(serde_json::to_value(issue.problem));
            match issue.index {
                Some(index) => format!("{field} {} {problem}", index + 1),
                None => format!("{field} {problem}"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Builds the launch: the flavour's program, then `+ensure observer`, `+exec <cfg>`, the extra
/// arguments and Observer's convars last, so that they win over the cfg.
pub fn prepare(launch: &ServerLaunch, flavour: Flavour) -> Result<Prepared, LaunchError> {
    let platform = match flavour {
        Flavour::Windows => Platform::Windows,
        Flavour::Linux => Platform::Unix,
    };
    let settings = &launch.settings;
    let issues = settings.issues(platform);
    if !issues.is_empty() {
        return Err(LaunchError::Invalid(issues));
    }
    let build = resolve(&launch.build)?;
    // On Linux the folder reaches FXServer's command line inside `+set citizen_dir`.
    let problem = match build.to_str() {
        Some(text) => rules::folder_problem(text, platform),
        None => Some(LaunchProblem::ControlCharacter),
    };
    if let Some(problem) = problem {
        return Err(LaunchError::BuildFolder { path: build, problem });
    }
    let data = resolve(Path::new(&settings.data_path))?;
    if !data.is_dir() {
        return Err(LaunchError::MissingData(data));
    }
    let cfg_file = data.join(&settings.cfg_file);

    let layout = Layout::new(build, flavour);
    let (program, mut args) = layout.program();
    if !program.is_file() {
        return Err(LaunchError::NotABuild {
            build: layout.build().to_path_buf(),
            program,
        });
    }

    args.extend(["+ensure".into(), "observer".into(), "+exec".into()]);
    args.push(settings.cfg_file.clone().into());
    for argument in &settings.extra_arguments {
        args.extend([
            "+set".into(),
            argument.name.clone().into(),
            argument.value.clone().into(),
        ]);
    }
    args.extend([
        "+set".into(),
        "onesync".into(),
        settings.onesync.as_str().into(),
        "+set".into(),
        "observer_txadmin_events".into(),
        if settings.txadmin_events { "1" } else { "0" }.into(),
    ]);

    let text = fs::read_to_string(&cfg_file).map_err(|source| LaunchError::Cfg {
        path: cfg_file.clone(),
        source,
    })?;
    let mut scan = Scan {
        data: &data,
        warnings: Vec::new(),
        endpoint: None,
        seen: HashSet::new(),
    };
    scan.file(&cfg_file, &text, 0);
    let (warnings, endpoint) = (scan.warnings, scan.endpoint);

    Ok(Prepared {
        launch: Launch {
            program,
            args,
            cwd: data,
        },
        layout,
        warnings,
        endpoint,
    })
}

fn resolve(path: &Path) -> Result<PathBuf, LaunchError> {
    std::path::absolute(path).map_err(|source| LaunchError::Resolve {
        path: path.to_path_buf(),
        source,
    })
}

struct Scan<'a> {
    data: &'a Path,
    warnings: Vec<CfgWarning>,
    endpoint: Option<SocketAddr>,
    seen: HashSet<PathBuf>,
}

impl Scan<'_> {
    fn file(&mut self, path: &Path, text: &str, depth: usize) {
        if !self.seen.insert(path.to_path_buf()) {
            return;
        }
        for (index, line) in text.lines().enumerate() {
            for command in command::split(line) {
                self.command(path, index + 1, &command::words(command), depth);
            }
        }
    }

    fn command(&mut self, file: &Path, line: usize, words: &[String], depth: usize) {
        let Some((name, args)) = words.split_first() else {
            return;
        };
        let name = name.to_ascii_lowercase();
        let first = args.first().map(String::as_str);
        // A convar is set by `set` and its variants, or by typing its name with a value.
        let set = match name.as_str() {
            "set" | "sets" | "setr" | "seta" => first,
            _ if first.is_some() => Some(words[0].as_str()),
            _ => None,
        };
        match (name.as_str(), first) {
            ("start" | "ensure" | "restart", Some(resource)) if resource.eq_ignore_ascii_case("monitor") => {
                self.warnings.push(CfgWarning::StartsMonitor {
                    file: file.to_path_buf(),
                    line,
                });
            }
            ("endpoint_add_tcp", Some(address)) if self.endpoint.is_none() => {
                self.endpoint = reachable(address);
            }
            ("exec", Some(target)) if depth < EXEC_DEPTH && !target.starts_with('@') => {
                let path = self.data.join(target);
                match fs::read_to_string(&path) {
                    Ok(text) => self.file(&path, &text, depth + 1),
                    Err(_) => self.warnings.push(CfgWarning::Unreadable { file: path }),
                }
            }
            _ => {}
        }
        let Some(convar) = set else {
            return;
        };
        if convar.eq_ignore_ascii_case("txAdminServerMode") {
            self.warnings.push(CfgWarning::SetsServerMode {
                file: file.to_path_buf(),
                line,
            });
        } else if let Some(name) = OBSERVER_CONVARS
            .iter()
            .find(|name| name.eq_ignore_ascii_case(convar))
        {
            self.warnings.push(CfgWarning::SetsObserverConvar {
                name: (*name).to_owned(),
                file: file.to_path_buf(),
                line,
            });
        }
    }
}

/// FXServer listens on the address it is given; a wildcard is reached through loopback.
fn reachable(address: &str) -> Option<SocketAddr> {
    let mut address: SocketAddr = address.parse().ok()?;
    match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => address.set_ip(Ipv4Addr::LOCALHOST.into()),
        IpAddr::V6(ip) if ip.is_unspecified() => address.set_ip(Ipv6Addr::LOCALHOST.into()),
        _ => {}
    }
    Some(address)
}

#[cfg(test)]
mod tests {
    use observer_domain::launch::{ExtraArgument, LaunchField, LaunchProblem, OneSync};
    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        dir: TempDir,
    }

    impl Fixture {
        fn new(cfg: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let (program, _) = Layout::new(dir.path().join("build"), Flavour::HOST).program();
            fs::create_dir_all(program.parent().unwrap()).unwrap();
            fs::write(&program, b"").unwrap();
            fs::create_dir_all(dir.path().join("data")).unwrap();
            fs::write(dir.path().join("data/server.cfg"), cfg).unwrap();
            Self { dir }
        }

        fn path(&self, name: &str) -> String {
            self.dir.path().join(name).to_str().unwrap().to_owned()
        }

        fn launch(&self) -> ServerLaunch {
            ServerLaunch {
                build: self.dir.path().join("build"),
                settings: self.settings(),
            }
        }

        fn settings(&self) -> LaunchSettings {
            LaunchSettings {
                data_path: self.path("data"),
                cfg_file: "server.cfg".to_owned(),
                onesync: OneSync::Off,
                auto_start: false,
                txadmin_events: false,
                extra_arguments: Vec::new(),
            }
        }

        fn file(&self, name: &str, text: &str) {
            fs::write(self.dir.path().join("data").join(name), text).unwrap();
        }

        fn data(&self) -> PathBuf {
            std::path::absolute(self.dir.path().join("data")).unwrap()
        }
    }

    /// The arguments after the flavour's own prefix.
    fn server_args(prepared: &Prepared) -> Vec<String> {
        let prefix = prepared.layout.program().1.len();
        prepared.launch.args[prefix..]
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn refused(launch: &ServerLaunch) -> Option<LaunchError> {
        prepare(launch, Flavour::HOST).err()
    }

    fn with(fixture: &Fixture, settings: LaunchSettings) -> ServerLaunch {
        ServerLaunch {
            settings,
            ..fixture.launch()
        }
    }

    #[test]
    fn the_bridge_and_the_cfg_come_first_and_observer_convars_last() {
        let fixture = Fixture::new("sv_hostname \"Observer lab\"\n");
        let settings = LaunchSettings {
            onesync: OneSync::Legacy,
            txadmin_events: true,
            extra_arguments: vec![ExtraArgument {
                name: "sv_enforceGameBuild".to_owned(),
                value: "3258".to_owned(),
            }],
            ..fixture.settings()
        };

        let prepared = prepare(&with(&fixture, settings), Flavour::HOST).unwrap();

        assert_eq!(
            server_args(&prepared),
            [
                "+ensure",
                "observer",
                "+exec",
                "server.cfg",
                "+set",
                "sv_enforceGameBuild",
                "3258",
                "+set",
                "onesync",
                "legacy",
                "+set",
                "observer_txadmin_events",
                "1",
            ]
        );
        assert_eq!(prepared.launch.cwd, fixture.data());
        assert_eq!(prepared.launch.program, prepared.layout.program().0);
        assert_eq!(prepared.warnings, []);
    }

    /// Each rule has its tests in `observer_domain::launch`; here, that the builder applies them
    /// before it touches anything.
    #[test]
    fn settings_the_rules_refuse_never_reach_the_command_line() {
        let fixture = Fixture::new("");
        let settings = LaunchSettings {
            cfg_file: "server.cfg;quit".to_owned(),
            extra_arguments: vec![
                ExtraArgument {
                    name: "sv_projectDesc".to_owned(),
                    value: "+quit".to_owned(),
                },
                ExtraArgument {
                    name: "rcon_password".to_owned(),
                    value: "hunter2".to_owned(),
                },
            ],
            ..fixture.settings()
        };

        let Some(LaunchError::Invalid(issues)) = refused(&with(&fixture, settings)) else {
            panic!("the settings were not refused as invalid");
        };

        let found: Vec<_> = issues
            .iter()
            .map(|issue| (issue.field, issue.index, issue.problem))
            .collect();
        assert_eq!(
            found,
            [
                (LaunchField::CfgFile, None, LaunchProblem::InvalidCfgFile),
                (LaunchField::ArgumentValue, Some(0), LaunchProblem::OptionLike),
                (LaunchField::ArgumentName, Some(1), LaunchProblem::Secret),
            ]
        );
        assert_eq!(
            LaunchError::Invalid(issues).to_string(),
            "the launch settings are refused: cfgFile invalidCfgFile, argumentValue 1 optionLike, \
             argumentName 2 secret"
        );
    }

    #[test]
    fn a_folder_without_fxserver_is_not_a_build() {
        let fixture = Fixture::new("");
        let launch = ServerLaunch {
            build: fixture.data(),
            ..fixture.launch()
        };

        assert!(matches!(refused(&launch), Some(LaunchError::NotABuild { .. })));
    }

    #[test]
    fn a_build_folder_the_command_line_would_misread_is_refused() {
        let fixture = Fixture::new("");
        let launch = ServerLaunch {
            build: fixture.dir.path().join("build;quit"),
            ..fixture.launch()
        };

        let refusal = refused(&launch);

        assert!(
            matches!(
                refusal,
                Some(LaunchError::BuildFolder {
                    problem: LaunchProblem::QuoteOrSemicolon | LaunchProblem::ForbiddenCharacter,
                    ..
                })
            ),
            "{refusal:?}"
        );
    }

    #[test]
    fn the_data_folder_and_the_cfg_must_exist() {
        let fixture = Fixture::new("");
        let missing_data = LaunchSettings {
            data_path: fixture.path("nowhere"),
            ..fixture.settings()
        };
        let missing_cfg = LaunchSettings {
            cfg_file: "other.cfg".to_owned(),
            ..fixture.settings()
        };

        assert!(matches!(
            refused(&with(&fixture, missing_data)),
            Some(LaunchError::MissingData(_))
        ));
        assert!(matches!(
            refused(&with(&fixture, missing_cfg)),
            Some(LaunchError::Cfg { .. })
        ));
    }

    #[test]
    fn the_cfg_is_scanned_for_what_works_against_observer() {
        let fixture = Fixture::new(
            "exec license.cfg\n\
             ensure monitor\n\
             set txAdminServerMode true\n\
             setr sv_prometheusBasicAuthPassword \"mine\"; onesync on\n\
             exec missing.cfg\n\
             exec @chat/extra.cfg\n",
        );
        fixture.file("license.cfg", "sv_playersToken abc\n# start monitor\n");

        let prepared = prepare(&fixture.launch(), Flavour::HOST).unwrap();

        let cfg = fixture.data().join("server.cfg");
        assert_eq!(
            prepared.warnings,
            [
                CfgWarning::SetsObserverConvar {
                    name: "sv_playersToken".to_owned(),
                    file: fixture.data().join("license.cfg"),
                    line: 1
                },
                CfgWarning::StartsMonitor {
                    file: cfg.clone(),
                    line: 2
                },
                CfgWarning::SetsServerMode {
                    file: cfg.clone(),
                    line: 3
                },
                CfgWarning::SetsObserverConvar {
                    name: "sv_prometheusBasicAuthPassword".to_owned(),
                    file: cfg.clone(),
                    line: 4
                },
                CfgWarning::SetsObserverConvar {
                    name: "onesync".to_owned(),
                    file: cfg,
                    line: 4
                },
                CfgWarning::Unreadable {
                    file: fixture.data().join("missing.cfg")
                },
            ]
        );
    }

    #[test]
    fn a_cfg_that_execs_itself_is_read_once() {
        let fixture = Fixture::new("exec server.cfg\nstart monitor\n");

        let prepared = prepare(&fixture.launch(), Flavour::HOST).unwrap();

        assert_eq!(prepared.warnings.len(), 1);
    }

    #[test]
    fn the_endpoint_comes_from_the_first_tcp_endpoint_of_the_cfg() {
        let fixture = Fixture::new(
            "endpoint_add_udp \"127.0.0.1:30141\"\n\
             endpoint_add_tcp \"0.0.0.0:30140\"\n\
             endpoint_add_tcp \"127.0.0.1:30149\"\n",
        );

        let prepared = prepare(&fixture.launch(), Flavour::HOST).unwrap();

        assert_eq!(prepared.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30140))));
        assert_eq!(reachable("[::]:30140"), Some("[::1]:30140".parse().unwrap()));
        assert_eq!(reachable("not an address"), None);
    }
}
