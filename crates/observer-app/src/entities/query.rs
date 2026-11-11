//! What the panel asks of a sweep's list: the entities its filters keep, sorted and cut into a
//! page, what the filters can pick from, the map's points or its cells of density, and the groups
//! a deletion names in the audit log. Pure functions over the list; owners' names are passed in.

use std::cmp::Ordering;
use std::collections::HashMap;

use observer_domain::entities::{
    BucketCount, ClassCount, CreatorFilter, EntityCell, EntityFacets, EntityFilters, EntityItem, EntityKind,
    EntityPoint, EntitySort, KindCounts, MAX_MAP_POINTS, ModelCount, OwnerCount, OwnerFilter, PAGE_SIZE,
    ResourceCount, VehicleClass,
};
use observer_domain::map::MapBounds;
use observer_fxserver::entities::EntityRow;
use serde_json::{Value, json};

const MAX_RESOURCE_FACETS: usize = 100;
const MAX_BUCKET_FACETS: usize = 50;
const MAX_OWNER_FACETS: usize = 20;
const MAX_MODEL_FACETS: usize = 10;
/// The groups a deletion names in the audit log, past which the rest is only counted.
const MAX_AUDIT_GROUPS: usize = 5;

/// A sweep's list as one caller may read it.
#[derive(Clone, Copy)]
pub(super) struct Source<'a> {
    pub rows: &'a [EntityRow],
    /// The names the rows point into, from 1.
    pub resources: &'a [String],
    /// The connected players' names, by net id.
    pub names: &'a HashMap<u16, String>,
    /// Whether the caller may know who owns an entity and whether a player sits in one: both
    /// say where a player is.
    pub owners_shown: bool,
}

