//! File access: the rights an access holds, the rules that grant and deny them, how a client's path
//! is normalised, and the decision for one operation on one path. Pure, so that what an access may
//! do is computed the same way by the SFTP server, the panel's rule tester and the tests.

use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::delegation::Account;
use crate::error::ErrorCode;
use crate::permissions::Permissions;
use ts_rs::TS;

/// One thing an access may do to an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Right {
    List,
    Read,
    Create,
    Modify,
    Rename,
    Delete,
    MakeFolder,
    RemoveFolder,
}

impl Right {
    pub const ALL: [Right; 8] = [
        Right::List,
        Right::Read,
        Right::Create,
        Right::Modify,
        Right::Rename,
        Right::Delete,
        Right::MakeFolder,
        Right::RemoveFolder,
    ];

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of rights. On the wire and in the database, a list of their names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, TS)]
#[ts(export, as = "Vec<Right>")]
pub struct Rights(u8);

impl Rights {
    pub const NONE: Self = Self(0);
    /// Each of these can be rebuilt from the others (a delete then a create is a modify), so a
    /// deny of one denies the four.
    pub const CHANGES: Self = Self::of(&[Right::Create, Right::Modify, Right::Rename, Right::Delete]);
    pub const VIEW: Self = Self::of(&[Right::List, Right::Read]);
    /// A drop box: writes, but cannot read back.
    pub const UPLOAD: Self = Self::of(&[Right::List, Right::Create, Right::Modify, Right::MakeFolder]);
    pub const EDIT: Self = Self::of(&[
        Right::List,
        Right::Read,
        Right::Create,
        Right::Modify,
        Right::Rename,
        Right::MakeFolder,
    ]);
    pub const FULL: Self = Self(0xff);
    /// What a group may grant and still reach no further than Read.
    const READ_REACH: Self = Self::of(&[Right::List, Right::Read, Right::MakeFolder]);

    pub const fn of(rights: &[Right]) -> Self {
        let mut bits = 0;
        let mut index = 0;
        while index < rights.len() {
            bits |= rights[index].bit();
            index += 1;
        }
        Self(bits)
    }

    pub const fn contains(self, right: Right) -> bool {
        self.0 & right.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn is_within(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Right> {
        Right::ALL.into_iter().filter(move |right| self.contains(*right))
    }

    /// The rights a deny of these refuses: a deny of any change refuses every change.
    const fn denied(self) -> Self {
        if self.0 & Self::CHANGES.0 == 0 {
            self
        } else {
            self.union(Self::CHANGES)
        }
    }
}

impl Serialize for Rights {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Rights {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::of(&Vec::<Right>::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Effect {
    Allow,
    Deny,
}

/// How far below its folder a rule reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Depth {
    /// The folder's own entries.
    Folder,
    /// The folder's entries and everything below them.
    Tree,
}

/// One rule of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Rule {
    pub effect: Effect,
    /// A folder of the server, `/`-separated; each component a name or a name glob. Empty is the
    /// server folder itself.
    pub folder: String,
    pub depth: Depth,
    /// Name globs. On an allow rule they filter entries; on a deny rule an entry is matched when
    /// its own name, or the name of a folder on the way below the rule's folder, matches. Empty
    /// means every entry.
    #[serde(default)]
    pub names: Vec<String>,
    pub rights: Rights,
    /// The largest file a create or modify may leave, for an allow rule.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub max_file_size: Option<u64>,
}

/// What makes a rule unusable, by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleField {
    Folder,
    Names,
    Rights,
    MaxFileSize,
}

impl RuleField {
    pub const fn key(self) -> &'static str {
        match self {
            RuleField::Folder => "folder",
            RuleField::Names => "names",
            RuleField::Rights => "rights",
            RuleField::MaxFileSize => "maxFileSize",
        }
    }
}

impl Rule {
    pub fn check(&self) -> Result<(), RuleField> {
        if !self.folder.is_empty() && !self.folder.split('/').all(valid_glob) {
            return Err(RuleField::Folder);
        }
        if !self.names.iter().all(|name| valid_glob(name)) {
            return Err(RuleField::Names);
        }
        if self.rights.is_empty() {
            return Err(RuleField::Rights);
        }
        match (self.effect, self.max_file_size) {
            (Effect::Deny, Some(_)) | (Effect::Allow, Some(0)) => Err(RuleField::MaxFileSize),
            _ => Ok(()),
        }
    }

    fn folder_components(&self) -> impl Iterator<Item = &str> {
        self.folder.split('/').filter(|component| !component.is_empty())
    }

