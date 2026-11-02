//! Players as Observer sees them: the identifiers they show, checked and put in one canonical form,
//! their names made safe to display, and what the panel reads of their profiles.
//!
//! The formats are FiveM's, as txAdmin checks them. Anything else is dropped rather than stored as
//! it came: a resource or a modified client can put anything in a report.

use std::net::IpAddr;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::drops::{DropCategory, DropKind};

/// The longest name kept, in bytes.
pub const MAX_NAME_BYTES: usize = 256;
/// The longest label name and label id the server's resources may give, in characters.
pub const MAX_LABEL_NAME_CHARS: usize = 64;
pub const MAX_LABEL_ID_CHARS: usize = 32;

/// The kinds of identifier Observer keeps beside the licence, which is the player's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum IdentifierKind {
    License2,
    Steam,
    Discord,
    Fivem,
    Xbl,
    Live,
    /// A hardware token from `GetPlayerTokens`, already a one-way hash made by FiveM.
    Token,
    /// The keyed fingerprint of an address, never the address.
    Ip,
}

impl IdentifierKind {
    pub const ALL: [Self; 8] = [
        Self::License2,
        Self::Steam,
        Self::Discord,
        Self::Fivem,
        Self::Xbl,
        Self::Live,
        Self::Token,
        Self::Ip,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::License2 => "license2",
            Self::Steam => "steam",
            Self::Discord => "discord",
            Self::Fivem => "fivem",
            Self::Xbl => "xbl",
            Self::Live => "live",
            Self::Token => "token",
            Self::Ip => "ip",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// An identifier other than the licence, in canonical form: lowercase and without its `kind:`
/// prefix. A token keeps its type (`4:<64 hex>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identifier {
    pub kind: IdentifierKind,
    pub value: String,
}

/// What a player shows on a connection, checked one by one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identifiers {
    /// `license:` and 40 hex digits, lowercase.
    pub license: Option<String>,
    /// Without duplicates, in a stable order.
    pub others: Vec<Identifier>,
    /// What was dropped: malformed, of a kind Observer does not keep, or a second licence.
    pub invalid: usize,
}

impl Identifiers {
    /// Sorts the identifiers of a report out of their `kind:value` strings and the player's tokens.
    /// The `ip:` identifier is not among them: it is fingerprinted apart, and never kept.
    pub fn parse<'a>(
        identifiers: impl IntoIterator<Item = &'a str>,
        tokens: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut parsed = Self::default();
        for text in identifiers {
            match parse_identifier(text) {
                Some(Parsed::License(license)) if parsed.license.is_none() => {
                    parsed.license = Some(license);
                }
                Some(Parsed::License(license)) if parsed.license.as_ref() == Some(&license) => {}
                Some(Parsed::Other(identifier)) => parsed.others.push(identifier),
                Some(Parsed::License(_)) | None => parsed.invalid += 1,
            }
        }
        for text in tokens {
            match token(text) {
                Some(value) => parsed.others.push(Identifier {
                    kind: IdentifierKind::Token,
                    value,
                }),
                None => parsed.invalid += 1,
            }
        }
        parsed.others.sort();
        parsed.others.dedup();
        parsed
    }
}

enum Parsed {
    License(String),
    Other(Identifier),
}

fn parse_identifier(text: &str) -> Option<Parsed> {
    let text = text.trim().to_ascii_lowercase();
    let (kind, value) = text.split_once(':')?;
    let other = |kind| {
        Some(Parsed::Other(Identifier {
            kind,
            value: value.to_owned(),
        }))
    };
    match kind {
        "license" if is_hex(value, 40, 40) => Some(Parsed::License(format!("license:{value}"))),
        "license2" if is_hex(value, 40, 40) => other(IdentifierKind::License2),
        "steam" if value.len() == 15 && value.starts_with("1100001") && is_hex(&value[7..], 8, 8) => {
            other(IdentifierKind::Steam)
        }
        "discord" if is_digits(value, 17, 20) => other(IdentifierKind::Discord),
        "fivem" if is_digits(value, 1, 8) => other(IdentifierKind::Fivem),
        "xbl" if is_digits(value, 14, 20) => other(IdentifierKind::Xbl),
        "live" if is_digits(value, 14, 20) => other(IdentifierKind::Live),
        _ => None,
    }
}

/// A hardware token: a one- or two-digit type, a colon, then 64 hex digits.
fn token(text: &str) -> Option<String> {
    let text = text.trim().to_ascii_lowercase();
    let (kind, hash) = text.split_once(':')?;
    (is_hex(kind, 1, 2) && is_hex(hash, 64, 64)).then_some(text)
}