impl<'a> Source<'a> {
    fn resource(&self, row: &EntityRow) -> Option<&'a str> {
        let place = usize::from(row.resource).checked_sub(1)?;
        self.resources.get(place).map(String::as_str)
    }

    fn owner(&self, row: &EntityRow) -> Option<u16> {
        row.owner.filter(|_| self.owners_shown)
    }

    fn owner_name(&self, row: &EntityRow) -> Option<&'a str> {
        self.names.get(&self.owner(row)?).map(String::as_str)
    }

    pub(super) fn item(&self, row: &EntityRow) -> EntityItem {
        EntityItem {
            net_id: row.net_id,
            serial: row.serial,
            kind: row.kind,
            class: row.class,
            model: row.model,
            x: row.x,
            y: row.y,
            z: row.z,
            owned: row.owner.is_some(),
            owner: self.owner(row),
            owner_name: self.owner_name(row).map(str::to_owned),
            bucket: row.bucket,
            population: row.population,
            resource: self.resource(row).map(str::to_owned),
            resource_pending: row.resource_pending,
            occupied: row.occupied && self.owners_shown,
        }
    }

    /// The rows the filters keep, in the list's order.
    pub(super) fn filtered(&self, filters: &EntityFilters) -> Vec<&'a EntityRow> {
        let creator = match &filters.creator {
            CreatorFilter::Any => None,
            CreatorFilter::None => Some(0),
            CreatorFilter::Resource(name) => match self.resources.iter().position(|known| known == name) {
                Some(place) => Some(place + 1),
                // No entity of the list comes from a resource it does not name.
                None => return Vec::new(),
            },
        };
        let search = Search::new(&filters.search, self);
        self.rows
            .iter()
            .filter(|row| filters.kinds.is_empty() || filters.kinds.contains(&row.kind))
            .filter(|row| filters.class.is_none_or(|class| row.class == Some(class)))
            .filter(|row| {
                creator.is_none_or(|place| {
                    // One not looked up yet is not known to have none.
                    usize::from(row.resource) == place && !(place == 0 && row.resource_pending)
                })
            })
            .filter(|row| match filters.owner {
                OwnerFilter::Any => true,
                OwnerFilter::None => row.owner.is_none(),
                OwnerFilter::Owned => row.owner.is_some(),
                OwnerFilter::Player(net_id) => self.owner(row) == Some(net_id),
            })
            .filter(|row| filters.bucket.is_none_or(|bucket| row.bucket == bucket))
            .filter(|row| search.as_ref().is_none_or(|search| search.matches(row)))
            .collect()
    }

    /// Sorts in place. The net id settles what the key leaves equal, so that a page holds the
    /// same entities from one sweep to the next.
    pub(super) fn sort(&self, rows: &mut [&EntityRow], sort: EntitySort, descending: bool) {
        let directed = |ordering: Ordering| if descending { ordering.reverse() } else { ordering };
        // What has no name sorts last whichever way the rest goes.
        let nameless_last =
            |left: Option<(u8, &str, u16)>, right: Option<(u8, &str, u16)>| match (left, right) {
                (Some(left), Some(right)) => directed(left.cmp(&right)),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            };
        let lower: Vec<String> = self.resources.iter().map(|name| name.to_lowercase()).collect();
        let owners: HashMap<u16, String> = self
            .names
            .iter()
            .map(|(net_id, name)| (*net_id, name.to_lowercase()))
            .collect();
        let creator = |row: &EntityRow| {
            let place = usize::from(row.resource).checked_sub(1)?;
            Some((0, lower.get(place)?.as_str(), 0))
        };
        // Named owners first, then those without a name by net id.
        let owner = |row: &EntityRow| {
            row.owner?;
            Some(match self.owner(row) {
                Some(net_id) => match owners.get(&net_id) {
                    Some(name) => (0, name.as_str(), net_id),
                    None => (1, "", net_id),
                },
                None => (0, "", 0),
            })
        };
        rows.sort_by(|left, right| {
            let by_key = match sort {
                EntitySort::Kind => {
                    directed((kind_order(left.kind), left.class).cmp(&(kind_order(right.kind), right.class)))
                }
                EntitySort::Model => directed(left.model.cmp(&right.model)),
                EntitySort::Resource => nameless_last(creator(left), creator(right)),
                EntitySort::Owner => nameless_last(owner(left), owner(right)),
                EntitySort::Bucket => directed(left.bucket.cmp(&right.bucket)),
                EntitySort::Position => directed(left.x.total_cmp(&right.x).then(left.y.total_cmp(&right.y))),
                EntitySort::NetId => directed(left.net_id.cmp(&right.net_id)),
            };
            by_key.then(left.net_id.cmp(&right.net_id))
        });
    }

    /// One page of `rows`, and its number: the last one when `page` is past it.
    pub(super) fn page(&self, rows: &[&EntityRow], page: u32) -> (u32, Vec<EntityItem>) {
        let last = rows.len().saturating_sub(1) / PAGE_SIZE;
        let page = usize::try_from(page).unwrap_or(usize::MAX).min(last);
        let items = rows
            .iter()
            .skip(page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .map(|row| self.item(row))
            .collect();
        (u32::try_from(page).unwrap_or(u32::MAX), items)
    }

    /// What the filters can pick from, over the whole list, and the most common models among
    /// `filtered`.
    pub(super) fn facets(&self, filtered: &[&EntityRow]) -> EntityFacets {
        let mut classes: HashMap<VehicleClass, u32> = HashMap::new();
        let mut resources: HashMap<u16, u32> = HashMap::new();
        let mut buckets: HashMap<i32, u32> = HashMap::new();
        let mut owners: HashMap<u16, u32> = HashMap::new();
        let (mut unowned, mut owned) = (0, 0);
        for row in self.rows {
            if let Some(class) = row.class {
                *classes.entry(class).or_default() += 1;
            }
            if !row.resource_pending {
                *resources.entry(row.resource).or_default() += 1;
            }
            *buckets.entry(row.bucket).or_default() += 1;
            match row.owner {
                None => unowned += 1,
                Some(net_id) => {
                    owned += 1;
                    if self.owners_shown {
                        *owners.entry(net_id).or_default() += 1;
                    }
                }
            }
        }
        let mut models: HashMap<(u32, EntityKind), u32> = HashMap::new();
        for row in filtered {
            *models.entry((row.model, row.kind)).or_default() += 1;
        }
        EntityFacets {
            kinds: kinds(self.rows),
            classes: VehicleClass::ALL
                .into_iter()
                .filter_map(|class| {
                    Some(ClassCount {
                        class,
                        count: *classes.get(&class)?,
                    })
                })
                .collect(),
            resources: most_common(resources, MAX_RESOURCE_FACETS)
                .into_iter()
                .map(|(place, count)| ResourceCount {
                    resource: usize::from(place)
                        .checked_sub(1)
                        .and_then(|place| self.resources.get(place).cloned()),
                    count,
                })
                .collect(),
            buckets: most_common(buckets, MAX_BUCKET_FACETS)
                .into_iter()
                .map(|(bucket, count)| BucketCount { bucket, count })
                .collect(),
            unowned,
            owned,
            owners: most_common(owners, MAX_OWNER_FACETS)
                .into_iter()
                .map(|(net_id, count)| OwnerCount {
                    net_id,
                    name: self.names.get(&net_id).cloned(),
                    count,
                })
                .collect(),
            models: most_common(models, MAX_MODEL_FACETS)
                .into_iter()
                .map(|((model, kind), count)| ModelCount { model, kind, count })
                .collect(),
        }
    }
}

