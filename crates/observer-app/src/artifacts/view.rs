//! What the panel sees of the builds: the installer's and the catalog's types as the API
//! sends them, codes and parameters rather than sentences.

use jiff::Timestamp;
use observer_artifacts::catalog::{
    self, Availability as Offered, CatalogBuild, Listing, PolicyDate, Source, Support,
};
use observer_artifacts::{
    Broken, Error as InstallError, Health, ImportRefusal, LayoutProblem, Origin, Progress, Refusal, Scanned,
};
use observer_domain::artifacts::{
    Availability, BrokenReason, BuildHealth, BuildOrigin, BuildPlatform, CatalogEntry, CatalogSource,
    CatalogView, CatalogWarning, InstallFailure, InstalledBuild, JobStep, PolicyDay, SupportLevel,
};

pub(crate) fn installed(scanned: &Scanned) -> InstalledBuild {
    let manifest = scanned.manifest.as_ref();
    let (origin, commit, branch) = match manifest.map(|manifest| &manifest.origin) {
        Some(Origin::Download { commit, branch, .. }) => (
            Some(BuildOrigin::Download),
            Some(commit.to_string()),
            Some(branch.to_string()),
        ),
        Some(Origin::Import { .. }) => (Some(BuildOrigin::Import), None, None),
        None => (None, None, None),
    };
    InstalledBuild {
        build: scanned.build,
        origin,
        commit,
        branch,
        installed_at: manifest.map(|manifest| manifest.installed_at),
        installed_by: manifest.and_then(|manifest| manifest.installed_by.clone()),
        bytes: scanned.bytes,
        files: manifest.map(|manifest| manifest.files),
        health: match scanned.health {
            Health::Valid => BuildHealth::Valid,
            Health::Broken(broken) => BuildHealth::Broken {
                reason: broken_reason(broken),
            },
        },
    }
}

pub(crate) fn broken_reason(broken: Broken) -> BrokenReason {
    match broken {
        Broken::NoManifest => BrokenReason::NoManifest,
        Broken::BadManifest => BrokenReason::BadManifest,
        Broken::Link => BrokenReason::Link,
        Broken::Layout { .. } => BrokenReason::Layout,
        Broken::ArchiveChanged => BrokenReason::ArchiveChanged,
    }
}

pub(crate) fn step(progress: &Progress) -> JobStep {
    match progress {
        Progress::Checking => JobStep::PreCheck,
        Progress::Downloading { .. } => JobStep::Download,
        Progress::Verifying => JobStep::Verify,
        Progress::Extracting { .. } => JobStep::Extract,
        Progress::Copying { .. } => JobStep::Copy,
        Progress::Validating => JobStep::Validate,
        Progress::Committing => JobStep::Commit,
    }
}

/// The code of a failed job; the full text, which may name entries of an archive or folders of the
/// owner, goes to Observer's log.
pub(crate) fn failure(error: &InstallError) -> InstallFailure {
    match error {
        InstallError::Network { .. }
        | InstallError::UrlRefused { .. }
        | InstallError::TooManyRedirects { .. } => InstallFailure::Unreachable,
        InstallError::Status { status, .. } => InstallFailure::HttpStatus { status: *status },
        InstallError::TooLarge { .. } => InstallFailure::ArchiveTooLarge,
        InstallError::LengthMissing { .. }
        | InstallError::LengthChanged { .. }
        | InstallError::Overlong { .. }
        | InstallError::BadRange { .. } => InstallFailure::BadLength,
        InstallError::Truncated { .. } => InstallFailure::Truncated,
        InstallError::ArchiveChanged { .. } => InstallFailure::ArchiveChanged,
        InstallError::DiskSpace {
            needed_mib,
            available_mib,
            ..
        } => InstallFailure::DiskSpace {
            needed_mib: *needed_mib,
            available_mib: *available_mib,
        },
        InstallError::Unsafe { refusal, .. } => InstallFailure::UnsafeEntry {
            refusal: refusal_code(refusal).to_owned(),
        },
        // Observer's own releases are never a build's install.
        InstallError::Damaged(_)
        | InstallError::RateLimited { .. }
        | InstallError::ReleaseFormat(_)
        | InstallError::NoReleaseKey
        | InstallError::NoArchive { .. }
        | InstallError::Unsigned
        | InstallError::BadSignature
        | InstallError::NotListed { .. }
        | InstallError::ChecksumMismatch { .. }
        | InstallError::DigestMismatch { .. }
        | InstallError::WontRun(_)
        | InstallError::ExecutableSignature(_) => InstallFailure::Damaged,
        InstallError::Layout { path, problem } => InstallFailure::Layout {
            path: (*path).to_owned(),
            problem: match problem {
                LayoutProblem::Missing => "missing",
                LayoutProblem::NotPlain => "notPlain",
                LayoutProblem::NotExecutable => "notExecutable",
            }
            .to_owned(),
        },
        InstallError::Signature(_) => InstallFailure::Signature,
        InstallError::AlreadyInstalled(_) => InstallFailure::AlreadyInstalled,
        InstallError::Cancelled => InstallFailure::Cancelled,
        InstallError::Interrupted => InstallFailure::Interrupted,
        InstallError::Import { refusal, .. } => match refusal {
            ImportRefusal::NotAbsolute | ImportRefusal::NotAFolder => InstallFailure::NotAFolder,
            ImportRefusal::Link => InstallFailure::Link,
            ImportRefusal::InsideArtifacts => InstallFailure::InsideArtifacts,
            ImportRefusal::TooManyFiles => InstallFailure::TooManyFiles,
            ImportRefusal::TooLarge => InstallFailure::TooLarge,
        },
        InstallError::FormatChanged { .. }
        | InstallError::Client(_)
        | InstallError::WrongPlatform(_)
        | InstallError::StagingLink(_)
        | InstallError::Manifest { .. }
        | InstallError::Io { .. }
        | InstallError::Random(_)
        | InstallError::NotInstalled(_)
        | InstallError::Busy(_) => InstallFailure::Io,
    }
}

