//! Project tasks, run with `cargo xtask <task>`.

mod compress;
mod dist;
mod icons;
mod licences;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, bail};
use observer_domain::permissions::CATALOG;

const USAGE: &str = "usage: cargo xtask <task>

tasks:
  web       build the panel into web/dist, the licence list into web/private, and precompress them
  bindings  regenerate the TypeScript types and permission constants in web/src/api/generated
  check     run every check CI runs: formatting, lints, tests, types, panel build
  dist      the release archive of a target, its checksum and its checks (--target, --max-glibc)
  e2e       the browser tests: the panel, the debug binaries, Chromium, then Playwright
  icons     draw the executable's icons from the mark (--check: fail if they differ)
  licences  list the third-party software a target ships, with its licence texts (--target)";

/// The browser tests' catalog in place of Cfx.re's, which a release never has.
const TEST_CATALOG: &str = "observer/test-catalog";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("web") => web(),
        Some("bindings") => bindings(),
        Some("check") => check(),
        Some("dist") => target(&args).and_then(|target| dist::dist(&target, value(&args, "--max-glibc"))),
        Some("e2e") => e2e(),
        Some("icons") => icons::run(&root(), args.iter().any(|arg| arg == "--check")),
        Some("licences") => target(&args).and_then(|target| licences::write(&root(), &target).map(drop)),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn web() -> anyhow::Result<()> {
    build_panel(&licences::host()?).map(drop)
}

/// The panel and the licence list for `target`, precompressed together. Says where the list's
/// plain text went.
fn build_panel(target: &str) -> anyhow::Result<PathBuf> {
    let web = root().join("web");
    if !web.join("node_modules").exists() {
        run(npm().arg("ci").current_dir(&web))?;
    }
    run(npm().args(["run", "build"]).current_dir(&web))?;
    let licences = licences::write(&root(), target)?;
    compress::dist(&web.join("dist"))?;
    compress::dist(&root().join(licences::PRIVATE))?;
    Ok(licences)
}

/// `--target <triple>`, or the host's.
fn target(args: &[String]) -> anyhow::Result<String> {
    match args.iter().position(|arg| arg == "--target") {
        Some(at) => args
            .get(at + 1)
            .cloned()
            .context("--target needs a target triple"),
        None => licences::host(),
    }
}

/// What follows `flag` on the command line.
fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let at = args.iter().position(|arg| arg == flag)?;
    args.get(at + 1).map(String::as_str)
}

/// Not part of `check`: it downloads a browser and takes minutes. Observer is built with the test
/// catalog, from which the flows install a build.
fn e2e() -> anyhow::Result<()> {
    web()?;
    let panel = root().join("web");
    let binaries = built(cargo().args([
        "build",
        "--package",
        "observer",
        "--package",
        "fake-fxserver",
        "--features",
        TEST_CATALOG,
    ]))?;
    run(npx()
        .args(["playwright", "install", "chromium"])
        .current_dir(&panel))?;
    run(npm()
        .args(["run", "e2e"])
        .current_dir(&panel)
        .env("OBSERVER_BIN", binary(&binaries, "observer")?)
        .env("FAKE_FXSERVER_BIN", binary(&binaries, "fake-fxserver")?)
        .env("FAKE_ARCHIVE_BIN", binary(&binaries, "fake-archive")?))
}

/// Runs a `cargo build` and says where each binary it built went, by name: Cargo's target
/// directory is not always `target/` (`CARGO_TARGET_DIR`, `build.target-dir`).
fn built(build: &mut Command) -> anyhow::Result<BTreeMap<String, PathBuf>> {
    let output = build
        .arg("--message-format=json-render-diagnostics")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("cannot run {build:?}"))?;
    if !output.status.success() {
        bail!("{build:?} failed with {}", output.status);
    }
    Ok(executables(&String::from_utf8_lossy(&output.stdout)))
}

/// The executables in Cargo's JSON messages, by target name.
fn executables(messages: &str) -> BTreeMap<String, PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter_map(|message| {
            let name = message["target"]["name"].as_str()?.to_owned();
            let executable = message["executable"].as_str()?;
            Some((name, PathBuf::from(executable)))
        })
        .collect()
}

fn binary<'a>(binaries: &'a BTreeMap<String, PathBuf>, name: &str) -> anyhow::Result<&'a Path> {
    binaries
        .get(name)
        .map(PathBuf::as_path)
        .with_context(|| format!("cargo built no {name} binary"))
}

/// ts-rs writes the bindings from the `export_bindings_*` tests it generates.
fn bindings() -> anyhow::Result<()> {
    run(cargo().args([
        "test",
        "--quiet",
        "--package",
        "observer-domain",
        "export_bindings",
    ]))?;
    permission_constants()
}

