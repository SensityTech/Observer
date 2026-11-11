//! The kick message the owner writes, kept among the settings.

use observer_domain::moderation::KickSettings;
use rusqlite::{Connection, Transaction};

use crate::database::StoreError;
use crate::settings;

const KICK_MESSAGE: &str = "kicks.message";

pub fn kick_settings(conn: &Connection) -> Result<KickSettings, StoreError> {
    let defaults = KickSettings::default();
    let stored = KickSettings {
        message: settings::get(conn, KICK_MESSAGE)?.unwrap_or(defaults.message.clone()),
    };
    // A value edited by hand that the panel could not have saved falls back to the default.
    Ok(stored.checked().unwrap_or(defaults))
}

pub fn save_kick_settings(tx: &Transaction<'_>, saved: &KickSettings) -> Result<(), StoreError> {
    settings::set(tx, KICK_MESSAGE, &saved.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    #[test]
    fn a_kick_message_edited_out_of_range_falls_back_to_the_default() {
        let mut conn = memory();
        assert_eq!(kick_settings(&conn).unwrap(), KickSettings::default());
        let tx = conn.transaction().unwrap();
        save_kick_settings(
            &tx,
            &KickSettings {
                message: "Kicked: {reason}".to_owned(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(kick_settings(&conn).unwrap().message, "Kicked: {reason}");

        settings::set(&conn, KICK_MESSAGE, " \n").unwrap();

        assert_eq!(kick_settings(&conn).unwrap(), KickSettings::default());
    }
}
