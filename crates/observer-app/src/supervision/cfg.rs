//! The launch's cfg edited from the panel: read with a mark in place of each secret, checked as it
//! is typed, and written back whole once the file as it was is kept. With the SFTP uploads and the
//! server tools' fixes, the only writes Observer makes in the server folder; each is the owner's,
//! password given again.

use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use jiff::Timestamp;
use observer_domain::audit::AuditAction;
use observer_domain::cfg::{
    CfgDocument, CfgDraft, CfgRefusal, CfgRestore, CfgReview, CfgSave, CfgVersion, MAX_TEXT_BYTES,
    VERSIONS_KEPT,
};
use observer_domain::changes::TextDiff;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::launch::{self, LaunchSettings, OneSync, Platform};
use observer_fxserver::cfg_edit::{self, Editable, Refused, Unreadable};
use observer_fxserver::changes::{diff, mask};
use observer_fxserver::server_data;
use observer_store::cfg_versions::{self, Replaced};
use serde_json::json;

use super::setup::blocking;
use super::{Supervision, read_launch};
use crate::audit::Record;
use crate::{Caller, Error, Servers, secrets};

/// Antivirus scanners hold a freshly written file for a moment on Windows, which makes the rename
/// over the cfg fail; the same call a little later succeeds.
const BUSY_DELAYS_MS: [u64; 4] = [50, 100, 200, 400];
const SAVING: &str = ".observer-saving";

/// The cfg of the saved launch, as its file holds it.
struct Read {
    data: PathBuf,
    /// Named from the server folder.
    file: String,
    onesync: OneSync,
    bytes: Vec<u8>,
    modified: Option<Timestamp>,
    editable: Editable,
}

impl Read {
    fn path(&self) -> PathBuf {
        self.data.join(&self.file)
    }

    fn base(&self) -> String {
        base(&self.bytes)
    }

    /// The bytes a save of `text` writes.
    fn written(&self, text: &str) -> Result<Vec<u8>, Error> {
        self.editable.write(text).map_err(|refused| {
            let reason = match refused {
                Refused::UnknownMark => "unknownMark",
                Refused::Control => "control",
                Refused::TooLarge => "tooLarge",
            };
            ApiError::new(ErrorCode::InvalidCfgText)
                .with("reason", json!(reason))
                .into()
        })
    }
}

fn base(bytes: &[u8]) -> String {
    secrets::hex(&secrets::sha256(bytes))
}

fn refusal(reason: CfgRefusal) -> Error {
    ApiError::new(ErrorCode::CfgNotEditable)
        .with("reason", serde_json::to_value(reason).unwrap_or_default())
        .into()
}

fn lines(bytes: &[u8]) -> u32 {
    u32::try_from(String::from_utf8_lossy(bytes).lines().count()).unwrap_or(u32::MAX)
}

impl Supervision {
    /// The launch's cfg, for the owner to edit. It changes nothing.
    pub async fn cfg_document(&self) -> Result<CfgDocument, Error> {
        let read = self.read_cfg().await?;
        self.document(read).await
    }

    /// What a text of the cfg sets and starts, and what the scan finds in it and in the files it
    /// runs. It changes nothing.
    pub async fn check_cfg_draft(&self, draft: CfgDraft) -> Result<CfgReview, Error> {
        if draft.text.len() > MAX_TEXT_BYTES {
            return Err(ApiError::new(ErrorCode::InvalidCfgText)
                .with("reason", json!("tooLarge"))
                .into());
        }
        let read = self.read_cfg().await?;
        let whole = read.editable.revealed(&draft.text);
        self.check(read, draft.text, whole).await
    }

    /// The lines a save of the text would change in the file, secrets hidden. It changes nothing.
    pub async fn diff_cfg_draft(&self, draft: CfgDraft) -> Result<TextDiff, Error> {
        let read = self.read_cfg().await?;
        let written = read.written(&draft.text)?;
        blocking(move || changed(&read.file, &read.bytes, &written)).await?
    }

    /// Writes the text as the cfg, once the file as it was is kept. Refused when the file is no
    /// longer the one the edit started from.
    pub async fn save_cfg(&self, caller: &Caller, save: CfgSave) -> Result<CfgDocument, Error> {
        let read = self.read_cfg().await?;
        if read.base() != save.base {
            return Err(ErrorCode::CfgChanged.into());
        }
        let written = read.written(&save.text)?;
        let record = Record::by(caller, AuditAction::ServerCfgChanged);
        self.replace_cfg(caller, read, written, record, json!({})).await?;
        self.cfg_document().await
    }

