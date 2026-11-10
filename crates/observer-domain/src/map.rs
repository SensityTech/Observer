//! The live map: where connected players stand, as the server sees them, drawn over Rockstar's map
//! of GTA V, which each browser loads from Rockstar's servers, or over an image the owner adds.
//! Observer ships and keeps no map of its own: GTA V's belongs to Rockstar Games.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::players::PlayerLabel;

/// The positions' cadences the owner can pick, in seconds.
pub const CADENCES: [u8; 3] = [1, 2, 5];
pub const MAX_HOURS: u32 = 100_000;
/// The largest map image Observer takes, a side and a file.
pub const MAX_IMAGE_SIDE: u32 = 16_384;
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// How far beyond GTA V's world, in game units, the image's edges may lie: room for any map.
pub const MAX_COORDINATE: f64 = 100_000.0;

/// Where the image's edges lie in the game: x grows to the east, y to the north.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MapBounds {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
}

impl MapBounds {
    pub fn is_valid(&self) -> bool {
        let edges = [self.west, self.east, self.south, self.north];
        edges
            .iter()
            .all(|edge| edge.is_finite() && edge.abs() <= MAX_COORDINATE)
            && self.east - self.west >= 1.0
            && self.north - self.south >= 1.0
    }
}

/// What the map draws under the players.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MapImagery {
    /// Rockstar's map of GTA V as the game draws it. Like the two after it, each browser that
    /// opens the map loads it from Rockstar's servers.
    #[default]
    Atlas,
    /// The same map, seen from the sky.
    Satellite,
    /// The same map, as a road map.
    Roads,
    /// The image the owner added, once it is placed in the game.
    Image,
    /// A grid of game coordinates.
    Grid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MapSettings {
    #[serde(default)]
    pub imagery: MapImagery,
    /// Where the owner's image sits; `None` until it is calibrated: the map then shows a grid of
    /// game coordinates in its place.
    pub bounds: Option<MapBounds>,
    /// A player is new under this many hours of play time.
    pub new_hours: u32,
    /// And a veteran from this many.
    pub veteran_hours: u32,
    /// How often the server sends positions while someone watches the map.
    pub every_seconds: u8,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            imagery: MapImagery::default(),
            bounds: None,
            new_hours: 10,
            veteran_hours: 100,
            every_seconds: 2,
        }
    }
}

impl MapSettings {
    pub fn is_valid(&self) -> bool {
        self.bounds.is_none_or(|bounds| bounds.is_valid())
            && self.new_hours >= 1
            && self.new_hours < self.veteran_hours
            && self.veteran_hours <= MAX_HOURS
            && CADENCES.contains(&self.every_seconds)
    }
}

/// The image the owner added, as Observer keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MapImage {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub added_at: Timestamp,
    pub added_by: String,
    /// Changes with each image, for the panel to load the new one.
    pub version: String,
}

/// `GET /api/servers/default/players/map`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MapView {
    pub settings: MapSettings,
    pub image: Option<MapImage>,
}

/// `PUT /api/servers/default/players/map/image`: a file on the host, which Observer copies into its
/// data folder.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MapImageRequest {
    pub path: String,
}

/// A connected player on the map.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MapPlayer {
    pub net_id: u32,
    /// The profile, once the player's identity is confirmed.
    pub player: Option<i64>,
    pub name: Option<String>,
    pub label: Option<PlayerLabel>,
    /// Play time with this session's, once confirmed.
    pub total_ms: Option<i64>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Degrees, 0 to the north, counter-clockwise as the game turns.
    pub heading: f32,
}

/// What the live map topic sends: every placed player, whole, at each report of the bridge.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MapLive {
    pub players: Vec<MapPlayer>,
    /// When the bridge took them; `None` before the first report.
    pub at: Option<Timestamp>,
    /// Connected players without a position yet: still loading, or just joined.
    pub waiting: u32,
    /// Positions need OneSync, which the server runs.
    pub onesync: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_need_ordered_thresholds_a_known_cadence_and_sane_bounds() {
        let bounds = MapBounds {
            west: -5640.0,
            east: 6470.0,
            south: -4140.0,
            north: 7970.0,
        };
        let valid = MapSettings {
            bounds: Some(bounds),
            ..MapSettings::default()
        };

        assert!(MapSettings::default().is_valid());
        assert!(valid.is_valid());
        assert_eq!(MapSettings::default().imagery, MapImagery::Atlas);
        // Settings sent without it, as a panel loaded before it existed sends them.
        let sent: MapSettings =
            serde_json::from_str(r#"{"bounds":null,"newHours":10,"veteranHours":100,"everySeconds":2}"#)
                .unwrap();
        assert_eq!(sent, MapSettings::default());
        assert_eq!(
            serde_json::to_value(MapSettings {
                imagery: MapImagery::Satellite,
                ..MapSettings::default()
            })
            .unwrap()["imagery"],
            "satellite"
        );
        for invalid in [
            MapSettings {
                new_hours: 100,
                ..MapSettings::default()
            },
            MapSettings {
                new_hours: 0,
                ..MapSettings::default()
            },
            MapSettings {
                every_seconds: 3,
                ..MapSettings::default()
            },
            MapSettings {
                bounds: Some(MapBounds {
                    east: -6000.0,
                    ..bounds
                }),
                ..MapSettings::default()
            },
            MapSettings {
                bounds: Some(MapBounds {
                    north: f64::NAN,
                    ..bounds
                }),
                ..MapSettings::default()
            },
        ] {
            assert!(!invalid.is_valid(), "{invalid:?}");
        }
    }
}
