//! The folder reads of the index, each bounded: FXServer's walk of the resource folders, a
//! resource's `stream/` listing in the order the system gives it, the files a data file's folder
//! holds, and the `.ytyp` stems a manifest of the streaming system names. Every loop checks the
//! cancel flag between entries.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, DirEntry, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use observer_gamefiles::rsc7::{self, FileKind};

use super::{Error, StreamFile};

/// The most entries one walk of a data file's folder lists: a `meta/` folder holds a few dozen.
pub(super) const MAX_DATA_FOLDER_ENTRIES: usize = 50_000;
/// The most entries one resource's `stream/` lists: the largest real ones hold some 20,000 files.
pub(super) const MAX_STREAM_ENTRIES: usize = 500_000;
/// The largest manifest of the streaming system read: real ones hold kilobytes.
const MAX_REFERENCE_BYTES: u64 = 16 * 1024 * 1024;

/// A resource folder FXServer's walk finds.
#[derive(Debug, Clone)]
pub(super) struct Found {
    pub name: String,
    pub dir: PathBuf,
    /// Relative to the server folder, with `/`.
    pub path: String,
    pub categories: Vec<String>,
    pub linked: bool,
    /// A resource of the same name was found before it.
    pub duplicate: bool,
}

pub(super) fn cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// The entry's name, and whether it is a folder and a link: a link or junction to a folder is a
/// folder, as `FILE_ATTRIBUTE_DIRECTORY` makes it one to FXServer.
fn kind(entry: &DirEntry) -> Option<(String, bool, bool)> {
    let name = entry.file_name().to_string_lossy().into_owned();
    let file_type = entry.file_type().ok()?;
    if file_type.is_symlink() {
        let target = fs::metadata(entry.path()).ok()?;
        Some((name, target.is_dir(), true))
    } else {
        Some((name, file_type.is_dir(), false))
    }
}

/// The resource folders under `root` the way `ServerResourceList::ScanResources` finds them:
/// `[category]` folders queued and opened breadth-first, every other folder a resource, `.git`,
/// hidden folders and `txadmin` skipped, as are `yarn`, `webpack` and `chat`, which the build's
/// system resources provide. Each folder lists in the system's order.
pub(super) fn resources(root: &Path, cancel: &AtomicBool) -> Result<Vec<Found>, Error> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), "resources".to_owned(), Vec::new(), false)]);
    while let Some((folder, relative, categories, linked)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            cancelled(cancel)?;
            let Some((name, is_dir, is_link)) = kind(&entry) else {
                continue;
            };
            if !is_dir || name == ".git" || ["chat", "yarn", "webpack"].contains(&name.as_str()) {
                continue;
            }
            let path = format!("{relative}/{name}");
            let linked = linked || is_link;
            if super::start::is_category(&name) {
                let mut inner = categories.clone();
                inner.push(name);
                queue.push_back((entry.path(), path, inner, linked));
                continue;
            }
            if name.starts_with('.') || name.eq_ignore_ascii_case("txadmin") {
                continue;
            }
            let duplicate = !seen.insert(name.clone());
            found.push(Found {
                name,
                dir: entry.path(),
                path,
                categories: categories.clone(),
                linked,
                duplicate,
            });
        }
    }
    Ok(found)
}

/// What a resource's root holds that the tools look at.
#[derive(Debug, Clone, Default)]
pub(super) struct Root {
    /// A `.fxap` file.
    pub escrowed: bool,
    /// `<resource>_cache_y.dat`, as the client opens it.
    pub cache: Option<String>,
}

pub(super) fn root(dir: &Path, name: &str) -> Root {
    let mut root = Root::default();
    let cache = format!("{name}_cache_y.dat");
    let Ok(entries) = fs::read_dir(dir) else {
        return root;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if super::ends_with_ignoring_case(&file, ".fxap") {
            root.escrowed = true;
        } else if root.cache.is_none() && file.eq_ignore_ascii_case(&cache) {
            root.cache = Some(file);
        }
    }
    root
}

/// Every file of `<resource>/stream/` in the order `ResourceStreamComponent`'s
/// `IterateRecursively` gives them: a folder's files in the system's listing order, then its
/// subfolders, depth first, folders whose name starts with `.` skipped. Nothing is sorted: on
/// Windows this is the order FXServer reads, and the first file of a name is the one it sends.
pub(super) fn stream(
    dir: &Path,
    relative: &str,
    linked: bool,
    cancel: &AtomicBool,
) -> Result<Listing, Error> {
    let mut listing = Listing::default();
    let stream = dir.join("stream");
    if stream.is_dir() {
        let link = fs::symlink_metadata(&stream).is_ok_and(|metadata| metadata.file_type().is_symlink());
        list(
            &stream,
            &format!("{relative}/stream"),
            linked || link,
            &mut listing,
            cancel,
        )?;
    }
    let mut names = HashSet::new();
    for file in &mut listing.files {
        file.sent = !file.path.contains(".stream_raw") && names.insert(file.name.clone());
    }
    Ok(listing)
}

#[derive(Debug, Clone, Default)]
pub(super) struct Listing {
    pub files: Vec<StreamFile>,
    /// More entries than [`MAX_STREAM_ENTRIES`]: the rest is unknown.
    pub truncated: bool,
}

