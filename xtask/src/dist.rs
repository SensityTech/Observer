//! `cargo xtask dist`: the release archive of one target, its checksum, and the checks that the
//! binary runs where it is meant to.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use observer_domain::updates;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::{build_panel, built, cargo, root};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LICENCES: &str = "THIRD-PARTY-LICENCES.txt";
/// How the DLLs start that a Windows executable imports when it needs the Visual C++
/// redistributable.
const CRT_IMPORTS: [&str; 3] = ["vcruntime140", "msvcp140", "api-ms-win-crt-"];
/// The variable only a binary built with the browser tests' catalog reads.
const TEST_CATALOG: &[u8] = b"OBSERVER_TEST_CATALOG";

/// The commit a release is built from, as the binary names it.
struct Source {
    commit: String,
    /// `2026-10-28`.
    date: String,
}

pub fn dist(target: &str, max_glibc: Option<&str>) -> anyhow::Result<()> {
    let system = System::of(target)?;
    // A release without the key could not verify the next one: its owners would update by hand.
    let key = fs::read_to_string(root().join(crate::release_key::PUBLIC)).unwrap_or_default();
    if key.trim().len() != 64 {
        bail!(
            "{} holds no release key: run `cargo xtask release-key <file>` once, then commit it",
            crate::release_key::PUBLIC
        );
    }
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
    if test_catalog(&bytes) {
        bail!("{} has the browser tests' catalog built in", binary.display());
    }
    match system {
        System::Windows => {
            let imports = imports(&bytes)
                .with_context(|| format!("cannot read the imports of {}", binary.display()))?;
            if let Some(import) = crt_import(&imports) {
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
    let archive = dist.join(updates::archive_name(VERSION, system.name(), arch(target)));
    let executable = updates::executable_name(system.name());
    match system {
        System::Windows => write_zip(&archive, &source.date, executable, &bytes, &notices)?,
        System::Linux => write_tar(&archive, &source.date, executable, &bytes, &notices)?,
    }
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

/// The first C runtime DLL among the imports, if any, in whatever case the linker wrote it.
fn crt_import(imports: &[String]) -> Option<&str> {
    imports.iter().map(String::as_str).find(|dll| {
        let dll = dll.to_ascii_lowercase();
        CRT_IMPORTS.iter().any(|prefix| dll.starts_with(prefix))
    })
}

/// The DLLs a Windows executable names in its import and delay-load tables. The same names
/// elsewhere in the file, such as the system libraries the hang analysis looks past, are data.
fn imports(binary: &[u8]) -> anyhow::Result<Vec<String>> {
    if binary.get(..2) != Some(b"MZ".as_slice()) {
        bail!("not a Windows executable");
    }
    let header = usize::try_from(u32_at(binary, 0x3c)?)?;
    if binary.get(header..).and_then(|rest| rest.get(..4)) != Some(b"PE\0\0".as_slice()) {
        bail!("not a Windows executable");
    }
    let sections = usize::from(u16_at(binary, header + 6)?);
    let optional = header + 24;
    let table = optional + usize::from(u16_at(binary, header + 20)?);
    // The data directories follow fields that are wider in PE32+ than in PE32.
    let directories = match u16_at(binary, optional)? {
        0x10b => optional + 96,
        0x20b => optional + 112,
        magic => bail!("unknown optional header {magic:#x}"),
    };
    let count = usize::try_from(u32_at(binary, directories - 4)?)?;
    // An address in the loaded image, as an offset in the file through the section that maps it.
    let offset = |address: u32| -> anyhow::Result<usize> {
        for section in (0..sections).map(|index| table + index * 40) {
            let start = u32_at(binary, section + 12)?;
            let size = u32_at(binary, section + 8)?.max(u32_at(binary, section + 16)?);
            if address >= start && address - start < size {
                return Ok(
                    usize::try_from(u32_at(binary, section + 20)?)? + usize::try_from(address - start)?
                );
            }
        }
        bail!("no section maps the address {address:#x}")
    };

    let mut names = Vec::new();
    // The import table's entries are 20 bytes and name their DLL at byte 12; the delay-load
    // table's are 32 bytes and name it at byte 4. An entry without a name ends its table.
    for (directory, entry, name) in [(1, 20, 12), (13, 32, 4)] {
        if directory >= count {
            continue;
        }
        let start = u32_at(binary, directories + directory * 8)?;
        if start == 0 {
            continue;
        }
        let mut at = offset(start)?;
        loop {
            match u32_at(binary, at + name)? {
                0 => break,
                address => names.push(c_string(binary, offset(address)?)?),
            }
            at += entry;
        }
    }
    Ok(names)
}

fn u16_at(binary: &[u8], at: usize) -> anyhow::Result<u16> {
    binary
        .get(at..)
        .and_then(|rest| rest.get(..2))
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_le_bytes)
        .context("the executable ends early")
}

fn u32_at(binary: &[u8], at: usize) -> anyhow::Result<u32> {
    binary
        .get(at..)
        .and_then(|rest| rest.get(..4))
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .context("the executable ends early")
}

fn c_string(binary: &[u8], at: usize) -> anyhow::Result<String> {
    let rest = binary.get(at..).context("the executable ends early")?;
    let end = rest
        .iter()
        .position(|byte| *byte == 0)
        .context("the executable ends early")?;
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

fn test_catalog(binary: &[u8]) -> bool {
    binary
        .windows(TEST_CATALOG.len())
        .any(|window| window == TEST_CATALOG)
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

fn write_zip(path: &Path, date: &str, executable: &str, binary: &[u8], notices: &[u8]) -> anyhow::Result<()> {
    let (year, month, day) = civil(date)?;
    let stamp = DateTime::from_date_and_time(u16::try_from(year)?, month, day, 0, 0, 0)
        .map_err(|error| anyhow::anyhow!("{date} cannot date a zip entry: {error:?}"))?;
    let mut zip = ZipWriter::new(BufWriter::new(File::create(path)?));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(stamp);
    for (name, data) in [(executable, binary), (LICENCES, notices)] {
        zip.start_file(name, options)?;
        zip.write_all(data)?;
    }
    zip.finish()?.flush()?;
    Ok(())
}

fn write_tar(path: &Path, date: &str, executable: &str, binary: &[u8], notices: &[u8]) -> anyhow::Result<()> {
    let (year, month, day) = civil(date)?;
    let seconds = u64::try_from(days_from_civil(year, month, day))? * 86_400;
    let gzip = GzEncoder::new(BufWriter::new(File::create(path)?), Compression::best());
    let mut tar = tar::Builder::new(gzip);
    for (name, data, mode) in [(executable, binary, 0o755), (LICENCES, notices, 0o644)] {
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

    /// A PE32+ executable that imports KERNEL32.dll and delay-loads the DLL named at `delayed`,
    /// with the C runtime's name lying unreferenced in its data.
    fn executable(delayed: u32) -> Vec<u8> {
        let mut file = vec![0; 0x400];
        let mut put = |at: usize, bytes: &[u8]| file[at..at + bytes.len()].copy_from_slice(bytes);
        put(0, b"MZ");
        put(0x3c, &0x40_u32.to_le_bytes());
        put(0x40, b"PE\0\0");
        put(0x46, &1_u16.to_le_bytes());
        put(0x54, &240_u16.to_le_bytes());
        put(0x58, &0x20b_u16.to_le_bytes());
        put(0xc4, &16_u32.to_le_bytes());
        put(0xd0, &0x1000_u32.to_le_bytes());
        put(0x130, &0x1040_u32.to_le_bytes());
        // The one section maps the image from 0x1000 to the file from 0x200.
        put(0x150, &0x200_u32.to_le_bytes());
        put(0x154, &0x1000_u32.to_le_bytes());
        put(0x158, &0x200_u32.to_le_bytes());
        put(0x15c, &0x200_u32.to_le_bytes());
        put(0x20c, &0x1100_u32.to_le_bytes());
        put(0x244, &delayed.to_le_bytes());
        put(0x300, b"KERNEL32.dll\0");
        put(0x320, b"dbghelp.dll\0");
        put(0x380, b"vcruntime140.dll\0");
        file
    }

    #[test]
    fn the_imports_are_read_from_the_tables_not_from_the_data() {
        let found = imports(&executable(0x1120)).unwrap();

        assert_eq!(found, ["KERNEL32.dll", "dbghelp.dll"]);
        assert_eq!(crt_import(&found), None);
        assert!(imports(b"MZ, then nothing").is_err());
        assert!(imports(b"no executable").is_err());
    }

    #[test]
    fn an_executable_that_imports_the_c_runtime_is_found_out() {
        let delayed = imports(&executable(0x1180)).unwrap();
        assert_eq!(crt_import(&delayed), Some("vcruntime140.dll"));

        let names = |dlls: &[&str]| dlls.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            crt_import(&names(&["KERNEL32.dll", "VCRUNTIME140_1.dll"])),
            Some("VCRUNTIME140_1.dll")
        );
        assert_eq!(
            crt_import(&names(&["api-ms-win-crt-runtime-l1-1-0.dll"])),
            Some("api-ms-win-crt-runtime-l1-1-0.dll")
        );
        assert_eq!(
            crt_import(&names(&["KERNEL32.dll", "USER32.dll", "ws2_32.dll"])),
            None
        );
    }

    /// The tests are built like the release, with the C runtime linked in.
    #[cfg(windows)]
    #[test]
    fn this_test_binary_imports_the_kernel_and_no_c_runtime() {
        let binary = fs::read(std::env::current_exe().unwrap()).unwrap();
        let found = imports(&binary).unwrap();

        assert!(found.iter().any(|dll| dll.eq_ignore_ascii_case("kernel32.dll")));
        assert_eq!(crt_import(&found), None);
    }

    #[test]
    fn a_binary_built_with_the_test_catalog_is_found_out() {
        assert!(test_catalog(b"\0OBSERVER_TEST_CATALOG is not a port\0"));
        assert!(!test_catalog(b"\0OBSERVER_LOG\0OBSERVER_COMMIT\0"));
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
