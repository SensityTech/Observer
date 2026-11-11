//! The entities' settings. The entities themselves are never stored.

use observer_domain::entities::EntitiesSettings;
use rusqlite::Connection;

use crate::database::StoreError;
use crate::settings;

const SETTINGS: &str = "entities.settings";

pub fn settings(conn: &Connection) -> Result<EntitiesSettings, StoreError> {
    Ok(settings::get(conn, SETTINGS)?
        .and_then(|text| serde_json::from_str::<EntitiesSettings>(&text).ok())
        // A value edited by hand that the panel could not have saved falls back to the default.
        .filter(EntitiesSettings::is_valid)
        .unwrap_or_default())
}

pub fn save_settings(conn: &Connection, saved: &EntitiesSettings) -> Result<(), StoreError> {
    let text = serde_json::to_string(saved).expect("numbers always serialise");
    settings::set(conn, SETTINGS, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    #[test]
    fn settings_come_back_as_saved_and_fall_back_to_the_default() {
        let conn = memory();

        assert_eq!(settings(&conn).unwrap(), EntitiesSettings::default());
        save_settings(&conn, &EntitiesSettings { every_seconds: 30 }).unwrap();
        assert_eq!(settings(&conn).unwrap(), EntitiesSettings { every_seconds: 30 });
        settings::set(&conn, SETTINGS, r#"{"everySeconds":7}"#).unwrap();
        assert_eq!(settings(&conn).unwrap(), EntitiesSettings::default());
    }
}