/// The rows that stand within `bounds`, edges included.
pub(super) fn within<'a>(rows: Vec<&'a EntityRow>, bounds: &MapBounds) -> Vec<&'a EntityRow> {
    let inside = |value: f32, low: f64, high: f64| (low..=high).contains(&f64::from(value));
    rows.into_iter()
        .filter(|row| inside(row.x, bounds.west, bounds.east) && inside(row.y, bounds.south, bounds.north))
        .collect()
}

/// How many of each kind `rows` hold.
pub(super) fn kinds(rows: &[EntityRow]) -> KindCounts {
    let mut kinds = KindCounts::default();
    for row in rows {
        match row.kind {
            EntityKind::Vehicle => kinds.vehicles += 1,
            EntityKind::Ped => kinds.peds += 1,
            EntityKind::Object => kinds.objects += 1,
        }
    }
    kinds
}

/// Each of `rows` as the map draws it, while they are few enough.
pub(super) fn points(rows: &[&EntityRow]) -> Vec<EntityPoint> {
    rows.iter()
        .take(MAX_MAP_POINTS)
        .map(|row| EntityPoint(row.net_id, row.serial, row.kind.code(), row.x, row.y))
        .collect()
}

/// `rows` counted by squares of `side` game units, each with its most common kind. The counts
/// add up to the rows.
pub(super) fn cells(rows: &[&EntityRow], side: u32) -> Vec<EntityCell> {
    #[expect(clippy::cast_precision_loss, reason = "a cell is 2000 game units at most")]
    let side = side.max(1) as f32;
    let mut cells: HashMap<(i32, i32), [u32; 3]> = HashMap::new();
    for row in rows {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "positions are checked to lie within the game's world"
        )]
        let cell = ((row.x / side).floor() as i32, (row.y / side).floor() as i32);
        cells.entry(cell).or_default()[usize::from(row.kind.code() - 1)] += 1;
    }
    let mut cells: Vec<EntityCell> = cells
        .into_iter()
        .map(|((column, row), kinds)| {
            let mut most = 0;
            for kind in 1..kinds.len() {
                if kinds[kind] > kinds[most] {
                    most = kind;
                }
            }
            EntityCell(
                column,
                row,
                kinds.iter().sum(),
                u8::try_from(most + 1).unwrap_or(1),
            )
        })
        .collect();
    cells.sort_unstable_by_key(|cell| (cell.0, cell.1));
    cells
}

