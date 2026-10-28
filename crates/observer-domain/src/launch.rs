//! How Observer launches FXServer. Only the owner changes these settings: a folder, a cfg or an
//! argument amounts to choosing what the host runs. The checks here are pure, so the panel's form
//! and the launch builder refuse exactly the same things.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The longest path, cfg name or argument value accepted, in characters.
pub const MAX_TEXT_CHARS: usize = 1024;
/// The most extra arguments one server takes.
pub const MAX_EXTRA_ARGUMENTS: usize = 64;

/// Names an extra argument may not set. Some would make FXServer escape Observer's supervision
/// (`exec`, `citizen_dir`) or behave as if txAdmin ran it; the rest are Observer's own: two
/// settings chosen in the form rather than as arguments, and the credentials it hands FXServer on
/// stdin at each launch, which an argument would both override and expose.
const RESERVED_NAMES: &[&str] = &[
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

/// Convars that hold secrets. The command line of a process is readable by anyone on the host, so
/// these belong in the cfg.
const SECRET_NAMES: &[&str] = &[
    "sv_licenseKey",
    "sv_tebexSecret",
    "steam_webApiKey",
    "rcon_password",
    "mysql_connection_string",
];

/// Windows opens these names as devices in any folder, with any extension.
const WINDOWS_DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
    "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// `GET` and `PUT /api/servers/default/launch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct LaunchSettings {
    /// The FXServer build: the folder of `FXServer.exe` on Windows, the one holding `alpine` on Linux.
    pub build_path: String,
    /// The folder of the server's resources and cfg. FXServer runs from it.
    pub data_path: String,
    /// The cfg FXServer executes, relative to the data folder.
    pub cfg_file: String,
    pub onesync: OneSync,
    /// Start FXServer when Observer starts.
    pub auto_start: bool,
    /// Raise txAdmin's events next to Observer's, for resources written against txAdmin.
    pub txadmin_events: bool,
    /// Each becomes `+set <name> <value>`, after the cfg and Observer's own convars.
    pub extra_arguments: Vec<ExtraArgument>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum OneSync {
    Off,
    On,
    Legacy,
}

impl OneSync {
    pub const ALL: [Self; 3] = [Self::Off, Self::On, Self::Legacy];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Legacy => "legacy",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ExtraArgument {
    pub name: String,
    pub value: String,
}

/// The rules paths follow on the host Observer runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

impl Platform {
    pub const fn host() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Unix }
    }
}

