//! Two snapshots compared: resources paired by path, then by name for moves, then by their files
//! for renames; each pair classified; and the files of what changed, with their line counts within
//! a budget. The pure parts work on rows and entries already read.

use std::collections::{HashMap, HashSet};

use observer_domain::changes::{
    ChangeCounts, FileChange, FileChangeKind, FileCounts, FileFlag, FileKind, LineCounts, PartialReason,
    ResourceChange, ResourceChangeKind, SnapshotKind, SnapshotState, SnapshotSummary, hidden,
};
use observer_domain::files::{NEVER_SHOW, glob_matches};
use observer_fxserver::changes::tree::TreeEntry;
use observer_store::changes::resources::ResourceRow;
use observer_store::changes::snapshots::Snapshot;

/// Share of bytes, by file digest, past which a removed and an added resource are one renamed.
const RENAMED: f64 = 0.6;
/// Removed and added resources compared file by file for renames, at most, on each side.
const MAX_RENAME_CANDIDATES: usize = 64;

/// How two rows were paired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Paired {
    /// The same path on both sides.
    Path,
    /// The same name under another category.
    Moved,
    /// Another folder with the same files, or most of them.
    Renamed,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Pairs {
    pub paired: Vec<(usize, usize, Paired)>,
    pub removed: Vec<usize>,
    pub added: Vec<usize>,
}

/// Pairs rows by path, then the rest by name: a name in another category moved.
pub(crate) fn pair(from: &[ResourceRow], to: &[ResourceRow]) -> Pairs {
    let by_path: HashMap<&str, usize> = to
        .iter()
        .enumerate()
        .map(|(index, row)| (row.path.as_str(), index))
        .collect();
    let mut pairs = Pairs::default();
    let mut taken = vec![false; to.len()];
    let mut left = Vec::new();
    for (index, row) in from.iter().enumerate() {
        match by_path.get(row.path.as_str()) {
            Some(&other) => {
                taken[other] = true;
                pairs.paired.push((index, other, Paired::Path));
            }
            None => left.push(index),
        }
    }
    for index in left {
        let name = &from[index].name;
        let other = to
            .iter()
            .enumerate()
            .find(|(other, row)| !taken[*other] && row.name == *name)
            .map(|(other, _)| other);
        match other {
            Some(other) => {
                taken[other] = true;
                pairs.paired.push((index, other, Paired::Moved));
            }
            None => pairs.removed.push(index),
        }
    }
    pairs.added = (0..to.len()).filter(|index| !taken[*index]).collect();
    pairs
}

/// Pairs removed and added resources whose trees are equal, then, among a bounded number, those
/// sharing most of their bytes by file digest. `entries` gives a row's entries.
pub(crate) fn renames(
    pairs: &mut Pairs,
    from: &[ResourceRow],
    to: &[ResourceRow],
    entries: &mut dyn FnMut(bool, usize) -> Option<Vec<TreeEntry>>,
) {
    let mut matched = Vec::new();
    for &removed in &pairs.removed {
        let Some(tree) = &from[removed].tree else {
            continue;
        };
        if let Some(&added) = pairs.added.iter().find(|added| {
            to[**added].tree.as_ref() == Some(tree) && !matched.iter().any(|(_, a)| a == *added)
        }) {
            matched.push((removed, added));
        }
    }
    let unmatched = |pairs: &Pairs, matched: &[(usize, usize)], removed: bool| -> Vec<usize> {
        let list = if removed { &pairs.removed } else { &pairs.added };
        list.iter()
            .copied()
            .filter(|index| {
                !matched
                    .iter()
                    .any(|(r, a)| if removed { r == index } else { a == index })
            })
            .take(MAX_RENAME_CANDIDATES)
            .collect()
    };
    let removed = unmatched(pairs, &matched, true);
    let added = unmatched(pairs, &matched, false);
    if !removed.is_empty() && !added.is_empty() {
        let removed: Vec<(usize, Vec<TreeEntry>)> = removed
            .into_iter()
            .filter_map(|index| Some((index, entries(true, index)?)))
            .collect();
        let added: Vec<(usize, Vec<TreeEntry>)> = added
            .into_iter()
            .filter_map(|index| Some((index, entries(false, index)?)))
            .collect();
        // Each side's files by digest, once: every removed resource meets every added one.
        let removed: Vec<(usize, Sizes)> = removed
            .iter()
            .map(|(r, entries)| (*r, Sizes::of(entries)))
            .collect();
        let added: Vec<(usize, Sizes)> = added
            .iter()
            .map(|(a, entries)| (*a, Sizes::of(entries)))
            .collect();
        let mut scores = Vec::new();
        for (r, before) in &removed {
            for (a, after) in &added {
                let score = shared(before, after);
                if score >= RENAMED {
                    scores.push((score, *r, *a));
                }
            }
        }
        scores.sort_by(|x, y| y.0.total_cmp(&x.0));
        for (_, r, a) in scores {
            if !matched.iter().any(|(mr, ma)| *mr == r || *ma == a) {
                matched.push((r, a));
            }
        }
    }
    for (r, a) in matched {
        pairs.removed.retain(|index| *index != r);
        pairs.added.retain(|index| *index != a);
        pairs.paired.push((r, a, Paired::Renamed));
    }
}

