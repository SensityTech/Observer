use std::fs::{self, File};
use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, ResolveFlags, Stat, StatxAttributes, StatxFlags,
};
use rustix::io::Errno;

use crate::confined::{Entry, EntryKind, RelativePath, TreeError};

/// Whether `openat2` with its resolution flags is available. The kernel and a container's
/// seccomp profile are the same for the whole process, so the first tree opened decides.
static OPENAT2: OnceLock<bool> = OnceLock::new();

const RESOLVE: ResolveFlags = ResolveFlags::BENEATH
    .union(ResolveFlags::NO_SYMLINKS)
    .union(ResolveFlags::NO_MAGICLINKS)
    .union(ResolveFlags::NO_XDEV);

/// The files and folders below one root, reached only through descriptors opened one component
/// at a time, never through a link or across a mount point.
#[derive(Debug)]
pub struct ConfinedTree {
    root: Folder,
    final_path: String,
    device: u64,
    openat2: bool,
}

/// A folder of the tree, held open.
#[derive(Debug)]
pub struct Folder {
    handle: OwnedFd,
    path: RelativePath,
}

impl Folder {
    pub fn path(&self) -> &RelativePath {
        &self.path
    }
}

/// A file created new for an upload, committed under its real name by one rename.
#[derive(Debug)]
pub struct TemporaryFile {
    file: File,
    folder: RelativePath,
    name: String,
}

