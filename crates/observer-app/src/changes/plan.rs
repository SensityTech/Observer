//! What a snapshot's job does with each entry the listing found: known from the index, read now,
//! or left for the hashing pass; and the trees and rows made once entries are known. The blocking
//! passes live here too, each run in `spawn_blocking` by the job.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use observer_domain::changes::{ARCHIVE_EXTENSIONS, FileKind, extension, hidden};
use observer_domain::launch::LaunchSettings;
use observer_domain::server_data::{CfgFileState, Game};
use observer_fxserver::cfg::{self, Context};
use observer_fxserver::changes::tree::{self, TreeEntry};
use observer_fxserver::changes::{
    Cancelled, Entry, FileRead, Found, Keys, ListError, Rate, ResourceListing, ScanSettings, Shortened,
    Treatment, list, read,
};
use observer_fxserver::server_data;
use observer_store::changes::index::IndexEntry;
use observer_store::changes::resources::ResourceRow;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

/// Text read during the listing pass, past which changed texts wait for the hashing pass.
pub(crate) const LISTING_TEXT: u64 = 64 * 1024 * 1024;

/// An entry as the job knows it.
#[derive(Debug, Clone)]
pub(crate) enum Slot {
    Known(TreeEntry),
    /// Waits for the hashing pass.
    Pending(Entry),
}

/// A resource and its entries.
#[derive(Debug, Clone)]
pub(crate) struct Planned {
    pub resource: ResourceListing,
    pub slots: Vec<Slot>,
}

impl Planned {
    pub fn pending(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| matches!(slot, Slot::Pending(_)))
            .count()
    }

    /// The tree once no entry waits, with the path and digest of each text entry.
    pub fn tree(&self) -> Option<(String, Vec<TreeEntry>)> {
        let mut entries: Vec<TreeEntry> = self
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Known(entry) => Some(entry.clone()),
                Slot::Pending(_) => None,
            })
            .collect::<Option<_>>()?;
        let hash = tree::hash(&mut entries);
        Some((hash, entries))
    }

    /// The entries known so far, those waiting with an empty key.
    pub fn partial_listing(&self) -> Vec<TreeEntry> {
        self.slots
            .iter()
            .map(|slot| match slot {
                Slot::Known(entry) => entry.clone(),
                Slot::Pending(entry) => TreeEntry {
                    path: entry.path.clone(),
                    size: entry.stat.size,
                    key: String::new(),
                    kind: FileKind::Binary,
                    sha256: None,
                },
            })
            .collect()
    }

    pub fn row(&self, tree: Option<String>) -> ResourceRow {
        let pending = self.pending() as u64;
        let resource = &self.resource;
        ResourceRow {
            path: resource.path.clone(),
            name: resource.name.clone(),
            shown_name: resource.shown_name.clone(),
            duplicate: resource.duplicate,
            pending_listing: (tree.is_none())
                .then(|| serde_json::to_vec(&self.partial_listing()).unwrap_or_default()),
            tree,
            pending,
            manifest: resource.manifest.clone(),
            escrowed: resource.escrowed,
            git_head: resource.git_head.clone(),
            packages: resource.packages.clone(),
            files: resource.files(),
            bytes: resource.bytes(),
        }
    }
}

/// The path and digest of each text entry of a tree, which `tree_text` lists.
pub(crate) fn texts(entries: &[TreeEntry]) -> Vec<(String, String)> {
    entries
        .iter()
        .filter(|entry| entry.kind == FileKind::Text)
        .filter_map(|entry| Some((entry.path.clone(), entry.digest()?.to_owned())))
        .collect()
}

/// A file read by the job, which goes to the index, the texts and the flags.
#[derive(Debug, Clone)]
pub(crate) struct Read {
    pub path: String,
    pub read: FileRead,
}

impl Read {
    pub fn entry(&self) -> TreeEntry {
        TreeEntry {
            path: self.path.clone(),
            size: self.read.stat.size,
            key: self
                .read
                .digest
                .clone()
                .unwrap_or_else(|| tree::UNREADABLE_KEY.to_owned()),
            kind: self.read.kind,
            sha256: self.read.sha256.clone(),
        }
    }