fn is_hex(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_digits(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// The part of an `ip:` identifier that says "same address as": the IPv4 address, or the /64 of
/// an IPv6 one, whose privacy addresses change daily inside it. An IPv4-mapped IPv6 address is
/// its IPv4 address. `None` for anything that does not parse.
pub fn address_network(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    let text = text.strip_prefix("ip:").unwrap_or(text);
    let host = match text.strip_prefix('[') {
        Some(bracketed) => bracketed.split_once(']')?.0,
        // One colon is IPv4 with a port; more is IPv6 without brackets, so without a port.
        None if text.matches(':').count() == 1 => text.split_once(':')?.0,
        None => text,
    };
    match host.parse::<IpAddr>().ok()?.to_canonical() {
        IpAddr::V4(address) => Some(address.octets().to_vec()),
        IpAddr::V6(address) => Some(address.octets()[..8].to_vec()),
    }
}

/// A name made safe to show: no control characters, no bidirectional overrides or invisible
/// characters that make one name pass for another, trimmed and cut to 256 bytes. `None` when
/// nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let kept: String = name.chars().filter(|&c| !hidden(c)).collect();
    let kept = kept.trim();
    let mut end = kept.len().min(MAX_NAME_BYTES);
    while !kept.is_char_boundary(end) {
        end -= 1;
    }
    let kept = kept[..end].trim_end();
    (!kept.is_empty()).then(|| kept.to_owned())
}

/// What the server's own resources call a player, given through the bridge: display data, never
/// part of who the player is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerLabel {
    pub name: Option<String>,
    pub id: Option<String>,
    /// The resource that gave it.
    pub resource: Option<String>,
}

impl PlayerLabel {
    /// A label safe to show, from what a resource gave: each text cleaned as a name is, and left
    /// out past its length rather than cut, as a cut id would be another one. `None` when neither
    /// the name nor the id is left.
    pub fn clean(name: Option<&str>, id: Option<&str>, resource: Option<&str>) -> Option<Self> {
        let text = |text: Option<&str>, max: usize| {
            let kept: String = text?.chars().filter(|&c| !hidden(c)).collect();
            let kept = kept.trim();
            (!kept.is_empty() && kept.chars().count() <= max).then(|| kept.to_owned())
        };
        let label = Self {
            name: text(name, MAX_LABEL_NAME_CHARS),
            id: text(id, MAX_LABEL_ID_CHARS),
            resource: text(resource, MAX_LABEL_NAME_CHARS),
        };
        (label.name.is_some() || label.id.is_some()).then_some(label)
    }
}

fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

/// What someone typed into a players search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Search {
    /// The start of a name or of a label's name, or the whole of a label's id.
    Name(String),
    License(String),
    /// Any other identifier, which only those who see identifiers may search by.
    Identifier(Identifier),
}

/// Why a search was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchRefusal {
    /// An address: the panel must not tell which licences used an address someone already knows.
    Address,
    /// A `kind:` prefix Observer knows, with a value that is not of that kind.
    Malformed,
}

impl Search {
    /// Reads a search. A value without its `kind:` prefix is a licence when it is one, never
    /// another kind: a bare number stays a name rather than turning into a Discord search.
    pub fn parse(text: &str) -> Result<Option<Self>, SearchRefusal> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("ip:") || text.parse::<IpAddr>().is_ok() {
            return Err(SearchRefusal::Address);
        }
        if is_hex(&lower, 40, 40) {
            return Ok(Some(Self::License(format!("license:{lower}"))));
        }
        if let Some((kind, _)) = lower.split_once(':') {
            let known = kind == "license" || IdentifierKind::from_name(kind).is_some();
            if token(&lower).is_some() {
                return Ok(Some(Self::Identifier(Identifier {
                    kind: IdentifierKind::Token,
                    value: lower,
                })));
            }
            if known {
                return match parse_identifier(&lower) {
                    Some(Parsed::License(license)) => Ok(Some(Self::License(license))),
                    Some(Parsed::Other(identifier)) => Ok(Some(Self::Identifier(identifier))),
                    None => Err(SearchRefusal::Malformed),
                };
            }
        }
        Ok(clean_name(text).map(Self::Name))
    }
}

/// How the players list is sorted, largest or latest first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PlayerSort {
    /// Total play time, imported time included.
    #[default]
    PlayTime,
    /// Play time in the last 30 days.
    Recent,
    LastSeen,
    FirstSeen,
    Sessions,
}