/// One thing wrong with launch settings, which the panel shows next to its field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LaunchIssue {
    pub field: LaunchField,
    /// The argument's position in the list, for `argumentName` and `argumentValue`.
    pub index: Option<usize>,
    pub problem: LaunchProblem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LaunchField {
    BuildPath,
    DataPath,
    CfgFile,
    ExtraArguments,
    ArgumentName,
    ArgumentValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LaunchProblem {
    Required,
    /// Longer than `MAX_TEXT_CHARS`.
    TooLong,
    /// Not a full path from the root: a drive letter on Windows, `/` on Unix.
    NotAbsolute,
    /// A UNC path, `\\server\share`.
    NetworkPath,
    /// A device rather than a folder: `\\?\`, `\\.\`, `NUL`, `COM1`, `/dev`.
    DevicePath,
    ControlCharacter,
    /// `-fxdk` starts FXServer for Cfx's development kit, outside any supervision.
    Fxdk,
    /// The cfg is not a plain relative path of letters, digits, `.`, `-`, `_` and `/`.
    InvalidCfgFile,
    /// Starts with `-` or `+`. FXServer hands every argument starting with `-` to Node.js as one of
    /// its options (`--inspect` opens a debugger to the host), and reads `+` as a new command.
    OptionLike,
    /// An argument's name is not 1 to 64 letters, digits or underscores.
    InvalidName,
    /// `"`, `;` or `\`, which FXServer's command line reads as syntax.
    ForbiddenCharacter,
    Reserved,
    Secret,
    /// The name was set by an earlier argument.
    Duplicate,
    /// More than `MAX_EXTRA_ARGUMENTS`.
    TooMany,
}

impl LaunchSettings {
    /// Everything that keeps these settings from launching FXServer on `platform`, at most one
    /// problem per field. None means they are fit to store and to launch with.
    pub fn issues(&self, platform: Platform) -> Vec<LaunchIssue> {
        let mut issues = Vec::new();
        let mut note = |field, index, problem: Option<LaunchProblem>| {
            if let Some(problem) = problem {
                issues.push(LaunchIssue {
                    field,
                    index,
                    problem,
                });
            }
        };

        note(
            LaunchField::BuildPath,
            None,
            path_problem(&self.build_path, platform),
        );
        note(
            LaunchField::DataPath,
            None,
            path_problem(&self.data_path, platform),
        );
        note(LaunchField::CfgFile, None, cfg_problem(&self.cfg_file, platform));
        if self.extra_arguments.len() > MAX_EXTRA_ARGUMENTS {
            note(LaunchField::ExtraArguments, None, Some(LaunchProblem::TooMany));
        }
        for (index, argument) in self.extra_arguments.iter().enumerate() {
            let earlier = &self.extra_arguments[..index];
            let duplicate = earlier
                .iter()
                .any(|other| other.name.eq_ignore_ascii_case(&argument.name));
            let name = name_problem(&argument.name).or(duplicate.then_some(LaunchProblem::Duplicate));
            note(LaunchField::ArgumentName, Some(index), name);
            note(
                LaunchField::ArgumentValue,
                Some(index),
                value_problem(&argument.value),
            );
        }
        issues
    }
}

/// What every text the settings hold must avoid, whatever its field.
fn text_problem(text: &str) -> Option<LaunchProblem> {
    if text.trim().is_empty() {
        Some(LaunchProblem::Required)
    } else if text.chars().count() > MAX_TEXT_CHARS {
        Some(LaunchProblem::TooLong)
    } else if text.chars().any(char::is_control) {
        Some(LaunchProblem::ControlCharacter)
    } else if text.to_ascii_lowercase().contains("-fxdk") {
        Some(LaunchProblem::Fxdk)
    } else {
        None
    }
}

/// The build and data folders. On Linux the build folder reaches FXServer's command line inside
/// `+set citizen_dir`, so it follows the rules of an argument value too.
fn path_problem(path: &str, platform: Platform) -> Option<LaunchProblem> {
    if let Some(problem) = text_problem(path) {
        return Some(problem);
    }
    match platform {
        Platform::Windows => windows_path_problem(path),
        Platform::Unix => unix_path_problem(path),
    }
    .or_else(|| {
        let forbidden = match platform {
            Platform::Windows => path.contains(['"', ';']),
            Platform::Unix => path.contains(['"', ';', '\\']),
        };
        forbidden.then_some(LaunchProblem::ForbiddenCharacter)
    })
}

fn windows_path_problem(path: &str) -> Option<LaunchProblem> {
    let bytes = path.as_bytes();
    let separator = |byte: u8| byte == b'\\' || byte == b'/';
    if path.starts_with(r"\??\") {
        return Some(LaunchProblem::DevicePath);
    }
    if bytes.len() >= 2 && separator(bytes[0]) && separator(bytes[1]) {
        let device = bytes.len() >= 4 && matches!(bytes[2], b'?' | b'.') && separator(bytes[3]);
        return Some(if device {
            LaunchProblem::DevicePath
        } else {
            LaunchProblem::NetworkPath
        });
    }
    let drive = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && separator(bytes[2]);
    if !drive {
        return Some(LaunchProblem::NotAbsolute);
    }
    path.split(['\\', '/'])
        .any(windows_device)
        .then_some(LaunchProblem::DevicePath)
}

/// `NUL`, `nul.txt`, `COM1 ` and `CON:x` all open a device: Windows ends the name at its first
/// dot or colon, then drops trailing spaces.
fn windows_device(component: &str) -> bool {
    let stem = component.split(['.', ':']).next().unwrap_or_default().trim_end();
    WINDOWS_DEVICES
        .iter()
        .any(|device| device.eq_ignore_ascii_case(stem))
}

fn unix_path_problem(path: &str) -> Option<LaunchProblem> {
    if !path.starts_with('/') {
        return Some(LaunchProblem::NotAbsolute);
    }
    // `//dev`, `/./dev` and `/srv/../dev` are all `/dev`.
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            name => components.push(name),
        }
    }
    (components.first() == Some(&"dev")).then_some(LaunchProblem::DevicePath)
}