    /// What the index keeps of it; nothing for a file that could not be read.
    pub fn index(&self) -> Option<IndexEntry> {
        let digest = self.read.digest.clone()?;
        Some(IndexEntry {
            size: self.read.stat.size,
            modified_ns: self.read.stat.modified_ns,
            inode: self.read.stat.inode,
            changed_ns: self.read.stat.changed_ns,
            digest: Some(digest),
            sha256: self.read.sha256.clone(),
            kind: self.read.kind.as_str().to_owned(),
            racy: self.read.racy || self.read.unsettled,
            verified_at: None,
        })
    }
}

/// What the index tells of an entry whose size and times did not move, or `None` when it must be
/// read: unknown, moved, racy, hidden before and maybe text now, or a text whose body is not kept.
pub(crate) fn known(
    entry: &Entry,
    index: Option<&IndexEntry>,
    settings: &ScanSettings,
    bodies: &HashSet<String>,
) -> Option<TreeEntry> {
    let found = index?;
    let stat = &entry.stat;
    let hidden = entry.hidden || hidden(&entry.path, &settings.hidden);
    let moved = found.size != stat.size
        || found.modified_ns != stat.modified_ns
        || found.inode != stat.inode
        || found.changed_ns != stat.changed_ns;
    if found.racy || moved {
        return None;
    }
    let digest = found.digest.clone()?;
    let stored = FileKind::from_name(&found.kind)?;
    let kind = match (stored, hidden) {
        (_, true) => FileKind::Hidden,
        (FileKind::Hidden, false) => return None,
        (FileKind::Text, false) if stat.size <= settings.limits.max_text && !bodies.contains(&digest) => {
            return None;
        }
        (kind, false) => kind,
    };
    Some(TreeEntry {
        path: entry.path.clone(),
        size: stat.size,
        key: digest,
        kind,
        sha256: if kind.shows_hash() {
            found.sha256.clone()
        } else {
            None
        },
    })
}

/// The entry of what is never read: links, cloud placeholders, archives and unreadable entries.
fn unread(entry: &Entry, settings: &ScanSettings) -> Option<TreeEntry> {
    let (key, kind) = match &entry.found {
        Found::Link { target, .. } => (tree::link_key(target), FileKind::Link),
        Found::Offline => (tree::time_key(entry.stat.modified_ns), FileKind::Offline),
        Found::Unreadable => (tree::UNREADABLE_KEY.to_owned(), FileKind::Unreadable),
        Found::File => {
            let archive = extension(&entry.path)
                .is_some_and(|extension| ARCHIVE_EXTENSIONS.contains(&extension.as_str()));
            if !archive || hidden(&entry.path, &settings.hidden) {
                return None;
            }
            (tree::time_key(entry.stat.modified_ns), FileKind::Archive)
        }
    };
    Some(TreeEntry {
        path: entry.path.clone(),
        size: entry.stat.size,
        key,
        kind,
        sha256: None,
    })
}

/// Whether a file may be read during the listing pass: a text whose contents at start time are
/// what a diff needs.
fn text_candidate(entry: &Entry, settings: &ScanSettings) -> bool {
    !hidden(&entry.path, &settings.hidden)
        && extension(&entry.path).is_some_and(|extension| settings.extensions.contains(&extension))
        && entry.stat.size <= settings.limits.max_text
}

/// What the listing pass works with.
pub(crate) struct ListingInput {
    pub data: PathBuf,
    pub launch: LaunchSettings,
    pub settings: ScanSettings,
    pub keys: Keys,
    pub index: HashMap<String, IndexEntry>,
    pub bodies: HashSet<String>,
    /// Changed texts are read at once, up to 64 MB, rather than left for the hashing pass.
    pub read_text_now: bool,
    /// For a full verification: what the index knows only counts when a read verified it since.
    pub verified_after: Option<Timestamp>,
}

