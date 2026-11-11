//! Bans: who the server refuses, for how long and why, and the refusal message the owner writes,
//! the one text Observer ever shows players.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::players::{Identifiers, Search, SearchRefusal, hidden};

pub const MAX_REASON_CHARS: usize = 200;
pub const MAX_LIFT_REASON_CHARS: usize = 200;
pub const MAX_MESSAGE_CHARS: usize = 600;
/// The longest ban short of a permanent one: ten years.
pub const MAX_BAN_MINUTES: u32 = 3650 * 24 * 60;
pub const MAX_TOKEN_MATCHES: u8 = 8;
/// The most identifiers a ban by hand may name.
pub const MAX_BANNED_IDENTIFIERS: usize = 64;
pub const MAX_BAN_TEMPLATES: usize = 50;
/// The longest refusal sent to the server once its placeholders are filled in, in bytes: it goes
/// to the console with the event that comes with it, within what the console takes.
pub const MAX_REFUSAL_BYTES: usize = 700;
/// Unambiguous when read aloud or typed from a screenshot: no I, O, 0 or 1.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const DEFAULT_MESSAGE: &str =
    "You are banned from this server.\nReason: {reason}\nEnds: {ends}\nBan ID: {id}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BanState {
    Active,
    /// Its time ran out.
    Ended,
    Lifted,
}

/// Which bans a list shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BanFilter {
    #[default]
    Active,
    Ended,
    Lifted,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Ban {
    /// What the player quotes to appeal, `K7QM-2XPA`.
    pub code: String,
    /// The profile the ban was made from, while Observer keeps it.
    pub player: Option<i64>,
    pub name: Option<String>,
    pub license: Option<String>,
    /// Identifiers held besides the licence, tokens apart.
    pub identifiers: u32,
    pub tokens: u32,
    pub reason: String,
    pub by: String,
    pub created_at: Timestamp,
    /// `None` for a permanent ban.
    pub ends_at: Option<Timestamp>,
    pub state: BanState,
    pub lifted: Option<BanLift>,
    /// Connections it refused and players it dropped.
    pub refused: u32,
    /// Each change of its reason or its end since it was made, latest first.
    pub edits: Vec<BanEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BanLift {
    pub at: Timestamp,
    pub by: String,
    pub reason: Option<String>,
}

/// One change of an active ban: who made it, when, and what the ban said before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BanEdit {
    pub at: Timestamp,
    pub by: String,
    /// `None` when the edit left the reason as it was.
    pub reason: Option<BanReasonChange>,
    /// `None` when the edit left the end as it was.
    pub ends: Option<BanEndChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BanReasonChange {
    pub before: String,
    pub after: String,
}

/// Each end is `None` for a permanent ban.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BanEndChange {
    pub before: Option<Timestamp>,
    pub after: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BanCounts {
    pub active: u32,
    pub ended: u32,
    pub lifted: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BanPage {
    /// Latest first.
    pub bans: Vec<Ban>,
    /// What to pass as `after` for the next page.
    pub next: Option<String>,
    /// Over every ban, whatever the filter and the search.
    pub counts: BanCounts,
    /// The time zone the refusal message writes its dates in: the schedule's.
    pub zone: String,
}

/// `POST /api/servers/default/bans`: a ban of a player Observer knows, which holds every
/// identifier and token their profile links to, or of identifiers typed by hand.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BanRequest {
    pub player: Option<i64>,
    #[serde(default)]
    pub identifiers: Vec<String>,
    pub reason: String,
    /// How long, in minutes; `None` for a permanent ban.
    pub minutes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct LiftRequest {
    pub reason: Option<String>,
}

/// `PATCH /api/servers/default/bans/{code}`: a new reason, a new end, or both, for an active ban.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BanEditRequest {
    /// `None` leaves the reason as it is.
    pub reason: Option<String>,
    /// `None` leaves the end as it is.
    pub ends: Option<BanEnd>,
}

/// When an edited ban ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum BanEnd {
    Never,
    /// Counted from the edit, as a new ban's length is counted from the ban.
    In {
        minutes: u32,
    },
}

impl BanEnd {
    /// The length a ban made now would take to end then; `None` for a permanent one.
    pub fn minutes(self) -> Option<u32> {
        match self {
            Self::Never => None,
            Self::In { minutes } => Some(minutes),
        }
    }
}

/// A reason and a length the ban dialog offers, which the staff can still correct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BanTemplate {
    pub reason: String,
    /// How long, in minutes; `None` for a permanent ban.
    pub minutes: Option<u32>,
}

