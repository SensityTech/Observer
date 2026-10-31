//! The listing: one walk of `resources/` that finds the resources the way FXServer does and lists
//! every entry with its size and time, reading no file but the manifests, git heads and npm
//! package files. Links and junctions are followed, as FXServer follows them; a loop is cut.

use std::collections::HashMap;
use std::fs::{self, DirEntry, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use observer_domain::changes::{GitHead, Manifest, Package, glob_covers, hidden};
use observer_domain::files::UPLOAD_PREFIX;
use tokio_util::sync::CancellationToken;

use super::{Cancelled, Limits, ScanSettings, git, manifest, packages};

/// Folder names FXServer passes over when it looks for resources.
const NOT_RESOURCES: [&str; 6] = ["cache", "db", "node_modules", ".git", ".idea", ".vscode"];
/// Folders inside a resource that are never walked: `node_modules` and `.git` are read apart.
const NEVER_WALKED: [&str; 2] = [".idea", ".vscode"];
const MANIFESTS: [&str; 2] = ["fxmanifest.lua", "__resource.lua"];
const MAX_MANIFEST: u64 = 256 * 1024;
const MAX_PACKAGES: usize = 2000;

/// One entry of a resource as the listing saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// From the data folder, `/`-separated.
    pub path: String,
    /// Where it is read from, which differs from `path` below a link.
    pub location: PathBuf,
    pub found: Found,
    pub stat: Stat,
}

/// What the listing found at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    File,
    /// A link and its target as written. A followed link's files are listed below its path.
    Link {
        target: String,
        followed: bool,
    },
    /// A cloud placeholder or another reparse point, listed by size and time and never read.
    Offline,
    /// A name that is not UTF-8, a path past the limit, a folder past the depth limit, or an entry
    /// whose metadata cannot be read.
    Unreadable,
}

/// What a file's metadata says, which the index compares to tell an unchanged file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stat {
    pub size: u64,
    pub modified_ns: i64,
    /// Linux only: the inode and the change time, which no copy tool sets back.
    pub inode: Option<u64>,
    pub changed_ns: Option<i64>,
}

impl Stat {
    pub fn of(metadata: &Metadata) -> Self {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| jiff::Timestamp::try_from(time).ok())
            .and_then(|time| i64::try_from(time.as_nanosecond()).ok())
            .unwrap_or(0);
        #[cfg(unix)]
        let (inode, changed_ns) = {
            use std::os::unix::fs::MetadataExt;
            (
                Some(metadata.ino()),
                metadata
                    .ctime()
                    .checked_mul(1_000_000_000)
                    .map(|ns| ns + metadata.ctime_nsec()),
            )
        };
        #[cfg(not(unix))]
        let (inode, changed_ns) = (None, None);
        Self {
            size: metadata.len(),
            modified_ns,
            inode,
            changed_ns,
        }
    }
}

/// A resource folder and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceListing {
    /// `resources/[qb]/qb-core`.
    pub path: String,
    /// In lower case, as FXServer compares names.
    pub name: String,
    pub shown_name: String,
    /// Another folder of the listing has the same name.
    pub duplicate: bool,
    pub manifest: Option<Manifest>,
    /// A `.fxap` file at its root: the Cfx.re escrow encrypted some of its files.
    pub escrowed: bool,
    pub git_head: Option<GitHead>,
    pub packages: Vec<Package>,
    pub entries: Vec<Entry>,
}

impl ResourceListing {
    pub fn files(&self) -> u64 {
        self.entries
            .iter()
            .filter(|entry| !matches!(entry.found, Found::Link { followed: true, .. }))
            .count() as u64
    }

    pub fn bytes(&self) -> u64 {
        self.entries.iter().map(|entry| entry.stat.size).sum()
    }
}

/// Why a listing stopped short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortened {
    /// It reached the file limit.
    Files,
    /// It reached the resource limit.
    Resources,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub resources: Vec<ResourceListing>,
    pub shortened: Option<Shortened>,
}

