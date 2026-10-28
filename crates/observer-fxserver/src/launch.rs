//! Turns launch settings into the exact process to start, refusing anything that would hand
//! FXServer a command Observer did not mean to give it.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use observer_platform::Launch;
use serde::Serialize;

use crate::command;
use crate::layout::{Flavour, Layout};

/// Names an extra argument may not set: they switch FXServer into txAdmin's modes, run other
/// files, move the citizen folder, or are set by Observer itself (some through stdin, as secrets).
const DENIED: [&str; 13] = [
    "exec",
    "citizen_dir",
    "txAdminServerMode",
    "monitorMode",
    "serverProfile",
    "txAdminPort",
    "txDataPath",
    "txAdminInterface",
    "onesync",
    "observer_txadmin_events",
    "sv_prometheusBasicAuthUser",
    "sv_prometheusBasicAuthPassword",
    "sv_playersToken",
];

/// The convars Observer sets on every launch, through argv or stdin.
const OBSERVER_CONVARS: [&str; 4] = [
    "observer_txadmin_events",
    "sv_prometheusBasicAuthUser",
    "sv_prometheusBasicAuthPassword",
    "sv_playersToken",
];

/// How deep `exec` lines are followed when the cfg is scanned for warnings.
const EXEC_DEPTH: usize = 5;

#[derive(Debug, Clone)]
pub struct LaunchSettings {
    pub build: PathBuf,
    /// The server data folder: FXServer's working directory, where the cfg and resources live.
    pub data: PathBuf,
    /// Relative to `data` unless absolute.
    pub cfg: PathBuf,
    pub onesync: Option<OneSync>,
    /// Lets the bridge also raise txAdmin's events, for resources written against txAdmin.
    pub txadmin_events: bool,
    /// Rendered as `+set <name> <value>`.
    pub extra: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OneSync {
    On,
    Legacy,
    Off,
}

impl OneSync {
    fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Legacy => "legacy",
            Self::Off => "off",
        }
    }
}

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
    #[error("{} is a network, device or verbatim path, which FXServer cannot be run from", .0.display())]
    RefusedPath(PathBuf),
    #[error("{} would read to FXServer as a command: it starts with `+` or `-`, or holds a quote, a semicolon or a line break", .0.display())]
    UnsafePath(PathBuf),
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
    #[error("`{0}` is not a convar name: use 1 to 64 letters, digits or underscores")]
    BadName(String),
    #[error("the value given for `{0}` contains a quote, a semicolon, a backslash or a control character")]
    BadValue(String),
    #[error(
        "the value given for `{0}` starts with `+` or `-`, which FXServer reads as a command or an option"
    )]
    CommandValue(String),
    #[error("`{0}` cannot be set as an extra argument: Observer manages it or it changes how FXServer runs")]
    DeniedName(String),
    #[error("`-fxdk` is not allowed: it starts FXServer as a development kit")]
    Fxdk,
}

/// Builds the launch: the flavour's program, then `+ensure observer`, `+exec <cfg>`, the extra
/// arguments and Observer's convars last, so that they win over the cfg.
pub fn prepare(settings: &LaunchSettings, flavour: Flavour) -> Result<Prepared, LaunchError> {
    let build = resolve(&settings.build)?;
    let data = resolve(&settings.data)?;
    if !data.is_dir() {
        return Err(LaunchError::MissingData(data));
    }
    refuse_special(&settings.cfg)?;
    refuse_commands(&settings.cfg)?;
    let cfg_file = data.join(&settings.cfg);

    let layout = Layout::new(build, flavour);
    let (program, mut args) = layout.program();
    if !program.is_file() {
        return Err(LaunchError::NotABuild {
            build: layout.build().to_path_buf(),
            program,
        });
    }
    refuse_commands(layout.citizen_root().as_path())?;

    args.extend(["+ensure".into(), "observer".into(), "+exec".into()]);
    args.push(settings.cfg.clone().into_os_string());
    for (name, value) in &settings.extra {
        check_extra(name, value)?;
        args.extend(["+set".into(), name.into(), value.into()]);
    }
    if let Some(onesync) = settings.onesync {
        args.extend(["+set".into(), "onesync".into(), onesync.as_str().into()]);
    }
    args.extend([
        "+set".into(),
        "observer_txadmin_events".into(),
        if settings.txadmin_events { "1" } else { "0" }.into(),
    ]);
    if args.iter().any(|arg| has_fxdk(arg)) {
        return Err(LaunchError::Fxdk);
    }

    let text = fs::read_to_string(&cfg_file).map_err(|source| LaunchError::Cfg {
        path: cfg_file.clone(),
        source,
    })?;
    let mut scan = Scan {
        data: &data,
        observer_convars: OBSERVER_CONVARS
            .into_iter()
            .chain(settings.onesync.map(|_| "onesync"))
            .collect(),
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
    refuse_special(path)?;
    let absolute = std::path::absolute(path).map_err(|source| LaunchError::Resolve {
        path: path.to_path_buf(),
        source,
    })?;
    refuse_special(&absolute)?;
    Ok(absolute)
}

/// UNC (`\\server\share`), verbatim (`\\?\`) and device (`\\.\`) paths all start with two
/// separators, in either direction.
fn refuse_special(path: &Path) -> Result<(), LaunchError> {
    let text = path.as_os_str().to_string_lossy();
    let mut chars = text.chars();
    let separator = |c: Option<char>| matches!(c, Some('\\' | '/'));
    if separator(chars.next()) && separator(chars.next()) {
        return Err(LaunchError::RefusedPath(path.to_path_buf()));
    }
    Ok(())
}

/// A path that ends up inside an FXServer command (`+exec`, `+set citizen_dir`).
fn refuse_commands(path: &Path) -> Result<(), LaunchError> {
    let text = path.as_os_str().to_string_lossy();
    if text.starts_with(['+', '-']) || text.contains(['"', ';', '\r', '\n']) {
        return Err(LaunchError::UnsafePath(path.to_path_buf()));
    }
    Ok(())
}

fn check_extra(name: &str, value: &str) -> Result<(), LaunchError> {
    let valid_name = (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !valid_name {
        return Err(LaunchError::BadName(name.to_owned()));
    }
    if DENIED.iter().any(|denied| denied.eq_ignore_ascii_case(name)) {
        return Err(LaunchError::DeniedName(name.to_owned()));
    }
    if value.contains(['"', ';', '\\']) || value.chars().any(char::is_control) {
        return Err(LaunchError::BadValue(name.to_owned()));
    }
    // An argument starting with `+` begins a command, one starting with `-` may be an option of
    // FXServer's own; a negative number is neither.
    let number = value
        .strip_prefix('-')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit() || c == '.'));
    if value.starts_with('+') || (value.starts_with('-') && !number) {
        return Err(LaunchError::CommandValue(name.to_owned()));
    }
    Ok(())
}

