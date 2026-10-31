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
    #[error(transparent)]
    Io(io::Error),
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
            tree.commit(&folder, &upload, "fxmanifest.lua", false),
            Err(TreeError::Exists)
        ));
        tree.commit(&folder, &upload, "copy.lua", false).unwrap();
        drop(upload);
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
        let held = tree.commit(&folder, &upload, "fxmanifest.lua", true);
        if cfg!(windows) {
            // Without POSIX semantics, any open handle on the target blocks the replace.
            assert!(matches!(held, Err(TreeError::InUse)), "{held:?}");
            drop(download);
            tree.commit(&folder, &upload, "fxmanifest.lua", true).unwrap();
        } else {
            held.unwrap();
        }
        drop(upload);
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

    #[test]
    fn a_commit_never_replaces_unless_asked() {
        let (_dir, root) = server();
        let tree = ConfinedTree::open(&root).unwrap();
        let folder = tree.open_folder(&path("resources/[maps]/harbour")).unwrap();

        let mut upload = tree.create_temporary(&folder, ".observer-upload-a").unwrap();
        upload.file().write_all(b"fx_version 'bodacious'").unwrap();
        assert!(matches!(
            tree.commit(&folder, &upload, "fxmanifest.lua", false),
            Err(TreeError::Exists)
        ));
        assert_eq!(
            read(&tree, "resources/[maps]/harbour/fxmanifest.lua").unwrap(),
            "fx_version 'cerulean'"
        );

        tree.commit(&folder, &upload, "fxmanifest.lua", true).unwrap();
        drop(upload);
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
        tree.commit(&folder, &fresh, "client.lua", false).unwrap();
        drop(fresh);
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
        tree.commit(&folder, &upload, "fxmanifest.lua", true).unwrap();
        drop(upload);

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
            tree.commit(&folder, &upload, "fxmanifest.lua", true),
            Err(TreeError::InUse)
        ));
        drop(held);
        let mut permissions = fs::metadata(&manifest).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&manifest, permissions).unwrap();
        assert!(matches!(
            tree.commit(&folder, &upload, "fxmanifest.lua", true),
            Err(TreeError::ReadOnly)
        ));
        assert!(matches!(
            tree.remove_file(&folder, "fxmanifest.lua"),
            Err(TreeError::ReadOnly)
        ));
        tree.discard(&folder, upload).unwrap();
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