pub(crate) struct ListingPass {
    pub resources: Vec<Planned>,
    pub reads: Vec<Read>,
    pub shortened: Option<Shortened>,
    /// Categories that could not be read.
    pub unreadable: Vec<String>,
    pub chain: Chain,
    /// Every path listed, for the index to forget the others.
    pub paths: HashSet<String>,
}

/// The listing pass: one walk, the index, and the changed texts read at once up to 64 MB.
pub(crate) fn listing_pass(
    input: &ListingInput,
    rate: &mut Rate,
    cancel: &CancellationToken,
) -> Result<ListingPass, ListError> {
    let listing = list(&input.data, &input.settings, cancel)?;
    // The chain first: its files under `resources/` are masked as cfg files when they are read.
    let chain = chain(
        &input.data,
        &input.launch,
        &listing.resources,
        &input.settings,
        &input.keys,
        rate,
        cancel,
    )?;
    let in_chain: HashSet<String> = chain.files.iter().map(|file| file.path.clone()).collect();
    let mut reads = Vec::new();
    let mut paths = HashSet::new();
    let mut budget = if input.read_text_now { LISTING_TEXT } else { 0 };
    let mut resources = Vec::with_capacity(listing.resources.len());
    for mut resource in listing.resources {
        let entries = std::mem::take(&mut resource.entries);
        let mut slots = Vec::with_capacity(entries.len());
        for entry in &entries {
            paths.insert(entry.path.clone());
            let index = input.index.get(&entry.path).filter(|index| {
                input
                    .verified_after
                    .is_none_or(|after| index.verified_at.is_some_and(|at| at >= after))
            });
            if let Some(known) =
                unread(entry, &input.settings).or_else(|| known(entry, index, &input.settings, &input.bodies))
            {
                slots.push(Slot::Known(known));
                continue;
            }
            if text_candidate(entry, &input.settings) && entry.stat.size <= budget {
                budget -= entry.stat.size;
                let read = Read {
                    path: entry.path.clone(),
                    read: read(
                        &entry.path,
                        &entry.location,
                        treatment(entry, &in_chain),
                        &input.settings,
                        &input.keys,
                        rate,
                        cancel,
                    )?,
                };
                slots.push(Slot::Known(read.entry()));
                reads.push(read);
            } else {
                slots.push(Slot::Pending(entry.clone()));
            }
        }
        resource.entries = entries;
        resources.push(Planned { resource, slots });
    }
    Ok(ListingPass {
        resources,
        reads,
        shortened: listing.shortened,
        unreadable: listing.unreadable,
        chain,
        paths,
    })
}

/// How an entry is read: as a file of the cfg exec chain when it is one, hidden when a link led
/// onto a hidden file.
pub(crate) fn treatment(entry: &Entry, in_chain: &HashSet<String>) -> Treatment {
    Treatment {
        chain: in_chain.contains(&entry.path),
        hidden: entry.hidden,
    }
}

/// Reads the entries left for the hashing pass, each under the key it came with.
pub(crate) fn hash<K>(
    entries: Vec<(K, Entry)>,
    in_chain: &HashSet<String>,
    settings: &ScanSettings,
    keys: &Keys,
    rate: &mut Rate,
    cancel: &CancellationToken,
) -> Result<Vec<(K, Read)>, Cancelled> {
    entries
        .into_iter()
        .map(|(key, entry)| {
            let treatment = treatment(&entry, in_chain);
            let read = read(
                &entry.path,
                &entry.location,
                treatment,
                settings,
                keys,
                rate,
                cancel,
            )?;
            Ok((
                key,
                Read {
                    path: entry.path,
                    read,
                },
            ))
        })
        .collect()
}

/// The cfg exec chain as the run starts it: its files with their digests and masked texts, the
/// resources it starts, and the game it sets.
#[derive(Debug, Clone, Default)]
pub(crate) struct Chain {
    pub files: Vec<ChainFile>,
    pub reads: Vec<Read>,
    pub started: Vec<String>,
    pub game: Option<Game>,
    pub game_build: Option<u32>,
}