impl TemporaryFile {
    pub fn file(&mut self) -> &mut File {
        &mut self.file
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl ConfinedTree {
    /// Opens the tree's root. The root itself may be reached through a link: it is the operator's
    /// choice, and everything below is compared with its final path.
    pub fn open(root: &Path) -> io::Result<Self> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let handle = rustix::fs::open(root, flags, Mode::empty())?;
        if OPENAT2.get().is_none() {
            let available = match rustix::fs::openat2(&handle, ".", flags, Mode::empty(), RESOLVE) {
                Ok(_) => true,
                // No such call, or a seccomp profile that answers EPERM to calls it does not list.
                Err(Errno::NOSYS | Errno::PERM) => false,
                Err(error) => return Err(error.into()),
            };
            let _ = OPENAT2.set(available);
        }
        let device = rustix::fs::fstat(&handle)?.st_dev;
        let final_path = descriptor_path(&handle)?;
        Ok(Self {
            root: Folder {
                handle,
                path: RelativePath::root(),
            },
            final_path,
            device,
            openat2: OPENAT2.get().copied().unwrap_or(false),
        })
    }

    /// The same tree, opening components with the walk that kernels without `openat2` get.
    #[cfg(test)]
    pub(crate) fn with_fallbacks(mut self) -> Self {
        self.openat2 = false;
        self
    }

    /// Whether components are opened with `openat2`, or by the slower walk with `openat`.
    pub fn uses_openat2() -> Option<bool> {
        OPENAT2.get().copied()
    }

    pub fn final_path(&self) -> &str {
        &self.final_path
    }

    pub fn root(&self) -> &Folder {
        &self.root
    }

    pub fn open_folder(&self, path: &RelativePath) -> Result<Folder, TreeError> {
        let mut handle = self.root.handle.try_clone().map_err(TreeError::Io)?;
        for name in path.components() {
            handle = self.open_in(&handle, name, OFlags::RDONLY | OFlags::DIRECTORY)?;
        }
        self.check_final(&handle, path)?;
        Ok(Folder {
            handle,
            path: path.clone(),
        })
    }

    /// Opens a regular file to read.
    pub fn open_file(&self, path: &RelativePath) -> Result<File, TreeError> {
        let Some((parent, name)) = path.split_last() else {
            return Err(TreeError::IsAFolder);
        };
        let parent = self.open_folder(&parent)?;
        // Non-blocking, so that a pipe in the file's place never makes the open wait.
        let handle = self.open_in(
            &parent.handle,
            name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY,
        )?;
        let stat = rustix::fs::fstat(&handle).map_err(errno)?;
        refuse_unless_regular(&stat)?;
        self.check_final(&handle, path)?;
        Ok(File::from(handle))
    }

    /// What the entry at `path` is, without following a link in its place.
    pub fn entry(&self, path: &RelativePath) -> Result<Entry, TreeError> {
        let Some((parent, name)) = path.split_last() else {
            let stat = rustix::fs::fstat(&self.root.handle).map_err(errno)?;
            return Ok(entry_of(String::new(), &stat, EntryKind::Folder));
        };
        let parent = self.open_folder(&parent)?;
        let stat = rustix::fs::statat(&parent.handle, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
        let kind = self.kind(&parent, name, &stat)?;
        if kind == EntryKind::File && stat.st_nlink > 1 {
            return Err(TreeError::HardLink);
        }
        Ok(entry_of(name.to_owned(), &stat, kind))
    }

    /// The folder's entries; links, mount points, pipes, sockets and devices left out.
    pub fn list(&self, folder: &Folder) -> Result<Vec<Entry>, TreeError> {
        let mut entries = Vec::new();
        for item in Dir::read_from(&folder.handle).map_err(errno)? {
            let item = item.map_err(errno)?;
            let Ok(name) = item.file_name().to_str() else {
                continue;
            };
            if name == "." || name == ".." {
                continue;
            }
            let Ok(stat) = rustix::fs::statat(&folder.handle, name, AtFlags::SYMLINK_NOFOLLOW) else {
                continue;
            };
            if let Ok(kind) = self.kind(folder, name, &stat) {
                entries.push(entry_of(name.to_owned(), &stat, kind));
            }
        }
        Ok(entries)
    }

    /// Creates a new file under `name` in the folder, to be committed under another name.
    pub fn create_temporary(&self, folder: &Folder, name: &str) -> Result<TemporaryFile, TreeError> {
        self.in_place(folder, name)?;
        let flags = OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let handle =
            rustix::fs::openat(&folder.handle, name, flags, Mode::from_raw_mode(0o666)).map_err(errno)?;
        Ok(TemporaryFile {
            file: File::from(handle),
            folder: folder.path.clone(),
            name: name.to_owned(),
        })
    }

    /// Makes the temporary file the folder's `target`, by one rename. Without `replace`, an entry
    /// already named `target` refuses the commit (`Exists`). On any refusal the temporary file is
    /// removed: the upload is over either way.
    pub fn commit(
        &self,
        folder: &Folder,
        temporary: TemporaryFile,
        target: &str,
        replace: bool,
    ) -> Result<(), TreeError> {
        if temporary.folder != folder.path || !Self::still_named(folder, &temporary) {
            return Err(TreeError::Escaped);
        }
        let committed = self.commit_inner(folder, &temporary, target, replace);
        if committed.is_err() && Self::still_named(folder, &temporary) {
            let _ = rustix::fs::unlinkat(&folder.handle, &*temporary.name, AtFlags::empty());
        }
        drop(temporary);
        committed
    }

    fn commit_inner(
        &self,
        folder: &Folder,
        temporary: &TemporaryFile,
        target: &str,
        replace: bool,
    ) -> Result<(), TreeError> {
        self.in_place(folder, target)?;
        temporary.file.sync_all().map_err(TreeError::Io)?;
        if replace {
            match rustix::fs::statat(&folder.handle, target, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => {
                    refuse_unless_regular(&stat)?;
                    let permissions = Mode::from_raw_mode(stat.st_mode & 0o777);
                    rustix::fs::fchmod(&temporary.file, permissions).map_err(errno)?;
                }
                Err(Errno::NOENT) => {}
                Err(error) => return Err(errno(error)),
            }
            rustix::fs::renameat(&folder.handle, &*temporary.name, &folder.handle, target).map_err(errno)?;
        } else {
            Self::rename_new(folder, &temporary.name, folder, target, true)?;
        }
        rustix::fs::fsync(&folder.handle).map_err(errno)
    }

    /// Whether the temporary file's name in the folder still leads to the file it was created as.
    fn still_named(folder: &Folder, temporary: &TemporaryFile) -> bool {
        let (Ok(named), Ok(held)) = (
            rustix::fs::statat(&folder.handle, &*temporary.name, AtFlags::SYMLINK_NOFOLLOW),
            rustix::fs::fstat(&temporary.file),
        ) else {
            return false;
        };
        (named.st_dev, named.st_ino) == (held.st_dev, held.st_ino)
    }

    /// Removes a temporary file that will not be committed.
    pub fn discard(&self, folder: &Folder, temporary: TemporaryFile) -> Result<(), TreeError> {
        if temporary.folder != folder.path || !Self::still_named(folder, &temporary) {
            return Err(TreeError::Escaped);
        }
        drop(temporary.file);
        rustix::fs::unlinkat(&folder.handle, &*temporary.name, AtFlags::empty()).map_err(errno)
    }

    /// Renames or moves `name` from one folder to another. A folder moves with everything in it.
    pub fn rename(
        &self,
        from: &Folder,
        name: &str,
        to: &Folder,
        new_name: &str,
        replace: bool,
    ) -> Result<(), TreeError> {
        self.in_place(from, name)?;
        self.in_place(to, new_name)?;
        let stat = rustix::fs::statat(&from.handle, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
        let kind = self.kind(from, name, &stat)?;
        if kind == EntryKind::File && stat.st_nlink > 1 {
            return Err(TreeError::HardLink);
        }
        if replace {
            match rustix::fs::statat(&to.handle, new_name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(target) => refuse_unless_regular(&target)?,
                Err(Errno::NOENT) => {}
                Err(error) => return Err(errno(error)),
            }
            rustix::fs::renameat(&from.handle, name, &to.handle, new_name).map_err(errno)?;
        } else {
            Self::rename_new(from, name, to, new_name, kind == EntryKind::File)?;
        }
        rustix::fs::fsync(&to.handle).map_err(errno)?;
        rustix::fs::fsync(&from.handle).map_err(errno)
    }

    pub fn remove_file(&self, folder: &Folder, name: &str) -> Result<(), TreeError> {
        self.in_place(folder, name)?;
        let stat = rustix::fs::statat(&folder.handle, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
        refuse_unless_regular(&stat)?;
        rustix::fs::unlinkat(&folder.handle, name, AtFlags::empty()).map_err(errno)
    }

    /// Removes an empty folder.
    pub fn remove_folder(&self, folder: &Folder, name: &str) -> Result<(), TreeError> {
        self.in_place(folder, name)?;
        let stat = rustix::fs::statat(&folder.handle, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
        if self.kind(folder, name, &stat)? != EntryKind::Folder {
            return Err(TreeError::NotAFolder);
        }
        rustix::fs::unlinkat(&folder.handle, name, AtFlags::REMOVEDIR).map_err(errno)
    }

    pub fn make_folder(&self, folder: &Folder, name: &str) -> Result<(), TreeError> {
        self.in_place(folder, name)?;
        rustix::fs::mkdirat(&folder.handle, name, Mode::from_raw_mode(0o777)).map_err(errno)
    }

    /// Checks that `name` is one component and that the folder is still where it was opened: a
    /// held descriptor follows its folder wherever a host process moves it, outside the root too.
    fn in_place(&self, folder: &Folder, name: &str) -> Result<(), TreeError> {
        folder.path.join(name).map_err(|_| TreeError::Escaped)?;
        self.check_final(&folder.handle, &folder.path)
    }

    /// Where the folder is now, read back from its descriptor: a folder renamed since it was
    /// opened shows its new path. `Escaped` when it left the root or was removed.
    pub fn current_path(&self, folder: &Folder) -> Result<RelativePath, TreeError> {
        let path = descriptor_path(&folder.handle).map_err(TreeError::Io)?;
        let rest = strip_root(&path, &self.final_path).ok_or(TreeError::Escaped)?;
        RelativePath::new(rest.split('/').filter(|part| !part.is_empty())).map_err(|_| TreeError::Escaped)
    }

    /// Opens one component below `parent`, never through a link or across a mount point.
    fn open_in(&self, parent: &OwnedFd, name: &str, flags: OFlags) -> Result<OwnedFd, TreeError> {
        let flags = flags | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let opened = if self.openat2 {
            rustix::fs::openat2(parent, name, flags, Mode::empty(), RESOLVE)
        } else {
            rustix::fs::openat(parent, name, flags, Mode::empty())
        };
        let handle = match opened {
            Ok(handle) => handle,
            // A link opened as a folder without following it reads as "not a folder".
            Err(Errno::NOTDIR) if is_link(parent, name) => return Err(TreeError::Link),
            Err(error) => return Err(errno(error)),
        };
        if self.openat2 {
            return Ok(handle);
        }
        let stat = rustix::fs::fstat(&handle).map_err(errno)?;
        if stat.st_dev != self.device || mount_root(parent, name) {
            return Err(TreeError::Link);
        }
        Ok(handle)
    }

    /// What `name` is, as a listing or an operation sees it; links and mount points are `Link`.
    fn kind(&self, folder: &Folder, name: &str, stat: &Stat) -> Result<EntryKind, TreeError> {
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Symlink => Err(TreeError::Link),
            FileType::RegularFile => Ok(EntryKind::File),
            FileType::Directory if stat.st_dev != self.device || mount_root(&folder.handle, name) => {
                Err(TreeError::Link)
            }
            FileType::Directory => Ok(EntryKind::Folder),
            _ => Err(TreeError::NotRegular),
        }
    }

    /// A rename that never replaces. Where the file system refuses `RENAME_NOREPLACE`, a file is
    /// linked under its new name, which fails on an existing one, then unlinked from the old; a
    /// folder, which cannot be linked, is renamed after checking that the name is free, which the
    /// caller's lock keeps true against other sessions.
    fn rename_new(
        from: &Folder,
        name: &str,
        to: &Folder,
        new_name: &str,
        file: bool,
    ) -> Result<(), TreeError> {
        match rustix::fs::renameat_with(&from.handle, name, &to.handle, new_name, RenameFlags::NOREPLACE) {
            Err(Errno::INVAL | Errno::NOSYS) => {}
            result => return result.map_err(errno),
        }
        if file {
            match rustix::fs::linkat(&from.handle, name, &to.handle, new_name, AtFlags::empty()) {
                // Linked under its new name, the file is there; an old name left behind is a
                // second link the start's sweep removes.
                Ok(()) => {
                    let _ = rustix::fs::unlinkat(&from.handle, name, AtFlags::empty());
                    return Ok(());
                }
                Err(Errno::PERM | Errno::OPNOTSUPP) => {}
                Err(error) => return Err(errno(error)),
            }
        }
        match rustix::fs::statat(&to.handle, new_name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Err(TreeError::Exists),
            Err(Errno::NOENT) => {
                rustix::fs::renameat(&from.handle, name, &to.handle, new_name).map_err(errno)
            }
            Err(error) => Err(errno(error)),
        }
    }

    /// The authoritative check: the opened entry's final path is the root's, followed by exactly
    /// the components asked for.
    fn check_final(&self, handle: &OwnedFd, expected: &RelativePath) -> Result<(), TreeError> {
        let path = descriptor_path(handle).map_err(TreeError::Io)?;
        let rest = strip_root(&path, &self.final_path).ok_or(TreeError::Escaped)?;
        let found = rest.split('/').filter(|part| !part.is_empty());
        if found.eq(expected.components().iter().map(String::as_str)) {
            Ok(())
        } else {
            Err(TreeError::Escaped)
        }
    }
}

fn refuse_unless_regular(stat: &Stat) -> Result<(), TreeError> {
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile if stat.st_nlink > 1 => Err(TreeError::HardLink),
        FileType::RegularFile => Ok(()),
        FileType::Directory => Err(TreeError::IsAFolder),
        FileType::Symlink => Err(TreeError::Link),
        _ => Err(TreeError::NotRegular),
    }
}

/// Whether `statx` marks `name` as the root of a mount (Linux 5.8), which catches a bind mount
/// of the same file system that the device number does not.
fn mount_root(parent: impl AsFd, name: &str) -> bool {
    rustix::fs::statx(parent, name, AtFlags::SYMLINK_NOFOLLOW, StatxFlags::BASIC_STATS).is_ok_and(|statx| {
        statx.stx_attributes_mask.contains(StatxAttributes::MOUNT_ROOT)
            && statx.stx_attributes.contains(StatxAttributes::MOUNT_ROOT)
    })
}

fn is_link(parent: impl AsFd, name: &str) -> bool {
    rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Symlink)
}

fn entry_of(name: String, stat: &Stat, kind: EntryKind) -> Entry {
    let modified = u64::try_from(stat.st_mtime)
        .ok()
        .and_then(|seconds| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds)));
    Entry {
        name,
        kind,
        size: if kind == EntryKind::File {
            u64::try_from(stat.st_size).unwrap_or(0)
        } else {
            0
        },
        modified,
        read_only: stat.st_mode & 0o200 == 0,
    }
}

/// The path the descriptor's entry in `/proc` leads to: where the file or folder is now.
fn descriptor_path(handle: &OwnedFd) -> io::Result<String> {
    fs::read_link(format!("/proc/self/fd/{}", handle.as_raw_fd()))?
        .into_os_string()
        .into_string()
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

/// What follows the root in `path`, or `None` when `path` is not below the root.
fn strip_root<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if root == "/" {
        return Some(path);
    }
    let rest = path.strip_prefix(root)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

fn errno(error: Errno) -> TreeError {
    match error {
        Errno::NOENT => TreeError::NotFound,
        Errno::EXIST => TreeError::Exists,
        // A link where the walk refused to follow one, or a mount point `RESOLVE_NO_XDEV` stopped.
        Errno::LOOP | Errno::XDEV => TreeError::Link,
        Errno::NOTDIR => TreeError::NotAFolder,
        Errno::ISDIR => TreeError::IsAFolder,
        Errno::NOTEMPTY => TreeError::NotEmpty,
        Errno::BUSY | Errno::TXTBSY => TreeError::InUse,
        error => TreeError::Io(error.into()),
    }
}