/// What a deletion says of `rows` in the audit log: the largest groups, each a model, a kind and
/// a creating resource, and how many entities the groups left out hold.
pub(super) fn groups(rows: &[&EntityRow], resources: &[String]) -> (Vec<Value>, usize) {
    let mut groups: HashMap<(u32, EntityKind, u16), u32> = HashMap::new();
    for row in rows {
        *groups.entry((row.model, row.kind, row.resource)).or_default() += 1;
    }
    let mut groups = most_common(groups, usize::MAX);
    let rest = groups.split_off(groups.len().min(MAX_AUDIT_GROUPS));
    let named = groups
        .into_iter()
        .map(|((model, kind, resource), count)| {
            json!({
                "model": format!("0x{model:08X}"),
                "kind": kind,
                "resource": usize::from(resource).checked_sub(1).and_then(|place| resources.get(place)),
                "count": count,
            })
        })
        .collect();
    (named, rest.iter().map(|(_, count)| *count as usize).sum())
}

/// The keys counted the most, the most first, a tie by the smaller key.
fn most_common<K: Ord + Copy>(counts: HashMap<K, u32>, most: usize) -> Vec<(K, u32)> {
    let mut counts: Vec<(K, u32)> = counts.into_iter().collect();
    counts.sort_unstable_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    counts.truncate(most);
    counts
}

const fn kind_order(kind: EntityKind) -> u8 {
    match kind {
        EntityKind::Vehicle => 0,
        EntityKind::Ped => 1,
        EntityKind::Object => 2,
    }
}

/// What a search looks for: a net id alone as `#12`, or anything that names an entity.
enum Search {
    NetId(u16),
    Any {
        number: Option<u64>,
        hash: Option<u32>,
        resources: Vec<u16>,
        owners: Vec<u16>,
    },
}

impl Search {
    fn new(text: &str, source: &Source<'_>) -> Option<Self> {
        let needle = text.trim().to_lowercase();
        if needle.is_empty() {
            return None;
        }
        if let Some(digits) = needle.strip_prefix('#')
            && let Ok(net_id) = digits.parse()
        {
            return Some(Self::NetId(net_id));
        }
        let digits = needle.len() <= 10 && needle.bytes().all(|byte| byte.is_ascii_digit());
        let hex = needle.strip_prefix("0x").unwrap_or(&needle);
        Some(Self::Any {
            number: if digits { needle.parse().ok() } else { None },
            hash: if hex.len() <= 8 {
                u32::from_str_radix(hex, 16).ok()
            } else {
                None
            },
            resources: (1..)
                .zip(source.resources)
                .filter(|(_, name)| name.to_lowercase().contains(&needle))
                .map(|(place, _)| place)
                .collect(),
            owners: if source.owners_shown {
                source
                    .names
                    .iter()
                    .filter(|(_, name)| name.to_lowercase().contains(&needle))
                    .map(|(net_id, _)| *net_id)
                    .collect()
            } else {
                Vec::new()
            },
        })
    }

