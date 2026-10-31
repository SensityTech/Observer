use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use observer_app::{Files, Grant};
use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::files::{self, CaseRules, Decision, Right, Rule, RuleSet};
use observer_platform::{ConfinedTree, Entry, EntryKind, Folder, RelativePath, TemporaryFile, TreeError};
use russh_sftp::protocol::{
    Attrs, Data, File as Listed, FileAttributes, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version,
};
use serde_json::json;

/// The largest file any rule allows.
const CEILING: u64 = 2 * 1024 * 1024 * 1024;
/// The most a read answers at once.
const MAX_READ: u32 = 256 * 1024;
const MAX_HANDLES: usize = 32;

/// One session's SFTP requests, answered from its tree under its access's rules.
pub(crate) struct Sftp {
    files: Files,
    grant: Grant,
    ip: String,
    tree: Arc<ConfinedTree>,
    handles: HashMap<String, Open>,
    next: u64,
}

enum Open {
    Read(File),
    Write(Box<Upload>),
    Listing(Option<Vec<Listed>>),
}

/// A file being written: a temporary file beside its target, committed by one rename on close.
struct Upload {
    folder: Folder,
    temporary: TemporaryFile,
    target: String,
    path: Vec<String>,
    replace: bool,
    limit: u64,
    size: u64,
    record: i64,
}

impl Sftp {
    pub(crate) fn new(files: Files, grant: Grant, ip: String, tree: Arc<ConfinedTree>) -> Self {
        Self {
            files,
            grant,
            ip,
            tree,
            handles: HashMap::new(),
            next: 0,
        }
    }

    fn rules(&self) -> Vec<(i64, &[Rule])> {
        self.grant
            .groups
            .iter()
            .map(|(id, rules)| (*id, rules.as_slice()))
            .collect()
    }

    fn decide(&self, path: &[String], right: Right) -> Decision {
        let groups = self.rules();
        RuleSet {
            groups: &groups,
            case: CaseRules::host(),
        }
        .decide(path, right)
    }

    fn shows(&self, path: &[String], folder: bool) -> bool {
        let groups = self.rules();
        RuleSet {
            groups: &groups,
            case: CaseRules::host(),
        }
        .shows(path, folder)
    }

    fn may_open_folder(&self, path: &[String]) -> bool {
        let groups = self.rules();
        let rules = RuleSet {
            groups: &groups,
            case: CaseRules::host(),
        };
        path.is_empty() || rules.may_list(path) || rules.on_the_way(path)
    }

    /// A refusal that says nothing about an entry the access cannot see.
    fn refused(&self, path: &[String]) -> StatusCode {
        if self.shows(path, false) || self.shows(path, true) {
            StatusCode::PermissionDenied
        } else {
            StatusCode::NoSuchFile
        }
    }

    /// Checks `right` on `path`, the built-in names first; the largest size a write may leave.
    fn allow(&self, path: &[String], right: Right) -> Result<Option<u64>, StatusCode> {
        if path.is_empty() || files::refused_by_name(path) {
            return Err(StatusCode::NoSuchFile);
        }
        match self.decide(path, right) {
            Decision::Allowed { max_file_size, .. } => Ok(max_file_size),
            Decision::Denied { .. } | Decision::NoRule => Err(self.refused(path)),
        }
    }

    fn handle(&mut self, open: Open) -> Result<String, StatusCode> {
        if self.handles.len() >= MAX_HANDLES {
            return Err(StatusCode::Failure);
        }
        self.next += 1;
        let handle = self.next.to_string();
        self.handles.insert(handle.clone(), open);
        Ok(handle)
    }

