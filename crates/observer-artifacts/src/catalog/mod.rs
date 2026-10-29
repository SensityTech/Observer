//! The builds Cfx.re publishes for a platform, gathered from five sources: the docs download page,
//! the changelogs API, the listings of the branches the caller names, the builds the support policy
//! mentions, and the builds the caller saw listed before. Each source has its own parser, so a
//! change in one leaves the others working, and a source that fails is replaced by its last good
//! answer for a day.

mod changelog;
mod docs_page;
mod listing;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use reqwest::Url;
use tokio::task::JoinSet;

use self::changelog::Changelog;
use self::docs_page::DocsPage;
use self::listing::ListedBuild;
use crate::archive::ArchiveLocation;
use crate::error::Error;
use crate::http::{Head, Http};
use crate::install::InstallRequest;
use crate::{ArchiveKind, Branch, CommitSha, Platform};

const DOCS_PAGE: &str = "https://docs.fivem.net/docs/server-download/?platform=legacy&os=";
const CHANGELOG: &str = "https://changelogs-live.fivem.net/api/changelog/versions/";
const LAST_GOOD_FOR: SignedDuration = SignedDuration::from_hours(24);
const PAGE_TIMEOUT: Duration = Duration::from_secs(60);
const API_TIMEOUT: Duration = Duration::from_secs(30);
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const PARALLEL_HEADS: usize = 6;

/// One of the places the catalog reads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    DocsPage,
    Changelog(Platform),
    Branch(Platform, Branch),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DocsPage => f.write_str("the docs download page"),
            Self::Changelog(platform) => write!(f, "the {platform} changelogs API"),
            Self::Branch(platform, branch) => write!(f, "the {platform} listing of branch {branch}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDate {
    At(Timestamp),
    Never,
}

impl PolicyDate {
    fn reached(self, now: Timestamp) -> bool {
        matches!(self, Self::At(at) if at <= now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Supported,
    /// Players see a grey banner, but can still join.
    EndOfSupport,
    /// Players can no longer join a server running it.
    EndOfLife,
    Unknown,
}

impl fmt::Display for Support {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Supported => "supported",
            Self::EndOfSupport => "end of support",
            Self::EndOfLife => "end of life",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// The archive to download. `size` comes from a `HEAD`, which also proved the archive exists.
    Downloadable {
        url: Url,
        kind: ArchiveKind,
        size: Option<u64>,
    },
    /// Only the support policy names the build, so its folder on runtime.fivem.net is unknown.
    /// Failed builds and builds of branches nobody named look like this.
    NoCommit,
    /// The build exists, but runtime.fivem.net has no archive of it for this platform.
    NoArchive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "four separate facts a source states about a build"
)]
pub struct CatalogBuild {
    pub build: u32,
    pub commit: Option<CommitSha>,
    pub branch: Option<Branch>,
    pub recommended: bool,
    pub latest: bool,
    pub availability: Availability,
    pub published_at: Option<Timestamp>,
    pub end_of_support: Option<PolicyDate>,
    pub end_of_life: Option<PolicyDate>,
    /// Older than every build of the support policy: Cfx.re stopped tracking it.
    pub predates_policy: bool,
    /// A source lists it now. A build only seen before is still offered, with what was learnt of
    /// it then: the `HEAD` of its install proves whether it is still there.
    pub listed: bool,
}

/// A downloadable build a listing named once, which the caller keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenBuild {
    pub build: u32,
    pub commit: CommitSha,
    pub branch: Branch,
    pub url: Url,
    pub published_at: Option<Timestamp>,
    pub size: Option<u64>,
}

impl CatalogBuild {
    pub fn support(&self, now: Timestamp) -> Support {
        match (self.end_of_support, self.end_of_life) {
            (None, None) if self.predates_policy => Support::EndOfLife,
            (None, None) => Support::Unknown,
            (_, Some(end_of_life)) if end_of_life.reached(now) => Support::EndOfLife,
            (Some(end_of_support), _) if end_of_support.reached(now) => Support::EndOfSupport,
            _ => Support::Supported,
        }
    }

