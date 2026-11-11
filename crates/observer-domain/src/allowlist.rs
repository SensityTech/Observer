//! The allowlist: the players the staff approved, who alone get in while it is on, what the others
//! asked, and the message the owner writes for them.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::bans::{CODE_ALPHABET, clean_message, fill};
use crate::players::{Search, SearchRefusal};

/// The longest note the staff writes beside a licence approved ahead of a first visit.
pub const MAX_NOTE_CHARS: usize = 64;
/// What `{id}` reads for a player who shows no licence, and so can ask nothing.
pub const NO_REQUEST: &str = "none";
const DEFAULT_MESSAGE: &str =
    "This server is open to approved players only.\nAsk the staff for access, and give them this ID: {id}";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AllowlistSettings {
    /// Whether only approved players get in.
    pub enabled: bool,
    /// What a player who is not approved reads, with `{id}` and `{name}` filled in.
    pub message: String,
}

impl Default for AllowlistSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            message: DEFAULT_MESSAGE.to_owned(),
        }
    }
}

impl AllowlistSettings {
    /// The settings as they are kept, or `None` for a message empty or too long.
    #[must_use]
    pub fn checked(&self) -> Option<Self> {
        Some(Self {
            enabled: self.enabled,
            message: clean_message(&self.message)?,
        })
    }
}

/// What a player who is not approved reads: the owner's message with the request's code and the
/// player's name filled in.
pub fn refusal(message: &str, request: &str, name: &str) -> String {
    fill(message, &[("{id}", request), ("{name}", name)])
}

/// A request's code from six random bytes: `XXX-XXX`, shorter than a ban's so that nobody takes
/// one for the other.
pub fn request_code(random: [u8; 6]) -> String {
    let mut code = String::with_capacity(7);
    for (index, byte) in random.into_iter().enumerate() {
        if index == 3 {
            code.push('-');
        }
        code.push(char::from(CODE_ALPHABET[usize::from(byte & 31)]));
    }
    code
}

/// A request's code as someone typed it, in any case, with or without its dash.
pub fn parse_request_code(text: &str) -> Option<String> {
    let compact: String = text.trim().chars().filter(|&c| c != '-').collect();
    let compact = compact.to_ascii_uppercase();
    if compact.len() != 6 || !compact.bytes().all(|byte| CODE_ALPHABET.contains(&byte)) {
        return None;
    }
    Some(format!("{}-{}", &compact[..3], &compact[3..]))
}

/// A licence as someone typed it: `license:` and 40 hex digits in any case, or the digits alone.
pub fn parse_license(text: &str) -> Option<String> {
    match Search::parse(text) {
        Ok(Some(Search::License(license))) => Some(license),
        _ => None,
    }
}

/// What a refused player asked: one per licence, kept until the staff answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AllowlistRequest {
    /// What the player quotes to the staff, `7KQ-2XP`.
    pub code: String,
    pub license: String,
    pub name: Option<String>,
    pub first_at: Timestamp,
    pub last_at: Timestamp,
    /// Connections refused since the request was made, the first included.
    pub tries: u32,
}

/// A licence the staff approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Approval {
    pub id: i64,
    pub license: String,
    /// The player's profile, once Observer has seen them.
    pub player: Option<i64>,
    /// Their latest name, or the one their request carried.
    pub name: Option<String>,
    /// Who it is, as the staff wrote it when approving ahead of a first visit.
    pub note: Option<String>,
    pub by: String,
    pub approved_at: Timestamp,
    /// The code of the request it answered.
    pub request: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RequestPage {
    /// Latest try first.
    pub requests: Vec<AllowlistRequest>,
    /// What to pass as `after` for the next page.
    pub next: Option<String>,
    /// The time zone the page's times are read in: the schedule's.
    pub zone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApprovedPage {
    /// Latest approved first.
    pub approved: Vec<Approval>,
    pub next: Option<String>,
    pub zone: String,
}

/// `POST /api/servers/default/allowlist/approved`: a licence approved ahead of a first visit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ApproveRequest {
    pub license: String,
    pub note: Option<String>,
}

/// `POST /api/servers/default/allowlist/requests/deny`: every request last tried at `until` or
/// before, so that one made while the staff was confirming stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DenyRequests {
    pub until: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeniedRequests {
    pub denied: u32,
}

/// What the panel shows of the allowlist wherever it is: whether it is on, and how much it holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AllowlistStatus {
    pub enabled: bool,
    /// Requests waiting for an answer.
    pub waiting: u32,
    pub approved: u32,
}

/// What someone typed into the allowlist's search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowlistSearch {
    License(String),
    /// The start of a name or words of a note, and a request's code when the text reads as one:
    /// a name of six letters does.
    Text {
        text: String,
        code: Option<String>,
    },
}

