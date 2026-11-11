//! What the staff does to the players connected now: a kick, with the message the owner writes for
//! it, an announcement to the whole server and a message to one player. A kicked player reads the
//! kick message; announcements and messages are events, which the server's resources display.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::bans::{clean_message, fill};

pub const MAX_KICK_REASON_CHARS: usize = 200;
pub const MAX_KICK_MESSAGE_CHARS: usize = 600;
/// The longest kick message sent to the server once its placeholders are filled in, in bytes: it
/// goes to the console with the event that comes with it, within what the console takes.
pub const MAX_KICK_BYTES: usize = 700;
/// The longest announcement or direct message: at four bytes a character, its event still fits
/// what the console takes.
pub const MAX_TEXT_CHARS: usize = 400;
const DEFAULT_KICK_MESSAGE: &str = "You were kicked from this server.\nReason: {reason}";

/// `POST /api/servers/default/kicks`: kicks one connected player.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct KickRequest {
    /// The player's server id.
    pub player: u32,
    /// The licence the panel showed under that id, when it knew one: the kick is refused if
    /// another player holds the id by now.
    pub license: Option<String>,
    pub reason: String,
}

/// `POST /api/servers/default/kicks/all`: kicks everyone connected.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct KickAllRequest {
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KickedAll {
    /// The players Observer counted as connected when it asked the server to kick everyone.
    pub players: u32,
}

/// `POST /api/servers/default/announcements`: raised for the server's resources to show.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AnnouncementRequest {
    pub message: String,
}

/// `POST /api/servers/default/messages`: a message to one connected player, raised for the
/// server's resources to show.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DirectMessageRequest {
    /// The player's server id.
    pub player: u32,
    /// As in [`KickRequest`].
    pub license: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct KickSettings {
    /// What a kicked player reads, with `{reason}` and `{by}` filled in.
    pub message: String,
}

impl Default for KickSettings {
    fn default() -> Self {
        Self {
            message: DEFAULT_KICK_MESSAGE.to_owned(),
        }
    }
}

impl KickSettings {
    /// The settings as they are kept, or `None` when the message is empty or too long.
    #[must_use]
    pub fn checked(&self) -> Option<Self> {
        clean_message(&self.message, MAX_KICK_MESSAGE_CHARS).map(|message| Self { message })
    }
}

/// What a kicked player reads: the owner's message with its placeholders filled in, in one pass,
/// cut to [`MAX_KICK_BYTES`] on a character boundary.
pub fn kick_message(message: &str, reason: &str, by: &str) -> String {
    fill(message, &[("{reason}", reason), ("{by}", by)], MAX_KICK_BYTES)
}

/// An announcement or a direct message as it is sent: its lines kept, hidden characters and the
/// spaces around it removed. `None` when nothing is left or more than [`MAX_TEXT_CHARS`]
/// characters are.
pub fn clean_text(text: &str) -> Option<String> {
    clean_message(text.trim_start(), MAX_TEXT_CHARS).map(|text| text.trim_start().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kick_message_fills_the_reason_and_who_kicked_once_each() {
        let text = kick_message("Kicked by {by}: {reason}. {name} {", "RDM {by}", "maxime");

        assert_eq!(text, "Kicked by maxime: RDM {by}. {name} {");
    }

    #[test]
    fn the_kick_message_is_cut_on_a_character_boundary() {
        let text = kick_message("{reason}", &"é".repeat(MAX_KICK_BYTES), "maxime");

        assert!(text.len() <= MAX_KICK_BYTES);
        assert!(text.chars().all(|c| c == 'é'));
    }

    #[test]
    fn an_empty_or_long_kick_message_is_refused() {
        let blank = KickSettings {
            message: " \n\u{200B}\n".to_owned(),
        };
        let long = KickSettings {
            message: "a".repeat(MAX_KICK_MESSAGE_CHARS + 1),
        };
        let kept = KickSettings {
            message: "\nKicked\u{202E}: {reason}  \n\n".to_owned(),
        };

        assert_eq!(blank.checked(), None);
        assert_eq!(long.checked(), None);
        assert_eq!(kept.checked().unwrap().message, "Kicked: {reason}");
        assert!(KickSettings::default().checked().is_some());
    }

    #[test]
    fn a_text_keeps_its_lines_and_is_bounded_in_characters() {
        assert_eq!(
            clean_text("\n\u{200B}  Restart at 6\u{200E}\r\n\nPark your cars  \n"),
            Some("Restart at 6\n\nPark your cars".to_owned())
        );
        assert_eq!(clean_text(" \u{FEFF}\n"), None);
        assert!(clean_text(&"🚗".repeat(MAX_TEXT_CHARS)).is_some());
        assert_eq!(clean_text(&"a".repeat(MAX_TEXT_CHARS + 1)), None);
    }
}