/// The panel gates on these constants, written from the Rust catalog so that no bit is ever typed
/// by hand on the TypeScript side.
fn permission_constants() -> anyhow::Result<()> {
    let mut source = String::from(
        "// This file was generated by `cargo xtask bindings` from the Rust permission catalog. \
         Do not edit this file manually.\n\nexport const Permission = {\n",
    );
    for info in CATALOG {
        writeln!(source, "    {}: 1n << {}n,", pascal_case(info.constant), info.bit)?;
    }
    source.push_str("} as const;\n");

    let path = root().join("web/src/api/generated/Permission.ts");
    fs::write(&path, source).with_context(|| format!("cannot write {}", path.display()))
}

/// `CONSOLE_WRITE` becomes `ConsoleWrite`.
fn pascal_case(constant: &str) -> String {
    constant
        .split('_')
        .flat_map(|word| {
            let mut chars = word.chars();
            chars.next().into_iter().chain(chars.flat_map(char::to_lowercase))
        })
        .collect()
}

fn check() -> anyhow::Result<()> {
    let web = root().join("web");

    run(cargo().args(["fmt", "--all", "--check"]))?;
    icons::run(&root(), true)?;
    run(cargo().args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))?;
    run(cargo().args([
        "clippy",
        "--package",
        "observer",
        "--features",
        TEST_CATALOG,
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]))?;
    run(cargo().args(["test", "--workspace", "--quiet"]))?;
    bindings_unchanged()?;

    if !web.join("node_modules").exists() {
        run(npm().arg("ci").current_dir(&web))?;
    }
    for script in ["format:check", "lint", "typecheck", "test", "build"] {
        run(npm().args(["run", script]).current_dir(&web))?;
    }
    Ok(())
}

/// The committed bindings must match what the Rust types generate.
fn bindings_unchanged() -> anyhow::Result<()> {
    bindings()?;
    let output = Command::new("git")
        .args(["status", "--porcelain", "--", "web/src/api/generated"])
        .current_dir(root())
        .output()
        .context("cannot run git")?;
    let changes = String::from_utf8_lossy(&output.stdout);
    if !changes.trim().is_empty() {
        bail!("the TypeScript bindings are out of date; run `cargo xtask bindings` and commit:\n{changes}");
    }
    Ok(())
}

fn run(command: &mut Command) -> anyhow::Result<()> {
    let status = command
        .status()
        .with_context(|| format!("cannot run {command:?}"))?;
    if !status.success() {
        bail!("{command:?} failed with {status}");
    }
    Ok(())
}

fn cargo() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root());
    command
}

fn npm() -> Command {
    Command::new(node_tool("npm"))
}

fn npx() -> Command {
    Command::new(node_tool("npx"))
}

/// npm's own installer puts batch scripts on Windows, which Command does not find without their
/// extension; some Node managers put executables instead, which it finds by their bare name.
fn node_tool(name: &str) -> String {
    let script = format!("{name}.cmd");
    let on_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|folder| folder.join(&script).is_file()));
    if cfg!(windows) && on_path {
        script
    } else {
        name.to_owned()
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binaries_are_found_where_cargo_says_it_built_them() {
        let messages = r#"{"reason":"compiler-artifact","package_id":"path+file:///work/observer/crates/observer-web#0.1.0","target":{"kind":["lib"],"name":"observer_web"},"executable":null,"fresh":true}
{"reason":"build-script-executed","package_id":"path+file:///work/observer/crates/observer#0.1.0"}
{"reason":"compiler-artifact","package_id":"path+file:///work/observer/crates/observer#0.1.0","target":{"kind":["bin"],"name":"observer"},"executable":"/scratch/cargo/x86_64-unknown-linux-gnu/release/observer","fresh":false}
{"reason":"compiler-artifact","package_id":"path+file:///work/observer/crates/fake-fxserver#0.1.0","target":{"kind":["bin"],"name":"fake-fxserver"},"executable":"/scratch/cargo/debug/fake-fxserver","fresh":true}
{"reason":"build-finished","success":true}
"#;

        let binaries = executables(messages);

        assert_eq!(binaries.keys().collect::<Vec<_>>(), ["fake-fxserver", "observer"]);
        assert_eq!(
            binary(&binaries, "observer").unwrap(),
            Path::new("/scratch/cargo/x86_64-unknown-linux-gnu/release/observer")
        );
        assert_eq!(
            binary(&binaries, "observer_web").unwrap_err().to_string(),
            "cargo built no observer_web binary"
        );
    }
}
