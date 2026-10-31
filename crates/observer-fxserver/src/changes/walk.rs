//! The listing: one walk of `resources/` that finds the resources the way FXServer does and lists
//! every entry with its size and time, reading no file but the manifests, git heads and npm
//! package files. Links and junctions are followed, as FXServer follows them; a loop is cut.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, DirEntry, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

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
    /// Reached through a link onto a file the owner hid: its contents are never kept, though no
    /// glob covers the path it is listed under.
    pub hidden: bool,
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
    /// Folders that could not be read, categories or folders of a resource: what they hold is not
    /// listed, which is no removal.
    pub unreadable: Vec<String>,
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
    /// Where it really is, when the file system resolves it.
    really: Option<PathBuf>,
    /// Below a followed link, or in a resource reached through one: what lies there is checked
    /// where it really is.
    linked: bool,
    /// The resource's real folder and the targets of the links followed down to this one: a link
    /// back to one of them is a loop.
    chain: Rc<[PathBuf]>,
}

impl Pending {
    /// Its folder `name`, reached through a link onto `target` when there is one.
    fn below(&self, location: PathBuf, path: String, name: &str, target: Option<PathBuf>) -> Self {
        let depth = self.depth + 1;
        match target {
            Some(target) => Self {
                location,
                path,
                depth,
                really: Some(target.clone()),
                linked: true,
                chain: self.chain.iter().cloned().chain([target]).collect(),
            },
            None => Self {
                location,
                path,
                depth,
                really: self.really.as_ref().map(|here| here.join(name)),
                linked: self.linked,
                chain: Rc::clone(&self.chain),
            },
        }
    }
}