    fn folder_len(&self) -> usize {
        self.folder_components().count()
    }
}

/// A glob is one path component: `*` and `?` are its only wildcards, and `**` is refused, since
/// depth is the rule's own field.
fn valid_glob(glob: &str) -> bool {
    !glob.is_empty()
        && glob != "."
        && glob != ".."
        && !glob.contains("**")
        && !glob.contains(['/', '\\', ':', '\0'])
        && !glob.chars().any(char::is_control)
}

/// Whether `name` matches `glob`: `*` is any run of characters, `?` one character, and every other
/// character, `[` and `]` included, stands for itself, since FiveM names its categories `[maps]`.
pub fn glob_matches(glob: &str, name: &str, ignore_case: bool) -> bool {
    let glob: Vec<char> = glob.chars().map(|c| fold(c, ignore_case)).collect();
    let name: Vec<char> = name.chars().map(|c| fold(c, ignore_case)).collect();
    let (mut g, mut n) = (0, 0);
    // Where the last `*` was, and the name position it currently stands for the run up to.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match glob.get(g) {
            Some('*') => {
                star = Some((g, n));
                g += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                g += 1;
                n += 1;
            }
            _ => match star {
                Some((star_g, star_n)) => {
                    g = star_g + 1;
                    n = star_n + 1;
                    star = Some((star_g, star_n + 1));
                }
                None => return false,
            },
        }
    }
    glob[g..].iter().all(|&c| c == '*')
}

/// Each character's simple uppercase, one to one as NTFS compares names: never expanded (ß stays ß).
fn fold(c: char, ignore_case: bool) -> char {
    if !ignore_case {
        return c;
    }
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// Names refused anywhere in the tree, to file access and to file changes alike: they hold
/// passwords, keys and the server's own credentials.
pub const NEVER_SHOW: &[&str] = &[
    "txData",
    ".replxx_history",
    "*.kdbx",
    "*.pem",
    "*.key",
    "*.pfx",
    "*.p12",
    "id_rsa*",
    "id_ecdsa*",
    "id_ed25519*",
    ".env",
    ".env.*",
];

/// File access's own refusals by name: a repository's objects hold every version it tracked, its
/// config often a token, and upload files are Observer's own.
pub const REFUSED_NAMES: &[&str] = &[".git", ".observer-upload-*"];

/// The prefix of Observer's temporary upload files.
pub const UPLOAD_PREFIX: &str = ".observer-upload-";

/// Whether a component of `path` is one of the names refused anywhere, compared without case.
pub fn refused_by_name(path: &[String]) -> bool {
    path.iter().any(|component| {
        NEVER_SHOW
            .iter()
            .chain(REFUSED_NAMES)
            .any(|glob| glob_matches(glob, component, true))
    })
}

/// Why a client's path was refused before any rule was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// NUL, a backslash or a control character.
    Character,
    /// A colon, a trailing dot or space, or a reserved device name: Windows would alias them.
    Name,
    TooLong,
}

pub const MAX_COMPONENTS: usize = 64;
pub const MAX_COMPONENT_BYTES: usize = 255;
pub const MAX_PATH_BYTES: usize = 1024;

/// Normalises a client's path into components below the root. `.` and `..` are resolved as text
/// and clamped at the root, as OpenSSH's chroot does; nothing is resolved through the file system.
pub fn normalise(raw: &str) -> Result<Vec<String>, PathError> {
    if raw.len() > MAX_PATH_BYTES {
        return Err(PathError::TooLong);
    }
    if raw.chars().any(|c| c == '\0' || c == '\\' || c.is_control()) {
        return Err(PathError::Character);
    }
    let mut components: Vec<String> = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            name => {
                if name.len() > MAX_COMPONENT_BYTES {
                    return Err(PathError::TooLong);
                }
                if !portable_name(name) {
                    return Err(PathError::Name);
                }
                components.push(name.to_owned());
            }
        }
    }
    if components.len() > MAX_COMPONENTS {
        return Err(PathError::TooLong);
    }
    Ok(components)
}

/// A name both systems keep as it is: no alternate data stream, no trailing dot or space that
/// Windows strips, no device name Windows opens instead of a file.
fn portable_name(name: &str) -> bool {
    if name.contains(':') || name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper: String = stem.chars().map(|c| fold(c, true)).collect();
    let device = match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|rest| {
                matches!(
                    rest,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        }),
    };
    !device
}