/// A player in the players list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerSummary {
    pub id: i64,
    pub name: String,
    pub license: String,
    /// Time connected that Observer saw, in milliseconds.
    pub play_ms: i64,
    /// From txAdmin's players database.
    pub imported_ms: i64,
    /// Play time in the last 30 days.
    pub recent_ms: i64,
    /// The session under way, in neither of the two above yet.
    pub in_progress_ms: i64,
    pub sessions: i64,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerPage {
    pub players: Vec<PlayerSummary>,
    /// What to pass as `after` for the next page, if there is one.
    pub next: Option<String>,
    /// The schedule's time zone, which visits are shown in.
    pub zone: String,
}

/// Whether FXServer's own player list confirmed what the bridge reported of a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Confirmation {
    Confirming,
    Confirmed,
    /// It did not match: the session has no identity, no play time and no link.
    Unconfirmed,
}

/// A player connected now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OnlinePlayer {
    pub net_id: u32,
    pub confirmation: Confirmation,
    /// The profile, once the identity is confirmed and written.
    pub player: Option<i64>,
    pub name: Option<String>,
    pub license: Option<String>,
    /// When Observer saw the session start, or first saw it.
    pub since: Timestamp,
    pub session_ms: i64,
    /// Total play time with this session, imported time included.
    pub total_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OnlinePlayers {
    pub players: Vec<OnlinePlayer>,
}

/// Unique players, new players and play time of a day, a date in the schedule's time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommunityDay {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub players: i64,
    pub new_players: i64,
    pub play_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommunityTotals {
    pub players: i64,
    pub new_players: i64,
    pub play_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Community {
    /// The schedule's time zone, which days are dates in.
    pub zone: String,
    /// Today in that zone: its figures are today's so far.
    pub today: String,
    /// The last 30 days, today included, oldest first, days without play included.
    pub days: Vec<CommunityDay>,
    /// The last 7 days, today included, and the 7 before.
    pub week: CommunityTotals,
    pub previous_week: CommunityTotals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerName {
    pub name: String,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
}

/// A label the server's resources gave the player, first and last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SeenLabel {
    pub name: Option<String>,
    pub id: Option<String>,
    /// The resource that gave it last.
    pub resource: Option<String>,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayDay {
    pub day: String,
    pub play_ms: i64,
}

/// A session of the player that ended, within the sessions' retention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerDrop {
    pub ended_at: Timestamp,
    pub category: DropCategory,
    pub kind: Option<DropKind>,
    /// The resource that kicked the player.
    pub resource: Option<String>,
    pub length_ms: Option<i64>,
}

/// An identifier a player showed, with how many other licences showed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShownIdentifier {
    pub kind: IdentifierKind,
    /// `None` for an address: only its fingerprint is kept, and it is not shown.
    pub value: Option<String>,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub times: i64,
    /// From txAdmin and not seen by Observer since.
    pub imported: bool,
    pub other_licences: i64,
}

/// Why two licences are linked: an identifier they share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Link {
    pub kind: IdentifierKind,
    pub value: Option<String>,
    pub strong: bool,
}

/// Another licence linked to the player's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LinkedLicence {
    pub player: i64,
    pub name: String,
    pub license: String,
    /// The identifiers shared with the licence it was reached from.
    pub through: Vec<Link>,
    /// Reached through strong links only.
    pub strong: bool,
    /// 1 when it shares an identifier with the player, 2 through another licence.
    pub steps: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerProfile {
    pub id: i64,
    pub name: String,
    pub license: String,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    /// When txAdmin's record was taken in, if it was.
    pub imported_at: Option<Timestamp>,
    pub online: bool,
    pub play_ms: i64,
    pub imported_ms: i64,
    /// The session under way, not in `play_ms` yet.
    pub in_progress_ms: i64,
    /// The last 7 and 30 days, the session under way included.
    pub last7_ms: i64,
    pub last30_ms: i64,
    pub sessions: i64,
    pub longest_ms: i64,
    pub crashes: i64,
    pub timeouts: i64,
    pub kicks: i64,
    /// Crashes per 100 hours of the play time Observer saw.
    pub crash_rate: Option<f64>,
    /// The server's, over the sessions kept.
    pub server_crash_rate: Option<f64>,
    pub names: Vec<PlayerName>,
    /// The days of the last 90 with play, the session under way included.
    pub days: Vec<PlayDay>,
    pub drops: Vec<PlayerDrop>,
    /// Only with the permission to see identifiers.
    pub identifiers: Option<Vec<ShownIdentifier>>,
    pub linked: Option<Vec<LinkedLicence>>,
    /// The schedule's time zone, which days are dates in.
    pub zone: String,
    pub today: String,
}