    /// The versions kept of the cfg, the latest first.
    pub async fn cfg_versions(&self) -> Result<Vec<CfgVersion>, Error> {
        let file = self.cfg_file().await?;
        self.inner
            .db
            .read(move |conn| Ok::<_, Error>(cfg_versions::list(conn, Servers::DEFAULT, &file)?))
            .await
    }

    /// The lines putting the version back would change in the file, secrets hidden.
    pub async fn cfg_version_diff(&self, id: i64) -> Result<TextDiff, Error> {
        let read = self.read_cfg().await?;
        let version = self.version(&read.file, id).await?;
        blocking(move || changed(&read.file, &read.bytes, &version)).await?
    }

    /// Puts a version back as the cfg, the file as it is kept first.
    pub async fn restore_cfg(
        &self,
        caller: &Caller,
        id: i64,
        restore: CfgRestore,
    ) -> Result<CfgDocument, Error> {
        let read = self.read_cfg().await?;
        if read.base() != restore.base {
            return Err(ErrorCode::CfgChanged.into());
        }
        let version = self.version(&read.file, id).await?;
        let record = Record::by(caller, AuditAction::ServerCfgRestored);
        self.replace_cfg(caller, read, version, record, json!({ "version": id }))
            .await?;
        self.cfg_document().await
    }

    async fn cfg_file(&self) -> Result<String, Error> {
        let (stored, _) = read_launch(&self.inner.db).await?;
        stored
            .map(|stored| stored.settings.cfg_file)
            .ok_or_else(|| refusal(CfgRefusal::NotConfigured))
    }

    async fn version(&self, file: &str, id: i64) -> Result<Vec<u8>, Error> {
        let file = file.to_owned();
        self.inner
            .db
            .read(move |conn| Ok::<_, Error>(cfg_versions::content(conn, Servers::DEFAULT, &file, id)?))
            .await?
            .ok_or_else(|| ErrorCode::CfgVersionNotFound.into())
    }

    async fn read_cfg(&self) -> Result<Read, Error> {
        let (stored, _) = read_launch(&self.inner.db).await?;
        let Some(stored) = stored else {
            return Err(refusal(CfgRefusal::NotConfigured));
        };
        blocking(move || read(&stored.settings)).await?
    }

    async fn document(&self, read: Read) -> Result<CfgDocument, Error> {
        let file = read.file.clone();
        let versions = self
            .inner
            .db
            .read(move |conn| Ok::<_, Error>(cfg_versions::count(conn, Servers::DEFAULT, &file)?))
            .await?;
        let shown = read.editable.text.clone();
        let whole = read.editable.revealed(&shown);
        let document = CfgDocument {
            file: read.file.clone(),
            text: shown.clone(),
            base: read.base(),
            bytes: u32::try_from(read.bytes.len()).unwrap_or(u32::MAX),
            modified_at: read.modified,
            secrets: u32::try_from(read.editable.secrets()).unwrap_or(u32::MAX),
            check: CfgReview::default(),
            versions,
        };
        let check = self.check(read, shown, whole).await?;
        Ok(CfgDocument { check, ..document })
    }

    async fn check(&self, read: Read, shown: String, whole: String) -> Result<CfgReview, Error> {
        let context = self.cfg_context(read.onesync).await?;
        blocking(move || {
            let resources = server_data::walk(&read.data);
            let mut review = cfg_edit::check(&read.data, &read.file, &shown, &whole, &resources, &context);
            let key = secrets::random::<32>()?;
            let file = read.editable.revealed(&read.editable.text);
            let old = mask::mask(&file, &read.file, true, &key);
            let new = mask::mask(&whole, &read.file, true, &key);
            (review.added, review.removed) = diff::counts(Some(&old), Some(&new));
            Ok(review)
        })
        .await?
    }