impl BanTemplate {
    /// The template as it is kept, or `None` when no ban could be made from it.
    #[must_use]
    pub fn checked(&self) -> Option<Self> {
        let reason = clean_line(&self.reason, MAX_REASON_CHARS)?;
        lasts(self.minutes).then_some(Self {
            reason,
            minutes: self.minutes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BansSettings {
    /// What a banned player reads, with `{reason}`, `{ends}`, `{id}`, `{name}` and `{by}` filled in.
    pub message: String,
    /// How many of a ban's hardware tokens a player must show to be refused by them alone; 0 for
    /// never.
    pub token_matches: u8,
    /// In the order the ban dialog lists them.
    pub templates: Vec<BanTemplate>,
}

impl Default for BansSettings {
    fn default() -> Self {
        Self {
            message: DEFAULT_MESSAGE.to_owned(),
            token_matches: 1,
            templates: Vec::new(),
        }
    }
}

impl BansSettings {
    /// The settings as they are kept, or `None` when they cannot be: a message empty or too long,
    /// more tokens than a ban is worth, a template no ban could be made from, or too many of
    /// them. A template listed twice is kept once.
    #[must_use]
    pub fn checked(&self) -> Option<Self> {
        let message = clean_message(&self.message)?;
        let mut templates: Vec<BanTemplate> = Vec::with_capacity(self.templates.len());
        for template in &self.templates {
            let template = template.checked()?;
            if !templates.contains(&template) {
                templates.push(template);
            }
        }
        (self.token_matches <= MAX_TOKEN_MATCHES && templates.len() <= MAX_BAN_TEMPLATES).then_some(Self {
            message,
            token_matches: self.token_matches,
            templates,
        })
    }
}

/// Whether a ban may last `minutes`, or for good when there are none.
pub fn lasts(minutes: Option<u32>) -> bool {
    minutes.is_none_or(|minutes| (1..=MAX_BAN_MINUTES).contains(&minutes))
}

/// A text typed in the panel, on one line: hidden characters removed, spaces trimmed. `None`
/// when nothing is left or more than `max` characters are.
pub fn clean_line(text: &str, max: usize) -> Option<String> {
    let kept: String = text.chars().filter(|&c| !hidden(c)).collect();
    let kept = kept.trim();
    (!kept.is_empty() && kept.chars().count() <= max).then(|| kept.to_owned())
}

/// The owner's message, line by line: hidden characters removed from each, blank lines at its
/// ends dropped.
fn clean_message(text: &str) -> Option<String> {
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            line.chars()
                .filter(|&c| !hidden(c))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    let first = lines.iter().position(|line| !line.trim().is_empty())?;
    let last = lines.iter().rposition(|line| !line.trim().is_empty())?;
    let message = lines[first..=last].join("\n");
    (message.chars().count() <= MAX_MESSAGE_CHARS).then_some(message)
}

/// What the refusal message's placeholders stand for.
#[derive(Debug, Clone, Copy)]
pub struct RefusalValues<'a> {
    pub reason: &'a str,
    pub ends: &'a str,
    pub code: &'a str,
    pub name: &'a str,
    pub by: &'a str,
}

/// The message with its placeholders filled in, in one pass: a value that holds `{name}` stays as
/// written. Cut to [`MAX_REFUSAL_BYTES`] on a character boundary.
pub fn refusal(message: &str, values: &RefusalValues<'_>) -> String {
    let placeholders = [
        ("{reason}", values.reason),
        ("{ends}", values.ends),
        ("{id}", values.code),
        ("{name}", values.name),
        ("{by}", values.by),
    ];
    let mut text = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.find('{') {
        text.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some((key, value)) = placeholders.iter().find(|(key, _)| rest.starts_with(key)) {
            text.push_str(value);
            rest = &rest[key.len()..];
        } else {
            text.push('{');
            rest = &rest[1..];
        }
    }
    text.push_str(rest);
    let mut end = text.len().min(MAX_REFUSAL_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// A ban's code from eight random bytes: `XXXX-XXXX`.
pub fn ban_code(random: [u8; 8]) -> String {
    let mut code = String::with_capacity(9);
    for (index, byte) in random.into_iter().enumerate() {
        if index == 4 {
            code.push('-');
        }
        code.push(char::from(CODE_ALPHABET[usize::from(byte & 31)]));
    }
    code
}

/// A code as someone typed it, in any case, with or without its dash.
pub fn parse_ban_code(text: &str) -> Option<String> {
    let compact: String = text.trim().chars().filter(|&c| c != '-').collect();
    let compact = compact.to_ascii_uppercase();
    if compact.len() != 8 || !compact.bytes().all(|byte| CODE_ALPHABET.contains(&byte)) {
        return None;
    }
    Some(format!("{}-{}", &compact[..4], &compact[4..]))
}

/// Whether a ban refuses a player who shows `identifiers` of its identifiers and `tokens` of its
/// hardware tokens.
pub fn refuses(identifiers: u32, tokens: u32, token_matches: u8) -> bool {
    identifiers > 0 || (token_matches > 0 && tokens >= u32::from(token_matches))
}

/// Identifiers typed by hand, one per entry: `kind:value`, or a hardware token.
pub fn parse_banned(texts: &[String]) -> Identifiers {
    let (tokens, identifiers): (Vec<&str>, Vec<&str>) =
        texts.iter().map(|text| text.trim()).partition(|text| {
            text.split_once(':')
                .is_some_and(|(kind, _)| kind.bytes().all(|b| b.is_ascii_digit()))
        });
    Identifiers::parse(identifiers, tokens)
}

/// What someone typed into the bans search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanSearch {
    Code(String),
    /// The start of a name, or words of a reason.
    Text(String),
    /// The licence, or another identifier, which only those who see identifiers may search by.
    Player(Search),
}

impl BanSearch {
    pub fn parse(text: &str) -> Result<Option<Self>, SearchRefusal> {
        if let Some(code) = parse_ban_code(text) {
            return Ok(Some(Self::Code(code)));
        }
        Ok(match Search::parse(text)? {
            Some(Search::Name(name)) => Some(Self::Text(name)),
            Some(search) => Some(Self::Player(search)),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::players::{Identifier, IdentifierKind};

    fn values() -> RefusalValues<'static> {
        RefusalValues {
            reason: "Aimbot {name}",
            ends: "never",
            code: "K7QM-2XPA",
            name: "kaito",
            by: "maxime",
        }
    }

    #[test]
    fn the_refusal_fills_each_placeholder_once_and_leaves_other_braces() {
        let text = refusal("Banned: {reason}. {id}, by {by}. {unknown} {ends}{", &values());

        assert_eq!(
            text,
            "Banned: Aimbot {name}. K7QM-2XPA, by maxime. {unknown} never{"
        );
    }

    #[test]
    fn the_refusal_is_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_REFUSAL_BYTES);

        let text = refusal(&long, &values());

        assert!(text.len() <= MAX_REFUSAL_BYTES);
        assert!(text.chars().all(|c| c == 'é'));
    }

    #[test]
    fn the_message_keeps_its_lines_and_loses_hidden_characters() {
        let settings = BansSettings {
            message: "\n\n  Banned\u{202E} \t\r\n\nReason: {reason}  \n\n".to_owned(),
            token_matches: 2,
            ..BansSettings::default()
        };

        let checked = settings.checked().unwrap();

        assert_eq!(checked.message, "  Banned\n\nReason: {reason}");
        assert_eq!(checked.token_matches, 2);
    }

    #[test]
    fn an_empty_or_long_message_or_too_many_tokens_is_refused() {
        let blank = BansSettings {
            message: " \n\u{200B}\n".to_owned(),
            ..BansSettings::default()
        };
        let long = BansSettings {
            message: "a".repeat(MAX_MESSAGE_CHARS + 1),
            ..BansSettings::default()
        };
        let greedy = BansSettings {
            token_matches: MAX_TOKEN_MATCHES + 1,
            ..BansSettings::default()
        };

        assert_eq!(blank.checked(), None);
        assert_eq!(long.checked(), None);
        assert_eq!(greedy.checked(), None);
        assert!(BansSettings::default().checked().is_some());
    }

    fn template(reason: &str, minutes: Option<u32>) -> BanTemplate {
        BanTemplate {
            reason: reason.to_owned(),
            minutes,
        }
    }

    fn with_templates(templates: Vec<BanTemplate>) -> BansSettings {
        BansSettings {
            templates,
            ..BansSettings::default()
        }
    }

    #[test]
    fn templates_are_kept_in_order_cleaned_and_once_each() {
        let settings = with_templates(vec![
            template("  Cheating\u{200B} ", None),
            template("Insults", Some(3 * 24 * 60)),
            template("Cheating", None),
            template("Cheating", Some(60)),
        ]);

        let checked = settings.checked().unwrap();

        assert_eq!(
            checked.templates,
            [
                template("Cheating", None),
                template("Insults", Some(3 * 24 * 60)),
                template("Cheating", Some(60)),
            ]
        );
    }

    #[test]
    fn a_template_no_ban_could_be_made_from_is_refused_and_so_are_too_many() {
        let many = |count: usize| (0..count).map(|n| template(&format!("Rule {n}"), None)).collect();
        let long = "a".repeat(MAX_REASON_CHARS + 1);

        for unusable in [
            template(" ", None),
            template(&long, None),
            template("Cheating", Some(0)),
            template("Cheating", Some(MAX_BAN_MINUTES + 1)),
        ] {
            assert_eq!(
                with_templates(vec![unusable.clone()]).checked(),
                None,
                "{unusable:?}"
            );
        }
        assert_eq!(with_templates(many(MAX_BAN_TEMPLATES + 1)).checked(), None);
        assert!(with_templates(many(MAX_BAN_TEMPLATES)).checked().is_some());
    }

    #[test]
    fn a_ban_lasts_a_minute_to_ten_years_or_for_good() {
        assert!(lasts(None));
        assert!(lasts(Some(1)));
        assert!(lasts(Some(MAX_BAN_MINUTES)));
        assert!(!lasts(Some(0)));
        assert!(!lasts(Some(MAX_BAN_MINUTES + 1)));
    }

    #[test]
    fn an_edit_names_its_end_by_kind() {
        let sooner: BanEditRequest =
            serde_json::from_str(r#"{"reason":null,"ends":{"kind":"in","minutes":90}}"#).unwrap();
        let for_good: BanEditRequest =
            serde_json::from_str(r#"{"reason":"Cheating","ends":{"kind":"never"}}"#).unwrap();
        let unknown = serde_json::from_str::<BanEditRequest>(r#"{"reason":null,"ends":{"kind":"at"}}"#);

        assert_eq!(sooner.ends.map(BanEnd::minutes), Some(Some(90)));
        assert_eq!(for_good.ends.map(BanEnd::minutes), Some(None));
        assert!(unknown.is_err());
    }

    #[test]
    fn codes_use_their_alphabet_and_read_back_in_any_case() {
        let code = ban_code([0, 1, 2, 31, 32, 255, 8, 9]);

        assert_eq!(code, "ABC9-A9JK");
        assert_eq!(parse_ban_code("abc9a9jk"), Some(code.clone()));
        assert_eq!(parse_ban_code(" ABC9-A9JK "), Some(code));
        for invalid in ["ABC9-A9J", "ABC9-A9JKL", "ABCO-A9JK", "ABC1-A9JK", ""] {
            assert_eq!(parse_ban_code(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn any_identifier_refuses_but_tokens_must_reach_the_setting() {
        assert!(refuses(1, 0, 1));
        assert!(refuses(1, 0, 0));
        assert!(refuses(0, 1, 1));
        assert!(!refuses(0, 1, 2));
        assert!(refuses(0, 2, 2));
        assert!(!refuses(0, 5, 0));
        assert!(!refuses(0, 0, 1));
    }

    #[test]
    fn identifiers_typed_by_hand_are_sorted_out_from_tokens() {
        let token = format!("4:{}", "ab".repeat(32));
        let typed = vec![
            format!("license:{}", "c".repeat(40)),
            "discord:123456789012345678".to_owned(),
            token.clone(),
            "steam:nonsense".to_owned(),
        ];

        let parsed = parse_banned(&typed);

        assert_eq!(parsed.license, Some(format!("license:{}", "c".repeat(40))));
        assert_eq!(
            parsed.others,
            vec![
                Identifier {
                    kind: IdentifierKind::Discord,
                    value: "123456789012345678".to_owned()
                },
                Identifier {
                    kind: IdentifierKind::Token,
                    value: token
                },
            ]
        );
        assert_eq!(parsed.invalid, 1);
    }

    #[test]
    fn a_search_reads_codes_first_then_as_the_players_search_does() {
        assert_eq!(
            BanSearch::parse("k7qm-2xpa"),
            Ok(Some(BanSearch::Code("K7QM-2XPA".to_owned())))
        );
        assert_eq!(
            BanSearch::parse("aimbot"),
            Ok(Some(BanSearch::Text("aimbot".to_owned())))
        );
        assert!(matches!(
            BanSearch::parse(&"d".repeat(40)),
            Ok(Some(BanSearch::Player(Search::License(_))))
        ));
        assert_eq!(BanSearch::parse("ip:1.2.3.4"), Err(SearchRefusal::Address));
        assert_eq!(BanSearch::parse("  "), Ok(None));
    }

    #[test]
    fn lines_typed_in_the_panel_are_trimmed_and_bounded() {
        assert_eq!(clean_line("  Aimbot\u{200E} ", 10), Some("Aimbot".to_owned()));
        assert_eq!(clean_line(" \u{FEFF} ", 10), None);
        assert_eq!(clean_line("abcdef", 5), None);
    }
}
