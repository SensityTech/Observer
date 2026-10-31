//! Turns a build and launch settings into the exact process to start. The settings follow the
//! rules of `observer_domain::launch`, which refuse anything that would hand FXServer a command
//! Observer did not mean to give it; what is left here is the file system and the command line
//! itself.

use std::fs::File;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use observer_domain::launch::{self as rules, LaunchIssue, LaunchProblem, LaunchSettings, Platform};
use observer_domain::server::SpawnFailure;
use observer_domain::server_data::{CfgFinding, Severity};
use observer_platform::Launch;

use crate::cfg;
use crate::layout::{Flavour, Layout};
use crate::server_data;

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
    /// What the cfg does that works against Observer, notes left out. The setup refused what
    /// blocks when it saved; a cfg edited since still starts, as FXServer will say.
    pub warnings: Vec<CfgFinding>,
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

    // FXServer runs the whole cfg; the scan reads its first MiB, as it reads every file it runs.
    let mut bytes = Vec::new();
    File::open(&cfg_file)
        .and_then(|file| file.take(cfg::MAX_BYTES).read_to_end(&mut bytes))
        .map_err(|source| LaunchError::Cfg {
            path: cfg_file.clone(),
            source,
        })?;
    let text = String::from_utf8_lossy(&bytes);
    let context = cfg::Context {
        panel: None,
        onesync: settings.onesync,
        probe_ports: false,
        system_resources: Some(layout.system_resources()),
        follow_outside: true,
    };
    let resources = server_data::walk(&data);
    let scan = cfg::scan(&data, &settings.cfg_file, &text, &resources, &context);
    let warnings = scan
        .findings
        .into_iter()
        .filter(|finding| finding.severity != Severity::Note)
        .collect();
    let endpoint = scan.endpoint;

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

#[cfg(test)]
mod tests {
    use std::fs;

    use observer_domain::launch::{ExtraArgument, LaunchField, LaunchProblem, OneSync};
    use observer_domain::server_data::FindingKind;
    use tempfile::TempDir;

    use super::*;

    /// Two endpoints and a key: a cfg the scan has nothing to say about. The key is a test fixture,
    /// not a real one.
    const QUIET: &str = "endpoint_add_tcp \"0.0.0.0:30140\"\n\
                         endpoint_add_udp \"0.0.0.0:30140\"\n\
                         sv_licenseKey cfxk_ObserverLabFixture_k3y\n";

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
        let fixture = Fixture::new(&format!("{QUIET}sv_hostname \"Observer lab\"\n"));
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

        let warnings: Vec<_> = prepared
            .warnings
            .iter()
            .map(|finding| (finding.kind, finding.file.as_str(), finding.line))
            .collect();
        // Setting Observer's own convars is a note now, which the launch leaves out; the file a
        // resource holds may add the endpoints, so their absence only warns.
        assert_eq!(
            warnings,
            [
                (FindingKind::StartsMonitor, "server.cfg", Some(2)),
                (FindingKind::SetsServerMode, "server.cfg", Some(3)),
                (FindingKind::ExecMissing, "server.cfg", Some(5)),
                (FindingKind::NoEndpoint, "server.cfg", None),
            ]
        );
        assert!(
            prepared
                .warnings
                .iter()
                .all(|finding| finding.severity == Severity::Warning)
        );
    }

    #[test]
    fn a_cfg_that_execs_itself_is_read_once() {
        let fixture = Fixture::new(&format!("{QUIET}exec server.cfg\nstart monitor\n"));

        let prepared = prepare(&fixture.launch(), Flavour::HOST).unwrap();

        let kinds: Vec<_> = prepared.warnings.iter().map(|finding| finding.kind).collect();
        assert_eq!(kinds, [FindingKind::ExecRepeated, FindingKind::StartsMonitor]);
    }

    #[test]
    fn a_cfg_named_by_the_settings_is_read_even_when_it_is_not_utf8() {
        let fixture = Fixture::new("");
        fs::write(
            fixture.data().join("server.cfg"),
            [QUIET.as_bytes(), b"sv_hostname \"Caf\xe9\"\nensure monitor\n"].concat(),
        )
        .unwrap();

        let prepared = prepare(&fixture.launch(), Flavour::HOST).unwrap();

        let kinds: Vec<_> = prepared.warnings.iter().map(|finding| finding.kind).collect();
        assert_eq!(kinds, [FindingKind::StartsMonitor]);
    }

    #[test]
    fn a_cfg_is_scanned_up_to_its_first_mib() {
        let fixture = Fixture::new(&format!(
            "{QUIET}{}ensure monitor\n",
            "# padding\n".repeat(110_000)
        ));

        let prepared = prepare(&fixture.launch(), Flavour::HOST).unwrap();

        assert_eq!(prepared.warnings, []);
        assert_eq!(prepared.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30140))));
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
    }

    #[test]
    fn the_endpoint_and_warnings_come_from_files_outside_the_data_folder_too() {
        let fixture = Fixture::new("exec ../shared.cfg\n");
        fs::write(
            fixture.dir.path().join("shared.cfg"),
            format!("{QUIET}ensure monitor\n"),
        )
        .unwrap();

        let prepared = prepare(&fixture.launch(), Flavour::HOST).unwrap();

        let warnings: Vec<_> = prepared
            .warnings
            .iter()
            .map(|finding| (finding.kind, finding.file.as_str(), finding.line))
            .collect();
        assert_eq!(warnings, [(FindingKind::StartsMonitor, "../shared.cfg", Some(4))]);
        assert_eq!(prepared.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30140))));
    }
}