/// A resource's files by keyed digest, and their total size.
struct Sizes<'a> {
    by_key: HashMap<&'a str, u64>,
    total: u64,
}

impl<'a> Sizes<'a> {
    fn of(entries: &'a [TreeEntry]) -> Self {
        let by_key: HashMap<&str, u64> = entries
            .iter()
            .filter(|entry| !entry.key.is_empty())
            .map(|entry| (entry.key.as_str(), entry.size))
            .collect();
        let total = by_key.values().sum();
        Self { by_key, total }
    }
}

/// The share of the larger side's bytes found, by digest, on both sides.
fn shared(before: &Sizes, after: &Sizes) -> f64 {
    let largest = before.total.max(after.total);
    if largest == 0 {
        return 0.0;
    }
    let common: u64 = before
        .by_key
        .iter()
        .filter(|(key, _)| after.by_key.contains_key(*key))
        .map(|(_, size)| size)
        .sum();
    #[allow(
        clippy::cast_precision_loss,
        reason = "a share needs no more than f64's 52 bits"
    )]
    let share = common as f64 / largest as f64;
    share
}

/// Whether the files differ, as far as the entries tell while some wait for the hashing pass: a
/// path on one side only, or a size that moved. `None` when nothing tells either way yet.
pub(crate) fn differ(before: &[TreeEntry], after: &[TreeEntry]) -> Option<bool> {
    let before: HashMap<&str, &TreeEntry> = before.iter().map(|entry| (entry.path.as_str(), entry)).collect();
    if before.len() != after.len() {
        return Some(true);
    }
    let mut unknown = false;
    for entry in after {
        let Some(other) = before.get(entry.path.as_str()) else {
            return Some(true);
        };
        if other.size != entry.size {
            return Some(true);
        }
        if entry.key.is_empty() || other.key.is_empty() {
            unknown = true;
        } else if entry.key != other.key {
            return Some(true);
        }
    }
    (!unknown).then_some(false)
}

/// How a paired resource changed, from its rows and, while a side waits for hashing, its entries.
pub(crate) fn classify(
    before: &ResourceRow,
    after: &ResourceRow,
    paired: Paired,
    entries: Option<(&[TreeEntry], &[TreeEntry])>,
    did_not_start: bool,
    settled: bool,
) -> (ResourceChangeKind, bool) {
    let changed = match (&before.tree, &after.tree) {
        (Some(a), Some(b)) => Some(a != b),
        _ => entries.and_then(|(a, b)| differ(a, b)),
    };
    let version_changed = before
        .manifest
        .as_ref()
        .map(|manifest| manifest.compared_version())
        != after
            .manifest
            .as_ref()
            .map(|manifest| manifest.compared_version());
    match (paired, changed) {
        (Paired::Moved, changed) => (ResourceChangeKind::Moved, changed.unwrap_or(false)),
        (Paired::Renamed, changed) => (ResourceChangeKind::Renamed, changed.unwrap_or(false)),
        (Paired::Path, None) if settled => (ResourceChangeKind::Unknown, false),
        (Paired::Path, None) => (ResourceChangeKind::Checking, false),
        (Paired::Path, Some(true)) if version_changed => (ResourceChangeKind::Updated, true),
        (Paired::Path, Some(true)) => (ResourceChangeKind::SameVersion, true),
        (Paired::Path, Some(false)) if did_not_start => (ResourceChangeKind::DidNotStart, false),
        (Paired::Path, Some(false)) => (ResourceChangeKind::Unchanged, false),
    }
}