    fn known(location: &ArchiveLocation) -> Self {
        Self {
            build: location.build,
            commit: Some(location.commit),
            branch: Some(location.branch.clone()),
            recommended: false,
            latest: false,
            availability: Availability::Downloadable {
                url: location.url(),
                kind: location.platform.archive(),
                size: None,
            },
            published_at: None,
            end_of_support: None,
            end_of_life: None,
            predates_policy: false,
            listed: true,
        }
    }

    fn policy_only(build: u32) -> Self {
        Self {
            build,
            commit: None,
            branch: None,
            recommended: false,
            latest: false,
            availability: Availability::NoCommit,
            published_at: None,
            end_of_support: None,
            end_of_life: None,
            predates_policy: false,
            listed: true,
        }
    }

    /// What a listing said of this build, when it is downloadable: what the caller keeps as seen.
    pub fn seen(&self) -> Option<SeenBuild> {
        let Availability::Downloadable { url, size, .. } = &self.availability else {
            return None;
        };
        Some(SeenBuild {
            build: self.build,
            commit: self.commit?,
            branch: self.branch.clone()?,
            url: url.clone(),
            published_at: self.published_at,
            size: *size,
        })
    }
}

/// Something the operator should know about how the catalog was put together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// The source failed; its answer from `since` stands in for it.
    Stale {
        source: Source,
        error: String,
        since: Timestamp,
    },
    /// The source failed and had no answer younger than a day.
    Missing { source: Source, error: String },
    /// The docs page and the API name different commits for the same build.
    CommitMismatch {
        build: u32,
        docs: CommitSha,
        changelog: CommitSha,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale { source, error, since } => {
                write!(f, "{error}; using the answer {source} gave at {since}")
            }
            Self::Missing { source, error } => write!(f, "{error}; the catalog goes without {source}"),
            Self::CommitMismatch {
                build,
                docs,
                changelog,
            } => write!(
                f,
                "build {build} is commit {docs} on the docs page but {changelog} in the changelogs API"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub platform: Platform,
    /// Newest first.
    pub builds: Vec<CatalogBuild>,
    pub warnings: Vec<Warning>,
}

impl Listing {
    pub fn find(&self, build: u32) -> Option<&CatalogBuild> {
        self.builds.iter().find(|entry| entry.build == build)
    }

    /// What installing `build` takes, when it can be downloaded.
    pub fn install_request(&self, build: u32) -> Option<InstallRequest> {
        let entry = self.find(build)?;
        let Availability::Downloadable { url, .. } = &entry.availability else {
            return None;
        };
        Some(InstallRequest {
            build,
            commit: entry.commit?,
            branch: entry.branch.clone()?,
            platform: self.platform,
            url: url.clone(),
            fingerprint: None,
            installed_by: None,
        })
    }
}

/// The catalog, with the last good answer of each source and what `HEAD` said about each archive.
/// Both live in memory.
#[derive(Debug)]
pub struct Catalog {
    http: Http,
    last_good: Mutex<LastGood>,
    archives: Mutex<HashMap<Url, Head>>,
    /// The listing a test catalog answers, without contacting anything.
    #[cfg(any(test, feature = "test-support"))]
    fixed: Option<Result<Listing, Source>>,
}

#[derive(Debug, Default)]
struct LastGood {
    docs: HashMap<Source, Answer<DocsPage>>,
    changelogs: HashMap<Source, Answer<Changelog>>,
    branches: HashMap<Source, Answer<Vec<ListedBuild>>>,
}

#[derive(Debug, Clone)]
struct Answer<T> {
    value: T,
    at: Timestamp,
}

impl Catalog {
    pub fn new(http: Http) -> Self {
        Self {
            http,
            last_good: Mutex::default(),
            archives: Mutex::default(),
            #[cfg(any(test, feature = "test-support"))]
            fixed: None,
        }
    }

    /// A catalog that answers `listing` to every request, or fails as `source` when it is an
    /// error, as when no source answers.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixed(listing: Result<Listing, Source>) -> Self {
        Self {
            fixed: Some(listing),
            ..Self::new(Http::new().expect("the HTTP client starts in tests"))
        }
    }

    /// Fails only when neither the docs page nor the API has an answer, fresh or from the last day.
    /// The `seen` builds no source lists any more come last, with `listed` false.
    pub async fn list(
        &self,
        platform: Platform,
        branches: &[Branch],
        seen: &[SeenBuild],
    ) -> Result<Listing, Error> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fixed) = &self.fixed {
            return match fixed {
                Ok(listing) => {
                    let mut listing = listing.clone();
                    remember(&mut listing.builds, platform, seen);
                    Ok(listing)
                }
                Err(source) => Err(Error::FormatChanged {
                    origin: source.clone(),
                    reason: "the test catalog has no answer".to_owned(),
                }),
            };
        }
        let mut listing = self.fetch(platform, branches).await?;
        remember(&mut listing.builds, platform, seen);
        Ok(listing)
    }

    async fn fetch(&self, platform: Platform, branches: &[Branch]) -> Result<Listing, Error> {
        let (docs, changelog) = tokio::join!(self.docs_page(platform), self.changelog(platform));
        let mut fetched = Vec::with_capacity(branches.len());
        for branch in branches {
            fetched.push((branch.clone(), self.branch(platform, branch).await));
        }

        let now = Timestamp::now();
        let mut warnings = Vec::new();
        let mut listed = Vec::with_capacity(fetched.len());
        let (docs, changelog) = {
            let mut last_good = self.last_good.lock().expect("the catalog lock is never poisoned");
            let docs = fall_back(&mut last_good.docs, Source::DocsPage, docs, now, &mut warnings);
            let changelog = fall_back(
                &mut last_good.changelogs,
                Source::Changelog(platform),
                changelog,
                now,
                &mut warnings,
            );
            for (branch, result) in fetched {
                let source = Source::Branch(platform, branch.clone());
                match fall_back(
                    &mut last_good.branches,
                    source.clone(),
                    result,
                    now,
                    &mut warnings,
                ) {
                    Ok(builds) => listed.push((branch, builds)),
                    Err(error) => warnings.push(Warning::Missing {
                        source,
                        error: error.to_string(),
                    }),
                }
            }
            (docs, changelog)
        };
        let (docs, changelog) = match (docs, changelog) {
            (Err(error), Err(_)) => return Err(error),
            (docs, changelog) => (
                without(docs, Source::DocsPage, &mut warnings),
                without(changelog, Source::Changelog(platform), &mut warnings),
            ),
        };

        let mut listing = merge(platform, docs.as_ref(), changelog.as_ref(), &listed);
        warnings.append(&mut listing.warnings);
        listing.warnings = warnings;
        self.probe(&mut listing.builds).await;
        Ok(listing)
    }

    async fn docs_page(&self, platform: Platform) -> Result<DocsPage, Error> {
        let url = Url::parse(&format!("{DOCS_PAGE}{platform}")).expect("the docs page URL parses");
        docs_page::parse(&self.http.page(&url, PAGE_TIMEOUT).await?)
    }

    async fn changelog(&self, platform: Platform) -> Result<Changelog, Error> {
        let name = match platform {
            Platform::Windows => "win32",
            Platform::Linux => "linux",
        };
        let url = Url::parse(&format!("{CHANGELOG}{name}/server")).expect("the changelogs URL parses");
        changelog::parse(&self.http.page(&url, API_TIMEOUT).await?, platform)
    }

    async fn branch(&self, platform: Platform, branch: &Branch) -> Result<Vec<ListedBuild>, Error> {
        let url = Url::parse(&listing::url(platform, branch)).expect("a validated branch makes a valid URL");
        listing::parse(&self.http.page(&url, PAGE_TIMEOUT).await?, platform, branch)
    }

    /// Dates the archives no source dated with a `HEAD`, which also gives their size and proves they
    /// exist: a 404 means this platform has no archive of the build.
    async fn probe(&self, builds: &mut [CatalogBuild]) {
        let mut unknown = Vec::new();
        {
            let archives = self.archives.lock().expect("the catalog lock is never poisoned");
            for (index, entry) in builds.iter_mut().enumerate() {
                let Availability::Downloadable { url, .. } = &entry.availability else {
                    continue;
                };
                if entry.published_at.is_some() {
                    continue;
                }
                match archives.get(url).copied() {
                    Some(head) => apply(entry, head),
                    None => unknown.push((index, url.clone())),
                }
            }
        }

        let mut probes = JoinSet::new();
        let mut queue = unknown.into_iter();
        loop {
            while probes.len() < PARALLEL_HEADS
                && let Some((index, url)) = queue.next()
            {
                let http = self.http.clone();
                probes.spawn(async move {
                    let head = http.head(&url, HEAD_TIMEOUT).await;
                    (index, url, head)
                });
            }
            let Some(joined) = probes.join_next().await else {
                break;
            };
            let (index, url, head) =
                joined.unwrap_or_else(|error| std::panic::resume_unwind(error.into_panic()));
            match head {
                Ok(head) => {
                    self.archives
                        .lock()
                        .expect("the catalog lock is never poisoned")
                        .insert(url, head);
                    apply(&mut builds[index], head);
                }
                Err(Error::Status { status: 404, .. }) => {
                    builds[index].availability = Availability::NoArchive;
                }
                Err(error) => tracing::debug!(%url, %error, "cannot date the archive"),
            }
        }
    }
}