/// Where txAdmin's players database is on the host: `<txData>/<profile>/data/playersDB.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PlayersImportRequest {
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ImportState {
    /// The file was read and checked whole; its players are being written.
    Writing,
    Finished,
    /// The writing stopped; running the import again finishes it, since every write is idempotent.
    Failed,
}

/// The txAdmin import under way, or the last one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayersImport {
    pub state: ImportState,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub bytes: u64,
    /// Records in the file.
    pub read: u64,
    /// Records written so far.
    pub written: u64,
    /// New profiles, and profiles Observer already had that took txAdmin's record in.
    pub imported: u64,
    pub updated: u64,
    /// Skipped: the licence was forgotten on request.
    pub forgotten: u64,
    /// Skipped: no valid licence.
    pub invalid: u64,
}

/// Why a players database was refused, whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PlayersFileProblem {
    Missing,
    /// A link, a pipe, a device or a folder: only a regular file is read.
    NotAFile,
    /// Over 512 MiB.
    TooLarge,
    Unreadable,
    /// Not txAdmin's players database, or cut short.
    NotPlayersDatabase,
    /// Written by a txAdmin whose format Observer does not read: open it once with a current
    /// txAdmin, then import it again.
    UnsupportedVersion,
    /// Not an absolute path on this host: a network share or a relative path is refused.
    NotLocal,
}

/// How long player data is kept, in days. A profile kept for good has `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayersSettings {
    /// Each link between a licence and an identifier or a name, after it was last seen.
    pub link_days: u32,
    /// Links to an address's fingerprint.
    pub address_days: u32,
    /// Play time per day.
    pub day_days: u32,
    /// A profile, after the player's last visit.
    pub profile_days: Option<u32>,
}

impl Default for PlayersSettings {
    fn default() -> Self {
        Self {
            link_days: 365,
            address_days: 90,
            day_days: 400,
            profile_days: Some(730),
        }
    }
}

/// Retentions out of their ranges: links 30 days to the profile's retention, addresses 7 days to
/// the links', days 30 days to the profile's, profiles 365 days or more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

impl PlayersSettings {
    /// The settings brought into their ranges, each as close as it can stay to what was stored.
    #[must_use]
    pub fn clamped(self) -> Self {
        const MOST: u32 = 36_500;
        let profile_days = self.profile_days.map(|days| days.clamp(365, MOST));
        let profile = profile_days.unwrap_or(MOST);
        let link_days = self.link_days.clamp(30, profile);
        Self {
            profile_days,
            link_days,
            address_days: self.address_days.clamp(7, link_days),
            day_days: self.day_days.clamp(30, profile),
        }
    }