    /// Keeps the cfg as it is, writes `written` in its place and records it. Nothing happens when
    /// the file already holds those bytes.
    async fn replace_cfg(
        &self,
        caller: &Caller,
        read: Read,
        written: Vec<u8>,
        record: Record,
        mut details: serde_json::Value,
    ) -> Result<(), Error> {
        if written == read.bytes {
            return Ok(());
        }
        let now = self.inner.clock.now();
        let (file, by, kept) = (read.file.clone(), caller.username.clone(), read.bytes.clone());
        self.inner
            .db
            .write(move |tx| {
                let replaced = Replaced {
                    server: Servers::DEFAULT,
                    file: &file,
                    content: &kept,
                    lines: lines(&kept),
                    by: &by,
                    at: now,
                };
                cfg_versions::keep(tx, replaced, VERSIONS_KEPT)?;
                Ok::<_, Error>(())
            })
            .await?;
        let file = read.file.clone();
        let (added, removed) = blocking(move || {
            replace(&read.path(), &read.bytes, &written)?;
            let key = secrets::random::<32>()?;
            let old = mask::mask(&String::from_utf8_lossy(&read.bytes), &read.file, true, &key);
            let new = mask::mask(&String::from_utf8_lossy(&written), &read.file, true, &key);
            Ok::<_, Error>(diff::counts(Some(&old), Some(&new)))
        })
        .await??;
        if let Some(details) = details.as_object_mut() {
            details.insert("added".to_owned(), json!(added));
            details.insert("removed".to_owned(), json!(removed));
        }
        self.audit(record.target(file).details(details)).await
    }
}

/// Reads the launch's cfg for editing: a plain file of the server folder, never a link.
fn read(settings: &LaunchSettings) -> Result<Read, Error> {
    let platform = Platform::host();
    if launch::folder_problem(&settings.data_path, platform).is_some()
        || launch::cfg_problem(&settings.cfg_file, platform).is_some()
    {
        return Err(refusal(CfgRefusal::NotConfigured));
    }
    let data = PathBuf::from(&settings.data_path);
    let path = data.join(&settings.cfg_file);
    let unreadable = |error: io::Error| {
        refusal(match error.kind() {
            io::ErrorKind::NotFound => CfgRefusal::Missing,
            _ => CfgRefusal::Unreadable,
        })
    };
    let metadata = fs::symlink_metadata(&path).map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(refusal(CfgRefusal::NotAFile));
    }
    if metadata.permissions().readonly() {
        return Err(refusal(CfgRefusal::ReadOnly));
    }
    let limit = u64::try_from(MAX_TEXT_BYTES).unwrap_or(u64::MAX);
    let mut bytes = Vec::new();
    // One byte past the limit tells a file that grew since its size was read.
    File::open(&path)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
        .map_err(unreadable)?;
    if metadata.len() > limit || bytes.len() > MAX_TEXT_BYTES {
        return Err(refusal(CfgRefusal::TooLarge));
    }
    let editable = Editable::read(&bytes, &settings.cfg_file).map_err(|unreadable| {
        refusal(match unreadable {
            Unreadable::NotUtf8 => CfgRefusal::NotUtf8,
            Unreadable::HoldsMarks => CfgRefusal::HoldsMarks,
        })
    })?;
    Ok(Read {
        data,
        file: settings.cfg_file.clone(),
        onesync: settings.onesync,
        modified: metadata
            .modified()
            .ok()
            .and_then(|modified| Timestamp::try_from(modified).ok()),
        bytes,
        editable,
    })
}

/// What changes between two versions of the cfg `file`, their secrets hidden.
fn changed(file: &str, old: &[u8], new: &[u8]) -> Result<TextDiff, Error> {
    let key = secrets::random::<32>()?;
    let old = mask::mask(&String::from_utf8_lossy(old), file, true, &key);
    let new = mask::mask(&String::from_utf8_lossy(new), file, true, &key);
    Ok(diff::diff(Some(&old), Some(&new), diff::DiffOptions::default()))
}

/// Writes `written` over the cfg at `path`, which must still hold `expected`: beside it under a
/// temporary name, flushed, then renamed over it, so that FXServer never reads half a file.
fn replace(path: &Path, expected: &[u8], written: &[u8]) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || fs::read(path)? != expected {
        return Err(ErrorCode::CfgChanged.into());
    }
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("cfg");
    let temporary = path.with_file_name(format!(".{name}{SAVING}"));
    let result = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(written)?;
        file.sync_all()?;
        drop(file);
        fs::set_permissions(&temporary, metadata.permissions())?;
        retry(|| fs::rename(&temporary, path))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    Ok(result?)
}

fn retry(mut call: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    for delay in BUSY_DELAYS_MS {
        match call() {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                thread::sleep(Duration::from_millis(delay));
            }
            result => return result,
        }
    }
    call()
}
