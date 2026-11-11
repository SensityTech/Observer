//! The server's entities: its vehicles, peds and objects as OneSync knows them, listed while a
//! panel looks at them, found on the live map, and deleted on request. Observer keeps nothing of
//! them: the list lives in memory, and goes once nobody looks.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PAGE_SIZE: usize = 100;
/// The most entities one request deletes.
pub const MAX_DELETE: usize = 100;
/// From this many, the panel has a word typed before it deletes.
pub const TYPED_CONFIRMATION_FROM: usize = 20;
/// Past this many matching entities, the map gets cells of density instead of each one.
pub const MAX_MAP_POINTS: usize = 5000;
/// A list this many sweeps old is out of date: shown as such, and nothing is deleted from it.
pub const STALE_SWEEPS: u32 = 3;
/// How often the server lists its entities while someone looks, in seconds, as the owner picks.
pub const CADENCES: [u8; 5] = [5, 10, 15, 30, 60];
pub const MAX_SEARCH_CHARS: usize = 64;
pub const MAX_RESOURCE_NAME_BYTES: usize = 64;
/// The sides a density cell may have, in game units.
pub const CELLS: [u32; 7] = [25, 50, 100, 200, 500, 1000, 2000];
pub const DEFAULT_CELL: u32 = 200;
/// `GetEntityPopulationType` for what a script created.
pub const POPULATION_SCRIPT: u8 = 7;
pub const MAX_POPULATION: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntityKind {
    Vehicle,
    /// Never a player's own: those are counted and left out.
    Ped,
    Object,
}

impl EntityKind {
    pub const ALL: [Self; 3] = [Self::Vehicle, Self::Ped, Self::Object];

    /// The number `GetEntityType` gives it, which the bridge and the map's points use.
    pub const fn code(self) -> u8 {
        match self {
            Self::Ped => 1,
            Self::Vehicle => 2,
            Self::Object => 3,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Ped),
            2 => Some(Self::Vehicle),
            3 => Some(Self::Object),
            _ => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vehicle => "vehicle",
            Self::Ped => "ped",
            Self::Object => "object",
        }
    }
}

/// What `GetVehicleType` says of a vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VehicleClass {
    Car,
    Bike,
    Boat,
    Helicopter,
    Plane,
    Submarine,
    Trailer,
    /// Never deleted from the panel.
    Train,
    Other,
}

impl VehicleClass {
    pub const ALL: [Self; 9] = [
        Self::Car,
        Self::Bike,
        Self::Boat,
        Self::Helicopter,
        Self::Plane,
        Self::Submarine,
        Self::Trailer,
        Self::Train,
        Self::Other,
    ];

    /// The bridge's number for it, from 1.
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code).checked_sub(1)?).copied()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.as_str() == name)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Car => "car",
            Self::Bike => "bike",
            Self::Boat => "boat",
            Self::Helicopter => "helicopter",
            Self::Plane => "plane",
            Self::Submarine => "submarine",
            Self::Trailer => "trailer",
            Self::Train => "train",
            Self::Other => "other",
        }
    }
}

/// Why the server has no entity to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntitiesUnavailable {
    /// Only OneSync knows the server's entities.
    Onesync,
    /// Only GTA V's entities are read.
    Game,
}

/// What the server counted in a sweep, whole even when the list is cut short. `None` for a kind
/// whose listing failed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityCounts {
    pub vehicles: Option<u32>,
    /// With the players' own.
    pub peds: Option<u32>,
    pub objects: Option<u32>,
    /// Counted among the peds, and never listed.
    pub player_peds: u32,
}

/// What the entities topic sends at each sweep, and what every answer about the list carries: the
/// state of the list, never its rows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntitiesLive {
    /// Names the start of the server and of its bridge that the net ids and serials belong to. A
    /// deletion gives it back, and is refused once it changed. `None` before the first sweep.
    pub run: Option<String>,
    /// Counts the sweeps of a run.
    pub sweep: u32,
    /// When Observer took the last sweep.
    pub at: Option<Timestamp>,
    pub unavailable: Option<EntitiesUnavailable>,
    pub counts: Option<EntityCounts>,
    /// Entities in the list, players' peds left out.
    pub listed: u32,
    /// The sweep stopped at its budget or its cap: the list is partial, the counts are not.
    pub truncated: bool,
    /// How often the server sweeps, in milliseconds: the cadence asked for, or more once a sweep
    /// cost it too much.
    pub every_ms: u32,
    /// The server spaced its sweeps out by itself.
    pub spaced: bool,
    /// What the last sweep took of the server's main thread, in microseconds.
    pub work_us: Option<u32>,
    /// The part of it spent listing, before reading each entity.
    pub list_us: Option<u32>,
    /// The frames the sweep was spread over.
    pub frames: Option<u32>,
}

