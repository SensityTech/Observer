use std::fmt::Write as _;

use anyhow::{bail, ensure};
use jiff::Timestamp;
use observer_artifacts::catalog::{Availability, Catalog, CatalogBuild, Listing, PolicyDate, Support};
use observer_artifacts::{Branch, Http, Installer, Platform, Progress};
use tokio_util::sync::CancellationToken;

use crate::config::Config;

const MIB: f64 = 1024.0 * 1024.0;

/// `observer artifacts list`: the catalog of one platform, newest first.
pub async fn list(platform: Option<Platform>, branches: &[Branch], all: bool) -> anyhow::Result<()> {
    let platform = platform.unwrap_or_else(Platform::host);
    let listing = catalog(platform, branches).await?;
    let now = Timestamp::now();

    // Hundreds of old builds survive only in the support policy, and nothing can be done with them.
    // The supported ones stay: they are the builds not listed yet, the failed ones, and the builds
    // of branches nobody named.
    let (shown, hidden): (Vec<&CatalogBuild>, Vec<&CatalogBuild>) =
        listing.builds.iter().partition(|entry| {
            all || entry.availability != Availability::NoCommit || entry.support(now) == Support::Supported
        });
    let header = [
        "BUILD",
        "CHANNEL",
        "PUBLISHED",
        "SUPPORT",
        "END OF SUPPORT",
        "END OF LIFE",
        "SIZE",
        "NOTE",
    ]
    .map(str::to_owned);
    let rows: Vec<[String; 8]> = shown.iter().map(|entry| row(entry, platform, now)).collect();
    let widths: Vec<usize> = (0..7)
        .map(|column| {
            rows.iter()
                .chain([&header])
                .map(|row| row[column].len())
                .max()
                .unwrap_or_default()
        })
        .collect();
    for row in [&header].into_iter().chain(&rows) {
        let mut line = String::new();
        for (cell, width) in row.iter().zip(&widths) {
            let _ = write!(line, "{cell:<width$}  ");
        }
        line.push_str(&row[7]);
        println!("{}", line.trim_end());
    }
    if !hidden.is_empty() {
        println!(
            "{} more builds that only the support policy names are past their end of support; --all lists them",
            hidden.len()
        );
    }
    Ok(())
}

fn row(entry: &CatalogBuild, platform: Platform, now: Timestamp) -> [String; 8] {
    let mut channels = Vec::new();
    if entry.recommended {
        channels.push("recommended".to_owned());
    }
    if entry.latest {
        channels.push("latest".to_owned());
    }
    if channels.is_empty() {
        channels.push(
            entry
                .branch
                .as_ref()
                .map_or_else(|| "-".to_owned(), ToString::to_string),
        );
    }
    let (size, note) = match &entry.availability {
        Availability::Downloadable { size: Some(size), .. } => (mebibytes(*size), String::new()),
        Availability::Downloadable { size: None, .. } => ("-".to_owned(), String::new()),
        Availability::NoCommit => (
            "-".to_owned(),
            "not downloadable: no source gives its commit".to_owned(),
        ),
        Availability::NoArchive => ("-".to_owned(), format!("not downloadable: no {platform} archive")),
    };
    [
        entry.build.to_string(),
        channels.join(", "),
        entry.published_at.map_or_else(|| "unknown".to_owned(), day),
        entry.support(now).to_string(),
        policy_day(entry.end_of_support),
        policy_day(entry.end_of_life),
        size,
        note,
    ]
}

fn day(at: Timestamp) -> String {
    at.strftime("%Y-%m-%d").to_string()
}

fn policy_day(date: Option<PolicyDate>) -> String {
    match date {
        Some(PolicyDate::At(at)) => day(at),
        Some(PolicyDate::Never) => "never".to_owned(),
        None => "-".to_owned(),
    }
}

#[expect(clippy::cast_precision_loss, reason = "sizes are shown to a tenth of a MiB")]
fn mebibytes(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / MIB)
}

/// `observer artifacts install`: downloads, verifies and extracts a build of the host's platform
/// into the data directory. It takes no data directory lock: it writes only under `artifacts/`, and
/// never over a build that is there.
pub async fn install(
    config: &Config,
    build: u32,
    branches: &[Branch],
    allow_end_of_life: bool,
) -> anyhow::Result<()> {
    let platform = Platform::host();
    let listing = catalog(platform, branches).await?;
    let Some(entry) = listing.find(build) else {
        bail!(
            "build {build} is not in the {platform} catalog; name its branch with --branch if it is a test build"
        );
    };
    match entry.availability {
        Availability::Downloadable { .. } => {}
        Availability::NoCommit => bail!(
            "build {build} cannot be downloaded: only the support policy names it, so its folder is unknown; \
             name its branch with --branch if it is a test build"
        ),
        Availability::NoArchive => bail!("build {build} has no {platform} archive on runtime.fivem.net"),
    }
    ensure!(
        allow_end_of_life || entry.support(Timestamp::now()) != Support::EndOfLife,
        "build {build} is past its end of life: players cannot join a server running it; \
         --allow-end-of-life installs it anyway"
    );
    let Some(request) = listing.install_request(build) else {
        bail!("build {build} cannot be downloaded");
    };

    let artifacts = Installer::new(Http::new()?, config.paths().artifacts);
    if let Some(length) = artifacts.long_path() {
        eprintln!(
            "warning: {} is {length} characters long; FXServer fails on paths over 260 characters, \
             and its deepest files sit 125 below the build folder",
            artifacts.root().display()
        );
    }
    let mut shown = None;
    // Nothing cancels it but the end of the process.
    let installed = artifacts
        .install(&request, &CancellationToken::new(), |progress| match progress {
            Progress::Downloading { received, total } => {
                if shown.is_none() {
                    eprintln!("downloading {} ({})", request.url, mebibytes(total));
                }
                let tenth = received.saturating_mul(10).checked_div(total).unwrap_or(10);
                if shown != Some(tenth) {
                    shown = Some(tenth);
                    eprintln!("downloaded {}%", tenth * 10);
                }
            }
            Progress::Verifying => eprintln!("verifying the archive"),
            Progress::Extracting { .. } => eprintln!("extracting"),
            Progress::Validating => eprintln!("validating the build"),
            Progress::Checking | Progress::Copying { .. } | Progress::Committing => {}
        })
        .await?;
    eprintln!(
        "installed build {build}, archive sha256 {}",
        installed.manifest.origin.archive_sha256().unwrap_or_default()
    );
    println!("{}", installed.path.display());
    Ok(())
}

async fn catalog(platform: Platform, branches: &[Branch]) -> anyhow::Result<Listing> {
    if branches.iter().any(Branch::is_master) {
        bail!("master needs no --branch: the docs page lists it");
    }
    let listing = Catalog::new(Http::new()?).list(platform, branches, &[]).await?;
    for warning in &listing.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(listing)
}