    async fn audit(
        &self,
        action: AuditAction,
        outcome: Outcome,
        path: &[String],
        details: serde_json::Value,
    ) {
        let target = display(path);
        if let Err(error) = self
            .files
            .record_session(&self.grant, self.ip.clone(), action, outcome, target, details)
            .await
        {
            tracing::error!(%error, "an SFTP audit entry could not be written");
        }
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&ConfinedTree) -> Result<T, TreeError> + Send + 'static,
    ) -> Result<T, StatusCode> {
        let tree = Arc::clone(&self.tree);
        tokio::task::spawn_blocking(move || work(&tree))
            .await
            .map_err(|_| StatusCode::Failure)?
            .map_err(|error| status(&error))
    }

    async fn open_write(&mut self, path: Vec<String>, flags: OpenFlags) -> Result<String, StatusCode> {
        let relative = relative(&path)?;
        let existing = self
            .blocking({
                let relative = relative.clone();
                move |tree| match tree.entry(&relative) {
                    Ok(entry) => Ok(Some(entry)),
                    Err(TreeError::NotFound) => Ok(None),
                    Err(error) => Err(error),
                }
            })
            .await?;
        let replace = match &existing {
            Some(entry) if entry.kind != EntryKind::File => return Err(StatusCode::Failure),
            Some(_) if flags.contains(OpenFlags::EXCLUDE) => return Err(StatusCode::Failure),
            Some(_) => true,
            None if !flags.contains(OpenFlags::CREATE) => return Err(StatusCode::NoSuchFile),
            None => false,
        };
        let name = path.last().cloned().ok_or(StatusCode::NoSuchFile)?;
        if !replace && !files::creatable(&name) {
            return Err(StatusCode::PermissionDenied);
        }
        let limit = self
            .allow(&path, if replace { Right::Modify } else { Right::Create })?
            .unwrap_or(CEILING)
            .min(CEILING);
        let keep = replace && !flags.contains(OpenFlags::TRUNCATE);
        if keep && existing.as_ref().is_some_and(|entry| entry.size > limit) {
            return Err(StatusCode::Failure);
        }
        let (parent, _) = relative.split_last().ok_or(StatusCode::NoSuchFile)?;
        let temporary_name = upload_name()?;
        let record = self
            .files
            .record_upload(
                self.tree.final_path().to_owned(),
                parent.to_string(),
                temporary_name.clone(),
            )
            .await
            .map_err(|_| StatusCode::Failure)?;
        let opened = self
            .blocking({
                let relative = relative.clone();
                move |tree| {
                    let folder = tree.open_folder(&parent)?;
                    let mut temporary = tree.create_temporary(&folder, &temporary_name)?;
                    let mut size = 0;
                    if keep {
                        let mut original = tree.open_file(&relative)?;
                        size = std::io::copy(&mut original, temporary.file()).map_err(TreeError::Io)?;
                    }
                    Ok((folder, temporary, size))
                }
            })
            .await;
        let (folder, temporary, size) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let _ = self.files.forget_upload(record).await;
                return Err(error);
            }
        };
        self.handle(Open::Write(Box::new(Upload {
            folder,
            temporary,
            target: name,
            path,
            replace,
            limit,
            size,
            record,
        })))
    }

    async fn commit(&mut self, upload: Upload) -> Result<(), StatusCode> {
        let Upload {
            folder,
            temporary,
            target,
            path,
            replace,
            size,
            record,
            ..
        } = upload;
        let committed = self
            .blocking(move |tree| tree.commit(&folder, temporary, &target, replace))
            .await;
        let _ = self.files.forget_upload(record).await;
        match committed {
            Ok(()) => {
                self.audit(
                    AuditAction::FileWritten,
                    Outcome::Success,
                    &path,
                    json!({ "size": size, "replaced": replace }),
                )
                .await;
                Ok(())
            }
            Err(code) => {
                self.audit(
                    AuditAction::FileRefused,
                    Outcome::Denied,
                    &path,
                    json!({ "operation": "write" }),
                )
                .await;
                Err(code)
            }
        }
    }
}

