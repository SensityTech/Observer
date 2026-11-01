use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_INFORMATION_CLASS, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
    FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, FileDispositionInformation,
    FileDispositionInformationEx, FileRenameInformation, FileRenameInformationEx,
    NTCREATEFILE_CREATE_DISPOSITION, NTCREATEFILE_CREATE_OPTIONS, NtCreateFile, NtSetInformationFile,
};
use windows_sys::Win32::Foundation::{
    ERROR_NO_MORE_FILES, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, STATUS_ACCESS_DENIED,
    STATUS_CANNOT_DELETE, STATUS_DELETE_PENDING, STATUS_DIRECTORY_NOT_EMPTY, STATUS_FILE_IS_A_DIRECTORY,
    STATUS_INVALID_INFO_CLASS, STATUS_INVALID_PARAMETER, STATUS_NOT_A_DIRECTORY, STATUS_NOT_SUPPORTED,
    STATUS_OBJECT_NAME_COLLISION, STATUS_OBJECT_NAME_INVALID, STATUS_OBJECT_NAME_NOT_FOUND,
    STATUS_OBJECT_PATH_NOT_FOUND, STATUS_SHARING_VIOLATION, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO,
    FILE_DISPOSITION_INFO_EX, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_INFO_BY_HANDLE_CLASS, FILE_LIST_DIRECTORY,
    FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_MODE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FILE_TRAVERSE, FileAttributeTagInfo,
    FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo, FileIdInfo, FileStandardInfo,
    GetFileInformationByHandle, GetFileInformationByHandleEx, GetFinalPathNameByHandleW, SYNCHRONIZE,
    VOLUME_NAME_NT,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
use windows_sys::Win32::System::WindowsProgramming::{
    FILE_RENAME_FLAG_POSIX_SEMANTICS, FILE_RENAME_FLAG_REPLACE_IF_EXISTS,
};

use crate::confined::{Entry, EntryKind, FileId, RelativePath, Replace, TreeError, same_name_ignoring_case};

const FOLDER_ACCESS: u32 = FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
/// Folders are held without sharing deletion, so that nobody renames one under an operation.
const FOLDER_SHARE: FILE_SHARE_MODE = FILE_SHARE_READ | FILE_SHARE_WRITE;
/// Files are opened sharing everything, so that a download never blocks another session's commit.
const FILE_SHARE: FILE_SHARE_MODE = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const IO_REPARSE_TAG_NAME_SURROGATE: u32 = 0x2000_0000;
const COMMIT_TRIES: u32 = 3;
const COMMIT_PAUSE: Duration = Duration::from_millis(50);

/// The files and folders below one root, reached only through handles opened one component at a
/// time, never through a link, and checked against their final path after each open.
#[derive(Debug)]
pub struct ConfinedTree {
    root: Folder,
    final_path: String,
    /// Whether renames and deletions may ask for POSIX semantics; cleared for good on a file
    /// system that refuses them (FAT, exFAT, older `ReFS`).
    posix: AtomicBool,
}

/// A folder of the tree, held open.
#[derive(Debug)]
pub struct Folder {
    handle: File,
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
    /// Opens the tree's root. The root itself may be reached through a link or an 8.3 name: it is
    /// the operator's choice, and everything below is compared with its final path.
    pub fn open(root: &Path) -> io::Result<Self> {
        let handle = File::options()
            .access_mode(FOLDER_ACCESS)
            .share_mode(FOLDER_SHARE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(root)?;
        if !attributes(&handle)?.is_folder() {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        }
        // A volume's root reads `\Device\HarddiskVolume3\`; its entries follow without a second `\`.
        let final_path = final_path(&handle)?.trim_end_matches('\\').to_owned();
        Ok(Self {
            root: Folder {
                handle,
                path: RelativePath::root(),
            },
            final_path,
            posix: AtomicBool::new(true),
        })
    }

    /// The same tree, renaming and deleting as on a file system without POSIX semantics.
    #[cfg(test)]
    pub(crate) fn with_fallbacks(self) -> Self {
        self.posix.store(false, Ordering::Relaxed);
        self
    }

    /// The root's final path, in the NT form that does not depend on drive letters.
    pub fn final_path(&self) -> &str {
        &self.final_path
    }

    pub fn root(&self) -> &Folder {
        &self.root
    }

    pub fn open_folder(&self, path: &RelativePath) -> Result<Folder, TreeError> {
        let Some((parent, name)) = path.split_last() else {
            let handle = self.root.handle.try_clone().map_err(TreeError::Io)?;
            return Ok(Folder {
                handle,
                path: RelativePath::root(),
            });
        };
        let parent = self.walk(&parent)?;
        let handle = open_folder_in(&parent.handle, name)?;
        self.check_final(&handle, path)?;
        Ok(Folder {
            handle,
            path: path.clone(),
        })
    }

    /// Opens a regular file to read, sharing everything.
    pub fn open_file(&self, path: &RelativePath) -> Result<File, TreeError> {
        self.open_regular(path, false)
    }

    /// Opens a regular file to read as [`open_file`](Self::open_file) does, even when other hard
    /// links share it: FXServer links every stream file into its cache. Links, junctions, mount
    /// points and reparse points stay refused, on the way and in the file's place. For Observer's
    /// own reads only, never to serve a file to someone: another link may lead to it from outside
    /// the tree. The file's identity comes with it, which
    /// [`commit_over_shared`](Self::commit_over_shared) checks before it replaces the name.
    pub fn open_file_shared(&self, path: &RelativePath) -> Result<(File, FileId), TreeError> {
        let file = self.open_regular(path, true)?;
        let id = file_id(&file)?;
        Ok((file, id))
    }

    /// The root folder's identity, which tells the same folder apart from another reached under the
    /// same path, and finds it again under another.
    pub fn root_id(&self) -> Result<FileId, TreeError> {
        file_id(&self.root.handle)
    }

    fn open_regular(&self, path: &RelativePath, shared: bool) -> Result<File, TreeError> {
        let Some((parent, name)) = path.split_last() else {
            return Err(TreeError::IsAFolder);
        };
        let parent = self.walk(&parent)?;
        let file = nt_open(
            &parent.handle,
            name,
            FILE_GENERIC_READ | SYNCHRONIZE,
            FILE_SHARE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
            0,
        )?;
        let attributes = attributes(&file).map_err(TreeError::Io)?;
        attributes.refuse_reparse()?;
        if !shared && links(&file)? > 1 {
            return Err(TreeError::HardLink);
        }
        self.check_final(&file, path)?;
        Ok(file)
    }

    /// What the entry at `path` is, without following a link in its place.
    pub fn entry(&self, path: &RelativePath) -> Result<Entry, TreeError> {
        let Some((parent, name)) = path.split_last() else {
            return entry_of(&self.root.handle, "");
        };
        let parent = self.walk(&parent)?;
        let handle = nt_open(
            &parent.handle,
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE,
            FILE_OPEN,
            0,
            0,
        )?;
        attributes(&handle).map_err(TreeError::Io)?.refuse_reparse()?;
        self.check_final(&handle, path)?;
        entry_of(&handle, name)
    }

    /// The folder's entries, links and mount points left out.
    pub fn list(&self, folder: &Folder) -> Result<Vec<Entry>, TreeError> {
        // A handle of its own: duplicated handles share one enumeration, which two listings of
        // the same folder would restart under each other.
        let listing = nt_open(
            &folder.handle,
            "",
            FOLDER_ACCESS,
            FOLDER_SHARE,
            FILE_OPEN,
            FILE_DIRECTORY_FILE,
            0,
        )?;
        let mut entries = Vec::new();
        let mut buffer = vec![0u64; 8192];
        let mut class: FILE_INFO_BY_HANDLE_CLASS = FileIdExtdDirectoryRestartInfo;
        loop {
            let size = u32::try_from(buffer.len() * size_of::<u64>())
                .map_err(|e| TreeError::Io(io::Error::other(e)))?;
            // SAFETY: the folder handle is open and synchronous, and `buffer` holds `size`
            // writable bytes, aligned for the entries the call writes there.
            let filled = unsafe {
                GetFileInformationByHandleEx(listing.as_raw_handle(), class, buffer.as_mut_ptr().cast(), size)
            };
            if filled == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error().and_then(|code| u32::try_from(code).ok()) == Some(ERROR_NO_MORE_FILES)
                {
                    break;
                }
                return Err(TreeError::Io(error));
            }
            class = FileIdExtdDirectoryInfo;
            let mut offset = 0usize;
            loop {
                // SAFETY: the call filled the buffer with a chain of entries; `offset` is 0 or a
                // `NextEntryOffset` it wrote, so it points at an entry inside the buffer.
                let entry = unsafe { buffer.as_ptr().cast::<FILE_ID_EXTD_DIR_INFO>().byte_add(offset) };
                // SAFETY: as above, the entry lies inside the buffer.
                let info = unsafe { &*entry };
                let units = info.FileNameLength as usize / size_of::<u16>();
                // SAFETY: the entry's name starts at its `FileName` field and runs `FileNameLength`
                // bytes inside the buffer; the pointer is taken from the buffer, not the field.
                let name = unsafe {
                    std::slice::from_raw_parts(
                        entry
                            .byte_add(offset_of!(FILE_ID_EXTD_DIR_INFO, FileName))
                            .cast::<u16>(),
                        units,
                    )
                };
                if let Some(entry) = listed_entry(info, name) {
                    entries.push(entry);
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                offset += info.NextEntryOffset as usize;
            }
        }
        Ok(entries)
    }

    /// Creates a new file under `name` in the folder, to be committed under another name.
    pub fn create_temporary(&self, folder: &Folder, name: &str) -> Result<TemporaryFile, TreeError> {
        child(folder, name)?;
        let file = nt_open(
            &folder.handle,
            name,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE | SYNCHRONIZE,
            FILE_SHARE,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE,
            FILE_ATTRIBUTE_NORMAL,
        )?;
        Ok(TemporaryFile {
            file,
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
        let replace = if replace { Replace::Alone } else { Replace::Never };
        self.commit_as(folder, temporary, target, replace)
    }

    /// Makes the temporary file the folder's `target` as a replacing [`commit`](Self::commit)
    /// does, even when other hard links share the file in its place: FXServer links every stream
    /// file into its cache. The rename takes the name alone, so the other links keep the old
    /// content.
    ///
    /// `expected` is the file the caller read there, from
    /// [`open_file_shared`](Self::open_file_shared), or `None` when the name was free: any other
    /// entry in its place, or none where a file was expected, refuses the commit (`Changed`), so
    /// that a file another writer committed since is never overwritten. The check comes just
    /// before the rename, which a writer racing within that instant can still beat. Every other
    /// refusal stays, and any refusal removes the temporary file.
    pub fn commit_over_shared(
        &self,
        folder: &Folder,
        temporary: TemporaryFile,
        target: &str,
        expected: Option<FileId>,
    ) -> Result<(), TreeError> {
        let replace = expected.map_or(Replace::Never, Replace::Shared);
        match self.commit_as(folder, temporary, target, replace) {
            Err(TreeError::Exists) => Err(TreeError::Changed),
            committed => committed,
        }
    }

    fn commit_as(
        &self,
        folder: &Folder,
        temporary: TemporaryFile,
        target: &str,
        replace: Replace,
    ) -> Result<(), TreeError> {
        let committed = self.commit_inner(folder, &temporary, target, replace);
        if committed.is_err() {
            let _ = self.delete_handle(&temporary.file);
        }
        drop(temporary);
        committed
    }

    fn commit_inner(
        &self,
        folder: &Folder,
        temporary: &TemporaryFile,
        target: &str,
        replace: Replace,
    ) -> Result<(), TreeError> {
        if temporary.folder != folder.path {
            return Err(TreeError::Escaped);
        }
        let path = child(folder, target)?;
        temporary.file.sync_all().map_err(TreeError::Io)?;
        if replace != Replace::Never {
            self.check_replaced(folder, target, &path, replace)?;
        }
        retried(|| self.rename_handle(&temporary.file, &folder.handle, target, replace != Replace::Never))
    }

    /// Removes a temporary file that will not be committed.
    pub fn discard(&self, _folder: &Folder, temporary: TemporaryFile) -> Result<(), TreeError> {
        let deleted = self.delete_handle(&temporary.file);
        drop(temporary);
        deleted
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
        let source_path = child(from, name)?;
        let target_path = child(to, new_name)?;
        let source = nt_open(
            &from.handle,
            name,
            DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE,
            FILE_OPEN,
            0,
            0,
        )?;
        let attributes = attributes(&source).map_err(TreeError::Io)?;
        attributes.refuse_reparse()?;
        if !attributes.is_folder() && links(&source)? > 1 {
            return Err(TreeError::HardLink);
        }
        self.check_final(&source, &source_path)?;
        if replace {
            self.check_replaced(to, new_name, &target_path, Replace::Alone)?;
        }
        retried(|| self.rename_handle(&source, &to.handle, new_name, replace))
    }

    pub fn remove_file(&self, folder: &Folder, name: &str) -> Result<(), TreeError> {
        self.remove(folder, name, FILE_NON_DIRECTORY_FILE)
    }

    /// Removes an empty folder.
    pub fn remove_folder(&self, folder: &Folder, name: &str) -> Result<(), TreeError> {
        self.remove(folder, name, FILE_DIRECTORY_FILE)
    }

    pub fn make_folder(&self, folder: &Folder, name: &str) -> Result<(), TreeError> {
        child(folder, name)?;
        nt_open(
            &folder.handle,
            name,
            FOLDER_ACCESS,
            FOLDER_SHARE,
            FILE_CREATE,
            FILE_DIRECTORY_FILE,
            FILE_ATTRIBUTE_NORMAL,
        )
        .map(drop)
    }

    /// Where the folder is now, read back from its handle: a folder renamed since it was opened
    /// shows its new path. `Escaped` when it left the root.
    pub fn current_path(&self, folder: &Folder) -> Result<RelativePath, TreeError> {
        let path = final_path(&folder.handle).map_err(TreeError::Io)?;
        let rest = strip_root(&path, &self.final_path).ok_or(TreeError::Escaped)?;
        RelativePath::new(rest.split('\\').filter(|part| !part.is_empty())).map_err(|_| TreeError::Escaped)
    }

    /// Opens every folder on the way to `path`, refusing links, then checks where it landed.
    fn walk(&self, path: &RelativePath) -> Result<Folder, TreeError> {
        if path.is_root() {
            return self.open_folder(path);
        }
        let mut current: Option<File> = None;
        for name in path.components() {
            let parent = current.as_ref().unwrap_or(&self.root.handle);
            current = Some(open_folder_in(parent, name)?);
        }
        let handle = current.ok_or(TreeError::NotFound)?;
        self.check_final(&handle, path)?;
        Ok(Folder {
            handle,
            path: path.clone(),
        })
    }

    /// The authoritative check: the opened entry's final path is the root's, followed by exactly
    /// the components asked for, case aside. An 8.3 alias or a name reached another way fails it.
    fn check_final(&self, handle: &File, expected: &RelativePath) -> Result<(), TreeError> {
        let path = final_path(handle).map_err(TreeError::Io)?;
        let rest = strip_root(&path, &self.final_path).ok_or(TreeError::Escaped)?;
        let mut found = rest.split('\\').filter(|part| !part.is_empty());
        for wanted in expected.components() {
            match found.next() {
                Some(part) if same_name_ignoring_case(part, wanted) => {}
                _ => return Err(TreeError::Escaped),
            }
        }
        if found.next().is_some() {
            return Err(TreeError::Escaped);
        }
        Ok(())
    }

    fn rename_handle(
        &self,
        source: &File,
        folder: &File,
        name: &str,
        replace: bool,
    ) -> Result<(), TreeError> {
        if self.posix.load(Ordering::Relaxed) {
            let mut flags = FILE_RENAME_FLAG_POSIX_SEMANTICS;
            if replace {
                flags |= FILE_RENAME_FLAG_REPLACE_IF_EXISTS;
            }
            match set_rename(source, folder, name, FileRenameInformationEx, flags) {
                Err(status) if unsupported(status) => self.posix.store(false, Ordering::Relaxed),
                result => return result.map_err(changing),
            }
        }
        set_rename(source, folder, name, FileRenameInformation, u32::from(replace)).map_err(changing)
    }

    fn delete_handle(&self, handle: &File) -> Result<(), TreeError> {
        if self.posix.load(Ordering::Relaxed) {
            let info = FILE_DISPOSITION_INFO_EX {
                Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
            };
            match set_information(handle, FileDispositionInformationEx, &info) {
                Err(status) if unsupported(status) => self.posix.store(false, Ordering::Relaxed),
                result => return result.map_err(changing),
            }
        }
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        set_information(handle, FileDispositionInformation, &info).map_err(changing)
    }

    fn remove(
        &self,
        folder: &Folder,
        name: &str,
        kind: NTCREATEFILE_CREATE_OPTIONS,
    ) -> Result<(), TreeError> {
        let path = child(folder, name)?;
        let handle = nt_open(
            &folder.handle,
            name,
            DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE,
            FILE_OPEN,
            kind,
            0,
        )?;
        let attributes = attributes(&handle).map_err(TreeError::Io)?;
        attributes.refuse_reparse()?;
        if attributes.file_attributes & FILE_ATTRIBUTE_READONLY != 0 {
            return Err(TreeError::ReadOnly);
        }
        if !attributes.is_folder() && links(&handle)? > 1 {
            return Err(TreeError::HardLink);
        }
        self.check_final(&handle, &path)?;
        self.delete_handle(&handle)
    }

    /// Checks the file a replacing rename would take the place of: reached under its own name, a
    /// regular file, not read-only, and either alone with its one link or, shared, the very file
    /// expected. No file is needed there to replace a file alone; a shared one must be there.
    fn check_replaced(
        &self,
        folder: &Folder,
        name: &str,
        path: &RelativePath,
        replace: Replace,
    ) -> Result<(), TreeError> {
        let existing = match nt_open(
            &folder.handle,
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
            0,
        ) {
            Ok(existing) => existing,
            Err(TreeError::NotFound) if matches!(replace, Replace::Shared(_)) => {
                return Err(TreeError::Changed);
            }
            Err(TreeError::NotFound) => return Ok(()),
            Err(error) => return Err(error),
        };
        let attributes = attributes(&existing).map_err(TreeError::Io)?;
        attributes.refuse_reparse()?;
        if replace == Replace::Alone && links(&existing)? > 1 {
            return Err(TreeError::HardLink);
        }
        self.check_final(&existing, path)?;
        if let Replace::Shared(expected) = replace
            && file_id(&existing)? != expected
        {
            return Err(TreeError::Changed);
        }
        if attributes.file_attributes & FILE_ATTRIBUTE_READONLY != 0 {
            return Err(TreeError::ReadOnly);
        }
        Ok(())
    }
}

/// The path of `name` in the folder, refusing a name that is not one component.
fn child(folder: &Folder, name: &str) -> Result<RelativePath, TreeError> {
    folder.path.join(name).map_err(|_| TreeError::Escaped)
}

/// Retries what a scan or another program holding the file briefly can refuse.
fn retried(mut attempt: impl FnMut() -> Result<(), TreeError>) -> Result<(), TreeError> {
    let mut tries = 1;
    loop {
        match attempt() {
            Err(TreeError::InUse) if tries < COMMIT_TRIES => {
                tries += 1;
                thread::sleep(COMMIT_PAUSE);
            }
            result => return result,
        }
    }
}

fn open_folder_in(parent: &File, name: &str) -> Result<File, TreeError> {
    let handle = nt_open(
        parent,
        name,
        FOLDER_ACCESS,
        FOLDER_SHARE,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
        0,
    )?;
    attributes(&handle).map_err(TreeError::Io)?.refuse_reparse()?;
    Ok(handle)
}

/// Opens `name` relative to `parent` without following a reparse point in its place.
fn nt_open(
    parent: &File,
    name: &str,
    access: u32,
    share: FILE_SHARE_MODE,
    disposition: NTCREATEFILE_CREATE_DISPOSITION,
    options: NTCREATEFILE_CREATE_OPTIONS,
    attributes: u32,
) -> Result<File, TreeError> {
    let mut wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().collect();
    let bytes = u16::try_from(wide.len() * size_of::<u16>()).map_err(|_| TreeError::NotFound)?;
    let object_name = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: wide.as_mut_ptr(),
    };
    let object = OBJECT_ATTRIBUTES {
        Length: struct_size::<OBJECT_ATTRIBUTES>(),
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &raw const object_name,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    // SAFETY: every pointer is to a local that outlives the call: the name's buffer, the
    // attributes that point at it, the handle and the status block the call writes; the parent
    // handle is open for as long as `parent` lives.
    let status = unsafe {
        NtCreateFile(
            &raw mut handle,
            access,
            &raw const object,
            &raw mut status_block,
            std::ptr::null(),
            attributes,
            share,
            disposition,
            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(status_error(status));
    }
    // SAFETY: the call succeeded, so `handle` is a new handle that nothing else owns.
    Ok(File::from(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

struct Attributes {
    file_attributes: u32,
    reparse_tag: u32,
}

impl Attributes {
    fn is_folder(&self) -> bool {
        self.file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    fn refuse_reparse(&self) -> Result<(), TreeError> {
        if self.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            Ok(())
        } else if self.reparse_tag & IO_REPARSE_TAG_NAME_SURROGATE != 0 {
            Err(TreeError::Link)
        } else {
            Err(TreeError::ReparsePoint)
        }
    }
}

fn attributes(handle: &File) -> io::Result<Attributes> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    get_information(handle, FileAttributeTagInfo, &mut info)?;
    Ok(Attributes {
        file_attributes: info.FileAttributes,
        reparse_tag: info.ReparseTag,
    })
}

fn links(handle: &File) -> Result<u32, TreeError> {
    let mut info = FILE_STANDARD_INFO::default();
    get_information(handle, FileStandardInfo, &mut info).map_err(TreeError::Io)?;
    Ok(info.NumberOfLinks)
}

/// The volume's serial number and the file's 128-bit index. FAT and exFAT, which have no such
/// index, answer with the 32-bit serial and the 64-bit index every file system gives.
fn file_id(handle: &File) -> Result<FileId, TreeError> {
    let mut info = FILE_ID_INFO::default();
    if get_information(handle, FileIdInfo, &mut info).is_ok() {
        return Ok(FileId::new(
            info.VolumeSerialNumber,
            u128::from_le_bytes(info.FileId.Identifier),
        ));
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is open for as long as `handle` lives, and `info` is the structure the
    // call fills.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &raw mut info) } == 0 {
        return Err(TreeError::Io(io::Error::last_os_error()));
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Ok(FileId::new(
        u64::from(info.dwVolumeSerialNumber),
        u128::from(index),
    ))
}

fn entry_of(handle: &File, name: &str) -> Result<Entry, TreeError> {
    let attributes = attributes(handle).map_err(TreeError::Io)?;
    let metadata = handle.metadata().map_err(TreeError::Io)?;
    Ok(Entry {
        name: name.to_owned(),
        kind: if attributes.is_folder() {
            EntryKind::Folder
        } else {
            EntryKind::File
        },
        size: if attributes.is_folder() { 0 } else { metadata.len() },
        modified: metadata.modified().ok(),
        read_only: attributes.file_attributes & FILE_ATTRIBUTE_READONLY != 0,
    })
}

fn listed_entry(info: &FILE_ID_EXTD_DIR_INFO, name: &[u16]) -> Option<Entry> {
    let name = String::from_utf16(name).ok()?;
    if name == "." || name == ".." {
        return None;
    }
    let attributes = Attributes {
        file_attributes: info.FileAttributes,
        reparse_tag: info.ReparsePointTag,
    };
    let kind = match attributes.refuse_reparse() {
        Err(TreeError::Link) => return None,
        Err(_) => EntryKind::ReparsePoint,
        Ok(()) if attributes.is_folder() => EntryKind::Folder,
        Ok(()) => EntryKind::File,
    };
    Some(Entry {
        name,
        kind,
        size: if kind == EntryKind::File {
            u64::try_from(info.EndOfFile).unwrap_or(0)
        } else {
            0
        },
        modified: file_time(info.LastWriteTime),
        read_only: info.FileAttributes & FILE_ATTRIBUTE_READONLY != 0,
    })
}

/// A FILETIME: 100-nanosecond intervals since 1601.
fn file_time(intervals: i64) -> Option<SystemTime> {
    const UNIX_EPOCH_INTERVALS: i64 = 116_444_736_000_000_000;
    let since_epoch = intervals.checked_sub(UNIX_EPOCH_INTERVALS)?;
    let since_epoch = u64::try_from(since_epoch).ok()?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_nanos(since_epoch.checked_mul(100)?))
}

fn get_information<T>(handle: &File, class: FILE_INFO_BY_HANDLE_CLASS, info: &mut T) -> io::Result<()> {
    // SAFETY: `info` is the structure the class fills, and the size given is its own.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            class,
            std::ptr::from_mut(info).cast(),
            struct_size::<T>(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Sets file information through the native call: kernel32's wrapper rewrites a rename's name as a
/// path of the current directory, which loses `RootDirectory`.
fn set_information<T>(handle: &File, class: FILE_INFORMATION_CLASS, info: &T) -> Result<(), NTSTATUS> {
    set_raw(handle, class, std::ptr::from_ref(info).cast(), size_of::<T>())
}

fn set_raw(
    handle: &File,
    class: FILE_INFORMATION_CLASS,
    info: *const u8,
    size: usize,
) -> Result<(), NTSTATUS> {
    let mut status_block = IO_STATUS_BLOCK::default();
    let size = u32::try_from(size).map_err(|_| STATUS_INVALID_PARAMETER)?;
    // SAFETY: `info` points at `size` readable bytes of the structure the class reads, and the
    // handle is open for as long as `handle` lives.
    let status = unsafe {
        NtSetInformationFile(
            handle.as_raw_handle(),
            &raw mut status_block,
            info.cast(),
            size,
            class,
        )
    };
    if status < 0 { Err(status) } else { Ok(()) }
}

/// Renames the file `source` holds to `name` inside the folder `folder` holds.
fn set_rename(
    source: &File,
    folder: &File,
    name: &str,
    class: FILE_INFORMATION_CLASS,
    flags: u32,
) -> Result<(), NTSTATUS> {
    let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().collect();
    let name_offset = offset_of!(FILE_RENAME_INFO, FileName);
    let name_bytes = u32::try_from(wide.len() * size_of::<u16>()).map_err(|_| STATUS_INVALID_PARAMETER)?;
    // Never shorter than the structure itself, which a one-character name would be.
    let size = (name_offset + wide.len() * size_of::<u16>()).max(size_of::<FILE_RENAME_INFO>());
    // u64 units keep the structure aligned; it is followed by the name, without a terminator.
    let mut buffer = vec![0u64; size.div_ceil(size_of::<u64>())];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the buffer is zeroed, aligned for the structure and large enough for it and the
    // name, which starts at the structure's own `FileName`, a `u16` field; the writes stay inside.
    unsafe {
        (*info).Anonymous.Flags = flags;
        (*info).RootDirectory = folder.as_raw_handle();
        (*info).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            wide.as_ptr(),
            (&raw mut (*info).FileName).cast::<u16>(),
            wide.len(),
        );
    }
    // The source handle was opened with DELETE access and the buffer holds a well-formed request.
    set_raw(source, class, buffer.as_ptr().cast(), size)
}

/// What a file system without POSIX renames or deletions answers.
fn unsupported(status: NTSTATUS) -> bool {
    matches!(
        status,
        STATUS_INVALID_PARAMETER | STATUS_NOT_SUPPORTED | STATUS_INVALID_INFO_CLASS
    )
}

/// An open's failure. A folder opened as a file is `IsAFolder` here, where the Win32 code would
/// say only "access denied".
fn status_error(status: NTSTATUS) -> TreeError {
    match status {
        STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND | STATUS_OBJECT_NAME_INVALID => {
            TreeError::NotFound
        }
        STATUS_OBJECT_NAME_COLLISION => TreeError::Exists,
        STATUS_NOT_A_DIRECTORY => TreeError::NotAFolder,
        STATUS_FILE_IS_A_DIRECTORY => TreeError::IsAFolder,
        STATUS_DIRECTORY_NOT_EMPTY => TreeError::NotEmpty,
        // A name deleted while a handle stays open is taken until that handle closes.
        STATUS_SHARING_VIOLATION | STATUS_DELETE_PENDING => TreeError::InUse,
        STATUS_CANNOT_DELETE => TreeError::ReadOnly,
        status => {
            // SAFETY: a pure translation of a status code.
            let code = unsafe { RtlNtStatusToDosError(status) };
            TreeError::Io(io::Error::from_raw_os_error(
                i32::try_from(code).unwrap_or(i32::MAX),
            ))
        }
    }
}

/// A rename's or a deletion's failure: denied access there means another program holds the
/// target without sharing deletion.
fn changing(status: NTSTATUS) -> TreeError {
    match status {
        STATUS_ACCESS_DENIED => TreeError::InUse,
        status => status_error(status),
    }
}

fn final_path(handle: &File) -> io::Result<String> {
    let mut buffer = vec![0u16; 1024];
    loop {
        let capacity = u32::try_from(buffer.len()).map_err(io::Error::other)?;
        // SAFETY: the handle is open for as long as `handle` lives, and `buffer` holds
        // `capacity` writable units; the call writes at most that many.
        let written = unsafe {
            GetFinalPathNameByHandleW(
                handle.as_raw_handle(),
                buffer.as_mut_ptr(),
                capacity,
                FILE_NAME_NORMALIZED | VOLUME_NAME_NT,
            )
        };
        let written = usize::try_from(written).map_err(io::Error::other)?;
        if written == 0 {
            return Err(io::Error::last_os_error());
        }
        if written < buffer.len() {
            buffer.truncate(written);
            return OsString::from_wide(&buffer)
                .into_string()
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidData));
        }
        buffer.resize(written, 0);
    }
}

/// What follows the root in `path`, or `None` when `path` is not below the root.
fn strip_root<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let head = path.get(..root.len())?;
    if !same_name_ignoring_case(head, root) {
        return None;
    }
    let rest = &path[root.len()..];
    (rest.is_empty() || rest.starts_with('\\')).then_some(rest)
}

fn struct_size<T>() -> u32 {
    u32::try_from(size_of::<T>()).expect("a Win32 structure is far smaller than 4 GiB")
}
