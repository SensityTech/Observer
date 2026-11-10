//! The launch's cfg as the panel edits it: its text with every secret replaced by a mark, what its
//! lines set and start, and what a save sends back.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::server_data::{CfgFinding, EndpointProtocol};

/// A secret of the file stands in the text as `<<secret-1>>`, numbered from 1 in the order of the
/// file. A save puts the secret back where its mark still is.
pub const SECRET_OPEN: &str = "<<secret-";
pub const SECRET_CLOSE: &str = ">>";

/// The largest cfg the panel edits, in bytes: a request holds it whole, escaped.
pub const MAX_TEXT_BYTES: usize = 512 * 1024;
/// Earlier versions kept of the cfg, the oldest dropped first.
pub const VERSIONS_KEPT: u32 = 20;

/// `GET /api/servers/default/cfg`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgDocument {
    /// The cfg, named from the server folder.
    pub file: String,
    /// The file's text, each secret replaced by its mark, every line ended by `\n`.
    pub text: String,
    /// The SHA-256 of the file as read. A save names it, and is refused once the file is another.
    pub base: String,
    pub bytes: u32,
    pub modified_at: Option<Timestamp>,
    /// How many secrets the text hides.
    pub secrets: u32,
    pub check: CfgReview,
    /// How many earlier versions are kept.
    pub versions: u32,
}

/// What a text of the cfg sets and starts, and what the scan of it and of the files it runs finds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgReview {
    pub outline: CfgOutline,
    pub findings: Vec<CfgFinding>,
    /// The lines a save of the text would add to the file and take from it: a line changed
    /// counts once in each.
    pub added: u32,
    pub removed: u32,
}

/// The lines of a cfg the panel edits by field. Lines count from 1; `start` and `end` say where a
/// value lies in its line, in UTF-16 code units, its quotes included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgOutline {
    /// Every `set` and its kin, and the convars the panel has a field for set by their name alone,
    /// in the order of the file. The last one of a name is the one FXServer keeps.
    pub settings: Vec<CfgSetting>,
    pub endpoints: Vec<CfgEndpointLine>,
    /// Every line that starts a resource or a category, and those commented out that would start
    /// one the folder holds.
    pub resources: Vec<CfgResourceLine>,
    /// The file has more of them than are listed.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgSetting {
    pub line: u32,
    /// `set`, `sets`, `setr` or `seta`; null when the convar's name is the command.
    pub command: Option<String>,
    /// As written.
    pub name: String,
    /// Null for a secret.
    pub value: Option<String>,
    pub start: u32,
    pub end: u32,
    /// The line holds nothing else that FXServer runs.
    pub alone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgEndpointLine {
    pub line: u32,
    pub protocol: EndpointProtocol,
    pub address: String,
    pub start: u32,
    pub end: u32,
    pub alone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgResourceLine {
    pub line: u32,
    /// The resource, or the category in brackets, as written.
    pub name: String,
    /// False for a line commented out.
    pub enabled: bool,
    /// The line holds nothing else: it can be commented out, or put back, as a whole.
    pub alone: bool,
    /// Neither the server folder nor the build holds it.
    pub missing: bool,
}

/// `POST /api/servers/default/cfg/check` and `/cfg/diff`: the text as edited, marks included.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CfgDraft {
    pub text: String,
}

/// `PUT /api/servers/default/cfg`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CfgSave {
    pub text: String,
    /// [`CfgDocument::base`] of the text the edit started from.
    pub base: String,
}

/// `POST /api/servers/default/cfg/versions/{id}/restore`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CfgRestore {
    /// [`CfgDocument::base`] of the file the version replaces.
    pub base: String,
}

/// The cfg as it was before a save replaced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfgVersion {
    pub id: i64,
    /// When it was replaced, and by whom.
    pub replaced_at: Timestamp,
    pub replaced_by: String,
    pub bytes: u32,
    pub lines: u32,
}

/// Why the cfg cannot be edited from the panel, as the `reason` of `cfg_not_editable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CfgRefusal {
    /// No launch settings are saved yet.
    NotConfigured,
    Missing,
    /// A folder, a link, or anything but a plain file.
    NotAFile,
    Unreadable,
    /// Marked read-only on the host, which a save could not write over.
    ReadOnly,
    /// Over 512 KiB.
    TooLarge,
    /// Its bytes are not UTF-8: saving it would change every accented letter.
    NotUtf8,
    /// It already holds text written like a secret's mark.
    HoldsMarks,
}