fn has_fxdk(arg: &std::ffi::OsStr) -> bool {
    arg.to_string_lossy().to_ascii_lowercase().contains("-fxdk")
}

struct Scan<'a> {
    data: &'a Path,
    observer_convars: Vec<&'static str>,
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
        } else if let Some(name) = self
            .observer_convars
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

        fn settings(&self) -> LaunchSettings {
            LaunchSettings {
                build: self.dir.path().join("build"),
                data: self.dir.path().join("data"),
                cfg: PathBuf::from("server.cfg"),
                onesync: None,
                txadmin_events: false,
                extra: Vec::new(),
            }
        }

        fn with_extra(&self, name: &str, value: &str) -> LaunchSettings {
            LaunchSettings {
                extra: vec![(name.to_owned(), value.to_owned())],
                ..self.settings()
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

    fn refused(settings: &LaunchSettings) -> Option<LaunchError> {
        prepare(settings, Flavour::HOST).err()
    }

    #[test]
    fn the_bridge_and_the_cfg_come_first_and_observer_convars_last() {
        let fixture = Fixture::new("sv_hostname \"Observer lab\"\n");
        let settings = LaunchSettings {
            onesync: Some(OneSync::Legacy),
            txadmin_events: true,
            extra: vec![("sv_enforceGameBuild".to_owned(), "3258".to_owned())],
            ..fixture.settings()
        };

        let prepared = prepare(&settings, Flavour::HOST).unwrap();

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

    #[test]
    fn onesync_is_left_to_the_cfg_unless_configured() {
        let fixture = Fixture::new("");

        let args = server_args(&prepare(&fixture.settings(), Flavour::HOST).unwrap());

        assert!(!args.iter().any(|arg| arg == "onesync"));
        assert_eq!(args[args.len() - 3..], ["+set", "observer_txadmin_events", "0"]);
    }

    #[test]
    fn an_extra_name_must_be_a_plain_convar_name() {
        let fixture = Fixture::new("");
        let long = "x".repeat(65);
        for name in ["", "sv host", "sv;quit", "+exec", "sv_\u{e9}", long.as_str()] {
            assert!(
                matches!(
                    refused(&fixture.with_extra(name, "1")),
                    Some(LaunchError::BadName(_))
                ),
                "{name:?} was accepted"
            );
        }
        assert!(refused(&fixture.with_extra(&"x".repeat(64), "1")).is_none());
    }

    #[test]
    fn an_extra_value_cannot_carry_a_command() {
        let fixture = Fixture::new("");
        for value in ["a\"b", "a;quit", "C:\\x", "a\rb", "a\nb", "a\tb", "a\u{0}b"] {
            assert!(
                matches!(
                    refused(&fixture.with_extra("sv_x", value)),
                    Some(LaunchError::BadValue(_))
                ),
                "{value:?} was accepted"
            );
        }
        assert!(refused(&fixture.with_extra("sv_hostname", "Observer lab 2")).is_none());
    }

    #[test]
    fn an_extra_value_read_as_a_command_or_an_option_is_refused() {
        let fixture = Fixture::new("");
        for value in ["+quit", "+_crash", "-dumpserver:1", "-fxdk", "-"] {
            assert!(
                matches!(
                    refused(&fixture.with_extra("sv_projectDesc", value)),
                    Some(LaunchError::CommandValue(_))
                ),
                "{value:?} was accepted"
            );
        }
        for value in ["-1", "-0.5", "-.5", "a+b", "Observer +1"] {
            assert!(
                refused(&fixture.with_extra("sv_x", value)).is_none(),
                "{value:?} was refused"
            );
        }
    }

    #[test]
    fn names_that_change_how_fxserver_runs_or_that_observer_sets_are_denied() {
        let fixture = Fixture::new("");
        for name in DENIED.iter().copied().chain(["TXADMINSERVERMODE", "Exec"]) {
            assert!(
                matches!(
                    refused(&fixture.with_extra(name, "1")),
                    Some(LaunchError::DeniedName(_))
                ),
                "{name} was accepted"
            );
        }
    }

    #[test]
    fn fxdk_is_refused_wherever_it_appears() {
        let fixture = Fixture::new("");
        fixture.file("-fxdk.cfg", "");
        let in_cfg = LaunchSettings {
            cfg: PathBuf::from("./-fxdk.cfg"),
            ..fixture.settings()
        };

        assert!(matches!(
            refused(&fixture.with_extra("sv_x", "x -fxdk")),
            Some(LaunchError::Fxdk)
        ));
        assert!(matches!(
            refused(&fixture.with_extra("sv_x", "a-FXDK")),
            Some(LaunchError::Fxdk)
        ));
        assert!(matches!(refused(&in_cfg), Some(LaunchError::Fxdk)));
    }

    #[test]
    fn network_verbatim_and_device_paths_are_refused() {
        let fixture = Fixture::new("");
        for path in [
            r"\\server\share\fx",
            r"\\?\C:\fx",
            r"\\.\PhysicalDrive0",
            "//server/share/fx",
        ] {
            let build = LaunchSettings {
                build: path.into(),
                ..fixture.settings()
            };
            let data = LaunchSettings {
                data: path.into(),
                ..fixture.settings()
            };
            let cfg = LaunchSettings {
                cfg: path.into(),
                ..fixture.settings()
            };
            for settings in [build, data, cfg] {
                assert!(
                    matches!(refused(&settings), Some(LaunchError::RefusedPath(_))),
                    "{path} was accepted in {settings:?}"
                );
            }
        }
    }

    #[test]
    fn a_cfg_path_that_reads_as_commands_is_refused() {
        let fixture = Fixture::new("");
        fixture.file("+quit", "");
        for cfg in ["server.cfg;quit", "+quit", "-dumpserver"] {
            let settings = LaunchSettings {
                cfg: PathBuf::from(cfg),
                ..fixture.settings()
            };

            assert!(
                matches!(refused(&settings), Some(LaunchError::UnsafePath(_))),
                "{cfg:?} was accepted"
            );
        }
    }

    #[test]
    fn a_folder_without_fxserver_is_not_a_build() {
        let fixture = Fixture::new("");
        let settings = LaunchSettings {
            build: fixture.dir.path().join("data"),
            ..fixture.settings()
        };

        assert!(matches!(refused(&settings), Some(LaunchError::NotABuild { .. })));
    }

    #[test]
    fn the_data_folder_and_the_cfg_must_exist() {
        let fixture = Fixture::new("");
        let missing_data = LaunchSettings {
            data: fixture.dir.path().join("nowhere"),
            ..fixture.settings()
        };
        let missing_cfg = LaunchSettings {
            cfg: PathBuf::from("other.cfg"),
            ..fixture.settings()
        };

        assert!(matches!(
            refused(&missing_data),
            Some(LaunchError::MissingData(_))
        ));
        assert!(matches!(refused(&missing_cfg), Some(LaunchError::Cfg { .. })));
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
        let settings = LaunchSettings {
            onesync: Some(OneSync::On),
            ..fixture.settings()
        };

        let prepared = prepare(&settings, Flavour::HOST).unwrap();

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

        let prepared = prepare(&fixture.settings(), Flavour::HOST).unwrap();

        assert_eq!(prepared.warnings.len(), 1);
    }

    #[test]
    fn the_endpoint_comes_from_the_first_tcp_endpoint_of_the_cfg() {
        let fixture = Fixture::new(
            "endpoint_add_udp \"127.0.0.1:30141\"\n\
             endpoint_add_tcp \"0.0.0.0:30140\"\n\
             endpoint_add_tcp \"127.0.0.1:30149\"\n",
        );

        let prepared = prepare(&fixture.settings(), Flavour::HOST).unwrap();

        assert_eq!(prepared.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30140))));
        assert_eq!(reachable("[::]:30140"), Some("[::1]:30140".parse().unwrap()));
        assert_eq!(reachable("not an address"), None);
    }
}