impl EntitiesLive {
    /// Whether the list is too old to delete from at `now`: the server stopped answering.
    pub fn is_stale(&self, now: Timestamp) -> bool {
        self.at.is_some_and(|at| {
            let limit = i64::from(STALE_SWEEPS) * i64::from(self.every_ms.max(1000));
            now.as_millisecond().saturating_sub(at.as_millisecond()) > limit
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct EntitiesSettings {
    /// How often the server lists its entities while someone looks at them.
    pub every_seconds: u8,
}

impl Default for EntitiesSettings {
    fn default() -> Self {
        Self { every_seconds: 15 }
    }
}

impl EntitiesSettings {
    pub fn is_valid(&self) -> bool {
        CADENCES.contains(&self.every_seconds)
    }
}

/// Which entities a list or the map shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityFilters {
    /// None for every kind.
    pub kinds: Vec<EntityKind>,
    /// Only vehicles of this class.
    pub class: Option<VehicleClass>,
    pub creator: CreatorFilter,
    pub owner: OwnerFilter,
    pub bucket: Option<i32>,
    /// A net id, a model's hash in hexadecimal or decimal, or part of a resource's or an owner's
    /// name; `#12` is the net id 12 and nothing else.
    pub search: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CreatorFilter {
    #[default]
    Any,
    /// No resource of the server created it: the game's own traffic, or a client's script.
    None,
    Resource(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OwnerFilter {
    #[default]
    Any,
    /// No player's game simulates it.
    None,
    Owned,
    /// The net id of the player whose game does. Matches nothing for who does not see the map.
    Player(u16),
}

impl EntityFilters {
    /// Reads the filters of a query string. A value that names nothing is refused rather than
    /// taken for no filter, which would show more than was asked for.
    pub fn parse(query: &EntityFilterQuery) -> Option<Self> {
        let kinds = match query.kinds.as_deref() {
            None | Some("") => Vec::new(),
            Some(list) => {
                let mut kinds = Vec::new();
                for name in list.split(',') {
                    let kind = EntityKind::from_name(name)?;
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
                kinds
            }
        };
        let class = match query.class.as_deref() {
            None => None,
            Some(name) => Some(VehicleClass::from_name(name)?),
        };
        let creator = match query.resource.as_deref() {
            None => CreatorFilter::Any,
            Some("") => CreatorFilter::None,
            Some(name) if name.len() <= MAX_RESOURCE_NAME_BYTES => CreatorFilter::Resource(name.to_owned()),
            Some(_) => return None,
        };
        let owner = match query.owner.as_deref() {
            None => OwnerFilter::Any,
            Some("none") => OwnerFilter::None,
            Some("owned") => OwnerFilter::Owned,
            Some(id) => OwnerFilter::Player(id.parse().ok().filter(|&id| id > 0)?),
        };
        let search = query.search.as_deref().unwrap_or_default().trim();
        if search.chars().count() > MAX_SEARCH_CHARS {
            return None;
        }
        Some(Self {
            kinds,
            class,
            creator,
            owner,
            bucket: query.bucket,
            search: search.to_owned(),
        })
    }
}

/// The filters as a query string gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityFilterQuery {
    /// `vehicle`, `ped`, `object`, separated by commas.
    pub kinds: Option<String>,
    pub class: Option<String>,
    /// A resource's name, or empty for the entities no resource created.
    pub resource: Option<String>,
    /// `none`, `owned`, or a player's net id.
    pub owner: Option<String>,
    pub bucket: Option<i32>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntitySort {
    Kind,
    #[default]
    Model,
    Resource,
    Owner,
    Bucket,
    /// From west to east.
    Position,
    NetId,
}

/// An entity in the list.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityItem {
    pub net_id: u16,
    /// Drawn by the bridge when it first saw the entity. Net ids are used again: only with its
    /// serial does one name an entity.
    pub serial: u32,
    pub kind: EntityKind,
    pub class: Option<VehicleClass>,
    /// The model's hash, unsigned.
    pub model: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Whether a player's game simulates it.
    pub owned: bool,
    /// That player's net id. `None` without one, and for who does not see where players stand:
    /// the owner of a vehicle is whoever drives it.
    pub owner: Option<u16>,
    pub owner_name: Option<String>,
    pub bucket: i32,
    /// How the game came to hold it, as `GetEntityPopulationType` says: 7 for a script's.
    pub population: u8,
    /// The resource that created it, as the server knew when the entity was first seen. `None`
    /// for the game's own traffic and for what a client's script created.
    pub resource: Option<String>,
    /// Not looked up yet: a sweep looks up a few hundred.
    pub resource_pending: bool,
    /// A player sits in it. Never told to who does not see where players stand.
    pub occupied: bool,
}

/// The limits the panel holds a selection to, which the server enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityLimits {
    pub page_size: u32,
    pub max_delete: u32,
    pub typed_confirmation_from: u32,
    pub max_map_points: u32,
    /// A list this many sweeps old is out of date.
    pub stale_sweeps: u32,
}

impl Default for EntityLimits {
    fn default() -> Self {
        let number = |limit: usize| u32::try_from(limit).unwrap_or(u32::MAX);
        Self {
            page_size: number(PAGE_SIZE),
            max_delete: number(MAX_DELETE),
            typed_confirmation_from: number(TYPED_CONFIRMATION_FROM),
            max_map_points: number(MAX_MAP_POINTS),
            stale_sweeps: STALE_SWEEPS,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct KindCounts {
    pub vehicles: u32,
    pub peds: u32,
    pub objects: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ClassCount {
    pub class: VehicleClass,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ResourceCount {
    /// `None` for the entities no resource created.
    pub resource: Option<String>,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct BucketCount {
    pub bucket: i32,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OwnerCount {
    pub net_id: u16,
    pub name: Option<String>,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ModelCount {
    pub model: u32,
    pub kind: EntityKind,
    pub count: u32,
}

/// What the filters can pick from, counted over the whole list, and the most common models of the
/// entities the filters keep.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityFacets {
    pub kinds: KindCounts,
    pub classes: Vec<ClassCount>,
    /// The most common first.
    pub resources: Vec<ResourceCount>,
    pub buckets: Vec<BucketCount>,
    pub unowned: u32,
    pub owned: u32,
    /// Who owns the most. Empty for who does not see where players stand.
    pub owners: Vec<OwnerCount>,
    pub models: Vec<ModelCount>,
}

/// `GET /api/servers/default/entities`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityPage {
    pub live: EntitiesLive,
    /// Whether who asks may see who owns an entity, and whether a player sits in one.
    pub owners_shown: bool,
    pub limits: EntityLimits,
    pub matching: u32,
    /// From 0, the last one when the page asked for is past it.
    pub page: u32,
    pub items: Vec<EntityItem>,
    pub facets: EntityFacets,
}

/// An entity on the map: its net id, its serial, the number of its kind (1 for a ped, 2 for a
/// vehicle, 3 for an object), then x and y in game units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct EntityPoint(pub u16, pub u32, pub u8, pub f32, pub f32);

/// A square of the map and the entities in it: its column and its row, counted from the game's
/// origin in cells, how many entities, and the number of the most common kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct EntityCell(pub i32, pub i32, pub u32, pub u8);

/// `GET /api/servers/default/entities/map`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityMap {
    pub live: EntitiesLive,
    /// How many of each kind the whole list holds.
    pub kinds: KindCounts,
    /// The entities the filters keep within the part of the map asked for.
    pub matching: u32,
    /// Each of them, while they are few enough to draw.
    pub points: Vec<EntityPoint>,
    /// The side of a cell in game units, when there are too many: `cells` then stands for them.
    pub cell: Option<u32>,
    pub cells: Vec<EntityCell>,
}

/// An entity to delete, as the list showed it: the server deletes it only if it still is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct EntityTarget {
    pub net_id: u16,
    pub serial: u32,
    pub model: u32,
}

/// `POST /api/servers/default/entities/delete`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct EntityDeleteRequest {
    /// The `run` of the list the entities were picked in.
    pub run: String,
    pub targets: Vec<EntityTarget>,
}

/// What became of an entity someone asked to delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntityOutcome {
    Deleted,
    /// Asked of the game, and still there a moment later.
    Unconfirmed,
    /// Sent to the server, which did not answer in time.
    Unanswered,
    /// It was gone already.
    Gone,
    /// Another entity holds the net id now.
    Changed,
    /// A player's own ped.
    Player,
    /// A player sits in it.
    Occupied,
    Train,
    /// The server read the request too late to act on it.
    Late,
    Error,
}

impl EntityOutcome {
    pub const ALL: [Self; 10] = [
        Self::Deleted,
        Self::Unconfirmed,
        Self::Unanswered,
        Self::Gone,
        Self::Changed,
        Self::Player,
        Self::Occupied,
        Self::Train,
        Self::Late,
        Self::Error,
    ];

    /// The word the bridge reports it by, and the audit log keeps.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deleted => "deleted",
            Self::Unconfirmed => "unconfirmed",
            Self::Unanswered => "unanswered",
            Self::Gone => "gone",
            Self::Changed => "changed",
            Self::Player => "player",
            Self::Occupied => "occupied",
            Self::Train => "train",
            Self::Late => "late",
            Self::Error => "error",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityResult {
    pub net_id: u16,
    pub outcome: EntityOutcome,
}

/// The answer to a deletion: every entity asked for, once each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct EntityDeletion {
    pub results: Vec<EntityResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str) -> EntityFilterQuery {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn kinds_and_classes_keep_the_numbers_the_bridge_reports() {
        for kind in EntityKind::ALL {
            assert_eq!(EntityKind::from_code(kind.code()), Some(kind));
            assert_eq!(EntityKind::from_name(kind.as_str()), Some(kind));
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
        }
        assert_eq!(EntityKind::from_code(0), None);
        assert_eq!(EntityKind::from_code(4), None);
        assert_eq!(VehicleClass::from_code(1), Some(VehicleClass::Car));
        assert_eq!(VehicleClass::from_code(8), Some(VehicleClass::Train));
        assert_eq!(VehicleClass::from_code(9), Some(VehicleClass::Other));
        assert_eq!(VehicleClass::from_code(0), None);
        assert_eq!(VehicleClass::from_code(10), None);
        for class in VehicleClass::ALL {
            assert_eq!(VehicleClass::from_name(class.as_str()), Some(class));
            assert_eq!(serde_json::to_value(class).unwrap(), class.as_str());
        }
        for outcome in EntityOutcome::ALL {
            assert_eq!(EntityOutcome::from_name(outcome.as_str()), Some(outcome));
            assert_eq!(serde_json::to_value(outcome).unwrap(), outcome.as_str());
        }
    }

    #[test]
    fn a_query_without_filters_shows_everything() {
        assert_eq!(
            EntityFilters::parse(&EntityFilterQuery::default()),
            Some(EntityFilters::default())
        );
    }

    #[test]
    fn filters_read_what_a_query_names() {
        let filters = EntityFilters::parse(&query(
            r#"{"kinds":"vehicle,ped,vehicle","class":"helicopter","resource":"qb-garages",
                "owner":"12","bucket":-3,"search":"  0xB779A091 "}"#,
        ))
        .unwrap();

        assert_eq!(filters.kinds, [EntityKind::Vehicle, EntityKind::Ped]);
        assert_eq!(filters.class, Some(VehicleClass::Helicopter));
        assert_eq!(filters.creator, CreatorFilter::Resource("qb-garages".to_owned()));
        assert_eq!(filters.owner, OwnerFilter::Player(12));
        assert_eq!(filters.bucket, Some(-3));
        assert_eq!(filters.search, "0xB779A091");
        assert_eq!(
            EntityFilters::parse(&query(r#"{"resource":"","owner":"none"}"#))
                .map(|filters| (filters.creator, filters.owner)),
            Some((CreatorFilter::None, OwnerFilter::None))
        );
        assert_eq!(
            EntityFilters::parse(&query(r#"{"owner":"owned"}"#)).map(|filters| filters.owner),
            Some(OwnerFilter::Owned)
        );
    }

    #[test]
    fn a_filter_that_names_nothing_is_refused() {
        for invalid in [
            r#"{"kinds":"vehicle,plane"}"#.to_owned(),
            r#"{"class":"hovercraft"}"#.to_owned(),
            r#"{"owner":"0"}"#.to_owned(),
            r#"{"owner":"70000"}"#.to_owned(),
            r#"{"owner":"someone"}"#.to_owned(),
            format!(r#"{{"resource":"{}"}}"#, "r".repeat(MAX_RESOURCE_NAME_BYTES + 1)),
            format!(r#"{{"search":"{}"}}"#, "s".repeat(MAX_SEARCH_CHARS + 1)),
        ] {
            assert_eq!(EntityFilters::parse(&query(&invalid)), None, "{invalid}");
        }
    }

    #[test]
    fn a_list_is_out_of_date_three_sweeps_after_the_last_one() {
        let at = Timestamp::from_millisecond(1_793_000_000_000).unwrap();
        let later = |ms: i64| Timestamp::from_millisecond(at.as_millisecond() + ms).unwrap();
        let live = EntitiesLive {
            at: Some(at),
            every_ms: 15_000,
            ..EntitiesLive::default()
        };

        assert!(!live.is_stale(later(45_000)));
        assert!(live.is_stale(later(45_001)));
        assert!(!EntitiesLive::default().is_stale(later(1_000_000)), "no list yet");
        // A sweep the server spaced out is given its own cadence.
        let spaced = EntitiesLive {
            every_ms: 60_000,
            ..live
        };
        assert!(!spaced.is_stale(later(180_000)));
        assert!(spaced.is_stale(later(180_001)));
    }

    #[test]
    fn settings_take_the_cadences_the_panel_offers() {
        assert!(EntitiesSettings::default().is_valid());
        for seconds in CADENCES {
            assert!(
                EntitiesSettings {
                    every_seconds: seconds
                }
                .is_valid()
            );
        }
        for seconds in [0, 1, 4, 20, 61] {
            assert!(
                !EntitiesSettings {
                    every_seconds: seconds
                }
                .is_valid(),
                "{seconds}"
            );
        }
    }
}