fn refusal_code(refusal: &Refusal) -> &'static str {
    match refusal {
        Refusal::Empty => "empty",
        Refusal::TooLong => "tooLong",
        Refusal::Control => "control",
        Refusal::Backslash => "backslash",
        Refusal::Colon => "colon",
        Refusal::TrailingDot => "trailingDot",
        Refusal::Device => "device",
        Refusal::Absolute => "absolute",
        Refusal::Parent => "parent",
        Refusal::Duplicate => "duplicate",
        Refusal::InsideFile => "insideFile",
        Refusal::InsideLink => "insideLink",
        Refusal::ZipLink => "zipLink",
        Refusal::Encrypted => "encrypted",
        Refusal::Method(_) => "method",
        Refusal::EntryType(_) => "entryType",
        Refusal::LinkOutsideAlpine => "linkOutsideAlpine",
        Refusal::LinkEscapes => "linkEscapes",
        Refusal::LinkLoop => "linkLoop",
        Refusal::TooManyEntries => "tooManyEntries",
        Refusal::TooLarge => "tooLarge",
    }
}

/// The catalog of the host's platform. Builds only the support policy names, past their end of
/// support, are left out: nothing can be done with them.
pub(crate) fn catalog(
    listing: &Listing,
    fetched_at: Timestamp,
    installed: &[u32],
    now: Timestamp,
) -> CatalogView {
    let builds = listing
        .builds
        .iter()
        .filter(|entry| entry.availability != Offered::NoCommit || entry.support(now) == Support::Supported)
        .map(|entry| catalog_entry(entry, installed.contains(&entry.build), now))
        .collect();
    CatalogView {
        platform: BuildPlatform::host(),
        fetched_at,
        builds,
        warnings: listing.warnings.iter().map(warning).collect(),
    }
}

fn catalog_entry(entry: &CatalogBuild, installed: bool, now: Timestamp) -> CatalogEntry {
    CatalogEntry {
        build: entry.build,
        commit: entry.commit.map(|commit| commit.to_string()),
        branch: entry.branch.as_ref().map(ToString::to_string),
        recommended: entry.recommended,
        latest: entry.latest,
        published_at: entry.published_at,
        end_of_support: entry.end_of_support.map(day),
        end_of_life: entry.end_of_life.map(day),
        support: support(entry.support(now)),
        availability: match &entry.availability {
            Offered::Downloadable { size, .. } => Availability::Downloadable { size: *size },
            Offered::NoCommit => Availability::NoCommit,
            Offered::NoArchive => Availability::NoArchive,
        },
        listed: entry.listed,
        installed,
    }
}

pub(crate) fn support(support: Support) -> SupportLevel {
    match support {
        Support::Supported => SupportLevel::Supported,
        Support::EndOfSupport => SupportLevel::EndOfSupport,
        Support::EndOfLife => SupportLevel::EndOfLife,
        Support::Unknown => SupportLevel::Unknown,
    }
}

fn day(date: PolicyDate) -> PolicyDay {
    match date {
        PolicyDate::At(at) => PolicyDay::At { at },
        PolicyDate::Never => PolicyDay::Never,
    }
}

fn warning(warning: &catalog::Warning) -> CatalogWarning {
    let source = |source: &Source| match source {
        Source::DocsPage => CatalogSource::DocsPage,
        Source::Changelog(_) => CatalogSource::Changelog,
        Source::Branch(_, branch) => CatalogSource::Branch {
            name: branch.to_string(),
        },
    };
    match warning {
        catalog::Warning::Stale {
            source: from, since, ..
        } => CatalogWarning::Stale {
            source: source(from),
            since: *since,
        },
        catalog::Warning::Missing { source: from, .. } => CatalogWarning::Missing { source: source(from) },
        catalog::Warning::CommitMismatch { build, .. } => CatalogWarning::CommitMismatch { build: *build },
    }
}
