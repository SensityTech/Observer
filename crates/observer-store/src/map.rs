//! The live map's settings and the record of its image, kept as settings.

use observer_domain::map::{MapImage, MapSettings};
use rusqlite::Connection;

use crate::database::StoreError;
use crate::settings;

const SETTINGS: &str = "map.settings";
const IMAGE: &str = "map.image";

pub fn settings(conn: &Connection) -> Result<MapSettings, StoreError> {
    Ok(settings::get(conn, SETTINGS)?
        .and_then(|text| read(&text))
        // A value edited by hand that the panel could not have saved falls back to the defaults.
        .filter(MapSettings::is_valid)
        .unwrap_or_default())
}

/// Settings saved before the map could draw Rockstar's name no imagery: an image their owner
/// placed stays what the map draws.
fn read(text: &str) -> Option<MapSettings> {
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    let fields = value.as_object_mut()?;
    if !fields.contains_key("imagery") && fields.get("bounds").is_some_and(|bounds| !bounds.is_null()) {
        fields.insert("imagery".to_owned(), "image".into());
    }
    serde_json::from_value(value).ok()
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
    use observer_domain::map::{MapBounds, MapImagery};

    use super::*;
    use crate::database::memory;

    #[test]
    fn settings_saved_before_the_imagery_keep_a_placed_image_and_take_the_default_without_one() {
        let conn = memory();
        settings::set(
            &conn,
            SETTINGS,
            r#"{"bounds":{"west":-5640.0,"east":6470.0,"south":-4140.0,"north":7970.0},"newHours":5,"veteranHours":50,"everySeconds":1}"#,
        )
        .unwrap();
        let placed = settings(&conn).unwrap();
        assert_eq!(placed.imagery, MapImagery::Image);
        assert_eq!(placed.new_hours, 5);

        settings::set(
            &conn,
            SETTINGS,
            r#"{"bounds":null,"newHours":5,"veteranHours":50,"everySeconds":1}"#,
        )
        .unwrap();
        assert_eq!(settings(&conn).unwrap().imagery, MapImagery::Atlas);

        // What the owner chose since is kept as chosen, bounds or not.
        let chosen = MapSettings {
            imagery: MapImagery::Roads,
            ..placed
        };
        save_settings(&conn, &chosen).unwrap();
        assert_eq!(settings(&conn).unwrap(), chosen);
    }

    #[test]
    fn settings_and_the_image_round_trip_and_bad_ones_fall_back() {
        let conn = memory();
        let saved = MapSettings {
            imagery: MapImagery::Image,
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