/// Whether a new entry may take this name: one shaped like a Windows short alias (`SERVER~1.CFG`)
/// could stand for another file, so it is never created.
pub fn creatable(name: &str) -> bool {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem, Some(extension)),
        None => (name, None),
    };
    let Some((base, number)) = stem.rsplit_once('~') else {
        return true;
    };
    let alias = !base.is_empty()
        && base.chars().count() <= 6
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
        && extension.is_none_or(|extension| extension.chars().count() <= 3);
    !alias
}

/// How names are compared: on Windows without case everywhere; on Linux allow rules compare with
/// case, while deny rules and the built-in names never do, so `Server.CFG` is refused like
/// `server.cfg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaseRules {
    pub allows_ignore_case: bool,
}

impl CaseRules {
    pub const WINDOWS: Self = Self {
        allows_ignore_case: true,
    };
    pub const LINUX: Self = Self {
        allows_ignore_case: false,
    };

    pub const fn host() -> Self {
        if cfg!(windows) { Self::WINDOWS } else { Self::LINUX }
    }

    const fn ignore_case(self, effect: Effect) -> bool {
        match effect {
            Effect::Allow => self.allows_ignore_case,
            Effect::Deny => true,
        }
    }
}

/// The rules of every group an access holds, as the decision reads them.
#[derive(Debug, Clone, Copy)]
pub struct RuleSet<'a> {
    pub groups: &'a [(i64, &'a [Rule])],
    pub case: CaseRules,
}

/// The rule that decided, by group id and position in the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleRef {
    pub group: i64,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed {
        rule: RuleRef,
        /// The largest `maxFileSize` among the allowing rules, when every one of them sets one.
        max_file_size: Option<u64>,
    },
    Denied {
        rule: RuleRef,
    },
    /// No rule grants it.
    NoRule,
}

impl Decision {
    pub const fn is_allowed(self) -> bool {
        matches!(self, Decision::Allowed { .. })
    }
}

impl RuleSet<'_> {
    fn rules(&self) -> impl Iterator<Item = (RuleRef, &Rule)> {
        self.groups.iter().flat_map(|(group, rules)| {
            rules
                .iter()
                .enumerate()
                .map(move |(index, rule)| (RuleRef { group: *group, index }, rule))
        })
    }

    /// The decision for `right` on the entry at `path` (never the root). Built-in refusals are
    /// checked apart, before this.
    pub fn decide(&self, path: &[String], right: Right) -> Decision {
        let Some((name, parent)) = path.split_last() else {
            return Decision::NoRule;
        };
        if let Some(rule) = self.denying(path, right) {
            return Decision::Denied { rule };
        }
        let ignore_case = self.case.ignore_case(Effect::Allow);
        let mut first: Option<RuleRef> = None;
        // `None` once a rule without a limit allows it.
        let mut limit: Option<u64> = Some(0);
        for (reference, rule) in self.rules() {
            if rule.effect != Effect::Allow
                || !rule.rights.contains(right)
                || !folder_reaches(rule, parent, ignore_case)
            {
                continue;
            }
            // `makeFolder` is not filtered by names, every other right is.
            if right != Right::MakeFolder && !names_match(&rule.names, name, ignore_case) {
                continue;
            }
            first.get_or_insert(reference);
            limit = limit.zip(rule.max_file_size).map(|(old, size)| old.max(size));
        }
        match first {
            Some(rule) => Decision::Allowed {
                rule,
                max_file_size: limit,
            },
            None => Decision::NoRule,
        }
    }

    /// The first deny rule that refuses `right` on the entry at `path`.
    pub fn denying(&self, path: &[String], right: Right) -> Option<RuleRef> {
        self.rules()
            .find(|(_, rule)| {
                rule.effect == Effect::Deny
                    && rule.rights.denied().contains(right)
                    && deny_matches(rule, path)
            })
            .map(|(reference, _)| reference)
    }

    /// Whether any deny rule, whatever rights it names, matches the entry at `path`.
    pub fn any_deny(&self, path: &[String]) -> Option<RuleRef> {
        self.rules()
            .find(|(_, rule)| rule.effect == Effect::Deny && deny_matches(rule, path))
            .map(|(reference, _)| reference)
    }

    /// Whether the folder at `path` may be listed: an allow rule whose folder is this one (or
    /// above it, for a tree) grants `list`, its names ignored, and no deny refuses `list` to it.
    pub fn may_list(&self, path: &[String]) -> bool {
        if !path.is_empty() && self.denying(path, Right::List).is_some() {
            return false;
        }
        let ignore_case = self.case.ignore_case(Effect::Allow);
        self.rules().any(|(_, rule)| {
            rule.effect == Effect::Allow
                && rule.rights.contains(Right::List)
                && folder_reaches(rule, path, ignore_case)
        })
    }

    /// Whether the folder at `path` lies on the way to an allow rule's folder, so that it is shown
    /// even though it grants nothing.
    pub fn on_the_way(&self, path: &[String]) -> bool {
        let ignore_case = self.case.ignore_case(Effect::Allow);
        self.rules().any(|(_, rule)| {
            rule.effect == Effect::Allow
                && rule.folder_len() > path.len()
                && rule
                    .folder_components()
                    .zip(path)
                    .all(|(glob, component)| glob_matches(glob, component, ignore_case))
        })
    }

    /// Whether a listing shows the entry at `path`: no deny refuses it `list`, and some rule
    /// lets the access do something with it, or, for a folder, leads through it.
    pub fn shows(&self, path: &[String], folder: bool) -> bool {
        if self.denying(path, Right::List).is_some() {
            return false;
        }
        if folder && (self.may_list(path) || self.on_the_way(path)) {
            return true;
        }
        Right::ALL
            .into_iter()
            .filter(|right| *right != Right::MakeFolder)
            .any(|right| self.decide(path, right).is_allowed())
    }
}