impl AllowlistSearch {
    /// Reads a search as the players' search does. The allowlist goes by the licence alone, so
    /// another identifier finds nothing and is refused.
    pub fn parse(text: &str) -> Result<Option<Self>, SearchRefusal> {
        let code = parse_request_code(text);
        match Search::parse(text)? {
            Some(Search::Name(text)) => Ok(Some(Self::Text { text, code })),
            Some(Search::License(license)) => Ok(Some(Self::License(license))),
            Some(Search::Identifier(_)) => Err(SearchRefusal::Malformed),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bans::{MAX_MESSAGE_CHARS, MAX_REFUSAL_BYTES};

    #[test]
    fn the_refusal_gives_the_request_and_the_name_and_leaves_other_braces() {
        let text = refusal("Hello {name}. ID: {id} {reason} {", "7KQ-2XP", "kaito {id}");

        assert_eq!(text, "Hello kaito {id}. ID: 7KQ-2XP {reason} {");
        assert!(refusal(&"é".repeat(MAX_REFUSAL_BYTES), "7KQ-2XP", "kaito").len() <= MAX_REFUSAL_BYTES);
    }

    #[test]
    fn the_settings_keep_a_clean_message_and_refuse_an_empty_or_long_one() {
        let settings = AllowlistSettings {
            enabled: true,
            message: "\n Ask us\u{202E}: {id}  \n\n".to_owned(),
        };
        let blank = AllowlistSettings {
            message: " \n\u{200B}".to_owned(),
            ..AllowlistSettings::default()
        };
        let long = AllowlistSettings {
            message: "a".repeat(MAX_MESSAGE_CHARS + 1),
            ..AllowlistSettings::default()
        };

        assert_eq!(
            settings.checked(),
            Some(AllowlistSettings {
                enabled: true,
                message: " Ask us: {id}".to_owned(),
            })
        );
        assert_eq!(blank.checked(), None);
        assert_eq!(long.checked(), None);
        let default = AllowlistSettings::default();
        assert!(!default.enabled && default.message.contains("{id}"));
        assert_eq!(default.checked(), Some(default));
    }

    #[test]
    fn request_codes_use_the_bans_alphabet_and_read_back_in_any_case() {
        let code = request_code([0, 1, 31, 32, 255, 9]);

        assert_eq!(code, "AB9-A9K");
        assert_eq!(parse_request_code("ab9a9k"), Some(code.clone()));
        assert_eq!(parse_request_code(" AB9-A9K "), Some(code));
        for invalid in ["AB9-A9", "AB9-A9KK", "ABO-A9K", "AB1-A9K", "", "K7QM-2XPA"] {
            assert_eq!(parse_request_code(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn a_licence_is_read_with_or_without_its_prefix_and_nothing_else_is_one() {
        let hex = "0123456789ABCDEF0123456789abcdef01234567";
        let license = format!("license:{}", hex.to_ascii_lowercase());

        assert_eq!(parse_license(hex), Some(license.clone()));
        assert_eq!(parse_license(&format!(" License:{hex} ")), Some(license));
        for invalid in [
            "",
            "kaito",
            "license:abc",
            "discord:123456789012345678",
            "ip:1.2.3.4",
        ] {
            assert_eq!(parse_license(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn a_search_reads_a_licence_or_a_text_that_may_be_a_code_and_refuses_other_identifiers() {
        assert_eq!(
            AllowlistSearch::parse("7kq-2xp"),
            Ok(Some(AllowlistSearch::Text {
                text: "7kq-2xp".to_owned(),
                code: Some("7KQ-2XP".to_owned())
            }))
        );
        assert_eq!(
            AllowlistSearch::parse("Nathan"),
            Ok(Some(AllowlistSearch::Text {
                text: "Nathan".to_owned(),
                code: Some("NAT-HAN".to_owned())
            }))
        );
        assert_eq!(
            AllowlistSearch::parse(&"d".repeat(40)),
            Ok(Some(AllowlistSearch::License(format!(
                "license:{}",
                "d".repeat(40)
            ))))
        );
        assert_eq!(
            AllowlistSearch::parse(" Sarah "),
            Ok(Some(AllowlistSearch::Text {
                text: "Sarah".to_owned(),
                code: None
            }))
        );
        assert_eq!(
            AllowlistSearch::parse("discord:123456789012345678"),
            Err(SearchRefusal::Malformed)
        );
        assert_eq!(AllowlistSearch::parse("ip:1.2.3.4"), Err(SearchRefusal::Address));
        assert_eq!(AllowlistSearch::parse("  "), Ok(None));
    }
}
