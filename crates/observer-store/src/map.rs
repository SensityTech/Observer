//! The live map's settings and the record of its image, kept as settings.

use observer_domain::map::{MapImage, MapSettings};
use rusqlite::Connection;

use crate::database::StoreError;
use crate::settings;

const SETTINGS: &str = "map.settings";
const IMAGE: &str = "map.image";

pub fn settings(conn: &Connection) -> Result<MapSettings, StoreError> {
    Ok(settings::get(conn, SETTINGS)?
        .and_then(|text| serde_json::from_str::<MapSettings>(&text).ok())
        // A value edited by hand that the panel could not have saved falls back to the defaults.
        .filter(MapSettings::is_valid)
        .unwrap_or_default())
}

pub fn save_settings(conn: &Connection, saved: &MapSettings) -> Result<(), StoreError> {
    let text = serde_json::to_string(saved).expect("numbers always serialise");
    settings::set(conn, SETTINGS, &text)
}

/// The image the owner added, and the file it was kept as, in the map folder.
pub fn image(conn: &Connection) -> Result<Option<(MapImage, String)>, StoreError> {
    Ok(settings::get(conn, IMAGE)?.and_then(|text| {
        let stored: Stored = serde_json::from_str(&text).ok()?;
        Some((stored.image, stored.file))
    }))
}

pub fn save_image(conn: &Connection, image: Option<(&MapImage, &str)>) -> Result<(), StoreError> {
    let Some((image, file)) = image else {
        conn.execute("DELETE FROM settings WHERE key = ?1", [IMAGE])?;
        return Ok(());
    };
    let stored = Stored {
        image: image.clone(),
        file: file.to_owned(),
    };
    let text = serde_json::to_string(&stored).expect("strings and numbers always serialise");
    settings::set(conn, IMAGE, &text)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    image: MapImage,
    file: String,
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use observer_domain::map::MapBounds;

    use super::*;
    use crate::database::memory;

    #[test]
    fn settings_and_the_image_round_trip_and_bad_ones_fall_back() {
        let conn = memory();
        let saved = MapSettings {
            bounds: Some(MapBounds {
                west: -5640.0,
                east: 6470.0,
                south: -4140.0,
                north: 7970.0,
            }),
            new_hours: 5,
            veteran_hours: 50,
            every_seconds: 1,
        };
        let kept = MapImage {
            width: 8192,
            height: 8192,
            bytes: 14_000_000,
            added_at: "2026-11-09T20:00:00Z".parse::<Timestamp>().unwrap(),
            added_by: "ada".to_owned(),
            version: "1a2b".to_owned(),
        };
        assert_eq!(settings(&conn).unwrap(), MapSettings::default());
        assert_eq!(image(&conn).unwrap(), None);

        save_settings(&conn, &saved).unwrap();
        save_image(&conn, Some((&kept, "map.webp"))).unwrap();

        assert_eq!(settings(&conn).unwrap(), saved);
        assert_eq!(image(&conn).unwrap(), Some((kept, "map.webp".to_owned())));
        settings::set(
            &conn,
            SETTINGS,
            r#"{"bounds":null,"newHours":9,"veteranHours":3,"everySeconds":2}"#,
        )
        .unwrap();
        assert_eq!(settings(&conn).unwrap(), MapSettings::default());
        save_image(&conn, None).unwrap();
        assert_eq!(image(&conn).unwrap(), None);
    }
}