// The trait's methods are async; the ones that need no I/O answer at once.
#[allow(clippy::unused_async_trait_impl)]
impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, StatusCode> {
        let mut version = Version::new();
        version
            .extensions
            .insert("posix-rename@openssh.com".to_owned(), "1".to_owned());
        Ok(version)
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        let path = files::normalise(&path).map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Name {
            id,
            files: vec![Listed::dummy(display(&path))],
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.lstat(id, path).await
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let path = files::normalise(&path).map_err(|_| StatusCode::NoSuchFile)?;
        if !path.is_empty()
            && (files::refused_by_name(&path) || !self.shows(&path, true) && !self.shows(&path, false))
        {
            return Err(StatusCode::NoSuchFile);
        }
        let relative = relative(&path)?;
        let entry = self.blocking(move |tree| tree.entry(&relative)).await?;
        Ok(Attrs {
            id,
            attrs: attributes(&entry),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, StatusCode> {
        match self.handles.get(&handle) {
            Some(Open::Read(file)) => {
                let metadata = file.metadata().map_err(|_| StatusCode::Failure)?;
                Ok(Attrs {
                    id,
                    attrs: FileAttributes::from(&metadata),
                })
            }
            Some(Open::Write(upload)) => {
                let mut attrs = FileAttributes::empty();
                attrs.size = Some(upload.size);
                attrs.set_regular(true);
                Ok(Attrs { id, attrs })
            }
            _ => Err(StatusCode::Failure),
        }
    }

    async fn setstat(&mut self, id: u32, path: String, _attrs: FileAttributes) -> Result<Status, StatusCode> {
        // Modes and times are ignored with success; owners are never changed.
        let path = files::normalise(&path).map_err(|_| StatusCode::NoSuchFile)?;
        self.allow(&path, Right::List)?;
        Ok(ok(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        _handle: String,
        _attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        let path = files::normalise(&path).map_err(|_| StatusCode::NoSuchFile)?;
        if files::refused_by_name(&path) || !self.may_open_folder(&path) {
            return Err(StatusCode::NoSuchFile);
        }
        let relative = relative(&path)?;
        let entries = self
            .blocking(move |tree| {
                let folder = tree.open_folder(&relative)?;
                tree.list(&folder)
            })
            .await?;
        let listed = entries
            .into_iter()
            .filter(|entry| {
                let mut child = path.clone();
                child.push(entry.name.clone());
                !files::refused_by_name(&child) && self.shows(&child, entry.kind == EntryKind::Folder)
            })
            .map(|entry| Listed::new(entry.name.clone(), attributes(&entry)))
            .collect();
        let handle = self.handle(Open::Listing(Some(listed)))?;
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        match self.handles.get_mut(&handle) {
            Some(Open::Listing(listed)) => match listed.take() {
                Some(files) if !files.is_empty() => Ok(Name { id, files }),
                _ => Err(StatusCode::Eof),
            },
            _ => Err(StatusCode::Failure),
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        let path = files::normalise(&filename).map_err(|_| StatusCode::NoSuchFile)?;
        let writing =
            flags.intersects(OpenFlags::WRITE | OpenFlags::APPEND | OpenFlags::CREATE | OpenFlags::TRUNCATE);
        let handle = if writing {
            self.open_write(path, flags).await?
        } else {
            self.allow(&path, Right::Read)?;
            let relative = relative(&path)?;
            let file = self.blocking(move |tree| tree.open_file(&relative)).await?;
            self.handle(Open::Read(file))?
        };
        Ok(Handle { id, handle })
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, StatusCode> {
        let Some(Open::Read(file)) = self.handles.remove(&handle) else {
            return Err(StatusCode::Failure);
        };
        let read = tokio::task::spawn_blocking(move || {
            let mut file = file;
            let mut data = vec![0; len.min(MAX_READ) as usize];
            let read = file
                .seek(SeekFrom::Start(offset))
                .and_then(|_| file.read(&mut data));
            (file, data, read)
        })
        .await
        .map_err(|_| StatusCode::Failure)?;
        let (file, mut data, read) = read;
        self.handles.insert(handle, Open::Read(file));
        match read {
            Ok(0) => Err(StatusCode::Eof),
            Ok(count) => {
                data.truncate(count);
                Ok(Data { id, data })
            }
            Err(_) => Err(StatusCode::Failure),
        }
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, StatusCode> {
        let Some(Open::Write(mut upload)) = self.handles.remove(&handle) else {
            return Err(StatusCode::Failure);
        };
        let end = offset.saturating_add(data.len() as u64);
        if end > upload.limit {
            self.handles.insert(handle, Open::Write(upload));
            return Err(StatusCode::Failure);
        }
        let written = tokio::task::spawn_blocking(move || {
            let file = upload.temporary.file();
            let written = file
                .seek(SeekFrom::Start(offset))
                .and_then(|_| file.write_all(&data));
            (upload, written)
        })
        .await
        .map_err(|_| StatusCode::Failure)?;
        let (mut upload, written) = written;
        upload.size = upload.size.max(end);
        self.handles.insert(handle, Open::Write(upload));
        written.map(|()| ok(id)).map_err(|_| StatusCode::Failure)
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        match self.handles.remove(&handle) {
            Some(Open::Write(upload)) => self.commit(*upload).await.map(|()| ok(id)),
            Some(_) => Ok(ok(id)),
            None => Err(StatusCode::Failure),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, StatusCode> {
        let path = files::normalise(&filename).map_err(|_| StatusCode::NoSuchFile)?;
        self.allow(&path, Right::Delete)?;
        let (folder, name) = parent(&path)?;
        self.blocking(move |tree| tree.remove_file(&tree.open_folder(&folder)?, &name))
            .await?;
        self.audit(AuditAction::FileDeleted, Outcome::Success, &path, json!({}))
            .await;
        Ok(ok(id))
    }

    async fn mkdir(&mut self, id: u32, path: String, _attrs: FileAttributes) -> Result<Status, StatusCode> {
        let path = files::normalise(&path).map_err(|_| StatusCode::NoSuchFile)?;
        self.allow(&path, Right::MakeFolder)?;
        let (folder, name) = parent(&path)?;
        if !files::creatable(&name) {
            return Err(StatusCode::PermissionDenied);
        }
        self.blocking(move |tree| tree.make_folder(&tree.open_folder(&folder)?, &name))
            .await?;
        self.audit(AuditAction::FolderCreated, Outcome::Success, &path, json!({}))
            .await;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let path = files::normalise(&path).map_err(|_| StatusCode::NoSuchFile)?;
        self.allow(&path, Right::RemoveFolder)?;
        let (folder, name) = parent(&path)?;
        self.blocking(move |tree| tree.remove_folder(&tree.open_folder(&folder)?, &name))
            .await?;
        self.audit(AuditAction::FolderRemoved, Outcome::Success, &path, json!({}))
            .await;
        Ok(ok(id))
    }

    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        self.move_entry(&from, &to, false).await.map(|()| ok(id))
    }

    async fn extended(&mut self, id: u32, request: String, data: Vec<u8>) -> Result<Packet, StatusCode> {
        match request.as_str() {
            "posix-rename@openssh.com" => {
                let (from, rest) = ssh_string(&data).ok_or(StatusCode::BadMessage)?;
                let (to, _) = ssh_string(rest).ok_or(StatusCode::BadMessage)?;
                self.move_entry(from, to, true).await?;
                Ok(Packet::Status(ok(id)))
            }
            _ => Err(StatusCode::OpUnsupported),
        }
    }
}

impl Sftp {
    /// A rename or a move. It never takes a file out from under a deny rule, and never gains a
    /// right: what it may do to the entry at the target it may already do at the source.
    async fn move_entry(&mut self, from: &str, to: &str, replace: bool) -> Result<(), StatusCode> {
        let source = files::normalise(from).map_err(|_| StatusCode::NoSuchFile)?;
        let target = files::normalise(to).map_err(|_| StatusCode::NoSuchFile)?;
        self.allow(&source, Right::Rename)?;
        let groups = self.rules();
        let rules = RuleSet {
            groups: &groups,
            case: CaseRules::host(),
        };
        if rules.any_deny(&source).is_some() {
            return Err(StatusCode::PermissionDenied);
        }
        let target_name = target.last().cloned().ok_or(StatusCode::NoSuchFile)?;
        if !files::creatable(&target_name) {
            return Err(StatusCode::PermissionDenied);
        }
        self.allow(&target, Right::Create)?;
        let gains = Right::ALL.into_iter().any(|right| {
            rules.decide(&target, right).is_allowed() && !rules.decide(&source, right).is_allowed()
        });
        if gains || rules.any_deny(&target).is_some() {
            return Err(StatusCode::PermissionDenied);
        }
        let (from_folder, name) = parent(&source)?;
        let (to_folder, new_name) = parent(&target)?;
        self.blocking(move |tree| {
            let from = tree.open_folder(&from_folder)?;
            let to = tree.open_folder(&to_folder)?;
            tree.rename(&from, &name, &to, &new_name, replace)
        })
        .await?;
        self.audit(
            AuditAction::FileRenamed,
            Outcome::Success,
            &source,
            json!({ "to": display(&target) }),
        )
        .await;
        Ok(())
    }
}

fn relative(path: &[String]) -> Result<RelativePath, StatusCode> {
    RelativePath::new(path.iter().cloned()).map_err(|_| StatusCode::NoSuchFile)
}

fn parent(path: &[String]) -> Result<(RelativePath, String), StatusCode> {
    let relative = relative(path)?;
    let (folder, name) = relative.split_last().ok_or(StatusCode::NoSuchFile)?;
    Ok((folder, name.to_owned()))
}

fn display(path: &[String]) -> String {
    format!("/{}", path.join("/"))
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_owned(),
        language_tag: "en-US".to_owned(),
    }
}

/// How a refusal of the tree reads to an SFTP client: nothing outside the access's reach is
/// confirmed, so a link or an escape reads as a missing file.
fn status(error: &TreeError) -> StatusCode {
    match error {
        TreeError::NotFound | TreeError::Link | TreeError::Escaped => StatusCode::NoSuchFile,
        TreeError::HardLink | TreeError::ReparsePoint | TreeError::NotRegular | TreeError::ReadOnly => {
            StatusCode::PermissionDenied
        }
        _ => StatusCode::Failure,
    }
}

fn attributes(entry: &Entry) -> FileAttributes {
    let mut attrs = FileAttributes::empty();
    attrs.size = Some(entry.size);
    if entry.kind == EntryKind::Folder {
        attrs.set_dir(true);
        attrs.permissions = Some(0o040_755);
    } else {
        attrs.set_regular(true);
        attrs.permissions = Some(if entry.read_only { 0o100_444 } else { 0o100_644 });
    }
    attrs.mtime = entry
        .modified
        .and_then(|at| at.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .and_then(|since| u32::try_from(since.as_secs()).ok());
    attrs
}

/// `.observer-upload-` and 26 base32 characters (130 random bits).
fn upload_name() -> Result<String, StatusCode> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bytes = [0u8; 26];
    getrandom::fill(&mut bytes).map_err(|_| StatusCode::Failure)?;
    let suffix: String = bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
        .collect();
    Ok(format!("{}{suffix}", files::UPLOAD_PREFIX))
}

fn ssh_string(data: &[u8]) -> Option<(&str, &[u8])> {
    let length = usize::try_from(u32::from_be_bytes(data.get(..4)?.try_into().ok()?)).ok()?;
    let text = std::str::from_utf8(data.get(4..4 + length)?).ok()?;
    Some((text, &data[4 + length..]))
}