    /// None outlives what it hangs on.
    pub fn check(&self) -> Result<(), OutOfRange> {
        const MOST: u32 = 36_500;
        let profile = self.profile_days.unwrap_or(MOST).min(MOST);
        let valid = self.profile_days.is_none_or(|days| (365..=MOST).contains(&days))
            && (30..=profile).contains(&self.link_days)
            && (7..=self.link_days).contains(&self.address_days)
            && (30..=profile).contains(&self.day_days);
        if valid { Ok(()) } else { Err(OutOfRange) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retentions_never_outlive_what_they_hang_on() {
        let defaults = PlayersSettings::default();

        assert_eq!(defaults.check(), Ok(()));
        for settings in [
            PlayersSettings {
                link_days: 29,
                ..defaults
            },
            PlayersSettings {
                address_days: 366,
                ..defaults
            },
            PlayersSettings {
                address_days: 6,
                ..defaults
            },
            PlayersSettings {
                day_days: 731,
                ..defaults
            },
            PlayersSettings {
                profile_days: Some(364),
                ..defaults
            },
            PlayersSettings {
                profile_days: Some(400),
                link_days: 401,
                ..defaults
            },
        ] {
            assert_eq!(settings.check(), Err(OutOfRange), "{settings:?}");
        }
        let kept = PlayersSettings {
            profile_days: None,
            link_days: 3000,
            day_days: 3000,
            ..defaults
        };
        assert_eq!(kept.check(), Ok(()));
    }

    #[test]
    fn a_setting_out_of_its_range_is_brought_back_without_shortening_what_was_kept() {
        let edited = PlayersSettings {
            link_days: 4_000_000_000,
            address_days: 1,
            day_days: 10,
            profile_days: None,
        };

        let clamped = edited.clamped();

        assert_eq!(
            clamped,
            PlayersSettings {
                link_days: 36_500,
                address_days: 7,
                day_days: 30,
                profile_days: None,
            }
        );
        assert_eq!(clamped.check(), Ok(()));
    }

    const LICENSE: &str = "license:0123456789abcdef0123456789abcdef01234567";
    const TOKEN: &str = "4:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn others(parsed: &Identifiers) -> Vec<(IdentifierKind, &str)> {
        parsed
            .others
            .iter()
            .map(|identifier| (identifier.kind, identifier.value.as_str()))
            .collect()
    }

    #[test]
    fn each_kind_is_checked_against_its_format_and_lowercased() {
        let parsed = Identifiers::parse(
            [
                "license:0123456789ABCDEF0123456789abcdef01234567",
                "license2:0123456789abcdef0123456789abcdef01234567",
                "steam:110000112345ABC",
                "discord:123456789012345678",
                "fivem:1234",
                "xbl:12345678901234",
                "live:98765432109876",
            ],
            [TOKEN],
        );

        assert_eq!(parsed.license.as_deref(), Some(LICENSE));
        assert_eq!(
            others(&parsed),
            [
                (
                    IdentifierKind::License2,
                    "0123456789abcdef0123456789abcdef01234567"
                ),
                (IdentifierKind::Steam, "110000112345abc"),
                (IdentifierKind::Discord, "123456789012345678"),
                (IdentifierKind::Fivem, "1234"),
                (IdentifierKind::Xbl, "12345678901234"),
                (IdentifierKind::Live, "98765432109876"),
                (IdentifierKind::Token, TOKEN),
            ]
        );
        assert_eq!(parsed.invalid, 0);
    }

    #[test]
    fn malformed_values_are_dropped_and_counted() {
        let parsed = Identifiers::parse(
            [
                "license:0123",
                "steam:2100001aabbccdd",
                "discord:1234",
                "fivem:123456789",
                "xbl:12",
                "ip:127.0.0.1",
                "unknown:abc",
                "no prefix",
            ],
            [
                "4:abc",
                "123:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            ],
        );

        assert_eq!(parsed.license, None);
        assert_eq!(parsed.others, []);
        assert_eq!(parsed.invalid, 10);
    }

    #[test]
    fn a_token_type_may_have_two_digits() {
        let two = "12:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef";

        let parsed = Identifiers::parse([], [two]);

        assert_eq!(parsed.others[0].value, two.to_ascii_lowercase());
    }

    #[test]
    fn duplicates_within_one_report_count_once() {
        let parsed = Identifiers::parse(
            [
                LICENSE,
                LICENSE,
                "discord:123456789012345678",
                "DISCORD:123456789012345678",
            ],
            [TOKEN, TOKEN],
        );

        assert_eq!(parsed.license.as_deref(), Some(LICENSE));
        assert_eq!(parsed.others.len(), 2);
        assert_eq!(parsed.invalid, 0);
    }

    #[test]
    fn a_second_licence_is_dropped() {
        let parsed = Identifiers::parse([LICENSE, "license:ffffffffffffffffffffffffffffffffffffffff"], []);

        assert_eq!(parsed.license.as_deref(), Some(LICENSE));
        assert_eq!(parsed.invalid, 1);
    }

    #[test]
    fn ipv4_mapped_and_bracketed_addresses_are_one_network() {
        let plain = address_network("ip:192.0.2.7");

        assert_eq!(plain, Some(vec![192, 0, 2, 7]));
        assert_eq!(address_network("ip:::ffff:192.0.2.7"), plain);
        assert_eq!(address_network("ip:[::ffff:192.0.2.7]:30120"), plain);
        assert_eq!(address_network("ip:192.0.2.7:30120"), plain);
    }

    #[test]
    fn ipv6_addresses_are_kept_to_their_64_bit_network() {
        let one = address_network("ip:2001:db8:1:2:aaaa:bbbb:cccc:dddd");
        let other = address_network("ip:[2001:db8:1:2:1111:2222:3333:4444]:30120");

        assert_eq!(one, Some(vec![0x20, 0x01, 0x0d, 0xb8, 0, 1, 0, 2]));
        assert_eq!(one, other);
        assert_ne!(one, address_network("ip:2001:db8:1:3::1"));
    }

    #[test]
    fn an_address_that_does_not_parse_is_none() {
        for text in ["ip:", "ip:localhost", "ip:300.1.1.1", "ip:[::1", "ip:1.2.3"] {
            assert_eq!(address_network(text), None, "{text}");
        }
    }

    #[test]
    fn names_lose_controls_bidi_overrides_and_invisible_characters() {
        assert_eq!(
            clean_name("  Ada\u{202E}nim\u{200B}da\u{0007}\n "),
            Some("Adanimda".to_owned())
        );
        assert_eq!(clean_name("\u{200B}\u{FEFF} \t"), None);
        assert_eq!(clean_name(""), None);
    }

    #[test]
    fn a_long_name_is_cut_on_a_character_boundary() {
        let name = "é".repeat(200);

        let cleaned = clean_name(&name).unwrap();

        assert_eq!(cleaned.len(), MAX_NAME_BYTES);
        assert_eq!(cleaned, "é".repeat(128));
    }

    #[test]
    fn a_label_loses_what_names_lose_and_keeps_its_resource() {
        assert_eq!(
            PlayerLabel::clean(
                Some(" John\u{202E} Doe\u{0007} "),
                Some("\u{200B}CHAR-1234\n"),
                Some("characters")
            ),
            Some(PlayerLabel {
                name: Some("John Doe".to_owned()),
                id: Some("CHAR-1234".to_owned()),
                resource: Some("characters".to_owned()),
            })
        );
    }

    #[test]
    fn a_label_text_past_its_length_is_left_out_and_never_cut() {
        let name = "é".repeat(MAX_LABEL_NAME_CHARS);
        let id = "9".repeat(MAX_LABEL_ID_CHARS);

        let kept = PlayerLabel::clean(Some(&name), Some(&id), None).unwrap();
        let past = PlayerLabel::clean(Some(&format!("{name}é")), Some(&format!("{id}9")), None);
        let id_past = PlayerLabel::clean(Some("John Doe"), Some(&format!("{id}9")), None);

        assert_eq!((kept.name, kept.id), (Some(name), Some(id)));
        assert_eq!(past, None);
        assert_eq!(id_past.map(|label| label.id), Some(None));
    }

    #[test]
    fn a_label_with_nothing_left_to_show_is_none() {
        assert_eq!(PlayerLabel::clean(None, None, Some("characters")), None);
        assert_eq!(PlayerLabel::clean(Some(" \u{200B}"), Some("\u{FEFF}"), None), None);
    }

    #[test]
    fn a_bare_number_is_a_name_and_never_a_discord_search() {
        assert_eq!(
            Search::parse("123456789012345678"),
            Ok(Some(Search::Name("123456789012345678".to_owned())))
        );
        assert_eq!(
            Search::parse("discord:123456789012345678"),
            Ok(Some(Search::Identifier(Identifier {
                kind: IdentifierKind::Discord,
                value: "123456789012345678".to_owned()
            })))
        );
    }

    #[test]
    fn a_licence_is_found_with_or_without_its_prefix() {
        let bare = "0123456789ABCDEF0123456789abcdef01234567";

        assert_eq!(Search::parse(bare), Ok(Some(Search::License(LICENSE.to_owned()))));
        assert_eq!(
            Search::parse(LICENSE),
            Ok(Some(Search::License(LICENSE.to_owned())))
        );
    }

    #[test]
    fn a_search_never_takes_an_address() {
        for text in ["ip:192.0.2.7", "192.0.2.7", "2001:db8::1", "IP:10.0.0.1"] {
            assert_eq!(Search::parse(text), Err(SearchRefusal::Address), "{text}");
        }
    }

    #[test]
    fn a_known_prefix_with_a_wrong_value_is_refused() {
        assert_eq!(Search::parse("discord:12"), Err(SearchRefusal::Malformed));
        assert_eq!(Search::parse("license:xyz"), Err(SearchRefusal::Malformed));
        assert_eq!(Search::parse("   "), Ok(None));
        assert_eq!(
            Search::parse("Ada: the first"),
            Ok(Some(Search::Name("Ada: the first".to_owned())))
        );
    }

    #[test]
    fn a_token_is_searched_as_a_token() {
        assert_eq!(
            Search::parse(TOKEN),
            Ok(Some(Search::Identifier(Identifier {
                kind: IdentifierKind::Token,
                value: TOKEN.to_owned()
            })))
        );
    }
}