impl Listing {
    pub fn files(&self) -> u64 {
        self.resources.iter().map(ResourceListing::files).sum()
    }

    pub fn bytes(&self) -> u64 {
        self.resources.iter().map(ResourceListing::bytes).sum()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error("the server's resources folder cannot be read: {0}")]
    Unreadable(io::Error),
    #[error("the listing was cancelled")]
    Cancelled,
}

impl From<Cancelled> for ListError {
    fn from(_: Cancelled) -> Self {
        Self::Cancelled
    }
}

/// Lists `resources/` of the data folder `data`.
pub fn list(data: &Path, settings: &ScanSettings, cancel: &CancellationToken) -> Result<Listing, ListError> {
    let folder = data.join("resources");
    fs::read_dir(&folder).map_err(ListError::Unreadable)?;
    let mut walk = Walk {
        settings,
        limits: &settings.limits,
        root: fs::canonicalize(data).ok(),
        never_read: settings
            .never_read
            .iter()
            .filter_map(|path| fs::canonicalize(path).ok())
            .collect(),
        cancel,
        listing: Listing::default(),
        files: 0,
    };
    walk.categories(folder, "resources".to_owned())?;
    let mut names: HashMap<String, usize> = HashMap::new();
    for resource in &walk.listing.resources {
        *names.entry(resource.name.clone()).or_default() += 1;
    }
    for resource in &mut walk.listing.resources {
        resource.duplicate = names.get(&resource.name).is_some_and(|count| *count > 1);
    }
    Ok(walk.listing)
}

struct Walk<'a> {
    settings: &'a ScanSettings,
    limits: &'a Limits,
    /// The data folder as the file system resolves it.
    root: Option<PathBuf>,
    /// Observer's own folders, resolved: never read, even through a link.
    never_read: Vec<PathBuf>,
    cancel: &'a CancellationToken,
    listing: Listing,
    files: usize,
}

/// A folder of a resource still to walk.
struct Pending {
    location: PathBuf,
    path: String,
    depth: usize,
}

