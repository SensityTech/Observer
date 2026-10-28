//! Project tasks, run with `cargo xtask <task>`.

mod compress;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, bail};

const USAGE: &str = "usage: cargo xtask <task>

tasks:
  web       build the panel into web/dist and precompress it
  bindings  regenerate the TypeScript types in web/src/api/generated
  check     run every check CI runs: formatting, lints, tests, types, panel build
  dist      build the panel, then a release binary that embeds it";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let result = match task.as_deref() {
        Some("web") => web(),
        Some("bindings") => bindings(),
        Some("check") => check(),
        Some("dist") => dist(),
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
    let web = root().join("web");
    if !web.join("node_modules").exists() {
        run(npm().arg("ci").current_dir(&web))?;
    }
    run(npm().args(["run", "build"]).current_dir(&web))?;
    compress::dist(&web.join("dist"))
}

/// ts-rs writes the bindings from the `export_bindings_*` tests it generates.
fn bindings() -> anyhow::Result<()> {
    run(cargo().args([
        "test",
        "--quiet",
        "--package",
        "observer-domain",
        "export_bindings",
    ]))
}

fn check() -> anyhow::Result<()> {
    let web = root().join("web");

    run(cargo().args(["fmt", "--all", "--check"]))?;
    run(cargo().args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))?;
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

fn dist() -> anyhow::Result<()> {
    web()?;
    run(cargo().args(["build", "--release", "--package", "observer"]))
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
    // npm is a batch script on Windows, which Command does not resolve without its extension.
    Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" })
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}