/// Adds the builds seen before that no source lists now, and gives a build only the policy names
/// the folder a listing gave it then.
fn remember(builds: &mut Vec<CatalogBuild>, platform: Platform, seen: &[SeenBuild]) {
    let first_tracked = builds
        .iter()
        .filter(|entry| entry.end_of_support.is_some() || entry.end_of_life.is_some())
        .map(|entry| entry.build)
        .min();
    for seen in seen {
        let found = builds.iter_mut().find(|entry| entry.build == seen.build);
        if found.as_ref().is_some_and(|entry| entry.commit.is_some()) {
            continue;
        }
        let known = CatalogBuild {
            commit: Some(seen.commit),
            branch: Some(seen.branch.clone()),
            availability: Availability::Downloadable {
                url: seen.url.clone(),
                kind: platform.archive(),
                size: seen.size,
            },
            published_at: seen.published_at,
            listed: false,
            ..CatalogBuild::policy_only(seen.build)
        };
        match found {
            Some(entry) => {
                *entry = CatalogBuild {
                    end_of_support: entry.end_of_support,
                    end_of_life: entry.end_of_life,
                    predates_policy: entry.predates_policy,
                    ..known
                };
            }
            None => builds.push(CatalogBuild {
                predates_policy: first_tracked.is_some_and(|first| seen.build < first),
                ..known
            }),
        }
    }
    builds.sort_by_key(|entry| std::cmp::Reverse(entry.build));
}