/// Whether the rule reaches into `folder`: its own folder, or one below it for a tree.
fn folder_reaches(rule: &Rule, folder: &[String], ignore_case: bool) -> bool {
    let length = rule.folder_len();
    let deep_enough = match rule.depth {
        Depth::Folder => folder.len() == length,
        Depth::Tree => folder.len() >= length,
    };
    deep_enough
        && rule
            .folder_components()
            .zip(folder)
            .all(|(glob, component)| glob_matches(glob, component, ignore_case))
}

fn deny_matches(rule: &Rule, path: &[String]) -> bool {
    let Some((_, parent)) = path.split_last() else {
        return false;
    };
    if !folder_reaches(rule, parent, true) {
        return false;
    }
    // The entry's own name, or a folder's on the way below the rule's folder.
    rule.names.is_empty()
        || path[rule.folder_len()..]
            .iter()
            .any(|component| names_match(&rule.names, component, true))
}

fn names_match(names: &[String], name: &str, ignore_case: bool) -> bool {
    names.is_empty() || names.iter().any(|glob| glob_matches(glob, name, ignore_case))
}

/// How far a group reaches, shown beside it with its danger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Reach {
    /// Lists, reads and makes folders only.
    Read,
    /// Changes only asset files: streamed models and textures, images, audio, fonts.
    Assets,
    /// Changes anything else, which can run code on the host.
    Code,
}

/// File types players download as data, which run nothing.
pub const ASSET_EXTENSIONS: &[&str] = &[
    "ytd", "ydr", "yft", "ybn", "ymap", "ytyp", "ycd", "ynv", "ypt", "ydd", "awc", "png", "jpg", "jpeg",
    "webp", "gif", "ogg", "mp3", "wav", "ttf", "otf", "woff", "woff2",
];

pub fn reach(rules: &[Rule]) -> Reach {
    let mut reach = Reach::Read;
    for rule in rules.iter().filter(|rule| rule.effect == Effect::Allow) {
        if rule.rights.is_within(Rights::READ_REACH) {
            continue;
        }
        if rule.names.is_empty() || !rule.names.iter().all(|glob| asset_glob(glob)) {
            return Reach::Code;
        }
        reach = Reach::Assets;
    }
    reach
}

/// A glob that can only match one asset type: it ends with a literal extension from the list.
fn asset_glob(glob: &str) -> bool {
    glob.rsplit_once('.').is_some_and(|(_, extension)| {
        !extension.contains(['*', '?'])
            && ASSET_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
    })
}

impl fmt::Display for Right {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Right::List => "list",
            Right::Read => "read",
            Right::Create => "create",
            Right::Modify => "modify",
            Right::Rename => "rename",
            Right::Delete => "delete",
            Right::MakeFolder => "makeFolder",
            Right::RemoveFolder => "removeFolder",
        };
        f.write_str(name)
    }
}

/// Whether `ip` falls in one of the CIDR blocks; an empty list allows every address. An IPv4
/// address reached over IPv6 (`::ffff:a.b.c.d`) is compared as IPv4. A block that does not parse
/// allows nothing.
pub fn network_allows(networks: &[String], ip: IpAddr) -> bool {
    if networks.is_empty() {
        return true;
    }
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    };
    networks.iter().any(|network| block_contains(network, ip))
}

