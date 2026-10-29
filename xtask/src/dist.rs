//! `cargo xtask dist`: the release archive of one target, its checksum, and the checks that the
//! binary runs where it is meant to.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::{build_panel, built, cargo, root};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LICENCES: &str = "THIRD-PARTY-LICENCES.txt";
/// What a Windows executable imports when it needs the Visual C++ redistributable.
const CRT_IMPORTS: [&str; 3] = ["vcruntime140.dll", "msvcp140.dll", "api-ms-win-crt-"];

/// The commit a release is built from, as the binary names it.
struct Source {
    commit: String,
    /// `2026-10-28`.
    date: String,
}

pub fn dist(target: &str, max_glibc: Option<&str>) -> anyhow::Result<()> {
    let system = System::of(target)?;
    let licences = build_panel(target)?;
    let source = source()?;
    let binaries = built(
        cargo()
            .args([
                "build",
                "--release",
                "--locked",
                "--package",
                "observer",
                "--target",
                target,
            ])
            .env("OBSERVER_COMMIT", &source.commit)
            .env("OBSERVER_COMMIT_DATE", &source.date),
    )?;
    let binary = crate::binary(&binaries, "observer")?;
    let bytes = fs::read(binary).with_context(|| format!("cannot read {}", binary.display()))?;
    match system {
        System::Windows => {
            if let Some(import) = crt_import(&bytes) {
                bail!(
                    "{} imports {import}: the C runtime must be linked in (+crt-static)",
                    binary.display()
                );
            }
            println!("C runtime: linked in");
        }
        System::Linux => {
            let floor = glibc_floor(&bytes).context("the binary names no glibc version")?;
            println!("glibc: {} and later", glibc(floor));
            if let Some(limit) = max_glibc
                && floor > parse_glibc(limit)?
            {
                bail!(
                    "the binary needs glibc {}, above the floor of {limit}",
                    glibc(floor)
                );
            }
        }
    }

    let dist = root().join("target/dist");
    fs::create_dir_all(&dist).with_context(|| format!("cannot create {}", dist.display()))?;
    let notices = fs::read(&licences).with_context(|| format!("cannot read {}", licences.display()))?;
    let name = format!("observer-{VERSION}-{}-{}", system.name(), arch(target));
    let archive = match system {
        System::Windows => {
            let path = dist.join(format!("{name}.zip"));
            write_zip(&path, &source.date, &bytes, &notices)?;
            path
        }
        System::Linux => {
            let path = dist.join(format!("{name}.tar.gz"));
            write_tar(&path, &source.date, &bytes, &notices)?;
            path
        }
    };
    let packed = fs::read(&archive)?;
    let checksum = hex(&Sha256::digest(&packed));
    let file_name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .context("the archive has no name")?;
    let sums = dist.join(format!("{file_name}.sha256"));
    fs::write(&sums, format!("{checksum}  {file_name}\n"))?;

    println!("{}", archive.display());
    println!("  archive {}, binary {}", size(packed.len()), size(bytes.len()));
    println!("  sha256 {checksum}");
    println!("  built from {} ({})", source.commit, source.date);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum System {
    Windows,
    Linux,
}

impl System {
    fn of(target: &str) -> anyhow::Result<Self> {
        if target.contains("-windows-") {
            Ok(Self::Windows)
        } else if target.contains("-linux-") {
            Ok(Self::Linux)
        } else {
            bail!("Observer ships for Windows and Linux only, not {target}")
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }
}

fn arch(target: &str) -> &str {
    target.split('-').next().unwrap_or(target)
}

/// The short commit, marked when tracked files differ from it, and its date: never the time of
/// the build, so that one commit always builds the same strings. A tree without git, such as an
/// exported archive, is named by `OBSERVER_COMMIT` and `OBSERVER_COMMIT_DATE`.
fn source() -> anyhow::Result<Source> {
    if let (Ok(commit), Ok(date)) = (
        std::env::var("OBSERVER_COMMIT"),
        std::env::var("OBSERVER_COMMIT_DATE"),
    ) {
        return Ok(Source { commit, date });
    }
    let git = |args: &[&str]| -> anyhow::Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(root())
            .output()
            .context("cannot run git")?;
        if !output.status.success() {
            bail!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let mut commit = git(&["rev-parse", "--short=12", "HEAD"])?;
    if !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        commit.push_str("-modified");
    }
    Ok(Source {
        commit,
        date: git(&["show", "-s", "--format=%cs", "HEAD"])?,
    })
}

/// The first C runtime DLL the executable names, if any. The import table spells its DLLs out
/// in ASCII, in whatever case the linker wrote.
fn crt_import(binary: &[u8]) -> Option<&'static str> {
    let lower = binary.to_ascii_lowercase();
    CRT_IMPORTS.into_iter().find(|import| {
        lower
            .windows(import.len())
            .any(|window| window == import.as_bytes())
    })
}

/// The highest `GLIBC_2.N[.M]` symbol version the binary asks for.
fn glibc_floor(binary: &[u8]) -> Option<(u32, u32)> {
    const PREFIX: &[u8] = b"GLIBC_2.";
    let mut highest = None;
    let mut at = 0;
    while let Some(found) = binary[at..]
        .windows(PREFIX.len())
        .position(|window| window == PREFIX)
    {
        let start = at + found + PREFIX.len();
        let end = binary[start..]
            .iter()
            .position(|byte| !(byte.is_ascii_digit() || *byte == b'.'))
            .map_or(binary.len(), |length| start + length);
        if let Ok(version) = parse_glibc(&format!("2.{}", String::from_utf8_lossy(&binary[start..end]))) {
            highest = highest.max(Some(version));
        }
        at = end.max(start);
    }
    highest
}

fn glibc((minor, patch): (u32, u32)) -> String {
    if patch == 0 {
        format!("2.{minor}")
    } else {
        format!("2.{minor}.{patch}")
    }
}

/// `2.35` as `(35, 0)`.
fn parse_glibc(version: &str) -> anyhow::Result<(u32, u32)> {
    let mut parts = version.trim_end_matches('.').split('.');
    let (Some("2"), Some(minor)) = (parts.next(), parts.next()) else {
        bail!("expected a glibc version such as 2.35, not {version}");
    };
    let patch = parts.next().map_or(Ok(0), str::parse)?;
    Ok((minor.parse()?, patch))
}

fn write_zip(path: &Path, date: &str, binary: &[u8], notices: &[u8]) -> anyhow::Result<()> {
    let (year, month, day) = civil(date)?;
    let stamp = DateTime::from_date_and_time(u16::try_from(year)?, month, day, 0, 0, 0)
        .map_err(|error| anyhow::anyhow!("{date} cannot date a zip entry: {error:?}"))?;
    let mut zip = ZipWriter::new(BufWriter::new(File::create(path)?));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(stamp);
    for (name, data) in [("observer.exe", binary), (LICENCES, notices)] {
        zip.start_file(name, options)?;
        zip.write_all(data)?;
    }
    zip.finish()?.flush()?;
    Ok(())
}

fn write_tar(path: &Path, date: &str, binary: &[u8], notices: &[u8]) -> anyhow::Result<()> {
    let (year, month, day) = civil(date)?;
    let seconds = u64::try_from(days_from_civil(year, month, day))? * 86_400;
    let gzip = GzEncoder::new(BufWriter::new(File::create(path)?), Compression::best());
    let mut tar = tar::Builder::new(gzip);
    for (name, data, mode) in [("observer", binary, 0o755), (LICENCES, notices, 0o644)] {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(data.len())?);
        header.set_mode(mode);
        header.set_mtime(seconds);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar.append_data(&mut header, name, data)?;
    }
    tar.into_inner()?.finish()?.flush()?;
    Ok(())
}