fn apply(entry: &mut CatalogBuild, head: Head) {
    entry.published_at = head.last_modified;
    if let Availability::Downloadable { size, .. } = &mut entry.availability {
        *size = head.length;
    }
}

/// Keeps a fresh answer as the source's last good one, or stands in with the last good one while it
/// is younger than a day.
fn fall_back<T: Clone>(
    answers: &mut HashMap<Source, Answer<T>>,
    source: Source,
    fetched: Result<T, Error>,
    now: Timestamp,
    warnings: &mut Vec<Warning>,
) -> Result<T, Error> {
    match fetched {
        Ok(value) => {
            answers.insert(
                source,
                Answer {
                    value: value.clone(),
                    at: now,
                },
            );
            Ok(value)
        }
        Err(error) => match answers.get(&source) {
            Some(answer) if now.duration_since(answer.at) <= LAST_GOOD_FOR => {
                warnings.push(Warning::Stale {
                    source,
                    error: error.to_string(),
                    since: answer.at,
                });
                Ok(answer.value.clone())
            }
            _ => Err(error),
        },
    }
}

fn without<T>(result: Result<T, Error>, source: Source, warnings: &mut Vec<Warning>) -> Option<T> {
    result
        .map_err(|error| {
            warnings.push(Warning::Missing {
                source,
                error: error.to_string(),
            });
        })
        .ok()
}

