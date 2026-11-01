use std::fmt;
use std::io;
use std::time::SystemTime;

/// A path below a confined tree's root, as validated components. It is the second line behind the
/// rules' own normalisation: it refuses on its own every component that could leave the tree or
/// reach something other than the named entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RelativePath {
    components: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a name a confined tree accepts")]
pub struct InvalidName(pub String);

impl RelativePath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn new<I, S>(components: I) -> Result<Self, InvalidName>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let components = components
            .into_iter()
            .map(|component| {
                let component = component.into();
                if valid_name(&component) {
                    Ok(component)
                } else {
                    Err(InvalidName(component))
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { components })
    }

    pub fn components(&self) -> &[String] {
        &self.components
    }

    pub fn is_root(&self) -> bool {
        self.components.is_empty()
    }

    /// The folder that holds the entry, and the entry's name; `None` for the root.
    pub fn split_last(&self) -> Option<(RelativePath, &str)> {
        let (name, parent) = self.components.split_last()?;
        Some((
            Self {
                components: parent.to_vec(),
            },
            name,
        ))
    }

    pub fn join(&self, name: &str) -> Result<Self, InvalidName> {
        if !valid_name(name) {
            return Err(InvalidName(name.to_owned()));
        }
        let mut components = self.components.clone();
        components.push(name.to_owned());
        Ok(Self { components })
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.components.is_empty() {
            return f.write_str("/");
        }
        for component in &self.components {
            write!(f, "/{component}")?;
        }
        Ok(())
    }
}

/// `:` names an alternate data stream on Windows and `\` separates components there; neither may
/// hide inside one component.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

/// Why an operation on a confined tree was refused.
#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    #[error("no such file or folder")]
    NotFound,
    #[error("the name is already taken")]
    Exists,
    /// A symbolic link, a junction or a mount point: never followed, and hidden from listings.
    #[error("a link, a junction or a mount point is never followed")]
    Link,
    /// A Windows reparse point that is not a link, such as a cloud file's placeholder.
    #[error("a reparse point (cloud or deduplicated file) is not served")]
    ReparsePoint,
    #[error("a file with several hard links is not served")]
    HardLink,
    /// A pipe, a socket or a device.
    #[error("only regular files and folders are served")]
    NotRegular,
    #[error("not a folder")]
    NotAFolder,
    #[error("a folder, not a file")]
    IsAFolder,
    #[error("the folder is not empty")]
    NotEmpty,
    /// The entry's path, read back from its handle, is not the path that was asked for, or lies
    /// outside the root.
    #[error("the entry is not where it was asked for")]
    Escaped,
    #[error("another program holds this file open")]
    InUse,
    #[error("the file is read-only")]
    ReadOnly,
    /// A commit that expected one file in its target's place found another entry there, or none:
    /// another writer replaced, removed or created it since it was read.
    #[error("the file changed since it was read")]
    Changed,
    #[error(transparent)]
    Io(io::Error),
}

/// What tells a file from every other on its volume, whichever name leads to it: the volume's
/// serial number and the file's index on Windows, the device and inode numbers on Unix. The hard
/// links of one file share it; a file written anew in a name's place has its own. It holds while the
/// volume stays mounted: Linux may number a device differently once it is mounted again. Unix may
/// also give the inode of a file removed, with its last link, to the next file created, where NTFS
/// counts each reuse of an index in the index itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    volume: u64,
    index: u128,
}

impl FileId {
    pub(crate) fn new(volume: u64, index: u128) -> Self {
        Self { volume, index }
    }

    /// The volume's serial number on Windows, the device number on Unix.
    pub fn volume(&self) -> u64 {
        self.volume
    }

    /// The file's index on its volume: 128 bits on Windows, which `ReFS` uses, the inode on Unix.
    pub fn index(&self) -> u128 {
        self.index
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}-{:032x}", self.volume, self.index)
    }
}

/// What a commit does with an entry already named as its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Replace {
    /// Refuses the commit (`Exists`).
    Never,
    /// Takes the place of a regular file no other hard link shares.
    Alone,
    /// Takes the place of the regular file the identity names, even when other hard links share
    /// it, and of nothing else (`Changed`). The rename moves a name, not the file: the other links
    /// keep the old content, and nothing is written through them.
    Shared(FileId),
}

/// One entry of a listing. Links, mount points, pipes, sockets and devices never appear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
    /// Listed so that its owner sees it, but refused to every operation.
    ReparsePoint,
}