    fn matches(&self, row: &EntityRow) -> bool {
        match self {
            Self::NetId(net_id) => row.net_id == *net_id,
            Self::Any {
                number,
                hash,
                resources,
                owners,
            } => {
                number.is_some_and(|number| u64::from(row.net_id) == number || u64::from(row.model) == number)
                    || *hash == Some(row.model)
                    || resources.contains(&row.resource)
                    || row.owner.is_some_and(|owner| owners.contains(&owner))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(net_id: u16, kind: EntityKind, model: u32) -> EntityRow {
        EntityRow {
            net_id,
            serial: u32::from(net_id) + 100,
            kind,
            class: (kind == EntityKind::Vehicle).then_some(VehicleClass::Car),
            model,
            x: f32::from(net_id) * 10.0,
            y: -f32::from(net_id),
            z: 30.0,
            owner: None,
            bucket: 0,
            population: 5,
            resource: 0,
            resource_pending: false,
            occupied: false,
        }
    }

    fn list() -> Vec<EntityRow> {
        vec![
            EntityRow {
                owner: Some(4),
                resource: 1,
                occupied: true,
                ..row(1, EntityKind::Vehicle, 0xB779_A091)
            },
            EntityRow {
                class: Some(VehicleClass::Train),
                ..row(2, EntityKind::Vehicle, 0x3D6A_AA9B)
            },
            EntityRow {
                owner: Some(9),
                bucket: 3,
                ..row(3, EntityKind::Ped, 0x705E_61F2)
            },
            EntityRow {
                resource: 2,
                ..row(4, EntityKind::Object, 0x0A1B_2C3D)
            },
            EntityRow {
                resource_pending: true,
                ..row(5, EntityKind::Object, 0x0A1B_2C3D)
            },
            EntityRow {
                owner: Some(4),
                resource: 1,
                ..row(6, EntityKind::Vehicle, 0xB779_A091)
            },
        ]
    }

    fn resources() -> Vec<String> {
        vec!["qb-garages".to_owned(), "Ox_Inventory".to_owned()]
    }

    fn names() -> HashMap<u16, String> {
        HashMap::from([(4, "Ada".to_owned())])
    }

    fn net_ids(rows: &[&EntityRow]) -> Vec<u16> {
        rows.iter().map(|row| row.net_id).collect()
    }

    fn kept(filters: &EntityFilters, owners_shown: bool) -> Vec<u16> {
        let (rows, resources, names) = (list(), resources(), names());
        let source = Source {
            rows: &rows,
            resources: &resources,
            names: &names,
            owners_shown,
        };
        net_ids(&source.filtered(filters))
    }

    fn searched(text: &str, owners_shown: bool) -> Vec<u16> {
        kept(
            &EntityFilters {
                search: text.to_owned(),
                ..EntityFilters::default()
            },
            owners_shown,
        )
    }

    #[test]
    fn filters_keep_what_they_name_and_add_up() {
        let all = EntityFilters::default();
        assert_eq!(kept(&all, true), [1, 2, 3, 4, 5, 6]);
        assert_eq!(
            kept(
                &EntityFilters {
                    kinds: vec![EntityKind::Object, EntityKind::Ped],
                    ..all.clone()
                },
                true
            ),
            [3, 4, 5]
        );
        assert_eq!(
            kept(
                &EntityFilters {
                    class: Some(VehicleClass::Train),
                    ..all.clone()
                },
                true
            ),
            [2]
        );
        assert_eq!(
            kept(
                &EntityFilters {
                    creator: CreatorFilter::Resource("qb-garages".to_owned()),
                    owner: OwnerFilter::Owned,
                    ..all.clone()
                },
                true
            ),
            [1, 6]
        );
        assert_eq!(
            kept(
                &EntityFilters {
                    creator: CreatorFilter::Resource("gone".to_owned()),
                    ..all.clone()
                },
                true
            ),
            Vec::<u16>::new()
        );
        assert_eq!(
            kept(
                &EntityFilters {
                    bucket: Some(3),
                    ..all.clone()
                },
                true
            ),
            [3]
        );
        assert_eq!(
            kept(
                &EntityFilters {
                    owner: OwnerFilter::None,
                    ..all
                },
                true
            ),
            [2, 4, 5]
        );
    }

    #[test]
    fn an_entity_whose_resource_is_not_looked_up_yet_is_not_one_without_a_resource() {
        let none = EntityFilters {
            creator: CreatorFilter::None,
            ..EntityFilters::default()
        };

        assert_eq!(kept(&none, true), [2, 3]);
    }

    #[test]
    fn a_search_finds_a_net_id_a_model_a_resource_or_an_owner() {
        assert_eq!(searched("#2", true), [2]);
        assert_eq!(searched(" 0xB779A091 ", true), [1, 6]);
        assert_eq!(searched("b779a091", true), [1, 6]);
        assert_eq!(searched("3078201489", true), [1, 6]);
        assert_eq!(searched("inventory", true), [4]);
        assert_eq!(searched("ADA", true), [1, 6]);
        // Digits are a net id, a model in decimal, or a model in hexadecimal.
        assert_eq!(searched("3", true), [3]);
        assert_eq!(searched("nothing like it", true), Vec::<u16>::new());
        assert_eq!(searched("#70000", true), Vec::<u16>::new());
    }

    #[test]
    fn who_does_not_see_where_players_stand_is_not_told_who_owns_an_entity() {
        let (rows, resources, names) = (list(), resources(), names());
        let hidden = Source {
            rows: &rows,
            resources: &resources,
            names: &names,
            owners_shown: false,
        };

        let item = hidden.item(&rows[0]);
        let facets = hidden.facets(&[]);

        assert!(item.owned);
        assert_eq!((item.owner, item.owner_name, item.occupied), (None, None, false));
        assert_eq!((facets.owned, facets.unowned), (3, 3));
        assert_eq!(facets.owners, []);
        assert_eq!(searched("ada", false), Vec::<u16>::new());
        assert_eq!(
            kept(
                &EntityFilters {
                    owner: OwnerFilter::Player(4),
                    ..EntityFilters::default()
                },
                false
            ),
            Vec::<u16>::new()
        );
        assert_eq!(
            kept(
                &EntityFilters {
                    owner: OwnerFilter::Player(4),
                    ..EntityFilters::default()
                },
                true
            ),
            [1, 6]
        );
        let shown = Source {
            owners_shown: true,
            ..hidden
        };
        let item = shown.item(&rows[0]);
        assert_eq!(
            (item.owner, item.owner_name.as_deref(), item.occupied),
            (Some(4), Some("Ada"), true)
        );
    }

    #[test]
    fn sorting_puts_what_has_no_name_last_either_way_and_settles_ties_by_net_id() {
        let (rows, resources, names) = (list(), resources(), names());
        let source = Source {
            rows: &rows,
            resources: &resources,
            names: &names,
            owners_shown: true,
        };
        let sorted = |sort: EntitySort, descending: bool| {
            let mut all = source.filtered(&EntityFilters::default());
            source.sort(&mut all, sort, descending);
            net_ids(&all)
        };

        assert_eq!(sorted(EntitySort::Resource, false), [4, 1, 6, 2, 3, 5]);
        assert_eq!(sorted(EntitySort::Resource, true), [1, 6, 4, 2, 3, 5]);
        assert_eq!(sorted(EntitySort::Owner, false), [1, 6, 3, 2, 4, 5]);
        assert_eq!(sorted(EntitySort::Owner, true), [3, 1, 6, 2, 4, 5]);
        assert_eq!(sorted(EntitySort::Kind, false), [1, 6, 2, 3, 4, 5]);
        assert_eq!(sorted(EntitySort::Model, false), [4, 5, 2, 3, 1, 6]);
        assert_eq!(sorted(EntitySort::Bucket, true), [3, 1, 2, 4, 5, 6]);
        assert_eq!(sorted(EntitySort::Position, true), [6, 5, 4, 3, 2, 1]);
        assert_eq!(sorted(EntitySort::NetId, true), [6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn a_page_past_the_last_one_is_the_last_one() {
        let rows: Vec<EntityRow> = (1..=250)
            .map(|net_id| row(net_id, EntityKind::Object, 1))
            .collect();
        let (resources, names) = (Vec::new(), HashMap::new());
        let source = Source {
            rows: &rows,
            resources: &resources,
            names: &names,
            owners_shown: true,
        };
        let all = source.filtered(&EntityFilters::default());

        let (first, items) = source.page(&all, 0);
        let (last, rest) = source.page(&all, 9);

        assert_eq!((first, items.len(), items[0].net_id), (0, 100, 1));
        assert_eq!((last, rest.len(), rest[0].net_id), (2, 50, 201));
        assert_eq!(source.page(&[], 3), (0, Vec::new()));
    }

    #[test]
    fn facets_count_the_whole_list_and_the_models_the_filters_keep() {
        let (rows, resources, names) = (list(), resources(), names());
        let source = Source {
            rows: &rows,
            resources: &resources,
            names: &names,
            owners_shown: true,
        };
        let vehicles = source.filtered(&EntityFilters {
            kinds: vec![EntityKind::Vehicle],
            ..EntityFilters::default()
        });

        let facets = source.facets(&vehicles);

        assert_eq!(
            facets.kinds,
            KindCounts {
                vehicles: 3,
                peds: 1,
                objects: 2
            }
        );
        assert_eq!(
            facets.classes,
            [
                ClassCount {
                    class: VehicleClass::Car,
                    count: 2
                },
                ClassCount {
                    class: VehicleClass::Train,
                    count: 1
                },
            ]
        );
        assert_eq!(
            facets.resources,
            [
                ResourceCount {
                    resource: None,
                    count: 2
                },
                ResourceCount {
                    resource: Some("qb-garages".to_owned()),
                    count: 2
                },
                ResourceCount {
                    resource: Some("Ox_Inventory".to_owned()),
                    count: 1
                },
            ]
        );
        assert_eq!(
            facets.buckets,
            [
                BucketCount { bucket: 0, count: 5 },
                BucketCount { bucket: 3, count: 1 }
            ]
        );
        assert_eq!(
            facets.owners,
            [
                OwnerCount {
                    net_id: 4,
                    name: Some("Ada".to_owned()),
                    count: 2
                },
                OwnerCount {
                    net_id: 9,
                    name: None,
                    count: 1
                },
            ]
        );
        assert_eq!(
            facets.models,
            [
                ModelCount {
                    model: 0xB779_A091,
                    kind: EntityKind::Vehicle,
                    count: 2
                },
                ModelCount {
                    model: 0x3D6A_AA9B,
                    kind: EntityKind::Vehicle,
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn cells_count_every_entity_once_and_name_the_most_common_kind() {
        let rows = [
            EntityRow {
                x: 10.0,
                y: 10.0,
                ..row(1, EntityKind::Object, 1)
            },
            EntityRow {
                x: 190.0,
                y: 199.9,
                ..row(2, EntityKind::Object, 1)
            },
            EntityRow {
                x: 20.0,
                y: 20.0,
                ..row(3, EntityKind::Vehicle, 1)
            },
            EntityRow {
                x: -0.1,
                y: -200.0,
                ..row(4, EntityKind::Ped, 1)
            },
        ];
        let all: Vec<&EntityRow> = rows.iter().collect();

        assert_eq!(
            cells(&all, 200),
            [EntityCell(-1, -1, 1, 1), EntityCell(0, 0, 3, 3)]
        );
        assert_eq!(points(&all)[2], EntityPoint(3, 103, 2, 20.0, 20.0));
        let corner = MapBounds {
            west: 10.0,
            east: 20.0,
            south: 10.0,
            north: 199.0,
        };
        assert_eq!(net_ids(&within(all, &corner)), [1, 3], "edges included");
    }

    #[test]
    fn a_deletion_names_its_largest_groups_and_counts_the_rest() {
        let rows: Vec<EntityRow> = (1..=9_u16)
            .map(|net_id| EntityRow {
                resource: u16::from(net_id <= 4),
                ..row(
                    net_id,
                    EntityKind::Object,
                    if net_id <= 4 { 7 } else { u32::from(net_id) },
                )
            })
            .collect();
        let all: Vec<&EntityRow> = rows.iter().collect();

        let (named, rest) = groups(&all, &resources());

        assert_eq!(
            named[0],
            json!({ "model": "0x00000007", "kind": "object", "resource": "qb-garages", "count": 4 })
        );
        assert_eq!(named[1]["resource"], Value::Null);
        assert_eq!((named.len(), rest), (5, 1));
    }
}