/// Puts the sources together into one entry per build, newest first.
fn merge(
    platform: Platform,
    docs: Option<&DocsPage>,
    changelog: Option<&Changelog>,
    branches: &[(Branch, Vec<ListedBuild>)],
) -> Listing {
    let mut builds: BTreeMap<u32, CatalogBuild> = BTreeMap::new();
    let mut warnings = Vec::new();

    if let Some(docs) = docs {
        let lists = docs.lists(platform);
        for location in &lists.recommended {
            builds
                .entry(location.build)
                .or_insert_with(|| CatalogBuild::known(location))
                .recommended = true;
        }
        for location in &lists.latest {
            builds
                .entry(location.build)
                .or_insert_with(|| CatalogBuild::known(location));
        }
        // The other platform's lists give the commits of builds this one leaves out; the `HEAD` that
        // dates them then tells whether this platform has an archive.
        let other = docs.lists(platform.other());
        for location in other.recommended.iter().chain(&other.latest) {
            builds
                .entry(location.build)
                .or_insert_with(|| CatalogBuild::known(&location.on(platform)));
        }
    }

    let mut first_tracked = None;
    if let Some(changelog) = changelog {
        // These carry their commit, so recommended and latest stay installable without the docs page.
        for (location, recommended) in [(&changelog.recommended, true), (&changelog.latest, false)] {
            let entry = builds
                .entry(location.build)
                .or_insert_with(|| CatalogBuild::known(location));
            if let Some(docs) = entry.commit.filter(|commit| *commit != location.commit) {
                warnings.push(Warning::CommitMismatch {
                    build: location.build,
                    docs,
                    changelog: location.commit,
                });
            }
            if recommended {
                entry.recommended = true;
            } else {
                entry.latest = true;
            }
        }
        for (build, date) in &changelog.end_of_support {
            builds
                .entry(*build)
                .or_insert_with(|| CatalogBuild::policy_only(*build))
                .end_of_support = Some(*date);
        }
        for (build, date) in &changelog.end_of_life {
            builds
                .entry(*build)
                .or_insert_with(|| CatalogBuild::policy_only(*build))
                .end_of_life = Some(*date);
        }
        first_tracked = changelog
            .end_of_support
            .keys()
            .chain(changelog.end_of_life.keys())
            .min()
            .copied();
    } else if let Some(newest) = docs.and_then(|docs| {
        docs.lists(platform)
            .latest
            .iter()
            .map(|location| location.build)
            .max()
    }) && let Some(entry) = builds.get_mut(&newest)
    {
        // Without the API, the newest build the docs page lists stands in for latest.
        entry.latest = true;
    }

    for (_, listed) in branches {
        for ListedBuild {
            location,
            published_at,
        } in listed
        {
            let entry = builds
                .entry(location.build)
                .or_insert_with(|| CatalogBuild::policy_only(location.build));
            // Build numbers are shared by every branch: one another source already placed stays there.
            if entry.commit.is_some() {
                continue;
            }
            *entry = CatalogBuild {
                published_at: *published_at,
                end_of_support: entry.end_of_support,
                end_of_life: entry.end_of_life,
                ..CatalogBuild::known(&location.on(platform))
            };
        }
    }

    for entry in builds.values_mut() {
        entry.predates_policy = first_tracked.is_some_and(|first| entry.build < first);
    }
    Listing {
        platform,
        builds: builds.into_values().rev().collect(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, TestServer};

    const DOCS: &str = include_str!("../../tests/fixtures/docs-page.html");
    const WINDOWS: &str = include_str!("../../tests/fixtures/changelog-win32.json");
    const LINUX: &str = include_str!("../../tests/fixtures/changelog-linux.json");
    const BRANCH: &str = include_str!("../../tests/fixtures/branch-windows.html");

    fn docs() -> DocsPage {
        docs_page::parse(DOCS).unwrap()
    }

    fn api(platform: Platform) -> Changelog {
        match platform {
            Platform::Windows => changelog::parse(WINDOWS, platform).unwrap(),
            Platform::Linux => changelog::parse(LINUX, platform).unwrap(),
        }
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn feature() -> Branch {
        "feature/network-improvements-clean".parse().unwrap()
    }

    #[test]
    fn the_catalog_joins_the_docs_page_the_api_and_the_policy() {
        let listing = merge(
            Platform::Windows,
            Some(&docs()),
            Some(&api(Platform::Windows)),
            &[],
        );

        assert!(listing.warnings.is_empty(), "{:?}", listing.warnings);
        assert_eq!(listing.builds[0].build, 37944);
        let latest = listing.find(37872).unwrap();
        assert!(latest.latest && !latest.recommended);
        assert_eq!(latest.branch, Some(Branch::master()));
        assert_eq!(latest.end_of_support, Some(PolicyDate::Never));
        assert_eq!(
            latest.end_of_life,
            Some(PolicyDate::At(at("2027-01-26T12:08:14Z")))
        );
        assert_eq!(
            latest.availability,
            Availability::Downloadable {
                url: "https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/37872-0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607/server.zip"
                    .parse()
                    .unwrap(),
                kind: ArchiveKind::Zip,
                size: None,
            }
        );
        assert!(listing.find(35245).unwrap().recommended);
        assert_eq!(listing.builds.iter().filter(|entry| entry.latest).count(), 1);
    }

    #[test]
    fn builds_only_the_policy_names_are_marked_not_downloadable() {
        let listing = merge(
            Platform::Windows,
            Some(&docs()),
            Some(&api(Platform::Windows)),
            &[],
        );

        // 37944 is newer than latest and not listed yet, 37739 failed, 37862 is a branch build.
        for build in [37944, 37739, 37862] {
            let entry = listing.find(build).unwrap();
            assert_eq!(entry.availability, Availability::NoCommit, "{build}");
            assert_eq!(entry.commit, None);
        }
        assert_eq!(listing.install_request(37739), None);
    }

    #[test]
    fn a_build_listed_for_the_other_platform_only_is_kept_for_a_head_to_settle() {
        let listing = merge(Platform::Linux, Some(&docs()), Some(&api(Platform::Linux)), &[]);

        assert_eq!(
            listing.find(36727).unwrap().availability,
            Availability::Downloadable {
                url: "https://runtime.fivem.net/artifacts/fivem/build_proot_linux/master/36727-e60d29ac2d6e894e20ba78d5fdf3c190d976fd2a/fx.tar.xz"
                    .parse()
                    .unwrap(),
                kind: ArchiveKind::TarXz,
                size: None,
            }
        );
    }

    #[test]
    fn recommended_and_latest_stay_installable_when_the_docs_page_breaks() {
        let listing = merge(Platform::Linux, None, Some(&api(Platform::Linux)), &[]);

        let request = listing.install_request(37872).unwrap();
        assert_eq!(
            request.commit.to_string(),
            "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607"
        );
        assert_eq!(request.platform, Platform::Linux);
        assert!(
            request
                .url
                .as_str()
                .ends_with("/master/37872-0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607/fx.tar.xz")
        );
        assert!(listing.find(35245).unwrap().recommended);
        assert!(listing.install_request(35245).is_some());
    }

    #[test]
    fn without_the_api_the_newest_listed_build_is_latest_and_support_is_unknown() {
        let listing = merge(Platform::Windows, Some(&docs()), None, &[]);

        let newest = listing.find(37872).unwrap();
        assert!(newest.latest);
        assert_eq!(newest.support(at("2026-10-28T12:00:00Z")), Support::Unknown);
        assert!(listing.find(35245).unwrap().recommended);
    }

    #[test]
    fn support_follows_the_policy_dates() {
        let listing = merge(
            Platform::Windows,
            Some(&docs()),
            Some(&api(Platform::Windows)),
            &[],
        );
        let support = |build, now| listing.find(build).unwrap().support(at(now));

        assert_eq!(support(37872, "2026-10-28T12:00:00Z"), Support::Supported);
        assert_eq!(support(35245, "2026-10-28T12:00:00Z"), Support::Supported);
        assert_eq!(support(37287, "2026-10-28T12:00:00Z"), Support::EndOfSupport);
        assert_eq!(support(34629, "2026-10-28T12:00:00Z"), Support::EndOfSupport);
        assert_eq!(support(34629, "2026-11-08T00:00:00Z"), Support::EndOfLife);
    }

    #[test]
    fn a_build_older_than_the_policy_is_end_of_life() {
        let mut changelog = api(Platform::Windows);
        changelog.end_of_support.retain(|build, _| *build >= 35000);
        changelog.end_of_life.retain(|build, _| *build >= 35000);

        let listing = merge(Platform::Windows, Some(&docs()), Some(&changelog), &[]);

        let old = listing.find(34629).unwrap();
        assert!(old.predates_policy);
        assert_eq!(old.support(at("2026-10-28T12:00:00Z")), Support::EndOfLife);
    }

    #[test]
    fn a_commit_the_sources_disagree_on_is_reported() {
        let mut changelog = api(Platform::Windows);
        changelog.latest.commit = "1111111111111111111111111111111111111111".parse().unwrap();

        let listing = merge(Platform::Windows, Some(&docs()), Some(&changelog), &[]);

        assert_eq!(
            listing.warnings,
            [Warning::CommitMismatch {
                build: 37872,
                docs: "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607".parse().unwrap(),
                changelog: "1111111111111111111111111111111111111111".parse().unwrap(),
            }]
        );
    }

    #[test]
    fn a_branch_build_gets_its_folder_and_date_from_the_listing() {
        let branch = listing::parse(BRANCH, Platform::Windows, &feature()).unwrap();

        let listing = merge(
            Platform::Windows,
            Some(&docs()),
            Some(&api(Platform::Windows)),
            &[(feature(), branch)],
        );

        let entry = listing.find(37862).unwrap();
        assert_eq!(entry.branch, Some(feature()));
        assert_eq!(entry.published_at, Some(at("2026-10-25T16:56:54Z")));
        assert_eq!(
            entry.end_of_support,
            Some(PolicyDate::At(at("2026-11-08T16:41:24Z")))
        );
        assert_eq!(
            listing.install_request(37862).unwrap().url.as_str(),
            "https://runtime.fivem.net/artifacts/fivem/build_server_windows/feature/network-improvements-clean/37862-358a6175a19018b0b8c951562fcb0ab8caf203fd/server.zip"
        );
    }

    #[test]
    fn a_failed_source_stands_in_with_its_last_good_answer_for_a_day() {
        let mut answers = HashMap::new();
        let mut warnings = Vec::new();
        let source = Source::Changelog(Platform::Linux);
        let broken = || changelog::parse("null", Platform::Linux).map(|_| 0);

        let fresh = fall_back(
            &mut answers,
            source.clone(),
            Ok(1),
            at("2026-10-28T00:00:00Z"),
            &mut warnings,
        );
        let stale = fall_back(
            &mut answers,
            source.clone(),
            broken(),
            at("2026-10-28T23:00:00Z"),
            &mut warnings,
        );
        let expired = fall_back(
            &mut answers,
            source,
            broken(),
            at("2026-10-29T01:00:00Z"),
            &mut warnings,
        );

        assert_eq!(fresh.unwrap(), 1);
        assert_eq!(stale.unwrap(), 1);
        assert!(matches!(expired, Err(Error::FormatChanged { .. })));
        assert_eq!(warnings.len(), 1);
        assert!(
            matches!(
                &warnings[0],
                Warning::Stale { since, error, .. }
                    if *since == at("2026-10-28T00:00:00Z") && error.contains("changed format")
            ),
            "{warnings:?}"
        );
    }

    fn seen(build: u32) -> SeenBuild {
        SeenBuild {
            build,
            commit: "1111111111111111111111111111111111111111".parse().unwrap(),
            branch: Branch::master(),
            url: format!(
                "https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/{build}-1111111111111111111111111111111111111111/server.zip"
            )
            .parse()
            .unwrap(),
            published_at: Some(at("2026-09-01T10:00:00Z")),
            size: Some(66_000_000),
        }
    }

    #[test]
    fn a_build_seen_before_is_offered_unlisted_and_one_still_listed_is_left_as_it_is() {
        let mut listing = merge(
            Platform::Windows,
            Some(&docs()),
            Some(&api(Platform::Windows)),
            &[],
        );

        // 37739 is policy-only now, 36000 nothing names any more, 37872 is listed.
        remember(
            &mut listing.builds,
            Platform::Windows,
            &[seen(37739), seen(36000), seen(37872)],
        );

        let failed = listing.find(37739).unwrap();
        assert!(!failed.listed);
        assert!(failed.end_of_support.is_some() || failed.end_of_life.is_some());
        assert_eq!(failed.published_at, Some(at("2026-09-01T10:00:00Z")));
        assert!(listing.install_request(37739).is_some());
        let gone = listing.find(36000).unwrap();
        assert!(!gone.listed);
        assert_eq!(
            gone.availability,
            Availability::Downloadable {
                url: seen(36000).url,
                kind: ArchiveKind::Zip,
                size: Some(66_000_000),
            }
        );
        let latest = listing.find(37872).unwrap();
        assert!(latest.listed);
        assert_eq!(
            latest.commit.unwrap().to_string(),
            "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607"
        );
        assert!(
            listing
                .builds
                .windows(2)
                .all(|pair| pair[0].build > pair[1].build)
        );
        assert_eq!(latest.seen().unwrap().build, 37872);
    }

    #[tokio::test]
    async fn a_head_dates_an_archive_and_a_404_marks_it_missing() {
        let server = TestServer::start(|request, _| match request.path.as_str() {
            "/37872/server.zip" => {
                Reply::ok(vec![0; 1234]).header("Last-Modified", "Mon, 26 Oct 2026 12:24:47 GMT")
            }
            _ => Reply::status(404),
        })
        .await;
        let catalog = Catalog::new(Http::local(server.port));
        let downloadable = |path: &str| Availability::Downloadable {
            url: server.url(path),
            kind: ArchiveKind::Zip,
            size: None,
        };
        let mut builds = vec![
            CatalogBuild {
                availability: downloadable("/37872/server.zip"),
                ..CatalogBuild::policy_only(37872)
            },
            CatalogBuild {
                availability: downloadable("/36727/server.zip"),
                ..CatalogBuild::policy_only(36727)
            },
        ];

        catalog.probe(&mut builds).await;
        builds[0].published_at = None;
        catalog.probe(&mut builds).await;

        assert_eq!(builds[0].published_at, Some(at("2026-10-26T12:24:47Z")));
        assert_eq!(
            builds[0].availability,
            Availability::Downloadable {
                url: server.url("/37872/server.zip"),
                kind: ArchiveKind::Zip,
                size: Some(1234),
            }
        );
        assert_eq!(builds[1].availability, Availability::NoArchive);
        let requests = server.requests();
        assert!(requests.iter().all(|request| request.method == "HEAD"));
        assert_eq!(
            requests.len(),
            2,
            "the second probe answers from what the first learned"
        );
    }
}