impl Walk<'_> {
    /// Walks the categories breadth first, in name order, as FXServer finds the resources.
    fn categories(&mut self, folder: PathBuf, path: String) -> Result<(), Cancelled> {
        let mut queue = std::collections::VecDeque::from([(folder, path, 1)]);
        while let Some((folder, path, depth)) = queue.pop_front() {
            for entry in sorted(&folder).unwrap_or_default() {
                self.check()?;
                if self.listing.shortened.is_some() {
                    return Ok(());
                }
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let child = format!("{path}/{name}");
                if self.excluded(&child) || !self.folder(&entry) {
                    continue;
                }
                let lower = name.to_ascii_lowercase();
                if name.starts_with('[') && name.ends_with(']') {
                    if depth < self.limits.depth {
                        queue.push_back((entry.path(), child, depth + 1));
                    }
                } else if !NOT_RESOURCES.contains(&lower.as_str()) && !name.starts_with(UPLOAD_PREFIX) {
                    if self.listing.resources.len() >= self.limits.resources {
                        self.listing.shortened = Some(Shortened::Resources);
                        return Ok(());
                    }
                    self.resource(entry.path(), child, name, lower, depth)?;
                }
            }
        }
        Ok(())
    }

    /// Whether the entry is a folder, through a link that may be followed.
    fn folder(&self, entry: &DirEntry) -> bool {
        let Ok(kind) = entry.file_type() else {
            return false;
        };
        if kind.is_symlink() {
            return self
                .followable(&entry.path())
                .is_some_and(|target| target.is_dir());
        }
        kind.is_dir()
    }

    fn resource(
        &mut self,
        location: PathBuf,
        path: String,
        shown_name: String,
        name: String,
        depth: usize,
    ) -> Result<(), Cancelled> {
        let mut resource = ResourceListing {
            manifest: MANIFESTS.into_iter().find_map(|file| {
                let text = read_small(&location.join(file), MAX_MANIFEST)?;
                Some(manifest::parse(file, &text))
            }),
            escrowed: sorted(&location).unwrap_or_default().into_iter().any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with(".fxap"))
            }),
            git_head: git::head(&location.join(".git")),
            path: path.clone(),
            name,
            shown_name,
            duplicate: false,
            packages: Vec::new(),
            entries: Vec::new(),
        };
        let resolved = fs::canonicalize(&location).ok();
        let mut stack = vec![(
            Pending {
                location,
                path,
                depth,
            },
            resolved,
        )];
        while let Some((folder, resolved)) = stack.pop() {
            self.check()?;
            let Ok(entries) = sorted(&folder.location) else {
                unreadable(&mut resource, folder.location, folder.path);
                continue;
            };
            // Reversed so that the stack walks folders in name order.
            let mut folders = Vec::new();
            for entry in entries {
                if self.files >= self.limits.files {
                    self.listing.shortened = Some(Shortened::Files);
                    break;
                }
                self.entry(&mut resource, &folder, resolved.as_deref(), &entry, &mut folders);
            }
            stack.extend(folders.into_iter().rev());
        }
        resource.packages.sort();
        resource.packages.dedup();
        self.listing.resources.push(resource);
        Ok(())
    }

    fn entry(
        &mut self,
        resource: &mut ResourceListing,
        folder: &Pending,
        resolved: Option<&Path>,
        entry: &DirEntry,
        folders: &mut Vec<(Pending, Option<PathBuf>)>,
    ) {
        let location = entry.path();
        let Ok(name) = entry.file_name().into_string() else {
            let path = format!("{}/{}", folder.path, entry.file_name().to_string_lossy());
            self.files += 1;
            unreadable(resource, location, path);
            return;
        };
        let path = format!("{}/{name}", folder.path);
        if name.starts_with(UPLOAD_PREFIX) || self.excluded(&path) {
            return;
        }
        if path.len() > self.limits.path {
            self.files += 1;
            unreadable(resource, location, path);
            return;
        }
        let Ok(kind) = entry.file_type() else {
            self.files += 1;
            unreadable(resource, location, path);
            return;
        };
        let mut metadata = entry.metadata().ok();
        let mut child = None;
        if kind.is_symlink() {
            let target = fs::read_link(&location)
                .map(|target| target.to_string_lossy().into_owned())
                .unwrap_or_default();
            let followed = self
                .followable(&location)
                .filter(|resolved_target| {
                    // A target that holds the folder being walked leads back to it.
                    resolved.is_none_or(|here| !here.starts_with(resolved_target))
                })
                .and_then(|resolved_target| Some((fs::metadata(&location).ok()?, resolved_target)));
            // A followed link to a file is listed as that file, at the link's path.
            if followed.as_ref().is_none_or(|(target, _)| target.is_dir()) {
                self.files += 1;
                resource.entries.push(Entry {
                    path: path.clone(),
                    location: location.clone(),
                    found: Found::Link {
                        target,
                        followed: followed.is_some(),
                    },
                    stat: Stat::default(),
                });
            }
            let Some((target_metadata, resolved_target)) = followed else {
                return;
            };
            if target_metadata.is_dir() {
                child = Some(resolved_target);
            }
            metadata = Some(target_metadata);
        }
        let Some(metadata) = metadata else {
            self.files += 1;
            unreadable(resource, location, path);
            return;
        };
        if metadata.is_dir() {
            let lower = name.to_ascii_lowercase();
            if lower == "node_modules" {
                packages::read(&location, &mut resource.packages, MAX_PACKAGES);
                return;
            }
            if lower == ".git" || NEVER_WALKED.contains(&lower.as_str()) {
                return;
            }
            if folder.depth + 1 > self.limits.depth {
                self.files += 1;
                unreadable(resource, location, path);
                return;
            }
            let resolved = child.or_else(|| resolved.map(|here| here.join(&name)));
            folders.push((
                Pending {
                    location,
                    path,
                    depth: folder.depth + 1,
                },
                resolved,
            ));
            return;
        }
        self.files += 1;
        let found = if offline(&metadata) {
            Found::Offline
        } else {
            Found::File
        };
        resource.entries.push(Entry {
            path,
            location,
            found,
            stat: Stat::of(&metadata),
        });
    }

    /// The resolved target of a link when it may be followed: not into Observer's own folders,
    /// nor onto what the owner excluded or hid, nor onto a name never shown.
    fn followable(&self, link: &Path) -> Option<PathBuf> {
        let target = fs::canonicalize(link).ok()?;
        if self.never_read.iter().any(|folder| target.starts_with(folder)) {
            return None;
        }
        let absolute = target.to_string_lossy().replace('\\', "/");
        if hidden(&absolute, &[]) {
            return None;
        }
        if let Some(inside) = self
            .root
            .as_deref()
            .and_then(|root| target.strip_prefix(root).ok())
        {
            let relative = inside.to_string_lossy().replace('\\', "/");
            if self.excluded(&relative) || hidden(&relative, &self.settings.hidden) {
                return None;
            }
            // FXServer's cache is never read, whichever path leads to it.
            if relative
                .split('/')
                .next()
                .is_some_and(|first| first.eq_ignore_ascii_case("cache"))
            {
                return None;
            }
        }
        Some(target)
    }

    fn excluded(&self, path: &str) -> bool {
        self.settings.excluded.iter().any(|glob| glob_covers(glob, path))
    }

    fn check(&self) -> Result<(), Cancelled> {
        if self.cancel.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

fn unreadable(resource: &mut ResourceListing, location: PathBuf, path: String) {
    resource.entries.push(Entry {
        path,
        location,
        found: Found::Unreadable,
        stat: Stat::default(),
    });
}

/// The entries of a folder in name order.
fn sorted(folder: &Path) -> io::Result<Vec<DirEntry>> {
    let mut entries: Vec<DirEntry> = fs::read_dir(folder)?.filter_map(Result::ok).collect();
    entries.sort_by_key(DirEntry::file_name);
    Ok(entries)
}

fn read_small(path: &Path, limit: u64) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A file that reading would recall from the cloud, or a reparse point that is not a link.
#[cfg(windows)]
fn offline(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    metadata.file_attributes()
        & (FILE_ATTRIBUTE_REPARSE_POINT
            | FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

#[cfg(not(windows))]
fn offline(metadata: &Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    let kind = metadata.file_type();
    // A pipe or a device would block or never end.
    kind.is_fifo() || kind.is_socket() || kind.is_block_device() || kind.is_char_device()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Server {
        dir: tempfile::TempDir,
    }

    impl Server {
        fn new() -> Self {
            let server = Self {
                dir: tempfile::tempdir().unwrap(),
            };
            fs::create_dir_all(server.dir.path().join("resources")).unwrap();
            server
        }

        fn data(&self) -> &Path {
            self.dir.path()
        }

        fn file(&self, path: &str, contents: &str) {
            let path = self.data().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        fn list(&self, settings: &ScanSettings) -> Listing {
            list(self.data(), settings, &CancellationToken::new()).unwrap()
        }
    }

    fn paths(resource: &ResourceListing) -> Vec<&str> {
        resource.entries.iter().map(|entry| entry.path.as_str()).collect()
    }

    /// A link to a folder: a junction on Windows, which needs no administrator.
    fn link_folder(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            // mklink takes backslashes only.
            let native = |path: &Path| path.to_string_lossy().replace('/', "\\");
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(native(link))
                .arg(native(target))
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    #[test]
    fn resources_are_found_as_fxserver_finds_them() {
        let server = Server::new();
        server.file("resources/[qb]/qb-core/fxmanifest.lua", "version '1.2.0'");
        server.file("resources/[qb]/[jobs]/police/client.lua", "");
        server.file("resources/chat/fxmanifest.lua", "");
        server.file("resources/[qb]/stray.lua", "");
        server.file("resources/node_modules/x/index.js", "");

        let listing = server.list(&ScanSettings::default());

        let names: Vec<&str> = listing.resources.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            names,
            [
                "resources/chat",
                "resources/[qb]/qb-core",
                "resources/[qb]/[jobs]/police"
            ]
        );
        let core = &listing.resources[1];
        assert_eq!(core.name, "qb-core");
        assert_eq!(core.manifest.as_ref().unwrap().version.as_deref(), Some("1.2.0"));
        assert_eq!(paths(core), ["resources/[qb]/qb-core/fxmanifest.lua"]);
    }

    #[test]
    fn two_folders_with_one_name_are_both_listed_as_duplicates() {
        let server = Server::new();
        server.file("resources/[a]/Map/a.lua", "");
        server.file("resources/[b]/map/a.lua", "");
        server.file("resources/other/a.lua", "");

        let listing = server.list(&ScanSettings::default());

        let duplicates: Vec<(&str, bool)> = listing
            .resources
            .iter()
            .map(|r| (r.shown_name.as_str(), r.duplicate))
            .collect();
        assert_eq!(duplicates, [("other", false), ("Map", true), ("map", true)]);
    }

    #[test]
    fn node_modules_and_git_are_read_apart_and_never_walked() {
        let server = Server::new();
        server.file("resources/web/package.json", "{}");
        server.file(
            "resources/web/node_modules/axios/package.json",
            r#"{ "name": "axios", "version": "1.7.2" }"#,
        );
        server.file("resources/web/node_modules/axios/index.js", "");
        server.file("resources/web/.git/HEAD", "ref: refs/heads/main\n");
        server.file("resources/web/.git/config", "[remote]");
        server.file("resources/web/.vscode/settings.json", "{}");
        server.file("resources/web/.observer-upload-1234", "");

        let listing = server.list(&ScanSettings::default());

        let web = &listing.resources[0];
        assert_eq!(paths(web), ["resources/web/package.json"]);
        assert_eq!(web.packages.len(), 1);
        assert_eq!(web.git_head.as_ref().unwrap().branch.as_deref(), Some("main"));
    }

    #[test]
    fn an_escrowed_resource_has_a_fxap_file_at_its_root() {
        let server = Server::new();
        server.file("resources/paid/.fxap", "");
        server.file("resources/free/sub/.fxap", "");

        let listing = server.list(&ScanSettings::default());

        assert!(!listing.resources[0].escrowed);
        assert!(listing.resources[1].escrowed);
    }

    #[test]
    fn the_owners_exclusions_are_never_walked() {
        let server = Server::new();
        server.file("resources/a/client.lua", "");
        server.file("resources/a/dist/bundle.js", "");
        server.file("resources/b/client.lua", "");
        let settings = ScanSettings {
            excluded: vec!["dist".to_owned(), "resources/b".to_owned()],
            ..ScanSettings::default()
        };

        let listing = server.list(&settings);

        assert_eq!(listing.resources.len(), 1);
        assert_eq!(paths(&listing.resources[0]), ["resources/a/client.lua"]);
    }

    #[test]
    fn a_folder_past_the_depth_limit_is_listed_as_unreadable() {
        let server = Server::new();
        server.file("resources/a/1/2/deep.lua", "");
        let mut settings = ScanSettings::default();
        settings.limits.depth = 2;

        let listing = server.list(&settings);

        let entry = &listing.resources[0].entries[0];
        assert_eq!(entry.path, "resources/a/1/2");
        assert_eq!(entry.found, Found::Unreadable);
    }

    #[test]
    fn the_file_and_resource_limits_shorten_the_listing() {
        let server = Server::new();
        for index in 0..3 {
            server.file(&format!("resources/r{index}/a.lua"), "");
            server.file(&format!("resources/r{index}/b.lua"), "");
        }
        let mut files = ScanSettings::default();
        files.limits.files = 3;
        let mut resources = ScanSettings::default();
        resources.limits.resources = 2;

        assert_eq!(server.list(&files).shortened, Some(Shortened::Files));
        assert_eq!(server.list(&files).files(), 3);
        let listing = server.list(&resources);
        assert_eq!(listing.shortened, Some(Shortened::Resources));
        assert_eq!(listing.resources.len(), 2);
    }

    #[test]
    fn a_link_into_another_resource_lists_its_files_under_the_link() {
        let server = Server::new();
        server.file("resources/shared/lib/util.lua", "");
        fs::create_dir_all(server.data().join("resources/app")).unwrap();
        link_folder(
            &server.data().join("resources/shared/lib"),
            &server.data().join("resources/app/lib"),
        );

        let listing = server.list(&ScanSettings::default());

        let app = &listing.resources[0];
        assert_eq!(paths(app), ["resources/app/lib", "resources/app/lib/util.lua"]);
        assert!(matches!(app.entries[0].found, Found::Link { followed: true, .. }));
        assert_eq!(paths(&listing.resources[1]), ["resources/shared/lib/util.lua"]);
    }

    #[test]
    fn a_link_back_to_an_ancestor_is_cut() {
        let server = Server::new();
        server.file("resources/a/sub/x.lua", "");
        link_folder(
            &server.data().join("resources/a"),
            &server.data().join("resources/a/sub/loop"),
        );

        let listing = server.list(&ScanSettings::default());

        let a = &listing.resources[0];
        assert_eq!(paths(a), ["resources/a/sub/loop", "resources/a/sub/x.lua"]);
        assert!(matches!(a.entries[0].found, Found::Link { followed: false, .. }));
    }

    #[test]
    fn a_link_into_observers_folders_is_never_followed() {
        let server = Server::new();
        let observer = tempfile::tempdir().unwrap();
        fs::write(observer.path().join("observer.sqlite3"), "state").unwrap();
        fs::create_dir_all(server.data().join("resources/a")).unwrap();
        link_folder(observer.path(), &server.data().join("resources/a/data"));
        let settings = ScanSettings {
            never_read: vec![observer.path().to_owned()],
            ..ScanSettings::default()
        };

        let listing = server.list(&settings);

        assert_eq!(paths(&listing.resources[0]), ["resources/a/data"]);
        assert!(matches!(
            listing.resources[0].entries[0].found,
            Found::Link { followed: false, .. }
        ));
    }

    #[test]
    fn a_listing_without_a_resources_folder_fails() {
        let dir = tempfile::tempdir().unwrap();

        assert!(matches!(
            list(dir.path(), &ScanSettings::default(), &CancellationToken::new()),
            Err(ListError::Unreadable(_))
        ));
    }

    #[test]
    fn a_cancelled_listing_stops() {
        let server = Server::new();
        server.file("resources/a/x.lua", "");
        let cancel = CancellationToken::new();
        cancel.cancel();

        assert!(matches!(
            list(server.data(), &ScanSettings::default(), &cancel),
            Err(ListError::Cancelled)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn paths_past_260_characters_are_listed() {
        let server = Server::new();
        let deep = format!("resources/a/{}/{}/x.lua", "d".repeat(150), "e".repeat(150));
        let location = server.data().join(&deep);
        fs::create_dir_all(location.parent().unwrap()).unwrap();
        fs::write(&location, "").unwrap();

        let listing = server.list(&ScanSettings::default());

        assert_eq!(paths(&listing.resources[0]), [deep.as_str()]);
        assert_eq!(listing.resources[0].entries[0].found, Found::File);
    }

    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_utf8_is_unreadable() {
        use std::os::unix::ffi::OsStrExt;
        let server = Server::new();
        let folder = server.data().join("resources/a");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(std::ffi::OsStr::from_bytes(b"caf\xe9.lua")), "").unwrap();

        let listing = server.list(&ScanSettings::default());

        assert_eq!(listing.resources[0].entries[0].found, Found::Unreadable);
    }
}