/// Compares two names the way NTFS does: each character mapped to its simple uppercase, one
/// character to one, never expanded (ß stays ß).
#[cfg(any(windows, test))]
pub(crate) fn same_name_ignoring_case(a: &str, b: &str) -> bool {
    a.chars()
        .map(simple_uppercase)
        .eq(b.chars().map(simple_uppercase))
}

#[cfg(any(windows, test))]
fn simple_uppercase(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_refuses_every_component_that_could_leave_or_alias() {
        for refused in ["", ".", "..", "a/b", "a\\b", "server.cfg:secret", "nul\0"] {
            assert!(RelativePath::new([refused]).is_err(), "{refused:?} was accepted");
        }
        let path = RelativePath::new(["resources", "[maps]", "map.ymap"]).unwrap();
        assert_eq!(path.to_string(), "/resources/[maps]/map.ymap");
        let (parent, name) = path.split_last().unwrap();
        assert_eq!(parent.to_string(), "/resources/[maps]");
        assert_eq!(name, "map.ymap");
        assert_eq!(RelativePath::root().to_string(), "/");
    }

    #[test]
    fn names_compare_by_simple_uppercase_only() {
        assert!(same_name_ignoring_case("Server.CFG", "server.cfg"));
        assert!(same_name_ignoring_case("é", "É"));
        assert!(!same_name_ignoring_case("straße", "STRASSE"));
    }

    use std::fs;
    use std::io::{Read, Write};
    use std::path::Path;

    use crate::ConfinedTree;

    fn path(text: &str) -> RelativePath {
        RelativePath::new(text.split('/').filter(|part| !part.is_empty())).unwrap()
    }

    fn read(tree: &ConfinedTree, at: &str) -> Result<String, TreeError> {
        let mut text = String::new();
        tree.open_file(&path(at))?
            .read_to_string(&mut text)
            .map_err(TreeError::Io)?;
        Ok(text)
    }

    /// A server folder with one resource, and a secret beside it, outside the tree.
    fn server() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("server");
        fs::create_dir_all(root.join("resources/[maps]/harbour")).unwrap();
        fs::write(
            root.join("resources/[maps]/harbour/fxmanifest.lua"),
            "fx_version 'cerulean'",
        )
        .unwrap();
        fs::write(dir.path().join("secret.cfg"), "set mysql_connection_string \"…\"").unwrap();
        (dir, root)
    }

    #[cfg(windows)]
    fn link_folder(link: &Path, target: &Path) {
        // A junction needs no administrator, unlike a symbolic link.
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[cfg(unix)]
    fn link_folder(link: &Path, target: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn a_file_is_read_through_its_folders_and_listed_with_them() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();

        assert_eq!(
            read(&tree, "resources/[maps]/harbour/fxmanifest.lua").unwrap(),
            "fx_version 'cerulean'"
        );
        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();
        let entries = tree.list(&folder).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "fxmanifest.lua");
        assert_eq!(entries[0].kind, EntryKind::File);
        assert_eq!(entries[0].size, 21);
        assert!(matches!(
            tree.open_file(&path("resources/missing.lua")),
            Err(TreeError::NotFound)
        ));
        assert!(matches!(
            tree.open_file(&path("resources")),
            Err(TreeError::IsAFolder)
        ));
    }

    #[test]
    fn a_linked_folder_is_neither_followed_nor_listed_at_any_depth() {
        let (dir, root) = server();
        link_folder(&root.join("outside"), dir.path());
        link_folder(&root.join("resources").join("[maps]").join("outside"), dir.path());
        let tree = ConfinedTree::open(&root).unwrap();

        assert!(matches!(
            tree.open_file(&path("outside/secret.cfg")),
            Err(TreeError::Link)
        ));
        assert!(matches!(
            tree.open_file(&path("resources/[maps]/outside/secret.cfg")),
            Err(TreeError::Link)
        ));
        assert!(matches!(tree.open_folder(&path("outside")), Err(TreeError::Link)));
        assert!(matches!(tree.entry(&path("outside")), Err(TreeError::Link)));
        let names: Vec<_> = tree
            .list(tree.root())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["resources"]);
        let maps = tree.open_folder(&path("resources/[maps]")).unwrap();
        let names: Vec<_> = tree
            .list(&maps)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["harbour"]);
    }

    #[test]
    fn the_fallbacks_refuse_links_and_never_replace_unless_asked() {
        let (dir, root) = server();
        link_folder(&root.join("outside"), dir.path());
        let tree = ConfinedTree::open(&root).unwrap().with_fallbacks();

        assert!(matches!(
            tree.open_file(&path("outside/secret.cfg")),
            Err(TreeError::Link)
        ));
        let names: Vec<_> = tree
            .list(tree.root())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["resources"]);

        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();
        let mut upload = tree.create_temporary(&folder, ".observer-upload-f").unwrap();
        upload.file().write_all(b"fx_version 'adamant'").unwrap();
        assert!(matches!(
            tree.commit(&folder, upload, "fxmanifest.lua", false),
            Err(TreeError::Exists)
        ));
        let mut upload = tree.create_temporary(&folder, ".observer-upload-f").unwrap();
        upload.file().write_all(b"fx_version 'adamant'").unwrap();
        tree.commit(&folder, upload, "copy.lua", false).unwrap();
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/copy.lua").unwrap(),
            "fx_version 'adamant'"
        );
        tree.remove_file(&folder, "copy.lua").unwrap();

        let mut upload = tree.create_temporary(&folder, ".observer-upload-g").unwrap();
        upload.file().write_all(b"new").unwrap();
        let download = tree
            .open_file(&path("resources/[maps]/harbour/fxmanifest.lua"))
            .unwrap();
        let held = tree.commit(&folder, upload, "fxmanifest.lua", true);
        if cfg!(windows) {
            // Without POSIX semantics, any open handle on the target blocks the replace.
            assert!(matches!(held, Err(TreeError::InUse)), "{held:?}");
            drop(download);
            let mut upload = tree.create_temporary(&folder, ".observer-upload-g").unwrap();
            upload.file().write_all(b"new").unwrap();
            tree.commit(&folder, upload, "fxmanifest.lua", true).unwrap();
        } else {
            held.unwrap();
        }
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/fxmanifest.lua").unwrap(),
            "new"
        );
    }

    #[test]
    fn a_folder_swapped_for_a_link_after_listing_is_refused() {
        let (dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let listed = tree.list(tree.root()).unwrap();
        assert_eq!(listed[0].name, "resources");

        fs::rename(root.join("resources"), dir.path().join("moved")).unwrap();
        link_folder(&root.join("resources"), &dir.path().join("moved"));

        assert!(matches!(
            tree.open_file(&path("resources/[maps]/harbour/fxmanifest.lua")),
            Err(TreeError::Link)
        ));
    }

    #[test]
    fn a_hard_link_is_refused_whether_its_twin_is_inside_or_outside() {
        let (dir, root) = server();
        fs::hard_link(dir.path().join("secret.cfg"), root.join("innocent.txt")).unwrap();
        fs::write(root.join("a.lua"), "print(1)").unwrap();
        fs::hard_link(root.join("a.lua"), root.join("b.lua")).unwrap();
        let tree = ConfinedTree::open(&root).unwrap();

        assert!(matches!(
            tree.open_file(&path("innocent.txt")),
            Err(TreeError::HardLink)
        ));
        assert!(matches!(tree.open_file(&path("a.lua")), Err(TreeError::HardLink)));
        assert!(matches!(
            tree.remove_file(tree.root(), "b.lua"),
            Err(TreeError::HardLink)
        ));
        let source = tree.root();
        let target = tree.open_folder(&path("resources")).unwrap();
        assert!(matches!(
            tree.rename(source, "a.lua", &target, "a.lua", false),
            Err(TreeError::HardLink)
        ));
    }

    /// A stream file as FXServer leaves it: hard-linked into the resource's cache.
    fn cached_stream_file(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let stream = root.join("resources/[maps]/harbour/stream");
        let cache = root.join("cache/files/harbour/stream_cache");
        fs::create_dir_all(&stream).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(stream.join("harbour.ytyp"), "old").unwrap();
        fs::hard_link(stream.join("harbour.ytyp"), cache.join("harbour.ytyp")).unwrap();
        (stream.join("harbour.ytyp"), cache.join("harbour.ytyp"))
    }

    /// The folder's entries by name, in byte order.
    fn names(tree: &ConfinedTree, folder: &crate::Folder) -> Vec<String> {
        let mut names: Vec<_> = tree
            .list(folder)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_file_shared_by_another_hard_link_is_replaced_over_shared_and_the_link_keeps_the_old_bytes() {
        for fallbacks in [false, true] {
            let (_dir, root) = server();
            let (stream, cached) = cached_stream_file(&root);
            let tree = ConfinedTree::open(&root).unwrap();
            let tree = if fallbacks { tree.with_fallbacks() } else { tree };
            let folder = tree
                .open_folder(&path("resources/[maps]/harbour/stream"))
                .unwrap();

            let mut upload = tree.create_temporary(&folder, ".observer-fix-a").unwrap();
            upload.file().write_all(b"new").unwrap();
            let ordinary = tree.commit(&folder, upload, "harbour.ytyp", true);
            assert!(matches!(ordinary, Err(TreeError::HardLink)), "{ordinary:?}");
            assert_eq!(fs::read_to_string(&stream).unwrap(), "old");

            // The refused commit removed its temporary file, so the name is free again.
            let (_, seen) = tree
                .open_file_shared(&path("resources/[maps]/harbour/stream/harbour.ytyp"))
                .unwrap();
            let mut upload = tree.create_temporary(&folder, ".observer-fix-a").unwrap();
            upload.file().write_all(b"new").unwrap();
            tree.commit_over_shared(&folder, upload, "harbour.ytyp", Some(seen))
                .unwrap();

            assert_eq!(
                read(&tree, "resources/[maps]/harbour/stream/harbour.ytyp").unwrap(),
                "new",
                "alone now, the new file is served"
            );
            assert_eq!(
                fs::read_to_string(&cached).unwrap(),
                "old",
                "fallbacks: {fallbacks}"
            );
            assert_eq!(names(&tree, &folder), ["harbour.ytyp"]);

            let mut upload = tree.create_temporary(&folder, ".observer-fix-b").unwrap();
            upload.file().write_all(b"fresh").unwrap();
            tree.commit_over_shared(&folder, upload, "harbour_props.ytyp", None)
                .unwrap();
            assert_eq!(
                read(&tree, "resources/[maps]/harbour/stream/harbour_props.ytyp").unwrap(),
                "fresh"
            );
        }
    }

    #[test]
    fn a_replace_over_shared_never_writes_through_a_link_to_outside() {
        let (dir, root) = server();
        fs::hard_link(dir.path().join("secret.cfg"), root.join("innocent.txt")).unwrap();
        let tree = ConfinedTree::open(&root).unwrap();
        let (_, seen) = tree.open_file_shared(&path("innocent.txt")).unwrap();

        let mut upload = tree.create_temporary(tree.root(), ".observer-fix-c").unwrap();
        upload.file().write_all(b"replaced").unwrap();
        tree.commit_over_shared(tree.root(), upload, "innocent.txt", Some(seen))
            .unwrap();

        assert_eq!(read(&tree, "innocent.txt").unwrap(), "replaced");
        assert_eq!(
            fs::read_to_string(dir.path().join("secret.cfg")).unwrap(),
            "set mysql_connection_string \"…\""
        );
    }

    #[test]
    fn a_replace_over_shared_keeps_every_other_refusal_and_removes_its_temporary_file() {
        let (dir, root) = server();
        let (stream_file, cached) = cached_stream_file(&root);
        // One component at a time: `mklink` reads a `/` as the start of an option.
        let link = ["resources", "[maps]", "harbour", "stream", "linked.ytyp"]
            .iter()
            .fold(root.clone(), |path, name| path.join(name));
        link_folder(&link, dir.path());
        let tree = ConfinedTree::open(&root).unwrap();
        let harbour = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();
        let stream = tree
            .open_folder(&path("resources/[maps]/harbour/stream"))
            .unwrap();
        // The very file in place where one is expected, so that each refusal is the entry's own.
        let (_, seen) = tree
            .open_file_shared(&path("resources/[maps]/harbour/stream/harbour.ytyp"))
            .unwrap();
        let expected = Some(seen);

        let upload = tree.create_temporary(&harbour, ".observer-fix-d").unwrap();
        let folder = tree.commit_over_shared(&harbour, upload, "stream", expected);
        assert!(matches!(folder, Err(TreeError::IsAFolder)), "{folder:?}");
        let upload = tree.create_temporary(&stream, ".observer-fix-d").unwrap();
        let escaped = tree.commit_over_shared(&stream, upload, "../fxmanifest.lua", expected);
        assert!(matches!(escaped, Err(TreeError::Escaped)), "{escaped:?}");
        let upload = tree.create_temporary(&stream, ".observer-fix-d").unwrap();
        let linked = tree.commit_over_shared(&stream, upload, "linked.ytyp", expected);
        if cfg!(windows) {
            // A junction is a folder, which a file never replaces.
            assert!(matches!(linked, Err(TreeError::IsAFolder)), "{linked:?}");
        } else {
            assert!(matches!(linked, Err(TreeError::Link)), "{linked:?}");
        }
        #[cfg(windows)]
        {
            // Windows grants file links to administrators and developer mode only.
            let file_link = stream_file.with_file_name("file-link.ytyp");
            if std::os::windows::fs::symlink_file(dir.path().join("secret.cfg"), &file_link).is_ok() {
                let upload = tree.create_temporary(&stream, ".observer-fix-d").unwrap();
                let linked = tree.commit_over_shared(&stream, upload, "file-link.ytyp", expected);
                assert!(matches!(linked, Err(TreeError::Link)), "{linked:?}");
                fs::remove_file(&file_link).unwrap();
            }

            let writable = fs::metadata(&stream_file).unwrap().permissions();
            let mut permissions = writable.clone();
            permissions.set_readonly(true);
            fs::set_permissions(&stream_file, permissions).unwrap();
            let upload = tree.create_temporary(&stream, ".observer-fix-d").unwrap();
            let read_only = tree.commit_over_shared(&stream, upload, "harbour.ytyp", expected);
            assert!(matches!(read_only, Err(TreeError::ReadOnly)), "{read_only:?}");
            fs::set_permissions(&stream_file, writable).unwrap();
        }

        assert_eq!(names(&tree, &harbour), ["fxmanifest.lua", "stream"]);
        assert_eq!(names(&tree, &stream), ["harbour.ytyp"]);
        assert_eq!(fs::read_to_string(&stream_file).unwrap(), "old");
        assert_eq!(fs::read_to_string(&cached).unwrap(), "old");
        assert!(dir.path().join("secret.cfg").exists());
    }

    #[test]
    fn a_shared_file_is_read_with_its_identity_and_replaced_only_while_it_is_still_that_file() {
        for fallbacks in [false, true] {
            let (_dir, root) = server();
            let (stream, cached) = cached_stream_file(&root);
            let tree = ConfinedTree::open(&root).unwrap();
            let tree = if fallbacks { tree.with_fallbacks() } else { tree };
            let at = path("resources/[maps]/harbour/stream/harbour.ytyp");
            let folder = tree
                .open_folder(&path("resources/[maps]/harbour/stream"))
                .unwrap();

            assert!(matches!(tree.open_file(&at), Err(TreeError::HardLink)));
            let (mut file, seen) = tree.open_file_shared(&at).unwrap();
            let mut old = String::new();
            file.read_to_string(&mut old).unwrap();
            assert_eq!(old, "old");
            // Without POSIX semantics an open handle blocks the replace: the read is over first.
            drop(file);
            let (_, twin) = tree
                .open_file_shared(&path("cache/files/harbour/stream_cache/harbour.ytyp"))
                .unwrap();
            assert_eq!(twin, seen, "two links of one file are one file");

            let mut upload = tree.create_temporary(&folder, ".observer-fix-e").unwrap();
            upload.file().write_all(b"new").unwrap();
            tree.commit_over_shared(&folder, upload, "harbour.ytyp", Some(seen))
                .unwrap();

            assert_eq!(fs::read_to_string(&stream).unwrap(), "new");
            assert_eq!(
                fs::read_to_string(&cached).unwrap(),
                "old",
                "fallbacks: {fallbacks}"
            );
            let (_, written) = tree.open_file_shared(&at).unwrap();
            assert_ne!(written, seen, "the file written anew is another file");

            // A writer still going by the first read is refused: the name leads elsewhere now.
            let mut upload = tree.create_temporary(&folder, ".observer-fix-e").unwrap();
            upload.file().write_all(b"stale").unwrap();
            let stale = tree.commit_over_shared(&folder, upload, "harbour.ytyp", Some(seen));
            assert!(matches!(stale, Err(TreeError::Changed)), "{stale:?}");
            assert_eq!(fs::read_to_string(&stream).unwrap(), "new");
            assert_eq!(names(&tree, &folder), ["harbour.ytyp"]);
        }
    }

    #[test]
    fn a_file_swapped_removed_or_created_since_the_read_is_never_overwritten() {
        let (dir, root) = server();
        let (stream, cached) = cached_stream_file(&root);
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree
            .open_folder(&path("resources/[maps]/harbour/stream"))
            .unwrap();
        let (file, seen) = tree
            .open_file_shared(&path("resources/[maps]/harbour/stream/harbour.ytyp"))
            .unwrap();
        drop(file);
        let commit = |expected| {
            let mut upload = tree.create_temporary(&folder, ".observer-fix-f").unwrap();
            upload.file().write_all(b"ours").unwrap();
            tree.commit_over_shared(&folder, upload, "harbour.ytyp", expected)
        };

        // Another writer commits its own file under the name, by a rename as an upload does.
        fs::write(dir.path().join("theirs"), "theirs").unwrap();
        fs::rename(dir.path().join("theirs"), &stream).unwrap();
        let swapped = commit(Some(seen));
        assert!(matches!(swapped, Err(TreeError::Changed)), "{swapped:?}");
        assert_eq!(fs::read_to_string(&stream).unwrap(), "theirs");

        fs::remove_file(&stream).unwrap();
        let removed = commit(Some(seen));
        assert!(matches!(removed, Err(TreeError::Changed)), "{removed:?}");
        assert!(!stream.exists(), "nothing is created where a file was expected");

        fs::write(&stream, "theirs too").unwrap();
        let created = commit(None);
        assert!(matches!(created, Err(TreeError::Changed)), "{created:?}");
        assert_eq!(fs::read_to_string(&stream).unwrap(), "theirs too");

        assert_eq!(names(&tree, &folder), ["harbour.ytyp"]);
        assert_eq!(fs::read_to_string(&cached).unwrap(), "old");
    }

    #[test]
    fn a_shared_read_still_refuses_links_folders_and_missing_files() {
        let (dir, root) = server();
        link_folder(&root.join("resources").join("[maps]").join("outside"), dir.path());
        let tree = ConfinedTree::open(&root).unwrap();

        let through = tree.open_file_shared(&path("resources/[maps]/outside/secret.cfg"));
        assert!(matches!(through, Err(TreeError::Link)), "{through:?}");
        let linked = tree.open_file_shared(&path("resources/[maps]/outside"));
        // A junction is a folder to Windows; a symbolic link is a link to Unix.
        assert!(
            matches!(linked, Err(TreeError::Link | TreeError::IsAFolder)),
            "{linked:?}"
        );
        let folder = tree.open_file_shared(&path("resources"));
        assert!(matches!(folder, Err(TreeError::IsAFolder)), "{folder:?}");
        assert!(matches!(
            tree.open_file_shared(&RelativePath::root()),
            Err(TreeError::IsAFolder)
        ));
        let missing = tree.open_file_shared(&path("resources/missing.ytyp"));
        assert!(matches!(missing, Err(TreeError::NotFound)), "{missing:?}");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("secret.cfg"), root.join("secret.cfg")).unwrap();
            let link = tree.open_file_shared(&path("secret.cfg"));
            assert!(matches!(link, Err(TreeError::Link)), "{link:?}");
            let folder = fs::File::open(&root).unwrap();
            rustix::fs::mknodat(
                &folder,
                "console",
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_raw_mode(0o600),
                0,
            )
            .unwrap();
            let pipe = tree.open_file_shared(&path("console"));
            assert!(matches!(pipe, Err(TreeError::NotRegular)), "{pipe:?}");
        }
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(dir.path().join("secret.cfg"), root.join("secret.cfg")).is_ok()
        {
            let link = tree.open_file_shared(&path("secret.cfg"));
            assert!(matches!(link, Err(TreeError::Link)), "{link:?}");
        }
    }

    #[test]
    fn the_root_and_each_file_have_an_identity_of_their_own() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let again = ConfinedTree::open(&root).unwrap();

        let folder = tree.root_id().unwrap();
        assert_eq!(again.root_id().unwrap(), folder, "one folder opened twice");
        let (_, manifest) = tree
            .open_file_shared(&path("resources/[maps]/harbour/fxmanifest.lua"))
            .unwrap();
        assert_ne!(manifest, folder);
        assert_eq!(manifest.volume(), folder.volume());
        let shown = manifest.to_string();
        assert_eq!(shown.len(), 16 + 1 + 32, "{shown}");
        assert!(shown.ends_with(&format!("{:032x}", manifest.index())), "{shown}");
    }

    #[test]
    fn a_commit_never_replaces_unless_asked() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();

        let mut upload = tree.create_temporary(&folder, ".observer-upload-a").unwrap();
        upload.file().write_all(b"fx_version 'bodacious'").unwrap();
        assert!(matches!(
            tree.commit(&folder, upload, "fxmanifest.lua", false),
            Err(TreeError::Exists)
        ));
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/fxmanifest.lua").unwrap(),
            "fx_version 'cerulean'"
        );
        // A refused commit removes its temporary file.
        let names: Vec<_> = tree
            .list(&folder)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["fxmanifest.lua"]);

        let mut upload = tree.create_temporary(&folder, ".observer-upload-a").unwrap();
        upload.file().write_all(b"fx_version 'bodacious'").unwrap();
        tree.commit(&folder, upload, "fxmanifest.lua", true).unwrap();
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/fxmanifest.lua").unwrap(),
            "fx_version 'bodacious'"
        );
        let names: Vec<_> = tree
            .list(&folder)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["fxmanifest.lua"]);

        let mut fresh = tree.create_temporary(&folder, ".observer-upload-b").unwrap();
        fresh.file().write_all(b"Citizen.Wait(0)").unwrap();
        tree.commit(&folder, fresh, "client.lua", false).unwrap();
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/client.lua").unwrap(),
            "Citizen.Wait(0)"
        );
        assert!(matches!(
            tree.create_temporary(&folder, "client.lua"),
            Err(TreeError::Exists)
        ));
    }

    #[test]
    fn a_name_that_is_not_one_component_is_refused_everywhere() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree.open_folder(&path("resources")).unwrap();

        for name in ["", ".", "..", "../secret.cfg", "a/b", "a\\b", "server.cfg:stream"] {
            assert!(
                matches!(tree.create_temporary(&folder, name), Err(TreeError::Escaped)),
                "{name:?}"
            );
            assert!(
                matches!(tree.make_folder(&folder, name), Err(TreeError::Escaped)),
                "{name:?}"
            );
            assert!(
                matches!(tree.remove_file(&folder, name), Err(TreeError::Escaped)),
                "{name:?}"
            );
            assert!(
                matches!(tree.remove_folder(&folder, name), Err(TreeError::Escaped)),
                "{name:?}"
            );
            assert!(
                matches!(
                    tree.rename(&folder, "[maps]", &folder, name, false),
                    Err(TreeError::Escaped)
                ),
                "{name:?}"
            );
            let upload = tree.create_temporary(&folder, ".observer-upload-i").unwrap();
            assert!(
                matches!(tree.commit(&folder, upload, name, false), Err(TreeError::Escaped)),
                "{name:?}"
            );
        }
        assert!(
            tree.entry(&path("resources/[maps]/harbour/fxmanifest.lua"))
                .is_ok()
        );
    }

    #[test]
    fn a_one_character_name_is_a_valid_rename_target() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();

        tree.rename(&folder, "fxmanifest.lua", &folder, "f", false)
            .unwrap();
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/f").unwrap(),
            "fx_version 'cerulean'"
        );
    }

    #[test]
    fn a_discarded_upload_leaves_the_target_and_no_temporary_file() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();
        let mut upload = tree.create_temporary(&folder, ".observer-upload-c").unwrap();
        upload.file().write_all(b"half").unwrap();

        tree.discard(&folder, upload).unwrap();

        let names: Vec<_> = tree
            .list(&folder)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["fxmanifest.lua"]);
    }

    #[test]
    fn a_replace_goes_through_while_a_download_holds_the_file() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();
        let mut download = tree
            .open_file(&path("resources/[maps]/harbour/fxmanifest.lua"))
            .unwrap();

        let mut upload = tree.create_temporary(&folder, ".observer-upload-d").unwrap();
        upload.file().write_all(b"new").unwrap();
        tree.commit(&folder, upload, "fxmanifest.lua", true).unwrap();

        let mut old = String::new();
        download.read_to_string(&mut old).unwrap();
        assert_eq!(old, "fx_version 'cerulean'");
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/fxmanifest.lua").unwrap(),
            "new"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_file_held_without_shared_deletion_answers_in_use_and_a_read_only_one_read_only() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let (_dir, root) = server();
        let manifest = root.join("resources/[maps]/harbour/fxmanifest.lua");
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();
        // FXServer opens its files this way.
        let held = fs::File::options()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&manifest)
            .unwrap();
        let mut upload = tree.create_temporary(&folder, ".observer-upload-e").unwrap();
        upload.file().write_all(b"new").unwrap();

        assert!(matches!(
            tree.commit(&folder, upload, "fxmanifest.lua", true),
            Err(TreeError::InUse)
        ));
        drop(held);
        let mut permissions = fs::metadata(&manifest).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&manifest, permissions).unwrap();
        let mut upload = tree.create_temporary(&folder, ".observer-upload-e").unwrap();
        upload.file().write_all(b"new").unwrap();
        assert!(matches!(
            tree.commit(&folder, upload, "fxmanifest.lua", true),
            Err(TreeError::ReadOnly)
        ));
        assert!(matches!(
            tree.remove_file(&folder, "fxmanifest.lua"),
            Err(TreeError::ReadOnly)
        ));
        let names: Vec<_> = tree
            .list(&folder)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["fxmanifest.lua"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_short_name_never_renames_removes_or_replaces_the_long_one() {
        let (_dir, root) = server();
        fs::create_dir(root.join(".gitignored-repository")).unwrap();
        fs::write(root.join("observer-settings.toml"), "listen").unwrap();
        let tree = ConfinedTree::open(&root).unwrap();
        let top = tree.root();

        // Whether the volume makes short names or not, nothing happens to the long-named entries.
        assert!(tree.rename(top, "GITIGN~1", top, "public", false).is_err());
        assert!(tree.remove_file(top, "OBSERV~1.TOM").is_err());
        let mut upload = tree.create_temporary(top, ".observer-upload-h").unwrap();
        upload.file().write_all(b"x").unwrap();
        assert!(tree.commit(top, upload, "OBSERV~1.TOM", true).is_err());
        assert_eq!(read(&tree, "observer-settings.toml").unwrap(), "listen");
        assert!(tree.entry(&path(".gitignored-repository")).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn a_short_name_never_reaches_the_file_but_another_case_does() {
        let (_dir, root) = server();
        fs::write(root.join("server-settings.cfg"), "sv_hostname x").unwrap();
        let tree = ConfinedTree::open(&root).unwrap();

        // Refused whether or not the volume makes 8.3 names: an alias fails the final path check,
        // and without aliases the name does not exist.
        assert!(tree.open_file(&path("SERVER~1.CFG")).is_err());
        assert_eq!(read(&tree, "SERVER-SETTINGS.CFG").unwrap(), "sv_hostname x");
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_is_neither_listed_nor_opened() {
        let (_dir, root) = server();
        let folder = fs::File::open(&root).unwrap();
        rustix::fs::mknodat(
            &folder,
            "console",
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        let tree = ConfinedTree::open(&root).unwrap();

        assert!(matches!(
            tree.open_file(&path("console")),
            Err(TreeError::NotRegular)
        ));
        let names: Vec<_> = tree
            .list(tree.root())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["resources"]);
    }

    #[test]
    fn folders_are_made_moved_and_removed_only_when_empty() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let resources = tree.open_folder(&path("resources")).unwrap();

        tree.make_folder(&resources, "[cars]").unwrap();
        assert!(matches!(
            tree.make_folder(&resources, "[cars]"),
            Err(TreeError::Exists)
        ));
        let maps = tree.open_folder(&path("resources/[maps]")).unwrap();
        let cars = tree.open_folder(&path("resources/[cars]")).unwrap();
        tree.rename(&maps, "harbour", &cars, "harbour", false).unwrap();
        assert_eq!(
            read(&tree, "resources/[cars]/harbour/fxmanifest.lua").unwrap(),
            "fx_version 'cerulean'"
        );
        tree.make_folder(&maps, "harbour").unwrap();
        assert!(matches!(
            tree.rename(&maps, "harbour", &cars, "harbour", false),
            Err(TreeError::Exists)
        ));

        assert!(matches!(
            tree.remove_folder(&cars, "harbour"),
            Err(TreeError::NotEmpty)
        ));
        tree.remove_folder(&maps, "harbour").unwrap();
        assert!(matches!(
            tree.entry(&path("resources/[maps]/harbour")),
            Err(TreeError::NotFound)
        ));
    }

    #[test]
    fn a_folder_renamed_under_an_open_handle_shows_its_new_path() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let harbour = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();
        let resources = tree.open_folder(&path("resources")).unwrap();

        let renamed = tree.rename(&resources, "[maps]", &resources, "[world]", false);

        if cfg!(windows) {
            // Windows refuses to rename a folder while a handle below it is open.
            assert!(matches!(renamed, Err(TreeError::InUse)), "{renamed:?}");
            assert_eq!(
                tree.current_path(&harbour).unwrap(),
                path("resources/[maps]/harbour")
            );
        } else {
            renamed.unwrap();
            assert_eq!(
                tree.current_path(&harbour).unwrap(),
                path("resources/[world]/harbour")
            );
        }
    }
}