/// `2026-10-28` as its parts.
fn civil(date: &str) -> anyhow::Result<(i64, u8, u8)> {
    let mut parts = date.split('-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(year), Some(month), Some(day)) => Ok((year.parse()?, month.parse()?, day.parse()?)),
        _ => bail!("expected a date such as 2026-10-28, not {date}"),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

#[expect(clippy::cast_precision_loss, reason = "a size shown to one decimal")]
fn size(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_that_imports_the_c_runtime_is_found_out() {
        assert_eq!(
            crt_import(b"\0KERNEL32.dll\0VCRUNTIME140.dll\0"),
            Some("vcruntime140.dll")
        );
        assert_eq!(
            crt_import(b"\0api-ms-win-crt-runtime-l1-1-0.dll\0"),
            Some("api-ms-win-crt-")
        );
        assert_eq!(crt_import(b"\0KERNEL32.dll\0USER32.dll\0ws2_32.dll\0"), None);
    }

    #[test]
    fn the_glibc_floor_is_the_highest_version_the_binary_names() {
        let binary = b"\0GLIBC_2.2.5\0GLIBC_2.34\0GLIBC_PRIVATE\0GLIBC_2.17\0GLIBC_2.3.4\0";

        assert_eq!(glibc_floor(binary), Some((34, 0)));
        assert_eq!(parse_glibc("2.35").unwrap(), (35, 0));
        assert!(parse_glibc("3.1").is_err());
        assert_eq!(glibc_floor(b"no versions"), None);
    }

    #[test]
    fn archive_entries_are_dated_at_the_commit_day() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 10, 28), 20_754);
        assert_eq!(civil("2026-10-28").unwrap(), (2026, 10, 28));
    }

    #[test]
    fn a_target_names_its_archive() {
        assert_eq!(System::of("x86_64-pc-windows-msvc").unwrap(), System::Windows);
        assert_eq!(System::of("x86_64-unknown-linux-gnu").unwrap(), System::Linux);
        assert!(System::of("aarch64-apple-darwin").is_err());
        assert_eq!(arch("x86_64-unknown-linux-gnu"), "x86_64");
    }
}
