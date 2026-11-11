//! Maintenance mode: the server closed to everyone but the players on a list, with the message the
//! owner writes for the others.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::bans::{clean_line, clean_message, fill};

/// The most players the list of those who still get in holds.
pub const MAX_PASSES: u32 = 200;
/// What FiveM's server list keeps of a server's name.
pub const MAX_SERVER_NAME_CHARS: usize = 40;
const DEFAULT_MESSAGE: &str = "This server is under maintenance.\nCome back a little later.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MaintenanceSettings {
    /// What a player reads when maintenance refuses them, with `{name}` filled in.
    pub message: String,
    /// The name the server takes in FiveM's server list during maintenance; without one it keeps
    /// its own.
    pub server_name: Option<String>,
}

impl Default for MaintenanceSettings {
    fn default() -> Self {
        Self {
            message: DEFAULT_MESSAGE.to_owned(),
            server_name: None,
        }
    }
}

impl MaintenanceSettings {
    /// The settings as they are kept, or `None` for a message empty or too long, or a name too
    /// long. A blank name is no name.
    #[must_use]
    pub fn checked(&self) -> Option<Self> {
        let server_name = match self
            .server_name
            .as_deref()
            .and_then(|name| clean_line(name, usize::MAX))
        {
            Some(name) if name.chars().count() > MAX_SERVER_NAME_CHARS => return None,
            name => name,
        };
        Some(Self {
            message: clean_message(&self.message)?,
            server_name,
        })
    }
}

/// What a player reads when maintenance refuses them: the owner's message with their name filled
/// in.
pub fn refusal(message: &str, name: &str) -> String {
    fill(message, &[("{name}", name)])
}

/// `PUT /api/servers/default/maintenance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SetMaintenance {
    pub on: bool,
}

/// `GET /api/servers/default/maintenance`: what whoever turns maintenance on is shown first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MaintenancePreview {
    /// The owner's message, its placeholders as written.
    pub message: String,
    /// The name the server will take in FiveM's server list, when the owner gave one.
    pub server_name: Option<String>,
}

/// A player who still gets in during maintenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MaintenancePass {
    pub id: i64,
    pub license: String,
    /// The player's profile, once Observer has seen them.
    pub player: Option<i64>,
    pub name: Option<String>,
    /// Who it is, as the owner wrote it beside a licence.
    pub note: Option<String>,
    pub by: String,
    pub added_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MaintenancePasses {
    /// Latest added first.
    pub passes: Vec<MaintenancePass>,
    /// The time zone the list's times are read in: the schedule's.
    pub zone: String,
}

/// `POST /api/servers/default/maintenance/passes`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AddPass {
    pub license: String,
    pub note: Option<String>,
}

/// What the panel shows of maintenance wherever it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MaintenanceStatus {
    pub on: bool,
    /// When it was turned on; absent while it is off.
    pub since: Option<Timestamp>,
    /// Who turned it on.
    pub by: Option<String>,
    /// Players on the list of those who still get in.
    pub passes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bans::MAX_MESSAGE_CHARS;

    #[test]
    fn the_refusal_gives_the_name_and_leaves_other_braces() {
        assert_eq!(
            refusal("Back at 8, {name}. {id} {", "kaito {name}"),
            "Back at 8, kaito {name}. {id} {"
        );
    }

    #[test]
    fn the_settings_keep_a_clean_message_and_refuse_an_empty_or_long_one() {
        let settings = MaintenanceSettings {
            message: "\n Back\u{202E} tonight  \n\n".to_owned(),
            server_name: Some("  Aperture RP\u{200B} | maintenance ".to_owned()),
        };
        let blank = MaintenanceSettings {
            message: " \n\u{200B}".to_owned(),
            server_name: None,
        };
        let long = MaintenanceSettings {
            message: "a".repeat(MAX_MESSAGE_CHARS + 1),
            server_name: None,
        };

        assert_eq!(
            settings.checked(),
            Some(MaintenanceSettings {
                message: " Back tonight".to_owned(),
                server_name: Some("Aperture RP | maintenance".to_owned()),
            })
        );
        assert_eq!(blank.checked(), None);
        assert_eq!(long.checked(), None);
        let default = MaintenanceSettings::default();
        assert_eq!(default.checked(), Some(default));
    }

    #[test]
    fn a_blank_server_name_is_none_and_one_past_what_the_list_keeps_is_refused() {
        let named = |name: &str| MaintenanceSettings {
            server_name: Some(name.to_owned()),
            ..MaintenanceSettings::default()
        };

        assert_eq!(
            named(" \u{200B} ").checked(),
            Some(MaintenanceSettings::default())
        );
        assert_eq!(
            named(&"é".repeat(MAX_SERVER_NAME_CHARS)).checked(),
            Some(named(&"é".repeat(MAX_SERVER_NAME_CHARS)))
        );
        assert_eq!(named(&"n".repeat(MAX_SERVER_NAME_CHARS + 1)).checked(), None);
    }
}