/// One file of the chain as `snapshot.cfg` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChainFile {
    pub path: String,
    pub state: CfgFileState,
    pub digest: Option<String>,
}

fn chain(
    data: &Path,
    launch: &LaunchSettings,
    listed: &[ResourceListing],
    settings: &ScanSettings,
    keys: &Keys,
    rate: &mut Rate,
    cancel: &CancellationToken,
) -> Result<Chain, Cancelled> {
    let Ok(text) = cfg::read(&data.join(&launch.cfg_file)) else {
        return Ok(Chain {
            files: vec![ChainFile {
                path: launch.cfg_file.clone(),
                state: CfgFileState::Missing,
                digest: None,
            }],
            ..Chain::default()
        });
    };
    let resources = server_data::walk(data);
    let context = Context {
        panel: None,
        onesync: launch.onesync,
        probe_ports: false,
        system_resources: None,
        follow_outside: true,
        convars: None,
    };
    let scan = cfg::scan(data, &launch.cfg_file, &text, &resources, &context);
    let mut chain = Chain {
        started: scan.started,
        game: scan.facts.game,
        game_build: scan.facts.game_build,
        ..Chain::default()
    };
    let never_read: Vec<PathBuf> = settings
        .never_read
        .iter()
        .filter_map(|folder| std::fs::canonicalize(folder).ok())
        .collect();
    for mut file in scan.files {
        // `exec @resource/file.cfg` runs a file of that resource, which FXServer finds by name.
        if file.state == CfgFileState::Resource
            && let Some((name, rest)) = file.path.strip_prefix('@').and_then(|path| path.split_once('/'))
            && let Some(resource) = listed
                .iter()
                .find(|resource| resource.name.eq_ignore_ascii_case(name))
        {
            file.path = format!("{}/{rest}", resource.path);
            file.state = CfgFileState::Read;
        }
        let location = if Path::new(&file.path).is_absolute() {
            PathBuf::from(&file.path)
        } else {
            data.join(&file.path)
        };
        let readable = file.state == CfgFileState::Read
            && std::fs::canonicalize(&location)
                .is_ok_and(|resolved| !never_read.iter().any(|folder| resolved.starts_with(folder)));
        let digest = if readable {
            let treatment = Treatment {
                chain: true,
                hidden: false,
            };
            let read = read(&file.path, &location, treatment, settings, keys, rate, cancel)?;
            let digest = read.digest.clone();
            chain.reads.push(Read {
                path: file.path.clone(),
                read,
            });
            digest
        } else {
            None
        };
        chain.files.push(ChainFile {
            path: file.path,
            state: file.state,
            digest,
        });
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use observer_fxserver::changes::Stat;

    use super::*;

    fn entry(path: &str, size: u64) -> Entry {
        Entry {
            path: path.to_owned(),
            location: PathBuf::from(path),
            found: Found::File,
            stat: Stat {
                size,
                modified_ns: 1_000,
                inode: None,
                changed_ns: None,
            },
            hidden: false,
        }
    }

    fn indexed(kind: FileKind, size: u64) -> IndexEntry {
        IndexEntry {
            size,
            modified_ns: 1_000,
            inode: None,
            changed_ns: None,
            digest: Some("d".repeat(64)),
            sha256: Some("s".repeat(64)),
            kind: kind.as_str().to_owned(),
            racy: false,
            verified_at: None,
        }
    }

    #[test]
    fn an_unmoved_file_keeps_its_digest_and_only_shows_a_hash_it_may() {
        let settings = ScanSettings::default();
        let bodies = HashSet::from(["d".repeat(64)]);

        let binary = known(
            &entry("r/a.ydr", 10),
            Some(&indexed(FileKind::Binary, 10)),
            &settings,
            &bodies,
        )
        .unwrap();
        let text = known(
            &entry("r/a.lua", 10),
            Some(&indexed(FileKind::Text, 10)),
            &settings,
            &bodies,
        )
        .unwrap();

        assert_eq!((binary.kind, binary.sha256.is_some()), (FileKind::Binary, true));
        assert_eq!((text.kind, text.sha256), (FileKind::Text, None));
    }

    #[test]
    fn a_moved_racy_or_unknown_file_is_read_again() {
        let settings = ScanSettings::default();
        let bodies = HashSet::from(["d".repeat(64)]);
        let racy = IndexEntry {
            racy: true,
            ..indexed(FileKind::Text, 10)
        };

        assert!(
            known(
                &entry("r/a.lua", 11),
                Some(&indexed(FileKind::Text, 10)),
                &settings,
                &bodies
            )
            .is_none()
        );
        assert!(known(&entry("r/a.lua", 10), Some(&racy), &settings, &bodies).is_none());
        assert!(known(&entry("r/a.lua", 10), None, &settings, &bodies).is_none());
    }

    #[test]
    fn a_text_whose_body_was_dropped_is_read_and_stored_again() {
        let settings = ScanSettings::default();

        assert!(
            known(
                &entry("r/a.lua", 10),
                Some(&indexed(FileKind::Text, 10)),
                &settings,
                &HashSet::new()
            )
            .is_none()
        );
    }

    #[test]
    fn hiding_a_file_needs_no_read_and_showing_it_again_does() {
        let mut settings = ScanSettings::default();
        let bodies = HashSet::from(["d".repeat(64)]);
        settings.hidden = vec!["secret.lua".to_owned()];

        let hidden = known(
            &entry("r/secret.lua", 10),
            Some(&indexed(FileKind::Text, 10)),
            &settings,
            &bodies,
        )
        .unwrap();
        settings.hidden.clear();
        let shown = known(
            &entry("r/secret.lua", 10),
            Some(&indexed(FileKind::Hidden, 10)),
            &settings,
            &bodies,
        );

        assert_eq!(hidden.kind, FileKind::Hidden);
        assert!(shown.is_none());
    }

    #[test]
    fn archives_links_and_placeholders_are_never_read() {
        let settings = ScanSettings::default();
        let archive = unread(&entry("r/maps.zip", 10), &settings).unwrap();
        let link = unread(
            &Entry {
                found: Found::Link {
                    target: "../x".to_owned(),
                    followed: false,
                },
                ..entry("r/x", 0)
            },
            &settings,
        )
        .unwrap();

        assert_eq!(
            (archive.kind, archive.key.as_str()),
            (FileKind::Archive, "t:1000")
        );
        assert_eq!((link.kind, link.key.as_str()), (FileKind::Link, "l:../x"));
        assert!(unread(&entry("r/a.lua", 10), &settings).is_none());
    }

    #[test]
    fn a_resource_has_a_tree_once_nothing_waits() {
        let known_entry = TreeEntry {
            path: "r/a.lua".to_owned(),
            size: 1,
            key: "k".repeat(64),
            kind: FileKind::Text,
            sha256: None,
        };
        let mut planned = Planned {
            resource: ResourceListing {
                path: "resources/r".to_owned(),
                name: "r".to_owned(),
                shown_name: "r".to_owned(),
                duplicate: false,
                manifest: None,
                escrowed: false,
                git_head: None,
                packages: Vec::new(),
                entries: vec![entry("r/a.lua", 1), entry("r/b.ydr", 5)],
            },
            slots: vec![
                Slot::Known(known_entry.clone()),
                Slot::Pending(entry("r/b.ydr", 5)),
            ],
        };

        assert!(planned.tree().is_none());
        let row = planned.row(None);
        assert_eq!((row.pending, row.files, row.bytes), (1, 2, 6));
        let listing: Vec<TreeEntry> =
            serde_json::from_slice(row.pending_listing.as_deref().unwrap()).unwrap();
        assert_eq!(listing[1].key, "");

        planned.slots[1] = Slot::Known(TreeEntry {
            path: "r/b.ydr".to_owned(),
            size: 5,
            key: "b".repeat(64),
            kind: FileKind::Binary,
            sha256: Some("s".repeat(64)),
        });
        let (_, entries) = planned.tree().unwrap();
        assert_eq!(texts(&entries), [("r/a.lua".to_owned(), "k".repeat(64))]);
    }
}