fn list(
    folder: &Path,
    relative: &str,
    linked: bool,
    listing: &mut Listing,
    cancel: &AtomicBool,
) -> Result<(), Error> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Ok(());
    };
    let mut folders = Vec::new();
    for entry in entries.flatten() {
        cancelled(cancel)?;
        if listing.files.len() >= MAX_STREAM_ENTRIES {
            listing.truncated = true;
            return Ok(());
        }
        let Some((name, is_dir, is_link)) = kind(&entry) else {
            continue;
        };
        let path = format!("{relative}/{name}");
        if is_dir {
            if !name.starts_with('.') {
                folders.push((entry.path(), path, linked || is_link));
            }
        } else {
            listing.files.push(StreamFile {
                name,
                path,
                sent: false,
                linked: linked || is_link,
            });
        }
    }
    for (folder, path, linked) in folders {
        list(&folder, &path, linked, listing, cancel)?;
    }
    Ok(())
}

/// The hashes of `stems` the file at `path` holds, as big- or little-endian words at any offset:
/// `.ymf` manifests are PSO, big-endian, and a resource-wrapped one is searched inflated too. A
/// conservative search: a word that only looks like a stem's hash counts as a reference.
pub(super) fn references(path: &Path, stems: &HashSet<u32>) -> Result<HashSet<u32>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_REFERENCE_BYTES {
        return Err(format!("larger than {} MiB", MAX_REFERENCE_BYTES >> 20));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_REFERENCE_BYTES).read_to_end(&mut bytes))
        .map_err(|error| error.to_string())?;
    let mut found = HashSet::new();
    search(&bytes, stems, &mut found);
    match rsc7::sniff(&bytes) {
        FileKind::Rsc7 => {
            let resource = rsc7::Resource::read(&bytes).map_err(|error| error.to_string())?;
            search(resource.system(), stems, &mut found);
            search(resource.graphics(), stems, &mut found);
        }
        FileKind::Escrowed => return Err(FileKind::Escrowed.describe().to_owned()),
        _ => {}
    }
    Ok(found)
}

fn search(bytes: &[u8], stems: &HashSet<u32>, found: &mut HashSet<u32>) {
    for window in bytes.windows(4) {
        let word = [window[0], window[1], window[2], window[3]];
        for hash in [u32::from_be_bytes(word), u32::from_le_bytes(word)] {
            if stems.contains(&hash) {
                found.insert(hash);
            }
        }
    }
}

/// The files a resource holds under a folder, found the way the resource's packfile looks the
/// folder up (each name exact first, then equal in ASCII case), with `/` between names and the
/// folders spelled as on disk. Kept per folder, since a resource's data files often share one.
#[derive(Debug)]
pub(super) struct Folders<'b> {
    walked: HashMap<String, Walked>,
    /// The entries the index's walks of data folders may still list, over every resource.
    budget: &'b mut usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Walked {
    /// Relative to the resource's folder.
    pub files: Vec<String>,
    /// The files reached through a link or junction.
    pub linked: HashSet<String>,
    /// The folder held more entries than one walk, or what is left of the index's budget, lists.
    pub truncated: bool,
}

impl<'b> Folders<'b> {
    pub fn new(budget: &'b mut usize) -> Self {
        Self {
            walked: HashMap::new(),
            budget,
        }
    }

    /// The files under `prefix` (names separated by `/`, empty for the resource's root).
    pub fn walk(&mut self, dir: &Path, prefix: &[&str], cancel: &AtomicBool) -> Result<&Walked, Error> {
        let key = prefix.join("/");
        if !self.walked.contains_key(&key) {
            let limit = MAX_DATA_FOLDER_ENTRIES.min(*self.budget);
            let walked = walk_prefix(dir, prefix, limit, cancel)?;
            *self.budget -= walked.files.len();
            self.walked.insert(key.clone(), walked);
        }
        Ok(&self.walked[&key])
    }
}

fn walk_prefix(dir: &Path, prefix: &[&str], limit: usize, cancel: &AtomicBool) -> Result<Walked, Error> {
    let mut walked = Walked::default();
    let mut folder = dir.to_path_buf();
    let mut relative = String::new();
    let mut linked = false;
    for name in prefix {
        let Ok(entries) = fs::read_dir(&folder) else {
            return Ok(walked);
        };
        let entries: Vec<(String, PathBuf, bool, bool)> = entries
            .flatten()
            .filter_map(|entry| {
                kind(&entry).map(|(found, is_dir, is_link)| (found, entry.path(), is_dir, is_link))
            })
            .collect();
        let found = entries
            .iter()
            .find(|entry| entry.0 == *name)
            .or_else(|| entries.iter().find(|entry| entry.0.eq_ignore_ascii_case(name)));
        let Some((found, path, true, is_link)) = found else {
            return Ok(walked);
        };
        relative.push_str(found);
        relative.push('/');
        folder.clone_from(path);
        linked |= *is_link;
    }
    let mut stack = vec![(folder, relative, linked)];
    while let Some((folder, relative, linked)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            cancelled(cancel)?;
            if walked.files.len() >= limit {
                walked.truncated = true;
                return Ok(walked);
            }
            let Some((name, is_dir, is_link)) = kind(&entry) else {
                continue;
            };
            let path = format!("{relative}{name}");
            if is_dir {
                stack.push((entry.path(), format!("{path}/"), linked || is_link));
            } else {
                if linked || is_link {
                    walked.linked.insert(path.clone());
                }
                walked.files.push(path);
            }
        }
    }
    Ok(walked)
}