impl Walk<'_> {
    /// Walks the categories breadth first, in name order, as FXServer finds the resources.
    fn categories(&mut self, folder: PathBuf, path: String) -> Result<(), Cancelled> {
        let chain: Rc<[PathBuf]> = fs::canonicalize(&folder).ok().into_iter().collect();
        let mut queue = VecDeque::from([(folder, path, chain, 1)]);
        while let Some((folder, path, chain, depth)) = queue.pop_front() {
            let Ok(entries) = sorted(&folder) else {
                self.listing.unreadable.push(path);
                continue;
            };
            for entry in entries {
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
                // A category or a resource is checked where it really is, as a link's target is:
                // one reached through a link may be one of Observer's own folders.
                let really = fs::canonicalize(entry.path()).ok();
                if let Some(really) = &really {
                    // A link back to a category on the way here would list it again at each level.
                    let looped = chain.iter().any(|category| category.starts_with(really));
                    if looped || matches!(self.guard(really), Guard::Refused) {
                        continue;
                    }
                }
                let lower = name.to_ascii_lowercase();
                if name.starts_with('[') && name.ends_with(']') {
                    if depth < self.limits.depth {
                        let chain = chain.iter().cloned().chain(really).collect();
                        queue.push_back((entry.path(), child, chain, depth + 1));
                    }
                } else if !NOT_RESOURCES.contains(&lower.as_str()) && !name.starts_with(UPLOAD_PREFIX) {
                    if self.listing.resources.len() >= self.limits.resources {
                        self.listing.shortened = Some(Shortened::Resources);
                        return Ok(());
                    }
                    self.resource(entry.path(), really, child, name, lower, depth)?;
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
        really: Option<PathBuf>,
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
        // A resource reached through a link, its own or a category's, is checked where it really
        // is, as what lies below a link inside a resource.
        let linked = match (&self.root, &really) {
            (Some(root), Some(really)) => {
                let mut there = root.clone();
                there.extend(path.split('/'));
                *really != there
            }
            _ => true,
        };
        let mut stack = vec![Pending {
            location,
            path,
            depth,
            chain: really.iter().cloned().collect(),
            really,
            linked,
        }];
        while let Some(folder) = stack.pop() {
            self.check()?;
            let Ok(entries) = sorted(&folder.location) else {
                // What it holds is unknown, which is no removal: the snapshot is partial.
                self.files += 1;
                self.listing.unreadable.push(folder.path.clone());
                unreadable(&mut resource, folder.location, folder.path);
                continue;
            };
            let mut folders = Vec::new();
            for entry in entries {
                if self.files >= self.limits.files {
                    self.listing.shortened = Some(Shortened::Files);
                    break;
                }
                self.entry(&mut resource, &folder, &entry, &mut folders);
            }
            // Reversed so that the stack walks folders in name order.
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
        entry: &DirEntry,
        folders: &mut Vec<Pending>,
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
        let mut hidden_below_link = false;
        if folder.linked
            && let Some(really) = folder.really.as_ref().map(|here| here.join(&name))
        {
            match self.guard(&really) {
                Guard::Refused => return,
                Guard::Hidden => hidden_below_link = true,
                Guard::Free => {}
            }
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
        let mut target = None;
        if kind.is_symlink() {
            let Some((target_metadata, resolved_target)) = self.link(resource, &path, &location, folder)
            else {
                return;
            };
            if target_metadata.is_dir() {
                target = Some(resolved_target);
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
            folders.push(folder.below(location, path, &name, target));
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
            hidden: hidden_below_link,
        });
    }

    /// Lists the link at `path` unless it is followed onto a file, and gives its target's metadata
    /// and resolved path when it is followed.
    fn link(
        &mut self,
        resource: &mut ResourceListing,
        path: &str,
        location: &Path,
        folder: &Pending,
    ) -> Option<(Metadata, PathBuf)> {
        let target = fs::read_link(location)
            .map(|target| target.to_string_lossy().into_owned())
            .unwrap_or_default();
        let followed = self
            .followable(location)
            .filter(|resolved_target| {
                // A target that holds the folder being walked, or that the walk already went
                // through on its way here, leads back to it. Two links onto one folder are no
                // loop: each lists it.
                folder
                    .really
                    .as_deref()
                    .is_none_or(|here| !here.starts_with(resolved_target))
                    && !folder.chain.contains(resolved_target)
            })
            .and_then(|resolved_target| Some((fs::metadata(location).ok()?, resolved_target)));
        // A followed link to a file is listed as that file, at the link's path.
        if followed.as_ref().is_none_or(|(target, _)| target.is_dir()) {
            self.files += 1;
            resource.entries.push(Entry {
                path: path.to_owned(),
                location: location.to_owned(),
                found: Found::Link {
                    target,
                    followed: followed.is_some(),
                },
                stat: Stat::default(),
                hidden: false,
            });
        }
        followed
    }

    /// What the real place of a path allows, for a path that may differ from it: one of
    /// Observer's own folders, FXServer's cache or what the owner excluded is never listed, and
    /// what the owner hid stays hidden.
    fn guard(&self, really: &Path) -> Guard {
        if self.never_read.iter().any(|folder| really.starts_with(folder)) {
            return Guard::Refused;
        }
        let (relative, inside) = self.relative(really);
        if inside {
            // FXServer's cache is never read, whichever path leads to it.
            let cache = relative
                .split('/')
                .next()
                .is_some_and(|first| first.eq_ignore_ascii_case("cache"));
            if cache || self.excluded(&relative) {
                return Guard::Refused;
            }
        }
        // The owner's globs are paths from the data folder: they say nothing of what lies outside.
        let globs: &[String] = if inside { &self.settings.hidden } else { &[] };
        if hidden(&relative, globs) {
            Guard::Hidden
        } else {
            Guard::Free
        }
    }

    /// The resolved target of a link when it may be followed: not into Observer's own folders,
    /// nor onto what the owner excluded or hid, nor onto a name never shown.
    fn followable(&self, link: &Path) -> Option<PathBuf> {
        let target = fs::canonicalize(link).ok()?;
        matches!(self.guard(&target), Guard::Free).then_some(target)
    }

    /// A real path as the checks read it, and whether it lies in the data folder: from the data
    /// folder, or else from the deepest folder that holds both. A data folder kept in txAdmin's
    /// `txData` must not make every path it reaches one never shown.
    fn relative(&self, really: &Path) -> (String, bool) {
        let base = self
            .root
            .as_deref()
            .and_then(|root| root.ancestors().find(|folder| really.starts_with(folder)));
        let inside = base.is_some() && base == self.root.as_deref();
        let below = base
            .and_then(|base| really.strip_prefix(base).ok())
            .unwrap_or(really);
        (slashed(below), inside)
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

/// What the real place of a path allows.
enum Guard {
    Free,
    Hidden,
    Refused,
}

fn unreadable(resource: &mut ResourceListing, location: PathBuf, path: String) {
    resource.entries.push(Entry {
        path,
        location,
        found: Found::Unreadable,
        stat: Stat::default(),
        hidden: false,
    });
}

/// The entries of a folder in name order. One that cannot be read makes the whole folder
/// unreadable, since leaving it out would read as its removal.
fn sorted(folder: &Path) -> io::Result<Vec<DirEntry>> {
    let mut entries = fs::read_dir(folder)?.collect::<io::Result<Vec<DirEntry>>>()?;
    entries.sort_by_key(DirEntry::file_name);
    Ok(entries)
}

/// The names of a path joined with `/`, as the globs read them.
fn slashed(path: &Path) -> String {
    let mut slashed = String::new();
    for component in path.components() {
        if let Component::Normal(name) = component {
            if !slashed.is_empty() {
                slashed.push('/');
            }
            slashed.push_str(&name.to_string_lossy());
        }
    }
    slashed
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

    /// Keeps a folder from being listed until dropped: a handle that shares nothing on Windows, as
    /// another process may hold one, and no permissions on unix.
    struct Locked {
        #[cfg(windows)]
        _handle: fs::File,
        #[cfg(unix)]
        folder: PathBuf,
    }

    fn lock_folder(folder: &Path) -> Locked {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            let handle = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(folder)
                .unwrap();
            Locked { _handle: handle }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(folder, fs::Permissions::from_mode(0o000)).unwrap();
            Locked {
                folder: folder.to_owned(),
            }
        }
    }

    #[cfg(unix)]
    impl Drop for Locked {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.folder, fs::Permissions::from_mode(0o755));
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
    fn two_links_pointing_at_each_others_folders_are_walked_once() {
        let server = Server::new();
        server.file("resources/a/a.lua", "");
        server.file("resources/b/b.lua", "");
        link_folder(
            &server.data().join("resources/b"),
            &server.data().join("resources/a/x"),
        );
        link_folder(
            &server.data().join("resources/a"),
            &server.data().join("resources/b/y"),
        );

        let listing = server.list(&ScanSettings::default());

        let a = &listing.resources[0];
        assert_eq!(
            paths(a),
            [
                "resources/a/a.lua",
                "resources/a/x",
                "resources/a/x/b.lua",
                "resources/a/x/y"
            ]
        );
        assert!(matches!(a.entries[3].found, Found::Link { followed: false, .. }));
        assert!(a.entries.iter().all(|entry| entry.found != Found::Unreadable));
    }

    #[test]
    fn two_links_onto_one_folder_are_both_walked() {
        let server = Server::new();
        server.file("resources/res/shared/lib/util.lua", "");
        fs::create_dir_all(server.data().join("resources/res/client")).unwrap();
        fs::create_dir_all(server.data().join("resources/res/server")).unwrap();
        let shared = server.data().join("resources/res/shared/lib");
        link_folder(&shared, &server.data().join("resources/res/client/lib"));
        link_folder(&shared, &server.data().join("resources/res/server/lib"));

        let listing = server.list(&ScanSettings::default());

        assert_eq!(
            paths(&listing.resources[0]),
            [
                "resources/res/client/lib",
                "resources/res/client/lib/util.lua",
                "resources/res/server/lib",
                "resources/res/server/lib/util.lua",
                "resources/res/shared/lib/util.lua"
            ]
        );
    }

    #[test]
    fn a_category_linked_above_observers_folders_never_lists_them() {
        let server = Server::new();
        let above = tempfile::tempdir().unwrap();
        for file in [
            "data/observer.sqlite3",
            "builds/17000/FXServer.exe",
            "tools/notes.txt",
        ] {
            let path = above.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        link_folder(above.path(), &server.data().join("resources/[ext]"));
        let settings = ScanSettings {
            never_read: vec![above.path().join("data"), above.path().join("builds")],
            ..ScanSettings::default()
        };

        let listing = server.list(&settings);

        let names: Vec<&str> = listing.resources.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(names, ["resources/[ext]/tools"]);
        assert_eq!(paths(&listing.resources[0]), ["resources/[ext]/tools/notes.txt"]);
    }

    #[test]
    fn a_resource_reached_through_a_linked_category_is_checked_where_it_really_is() {
        let server = Server::new();
        server.file("resources/[real]/bank/config.lua", "token");
        server.file("resources/[real]/vault/notes.lua", "");
        link_folder(
            &server.data().join("resources/[real]"),
            &server.data().join("resources/[mirror]"),
        );
        let settings = ScanSettings {
            excluded: vec!["resources/[real]/vault".to_owned()],
            hidden: vec!["resources/[real]/bank/config.lua".to_owned()],
            ..ScanSettings::default()
        };

        let listing = server.list(&settings);

        let seen: Vec<(&str, bool)> = listing
            .resources
            .iter()
            .flat_map(|resource| &resource.entries)
            .map(|entry| (entry.path.as_str(), entry.hidden))
            .collect();
        assert_eq!(
            seen,
            [
                ("resources/[mirror]/bank/config.lua", true),
                ("resources/[real]/bank/config.lua", false)
            ]
        );
    }

    #[test]
    fn a_category_linked_back_to_a_folder_above_it_is_cut() {
        let server = Server::new();
        server.file("resources/chat/client.lua", "");
        link_folder(
            &server.data().join("resources"),
            &server.data().join("resources/[loop]"),
        );

        let listing = server.list(&ScanSettings::default());

        let names: Vec<&str> = listing.resources.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(names, ["resources/chat"]);
    }

    #[test]
    fn a_data_folder_kept_in_txdata_still_follows_links() {
        let host = tempfile::tempdir().unwrap();
        let data = host.path().join("txData/default.base");
        let common = host.path().join("txData/common");
        fs::create_dir_all(data.join("resources/shared/lib")).unwrap();
        fs::write(data.join("resources/shared/lib/util.lua"), "").unwrap();
        fs::create_dir_all(data.join("resources/app")).unwrap();
        fs::create_dir_all(&common).unwrap();
        fs::write(common.join("config.lua"), "").unwrap();
        fs::write(common.join("server.pem"), "").unwrap();
        link_folder(
            &data.join("resources/shared/lib"),
            &data.join("resources/app/lib"),
        );
        link_folder(&common, &data.join("resources/app/common"));

        let listing = list(&data, &ScanSettings::default(), &CancellationToken::new()).unwrap();

        let app = &listing.resources[0];
        let seen: Vec<(&str, bool)> = app
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.hidden))
            .collect();
        assert_eq!(
            seen,
            [
                ("resources/app/common", false),
                ("resources/app/lib", false),
                ("resources/app/common/config.lua", false),
                ("resources/app/common/server.pem", true),
                ("resources/app/lib/util.lua", false)
            ]
        );
        assert!(matches!(app.entries[0].found, Found::Link { followed: true, .. }));
        assert!(matches!(app.entries[1].found, Found::Link { followed: true, .. }));
    }

    #[test]
    fn a_folder_that_cannot_be_read_makes_the_listing_partial() {
        let server = Server::new();
        server.file("resources/a/client.lua", "");
        server.file("resources/a/locked/server.lua", "");
        server.file("resources/b/client.lua", "");
        let folder = server.data().join("resources/a/locked");
        let _locked = lock_folder(&folder);
        #[cfg(unix)]
        if fs::read_dir(&folder).is_ok() {
            // Root lists any folder.
            return;
        }
        let mut limited = ScanSettings::default();
        limited.limits.files = 2;

        let listing = server.list(&ScanSettings::default());

        let a = &listing.resources[0];
        assert_eq!(paths(a), ["resources/a/client.lua", "resources/a/locked"]);
        assert_eq!(a.entries[1].found, Found::Unreadable);
        assert_eq!(listing.unreadable, ["resources/a/locked"]);
        assert_eq!(server.list(&limited).shortened, Some(Shortened::Files));
    }

    #[test]
    fn a_link_above_observers_folders_never_reaches_them() {
        let server = Server::new();
        let above = tempfile::tempdir().unwrap();
        fs::create_dir_all(above.path().join("data")).unwrap();
        fs::write(above.path().join("data/observer.sqlite3"), "state").unwrap();
        fs::write(above.path().join("notes.txt"), "").unwrap();
        fs::create_dir_all(server.data().join("resources/a")).unwrap();
        link_folder(above.path(), &server.data().join("resources/a/up"));
        let settings = ScanSettings {
            never_read: vec![above.path().join("data")],
            ..ScanSettings::default()
        };

        let listing = server.list(&settings);

        assert_eq!(
            paths(&listing.resources[0]),
            ["resources/a/up", "resources/a/up/notes.txt"]
        );
    }

    #[test]
    fn a_hidden_file_stays_hidden_through_a_link() {
        let server = Server::new();
        server.file("resources/b/keys.lua", "token");
        fs::create_dir_all(server.data().join("resources/a")).unwrap();
        link_folder(
            &server.data().join("resources/b"),
            &server.data().join("resources/a/lib"),
        );
        let settings = ScanSettings {
            hidden: vec!["resources/b/keys.lua".to_owned()],
            ..ScanSettings::default()
        };

        let listing = server.list(&settings);

        let through = &listing.resources[0].entries[1];
        assert_eq!(through.path, "resources/a/lib/keys.lua");
        assert!(through.hidden);
        assert!(!listing.resources[1].entries[0].hidden);
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