/// The cfg becomes `+exec <cfg>`, read from the data folder: a plain relative path is all it needs.
fn cfg_problem(cfg: &str, platform: Platform) -> Option<LaunchProblem> {
    if let Some(problem) = text_problem(cfg) {
        return Some(problem);
    }
    let plain = cfg.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    });
    if !plain {
        Some(LaunchProblem::InvalidCfgFile)
    } else if cfg.starts_with('-') {
        Some(LaunchProblem::OptionLike)
    } else if platform == Platform::Windows && cfg.split('/').any(windows_device) {
        Some(LaunchProblem::DevicePath)
    } else {
        None
    }
}

fn name_problem(name: &str) -> Option<LaunchProblem> {
    let named = |list: &[&str]| list.iter().any(|known| known.eq_ignore_ascii_case(name));
    if name.is_empty() {
        Some(LaunchProblem::Required)
    } else if name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Some(LaunchProblem::InvalidName)
    } else if named(RESERVED_NAMES) {
        Some(LaunchProblem::Reserved)
    } else if named(SECRET_NAMES) {
        Some(LaunchProblem::Secret)
    } else {
        None
    }
}

fn value_problem(value: &str) -> Option<LaunchProblem> {
    text_problem(value).or_else(|| {
        if value.contains(['"', ';', '\\']) {
            Some(LaunchProblem::ForbiddenCharacter)
        } else if value.starts_with(['-', '+']) {
            Some(LaunchProblem::OptionLike)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> LaunchSettings {
        LaunchSettings {
            build_path: r"C:\FXServer\build".to_owned(),
            data_path: r"C:\FXServer\server-data".to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::On,
            auto_start: true,
            txadmin_events: false,
            extra_arguments: vec![argument("sv_enforceGameBuild", "3258")],
        }
    }

    /// The same on Linux.
    fn linux() -> LaunchSettings {
        LaunchSettings {
            build_path: "/opt/fxserver/build".to_owned(),
            data_path: "/srv/fivem/server-data".to_owned(),
            ..settings()
        }
    }

    fn on(platform: Platform) -> LaunchSettings {
        match platform {
            Platform::Windows => settings(),
            Platform::Unix => linux(),
        }
    }

    fn argument(name: &str, value: &str) -> ExtraArgument {
        ExtraArgument {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    fn issue(field: LaunchField, index: Option<usize>, problem: LaunchProblem) -> LaunchIssue {
        LaunchIssue {
            field,
            index,
            problem,
        }
    }

    fn build_path(path: &str, platform: Platform) -> Vec<LaunchProblem> {
        let settings = LaunchSettings {
            build_path: path.to_owned(),
            ..on(platform)
        };
        settings
            .issues(platform)
            .into_iter()
            .map(|issue| issue.problem)
            .collect()
    }

    #[test]
    fn settings_a_server_runs_with_have_no_issue() {
        assert_eq!(settings().issues(Platform::Windows), []);

        let linux = LaunchSettings {
            cfg_file: "configs/server.cfg".to_owned(),
            extra_arguments: vec![
                argument("sv_hostname", "Observer lab 2"),
                argument("gamename", "rdr3"),
            ],
            ..linux()
        };
        assert_eq!(linux.issues(Platform::Unix), []);
    }

    #[test]
    fn windows_paths_must_be_full_paths_to_a_local_folder() {
        for (path, problem) in [
            (r"FXServer\build", LaunchProblem::NotAbsolute),
            (r"\FXServer\build", LaunchProblem::NotAbsolute),
            (r"C:FXServer", LaunchProblem::NotAbsolute),
            ("/opt/fxserver", LaunchProblem::NotAbsolute),
            (r"\\fileserver\share\build", LaunchProblem::NetworkPath),
            ("//fileserver/share/build", LaunchProblem::NetworkPath),
            (r"\\?\C:\FXServer\build", LaunchProblem::DevicePath),
            (r"\\.\PhysicalDrive0", LaunchProblem::DevicePath),
            (r"\??\C:\FXServer", LaunchProblem::DevicePath),
            (r"C:\FXServer\NUL", LaunchProblem::DevicePath),
            (r"C:\FXServer\com1.txt\build", LaunchProblem::DevicePath),
            (r"C:\FXServer\CON \build", LaunchProblem::DevicePath),
            (r"C:\FXServer\NUL:", LaunchProblem::DevicePath),
            (r"C:\FXServer\COM1:\build", LaunchProblem::DevicePath),
            (r"C:\FXServer\CON:x", LaunchProblem::DevicePath),
            (r#"C:\FX"Server"#, LaunchProblem::ForbiddenCharacter),
            (r"C:\FXServer;quit", LaunchProblem::ForbiddenCharacter),
        ] {
            assert_eq!(build_path(path, Platform::Windows), [problem], "{path:?}");
        }
        assert_eq!(build_path("D:/FXServer/build", Platform::Windows), []);
        assert_eq!(build_path(r"C:\FXServer\console\build", Platform::Windows), []);
        assert_eq!(build_path(r"C:\FXServer\nullable.d\build", Platform::Windows), []);
    }

    #[test]
    fn unix_paths_must_start_at_the_root_and_stay_out_of_dev() {
        for (path, problem) in [
            ("opt/fxserver", LaunchProblem::NotAbsolute),
            (r"C:\FXServer\build", LaunchProblem::NotAbsolute),
            ("/dev/sda1", LaunchProblem::DevicePath),
            ("/dev", LaunchProblem::DevicePath),
            ("//dev/sda1", LaunchProblem::DevicePath),
            ("/./dev/sda1", LaunchProblem::DevicePath),
            ("/srv/../dev/sda1", LaunchProblem::DevicePath),
            ("/../dev/shm/x", LaunchProblem::DevicePath),
            (r"/opt/fx\server", LaunchProblem::ForbiddenCharacter),
            ("/opt/fx;server", LaunchProblem::ForbiddenCharacter),
        ] {
            assert_eq!(build_path(path, Platform::Unix), [problem], "{path:?}");
        }
        assert_eq!(build_path("/opt/devices/fxserver", Platform::Unix), []);
        assert_eq!(build_path("/opt/dev/../fxserver/", Platform::Unix), []);
    }

    #[test]
    fn every_text_must_be_filled_short_printable_and_free_of_fxdk() {
        let long = format!("C:\\{}", "a".repeat(MAX_TEXT_CHARS));
        for (path, problem) in [
            ("", LaunchProblem::Required),
            ("   ", LaunchProblem::Required),
            (long.as_str(), LaunchProblem::TooLong),
            ("C:\\FXServer\nbuild", LaunchProblem::ControlCharacter),
            ("C:\\FXServer\0", LaunchProblem::ControlCharacter),
            (r"C:\FXServer\-FxDK", LaunchProblem::Fxdk),
        ] {
            assert_eq!(build_path(path, Platform::Windows), [problem], "{path:?}");
        }
    }

    #[test]
    fn the_cfg_is_a_plain_path_inside_the_data_folder() {
        let cfg = |cfg: &str, platform| {
            LaunchSettings {
                cfg_file: cfg.to_owned(),
                ..on(platform)
            }
            .issues(platform)
        };

        for invalid in [
            "../server.cfg",
            "configs/../../server.cfg",
            "./server.cfg",
            "/etc/server.cfg",
            r"C:\server.cfg",
            r"configs\server.cfg",
            "server.cfg;quit",
            "my server.cfg",
            "configs//server.cfg",
            "server.cfg +exec other.cfg",
        ] {
            assert_eq!(
                cfg(invalid, Platform::Unix),
                [issue(LaunchField::CfgFile, None, LaunchProblem::InvalidCfgFile)],
                "{invalid:?}"
            );
        }
        assert_eq!(
            cfg("configs/nul.cfg", Platform::Windows),
            [issue(LaunchField::CfgFile, None, LaunchProblem::DevicePath)]
        );
        assert_eq!(cfg("configs/nul.cfg", Platform::Unix), []);
        assert_eq!(
            cfg("server-fxdk.cfg", Platform::Unix),
            [issue(LaunchField::CfgFile, None, LaunchProblem::Fxdk)]
        );
        for option in ["--inspect", "-V", "--version"] {
            assert_eq!(
                cfg(option, Platform::Unix),
                [issue(LaunchField::CfgFile, None, LaunchProblem::OptionLike)],
                "{option:?}"
            );
        }
        assert_eq!(cfg("configs/live_2.cfg", Platform::Windows), []);
        assert_eq!(cfg("configs/-old.cfg", Platform::Windows), []);
    }

    #[test]
    fn argument_names_are_plain_and_never_reserved_or_secret() {
        let name = |name: &str| {
            LaunchSettings {
                extra_arguments: vec![argument(name, "1")],
                ..settings()
            }
            .issues(Platform::Windows)
            .into_iter()
            .map(|issue| issue.problem)
            .collect::<Vec<_>>()
        };

        for (invalid, problem) in [
            ("", LaunchProblem::Required),
            ("sv hostname", LaunchProblem::InvalidName),
            ("sv-hostname", LaunchProblem::InvalidName),
            ("+exec", LaunchProblem::InvalidName),
            ("-fxdk", LaunchProblem::InvalidName),
            ("convar;quit", LaunchProblem::InvalidName),
            ("é", LaunchProblem::InvalidName),
            ("exec", LaunchProblem::Reserved),
            ("citizen_dir", LaunchProblem::Reserved),
            ("txadminservermode", LaunchProblem::Reserved),
            ("monitorMode", LaunchProblem::Reserved),
            ("serverProfile", LaunchProblem::Reserved),
            ("txAdminPort", LaunchProblem::Reserved),
            ("txDataPath", LaunchProblem::Reserved),
            ("txAdminInterface", LaunchProblem::Reserved),
            ("OneSync", LaunchProblem::Reserved),
            ("observer_txadmin_events", LaunchProblem::Reserved),
            ("sv_prometheusBasicAuthUser", LaunchProblem::Reserved),
            ("sv_prometheusBasicAuthPassword", LaunchProblem::Reserved),
            ("SV_PLAYERSTOKEN", LaunchProblem::Reserved),
            ("sv_licenseKey", LaunchProblem::Secret),
            ("sv_tebexSecret", LaunchProblem::Secret),
            ("steam_webApiKey", LaunchProblem::Secret),
            ("RCON_PASSWORD", LaunchProblem::Secret),
            ("mysql_connection_string", LaunchProblem::Secret),
        ] {
            assert_eq!(name(invalid), [problem], "{invalid:?}");
        }
        assert_eq!(name(&"a".repeat(64)), []);
        assert_eq!(name(&"a".repeat(65)), [LaunchProblem::InvalidName]);
        assert_eq!(name("onesync_population"), []);
    }

    #[test]
    fn argument_values_carry_no_command_line_syntax() {
        let value = |value: &str| {
            LaunchSettings {
                extra_arguments: vec![argument("sv_hostname", value)],
                ..linux()
            }
            .issues(Platform::Unix)
        };

        for (invalid, problem) in [
            ("", LaunchProblem::Required),
            ("Lab\"; quit \"", LaunchProblem::ForbiddenCharacter),
            ("Lab; quit", LaunchProblem::ForbiddenCharacter),
            (r"Lab\", LaunchProblem::ForbiddenCharacter),
            ("Lab\rquit", LaunchProblem::ControlCharacter),
            ("Lab\nquit", LaunchProblem::ControlCharacter),
            ("-fxdk", LaunchProblem::Fxdk),
            ("--inspect=0.0.0.0:9229", LaunchProblem::OptionLike),
            ("-V", LaunchProblem::OptionLike),
            ("+quit", LaunchProblem::OptionLike),
        ] {
            assert_eq!(
                value(invalid),
                [issue(LaunchField::ArgumentValue, Some(0), problem)],
                "{invalid:?}"
            );
        }
        assert_eq!(value("Observer lab ^2 #1"), []);
        assert_eq!(value("Lab -1 +2"), []);
    }

    #[test]
    fn a_name_set_twice_and_a_list_too_long_are_refused_by_position() {
        let twice = LaunchSettings {
            extra_arguments: vec![
                argument("sv_maxclients", "48"),
                argument("sv_hostname", "Lab"),
                argument("SV_MAXCLIENTS", "64"),
            ],
            ..settings()
        };
        assert_eq!(
            twice.issues(Platform::Windows),
            [issue(
                LaunchField::ArgumentName,
                Some(2),
                LaunchProblem::Duplicate
            )]
        );

        let many = LaunchSettings {
            extra_arguments: (0..=MAX_EXTRA_ARGUMENTS)
                .map(|index| argument(&format!("observer_lab_{index}"), "1"))
                .collect(),
            ..settings()
        };
        assert_eq!(
            many.issues(Platform::Windows),
            [issue(LaunchField::ExtraArguments, None, LaunchProblem::TooMany)]
        );
    }

    #[test]
    fn issues_name_their_field_and_the_argument_they_concern() {
        let broken = LaunchSettings {
            build_path: "build".to_owned(),
            data_path: String::new(),
            cfg_file: "../server.cfg".to_owned(),
            extra_arguments: vec![argument("sv_hostname", "Lab"), argument("exec", "a;b")],
            ..settings()
        };

        assert_eq!(
            broken.issues(Platform::Windows),
            [
                issue(LaunchField::BuildPath, None, LaunchProblem::NotAbsolute),
                issue(LaunchField::DataPath, None, LaunchProblem::Required),
                issue(LaunchField::CfgFile, None, LaunchProblem::InvalidCfgFile),
                issue(LaunchField::ArgumentName, Some(1), LaunchProblem::Reserved),
                issue(
                    LaunchField::ArgumentValue,
                    Some(1),
                    LaunchProblem::ForbiddenCharacter
                ),
            ]
        );
        assert_eq!(
            serde_json::to_value(broken.issues(Platform::Windows)[3]).unwrap(),
            serde_json::json!({ "field": "argumentName", "index": 1, "problem": "reserved" })
        );
    }

    #[test]
    fn the_wire_form_is_camel_case_and_refuses_unknown_fields() {
        let json = serde_json::to_value(settings()).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "buildPath": r"C:\FXServer\build",
                "dataPath": r"C:\FXServer\server-data",
                "cfgFile": "server.cfg",
                "onesync": "on",
                "autoStart": true,
                "txadminEvents": false,
                "extraArguments": [{ "name": "sv_enforceGameBuild", "value": "3258" }],
            })
        );
        assert_eq!(
            serde_json::from_value::<LaunchSettings>(json.clone()).unwrap(),
            settings()
        );
        let mut extra = json;
        extra["monitorMode"] = serde_json::json!(true);
        assert!(serde_json::from_value::<LaunchSettings>(extra).is_err());
        for mode in OneSync::ALL {
            assert_eq!(serde_json::to_value(mode).unwrap(), mode.as_str());
            assert_eq!(OneSync::from_name(mode.as_str()), Some(mode));
        }
    }
}