/// The files of a resource on two sides, by their path below the resource, unchanged ones too.
/// A removed and an added file with one digest were renamed.
pub(crate) fn files(
    before: Option<(&str, &[TreeEntry])>,
    after: Option<(&str, &[TreeEntry])>,
    hidden_globs: &[String],
) -> Vec<FileChange> {
    let relative = |root: &str, entry: &TreeEntry| -> String {
        entry
            .path
            .strip_prefix(root)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(&entry.path)
            .to_owned()
    };
    let side = |side: Option<(&str, &[TreeEntry])>| -> HashMap<String, TreeEntry> {
        side.map(|(root, entries)| {
            entries
                .iter()
                .map(|entry| (relative(root, entry), entry.clone()))
                .collect()
        })
        .unwrap_or_default()
    };
    let (old, new) = (side(before), side(after));
    let mut changes = Vec::new();
    let mut gone: Vec<&TreeEntry> = Vec::new();
    let mut came: Vec<&TreeEntry> = Vec::new();
    for (path, entry) in &new {
        match old.get(path) {
            Some(was) => changes.push(change(Some(was), Some(entry), hidden_globs)),
            None => came.push(entry),
        }
    }
    for (path, entry) in &old {
        if !new.contains_key(path) {
            gone.push(entry);
        }
    }
    for entry in came {
        let renamed = gone.iter().position(|was| {
            !was.key.is_empty() && entry.digest().is_some() && was.key == entry.key && was.size == entry.size
        });
        match renamed {
            Some(index) => {
                let was = gone.remove(index);
                let mut file = change(Some(was), Some(entry), hidden_globs);
                file.change = FileChangeKind::Renamed;
                file.from_path = Some(was.path.clone());
                changes.push(file);
            }
            None => changes.push(change(None, Some(entry), hidden_globs)),
        }
    }
    for was in gone {
        changes.push(change(Some(was), None, hidden_globs));
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

fn change(before: Option<&TreeEntry>, after: Option<&TreeEntry>, hidden_globs: &[String]) -> FileChange {
    let shown = after.or(before).expect("a file is on one side at least");
    let kind = if hidden(&shown.path, hidden_globs) {
        FileKind::Hidden
    } else {
        shown.kind
    };
    let change = match (before, after) {
        (Some(a), Some(b)) if a.key == b.key && a.size == b.size => FileChangeKind::Unchanged,
        (Some(_), Some(_)) => FileChangeKind::Modified,
        (None, _) => FileChangeKind::Added,
        (_, None) => FileChangeKind::Removed,
    };
    let sha256 = |entry: Option<&TreeEntry>| {
        entry
            .filter(|entry| kind.shows_hash() && entry.kind.shows_hash())
            .and_then(|entry| entry.sha256.clone())
    };
    FileChange {
        path: shown.path.clone(),
        // A file of a resource that moved, or a file found under another path.
        from_path: before
            .zip(after)
            .filter(|(a, b)| a.path != b.path)
            .map(|(a, _)| a.path.clone()),
        kind,
        change,
        from_size: before.map(|entry| entry.size),
        to_size: after.map(|entry| entry.size),
        from_sha256: sha256(before),
        to_sha256: sha256(after),
        lines: None,
        flags: Vec::new(),
        hidden_by: (kind == FileKind::Hidden).then(|| hidden_by(&shown.path, hidden_globs)),
        kept: false,
        written_by: None,
        written_at: None,
    }
}

/// The never-show name or the owner's glob that hides `path`.
pub(crate) fn hidden_by(path: &str, globs: &[String]) -> String {
    for name in path.split('/') {
        if let Some(glob) = NEVER_SHOW.iter().find(|glob| glob_matches(glob, name, true)) {
            return (*glob).to_owned();
        }
    }
    globs
        .iter()
        .find(|glob| observer_domain::changes::glob_covers(glob, path))
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn file_counts(files: &[FileChange]) -> FileCounts {
    let mut counts = FileCounts::default();
    for file in files {
        match file.change {
            FileChangeKind::Added => counts.added += 1,
            FileChangeKind::Removed => counts.removed += 1,
            FileChangeKind::Modified | FileChangeKind::Renamed => counts.modified += 1,
            FileChangeKind::Unchanged => {}
        }
    }
    counts
}

/// A resource change before its files are counted.
pub(crate) fn resource_change(
    before: Option<&ResourceRow>,
    after: Option<&ResourceRow>,
    kind: ResourceChangeKind,
    changed: bool,
) -> ResourceChange {
    let shown = after.or(before).expect("a resource is on one side at least");
    let version = |row: Option<&ResourceRow>| row.and_then(|row| row.version().map(str::to_owned));
    ResourceChange {
        path: shown.path.clone(),
        from_path: before
            .zip(after)
            .filter(|(a, b)| a.path != b.path)
            .map(|(a, _)| a.path.clone()),
        name: shown.shown_name.clone(),
        change: kind,
        changed,
        from_version: version(before),
        to_version: version(after),
        duplicate: shown.duplicate,
        escrowed: shown.escrowed,
        files: FileCounts::default(),
        lines: None,
        by: Vec::new(),
    }
}

/// Counts the kinds of change.
pub(crate) fn count(resources: &[ResourceChange], unchanged: u32) -> ChangeCounts {
    let mut counts = ChangeCounts {
        unchanged,
        ..ChangeCounts::default()
    };
    for resource in resources {
        let counter = match resource.change {
            ResourceChangeKind::Added => &mut counts.added,
            ResourceChangeKind::Removed => &mut counts.removed,
            ResourceChangeKind::Renamed => &mut counts.renamed,
            ResourceChangeKind::Moved => &mut counts.moved,
            ResourceChangeKind::Updated => &mut counts.updated,
            ResourceChangeKind::SameVersion => &mut counts.same_version,
            ResourceChangeKind::Checking | ResourceChangeKind::Unknown => &mut counts.checking,
            ResourceChangeKind::DidNotStart => &mut counts.did_not_start,
            ResourceChangeKind::Unchanged => &mut counts.unchanged,
        };
        *counter += 1;
        counts.files += resource.files.total();
    }
    counts
}

/// The order the table lists them in: what a version string hides first.
pub(crate) fn rank(kind: ResourceChangeKind) -> u8 {
    match kind {
        ResourceChangeKind::SameVersion => 0,
        ResourceChangeKind::Updated => 1,
        ResourceChangeKind::Added => 2,
        ResourceChangeKind::Removed => 3,
        ResourceChangeKind::Renamed => 4,
        ResourceChangeKind::Moved => 5,
        ResourceChangeKind::DidNotStart => 6,
        ResourceChangeKind::Checking | ResourceChangeKind::Unknown => 7,
        ResourceChangeKind::Unchanged => 8,
    }
}

/// Whether a resource started in a run whose bridge reported what started.
pub(crate) fn started(snapshot: &Snapshot, name: &str) -> Option<bool> {
    snapshot
        .started
        .as_ref()
        .map(|started| started.iter().any(|known| known.eq_ignore_ascii_case(name)))
}

pub(crate) fn summary(snapshot: &Snapshot, counts: Option<ChangeCounts>, crashed: bool) -> SnapshotSummary {
    SnapshotSummary {
        id: snapshot.id,
        kind: snapshot.kind,
        taken_at: snapshot.taken_at,
        cause: snapshot.cause,
        by: snapshot.by.clone(),
        state: snapshot.state,
        partial_reason: snapshot.partial_reason,
        baseline: snapshot.baseline,
        build: snapshot.build,
        game_build: snapshot.game_build,
        files: snapshot.files,
        bytes: snapshot.bytes,
        counts,
        crashed,
        run_started: snapshot.run_started,
        generation: snapshot.generation,
    }
}

/// Whether the listing left resources out, at a limit or in a category it could not read: one
/// missing from it is not known to be gone.
pub(crate) fn shortened(snapshot: &Snapshot) -> bool {
    matches!(
        snapshot.partial_reason,
        Some(PartialReason::Limit | PartialReason::Unreadable)
    )
}

/// Whether the snapshot's job is over: a resource still without a tree then stays unknown.
pub(crate) fn settled(snapshot: &Snapshot) -> bool {
    snapshot.settled() && !(snapshot.kind == SnapshotKind::Start && snapshot.state == SnapshotState::Listing)
}

/// The flags of a file in a snapshot.
pub(crate) fn flags_of(flags: &HashMap<String, Vec<FileFlag>>, path: &str) -> Vec<FileFlag> {
    flags.get(path).cloned().unwrap_or_default()
}

/// The text entries both sides hold of a modified file, whose lines can be counted.
pub(crate) fn texts_of(
    file: &FileChange,
    before: &[TreeEntry],
    after: &[TreeEntry],
) -> Option<(String, String)> {
    if file.change != FileChangeKind::Modified || file.kind != FileKind::Text {
        return None;
    }
    let find = |entries: &[TreeEntry], path: &str| {
        entries
            .iter()
            .find(|entry| entry.path == path && entry.kind == FileKind::Text)
            .and_then(|entry| entry.digest().map(str::to_owned))
    };
    Some((
        find(before, file.from_path.as_deref().unwrap_or(&file.path))?,
        find(after, &file.path)?,
    ))
}

/// Lines added and removed over several files.
pub(crate) fn sum_lines(lines: impl Iterator<Item = LineCounts>) -> LineCounts {
    lines.fold(LineCounts::default(), |total, lines| LineCounts {
        added: total.added + lines.added,
        removed: total.removed + lines.removed,
    })
}

/// Distinct names in `names`, in order.
pub(crate) fn distinct(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    names.filter(|name| seen.insert(name.clone())).collect()
}

#[cfg(test)]
mod tests {
    use observer_domain::changes::Manifest;

    use super::*;

    fn row(path: &str, tree: Option<&str>, version: &str) -> ResourceRow {
        let name = path.rsplit('/').next().unwrap().to_owned();
        ResourceRow {
            path: path.to_owned(),
            name: name.to_ascii_lowercase(),
            shown_name: name,
            duplicate: false,
            tree: tree.map(str::to_owned),
            pending_listing: None,
            pending: 0,
            manifest: Some(Manifest {
                file: "fxmanifest.lua".to_owned(),
                version: Some(version.to_owned()),
                ..Manifest::default()
            }),
            escrowed: false,
            git_head: None,
            packages: Vec::new(),
            files: 1,
            bytes: 1,
        }
    }

    /// A file entry; a key that is not empty stands for a 64-character digest.
    fn entry(path: &str, key: &str, size: u64) -> TreeEntry {
        TreeEntry {
            path: path.to_owned(),
            size,
            key: if key.is_empty() {
                String::new()
            } else {
                format!("{key:0>64}")
            },
            kind: FileKind::Text,
            sha256: None,
        }
    }

    #[test]
    fn rows_pair_by_path_then_by_name_for_a_move() {
        let from = [
            row("resources/a", Some("t"), "1"),
            row("resources/[x]/b", Some("t"), "1"),
            row("resources/c", Some("t"), "1"),
        ];
        let to = [
            row("resources/a", Some("t"), "1"),
            row("resources/[y]/b", Some("t"), "1"),
            row("resources/d", Some("t"), "1"),
        ];

        let pairs = pair(&from, &to);

        assert_eq!(pairs.paired, [(0, 0, Paired::Path), (1, 1, Paired::Moved)]);
        assert_eq!((pairs.removed, pairs.added), (vec![2], vec![2]));
    }

    #[test]
    fn a_rename_keeps_the_tree_or_most_of_the_bytes() {
        let from = [
            row("resources/old", Some("same"), "1"),
            row("resources/big", Some("t1"), "1"),
        ];
        let to = [
            row("resources/new", Some("same"), "1"),
            row("resources/bigger", Some("t2"), "1"),
        ];
        let mut pairs = pair(&from, &to);
        let mut entries = |before: bool, index: usize| {
            Some(match (before, index) {
                (true, 1) => vec![
                    entry("resources/big/a", "k1", 80),
                    entry("resources/big/b", "k2", 20),
                ],
                (false, 1) => vec![
                    entry("resources/bigger/a", "k1", 80),
                    entry("resources/bigger/c", "k3", 30),
                ],
                _ => Vec::new(),
            })
        };

        renames(&mut pairs, &from, &to, &mut entries);

        assert_eq!(pairs.paired, [(0, 0, Paired::Renamed), (1, 1, Paired::Renamed)]);
        assert!(pairs.removed.is_empty() && pairs.added.is_empty());
    }

    #[test]
    fn less_than_sixty_percent_shared_is_a_removal_and_an_addition() {
        let from = [row("resources/a", Some("t1"), "1")];
        let to = [row("resources/b", Some("t2"), "1")];
        let mut pairs = pair(&from, &to);

        renames(&mut pairs, &from, &to, &mut |before, _| {
            Some(if before {
                vec![entry("resources/a/x", "k1", 50), entry("resources/a/y", "k2", 50)]
            } else {
                vec![entry("resources/b/x", "k1", 50), entry("resources/b/z", "k3", 60)]
            })
        });

        assert_eq!((pairs.removed, pairs.added), (vec![0], vec![0]));
    }

    #[test]
    fn a_changed_tree_is_updated_or_same_version_and_an_equal_one_unchanged() {
        let before = row("resources/a", Some("t1"), "1.0");

        let same_version = classify(
            &before,
            &row("resources/a", Some("t2"), "1.0"),
            Paired::Path,
            None,
            false,
            true,
        );
        let updated = classify(
            &before,
            &row("resources/a", Some("t2"), "1.1"),
            Paired::Path,
            None,
            false,
            true,
        );
        let unchanged = classify(
            &before,
            &row("resources/a", Some("t1"), "1.0"),
            Paired::Path,
            None,
            false,
            true,
        );
        let not_started = classify(
            &before,
            &row("resources/a", Some("t1"), "1.0"),
            Paired::Path,
            None,
            true,
            true,
        );

        assert_eq!(same_version, (ResourceChangeKind::SameVersion, true));
        assert_eq!(updated, (ResourceChangeKind::Updated, true));
        assert_eq!(unchanged, (ResourceChangeKind::Unchanged, false));
        assert_eq!(not_started, (ResourceChangeKind::DidNotStart, false));
    }

    #[test]
    fn a_resource_waiting_for_hashing_is_checking_unless_a_size_already_tells() {
        let before = row("resources/a", Some("t1"), "1.0");
        let after = row("resources/a", None, "1.0");
        let old = [entry("resources/a/x", "k1", 10)];
        let same_size = [entry("resources/a/x", "", 10)];
        let bigger = [entry("resources/a/x", "", 11)];

        let checking = classify(
            &before,
            &after,
            Paired::Path,
            Some((&old, &same_size)),
            false,
            false,
        );
        let unknown = classify(
            &before,
            &after,
            Paired::Path,
            Some((&old, &same_size)),
            false,
            true,
        );
        let changed = classify(&before, &after, Paired::Path, Some((&old, &bigger)), false, false);

        assert_eq!(checking.0, ResourceChangeKind::Checking);
        assert_eq!(unknown.0, ResourceChangeKind::Unknown);
        assert_eq!(changed, (ResourceChangeKind::SameVersion, true));
    }

    #[test]
    fn files_are_added_removed_modified_or_renamed_below_the_resource() {
        let before = [
            entry("resources/a/keep.lua", "k1", 1),
            entry("resources/a/edit.lua", "k2", 1),
            entry("resources/a/old.lua", "k3", 1),
            entry("resources/a/gone.lua", "k4", 1),
        ];
        let after = [
            entry("resources/b/keep.lua", "k1", 1),
            entry("resources/b/edit.lua", "k5", 2),
            entry("resources/b/new.lua", "k3", 1),
            entry("resources/b/added.lua", "k6", 1),
        ];

        let files = files(Some(("resources/a", &before)), Some(("resources/b", &after)), &[]);

        let kinds: Vec<(&str, FileChangeKind)> = files
            .iter()
            .map(|file| (file.path.as_str(), file.change))
            .collect();
        assert_eq!(
            kinds,
            [
                ("resources/a/gone.lua", FileChangeKind::Removed),
                ("resources/b/added.lua", FileChangeKind::Added),
                ("resources/b/edit.lua", FileChangeKind::Modified),
                ("resources/b/keep.lua", FileChangeKind::Unchanged),
                ("resources/b/new.lua", FileChangeKind::Renamed),
            ]
        );
        assert_eq!(files[4].from_path.as_deref(), Some("resources/a/old.lua"));
        // The resource moved: each file says where it was, which its diff reads.
        assert_eq!(files[2].from_path.as_deref(), Some("resources/a/edit.lua"));
        assert_eq!(files[3].from_path.as_deref(), Some("resources/a/keep.lua"));
        assert_eq!(
            file_counts(&files),
            FileCounts {
                added: 1,
                removed: 1,
                modified: 2
            }
        );
    }

    #[test]
    fn a_hidden_file_shows_no_hash_and_names_what_hides_it() {
        let binary = TreeEntry {
            kind: FileKind::Binary,
            sha256: Some("s".repeat(64)),
            ..entry("resources/a/keys/server.pem", "k1", 1)
        };

        let files = files(None, Some(("resources/a", &[binary])), &[]);

        assert_eq!(files[0].kind, FileKind::Hidden);
        assert_eq!(files[0].to_sha256, None);
        assert_eq!(files[0].hidden_by.as_deref(), Some("*.pem"));
    }
}