fn block_contains(block: &str, ip: IpAddr) -> bool {
    let (address, length) = match block.split_once('/') {
        Some((address, length)) => (address, length.parse::<u32>().ok()),
        None => (block, None),
    };
    match (address.parse::<IpAddr>(), ip) {
        (Ok(IpAddr::V4(network)), IpAddr::V4(ip)) => {
            let length = length.unwrap_or(32);
            length <= 32 && mask_v4(u32::from(network), length) == mask_v4(u32::from(ip), length)
        }
        (Ok(IpAddr::V6(network)), IpAddr::V6(ip)) => {
            let length = length.unwrap_or(128);
            length <= 128 && mask_v6(u128::from(network), length) == mask_v6(u128::from(ip), length)
        }
        _ => false,
    }
}

fn mask_v4(address: u32, length: u32) -> u32 {
    address.checked_shr(32 - length).unwrap_or(0)
}

fn mask_v6(address: u128, length: u32) -> u128 {
    address.checked_shr(128 - length).unwrap_or(0)
}

/// A public key an access logs in with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AccessKey {
    pub id: i64,
    pub algorithm: String,
    /// `SHA256:` and the digest, as OpenSSH prints it.
    pub fingerprint: String,
    pub comment: String,
    pub created_at: jiff::Timestamp,
    pub expires_at: Option<jiff::Timestamp>,
    pub last_used_at: Option<jiff::Timestamp>,
    pub last_used_ip: Option<String>,
}

/// What the signed-in account reaches over SFTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MyFileAccess {
    /// The name to log in with: the username.
    pub login: String,
    pub keys: Vec<AccessKey>,
    pub groups: Vec<FileGroup>,
}

/// A public key to register: one line of an OpenSSH `.pub` file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AddKeyRequest {
    pub public_key: String,
}

/// A group of rules, as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileGroup {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub rules: Vec<Rule>,
    pub reach: Reach,
    pub version: i64,
}

/// A group to create, or to save at the version it was read at.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct FileGroupRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub rules: Vec<Rule>,
    /// Required to save an existing group.
    #[serde(default)]
    #[ts(optional)]
    pub version: Option<i64>,
}

/// The groups an account is to hold, all of them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct UserGroupsRequest {
    pub groups: Vec<i64>,
}

/// Whether a change gives an access more than it had, or only takes some away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessChange {
    Give,
    TakeAway,
}

/// The account whose access is edited; `None` for a pipeline's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessHolder {
    pub user_id: i64,
    pub is_owner: bool,
    pub permissions: Permissions,
}

/// Whether `editor` may make `change` to the access of `holder`. The owner edits any access,
/// their own included. Anyone else never edits their own (`SelfEdit`), never the owner's
/// (`OwnerProtected`), and gives access only to an account holding no panel permission they lack
/// (`Hierarchy`); taking access away skips that last rule. A pipeline's access belongs to no
/// account: any manager may take from it, and only the owner gives to it.
pub fn may_manage(
    editor: &Account,
    holder: Option<&AccessHolder>,
    change: AccessChange,
) -> Result<(), ErrorCode> {
    if editor.is_owner {
        return Ok(());
    }
    let Some(holder) = holder else {
        return match change {
            AccessChange::Give => Err(ErrorCode::OwnerOnly),
            AccessChange::TakeAway => Ok(()),
        };
    };
    if holder.user_id == editor.id {
        return Err(ErrorCode::SelfEdit);
    }
    if holder.is_owner {
        return Err(ErrorCode::OwnerProtected);
    }
    if change == AccessChange::Give && !editor.permissions.contains(holder.permissions) {
        return Err(ErrorCode::Hierarchy);
    }
    Ok(())
}

/// Whether removing `removed` from an access that keeps `kept` lifts a denial: the removed group
/// denies a right that a kept group allows. Rights are compared, folders and names are not, so the
/// answer errs toward asking the owner.
pub fn lifts_denial(removed: &[Rule], kept: &[&[Rule]]) -> bool {
    let denied = removed
        .iter()
        .filter(|rule| rule.effect == Effect::Deny)
        .fold(Rights::NONE, |rights, rule| rights.union(rule.rights.denied()));
    kept.iter()
        .flat_map(|rules| rules.iter())
        .any(|rule| rule.effect == Effect::Allow && rule.rights.iter().any(|right| denied.contains(right)))
}

#[cfg(test)]
mod tests;
