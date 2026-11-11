//! What the bridge reports about the server's entities: the parts of a sweep, each a string of
//! rows, and what became of the entities Observer asked it to delete. Like the players' reports
//! they go on a bus of their own, and only the bridge of this start may send them.
//!
//! A row outside its ranges is left out alone; a report that is not shaped as the bridge shapes
//! them is dropped whole.

use observer_domain::entities::{
    EntitiesUnavailable, EntityCounts, EntityKind, EntityOutcome, MAX_DELETE, MAX_POPULATION,
    MAX_RESOURCE_NAME_BYTES, VehicleClass,
};
use serde_json::Value;

use crate::bridge::REPORT_TYPE;
use crate::players::MAX_COORDINATE;
use crate::trace::TraceKind;

/// The bridge lists this many entities at most, 500 a part.
pub const MAX_ROWS: usize = 20_000;
pub const MAX_PARTS: u32 = 64;
/// Twice what the bridge puts in a part.
const MAX_PART_BYTES: usize = 128 * 1024;
const MAX_RESOURCES: usize = 2048;
const MAX_COUNT: u64 = 1_000_000;
const MAX_SERIAL: u64 = 0x7fff_ffff;
const FIELDS: usize = 13;
const OCCUPIED: u8 = 1;
const RESOURCE_PENDING: u8 = 2;

/// A report and the start of FXServer it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityReport {
    pub generation: u32,
    pub kind: EntityReportKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntityReportKind {
    Sweep(SweepPart),
    Deleted(DeleteReply),
}

/// One part of a sweep, in the order the bridge sent them.
#[derive(Debug, Clone, PartialEq)]
pub struct SweepPart {
    /// Which start of the bridge numbered the entities: it numbers them again from 1 after each.
    pub epoch: i64,
    pub seq: u32,
    /// From 1.
    pub part: u32,
    /// This part's rows as the bridge wrote them. [`read_rows`] reads them once the sweep is
    /// whole: only its last part says how many resources they may name.
    pub rows: String,
    /// On the last part.
    pub end: Option<SweepEnd>,
}

/// What the last part of a sweep says of the whole.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SweepEnd {
    pub unavailable: Option<EntitiesUnavailable>,
    pub counts: EntityCounts,
    pub truncated: bool,
    /// Microseconds of the server's main thread.
    pub work_us: Option<u32>,
    pub list_us: Option<u32>,
    pub frames: Option<u32>,
    /// The cadence the bridge sweeps at from now on.
    pub every_ms: Option<u32>,
    pub spaced: bool,
    /// The resources the rows name by their place in this list, from 1.
    pub resources: Vec<String>,
}

/// What became of the entities of a deletion, by net id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteReply {
    pub request: u32,
    pub outcomes: Vec<(u16, EntityOutcome)>,
}

/// An entity as a sweep saw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityRow {
    pub net_id: u16,
    /// Drawn by the bridge when it first saw the entity: net ids are used again.
    pub serial: u32,
    pub kind: EntityKind,
    pub class: Option<VehicleClass>,
    pub model: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// The net id of the player whose game simulates it.
    pub owner: Option<u16>,
    pub bucket: i32,
    pub population: u8,
    /// The place of its creating resource in the sweep's list, from 1; 0 for none.
    pub resource: u16,
    /// Not looked up yet, which is not the same as none.
    pub resource_pending: bool,
    /// A player sits in it.
    pub occupied: bool,
}

impl EntityReportKind {
    pub fn from_trace(trace: &TraceKind, bridge: &str) -> Option<Self> {
        let TraceKind::Script { resource, payload } = trace else {
            return None;
        };
        if resource != bridge || payload.get("type").and_then(Value::as_str) != Some(REPORT_TYPE) {
            return None;
        }
        match payload.get("kind").and_then(Value::as_str)? {
            "entities" => sweep_part(payload).map(Self::Sweep),
            "entitiesDeleted" => Some(Self::Deleted(DeleteReply {
                request: number(payload.get("request"), 1, u64::from(u32::MAX))?,
                outcomes: payload
                    .get("outcomes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|outcome| {
                        let net_id = number(outcome.get(0), 1, u64::from(u16::MAX))?;
                        // A word this Observer does not know is no deletion it can vouch for.
                        let outcome = outcome
                            .get(1)
                            .and_then(Value::as_str)
                            .and_then(EntityOutcome::from_name)
                            .filter(|outcome| *outcome != EntityOutcome::Unanswered)
                            .unwrap_or(EntityOutcome::Error);
                        Some((net_id, outcome))
                    })
                    .take(MAX_DELETE)
                    .collect(),
            })),
            _ => None,
        }
    }
}

fn sweep_part(payload: &Value) -> Option<SweepPart> {
    let rows = payload.get("rows").and_then(Value::as_str)?;
    if rows.len() > MAX_PART_BYTES {
        return None;
    }
    let last = payload.get("last").and_then(Value::as_bool).unwrap_or(false);
    Some(SweepPart {
        epoch: payload
            .get("epoch")
            .and_then(Value::as_i64)
            .filter(|&epoch| epoch >= 0)?,
        seq: number(payload.get("seq"), 1, u64::from(u32::MAX))?,
        part: number(payload.get("part"), 1, u64::from(MAX_PARTS))?,
        rows: rows.to_owned(),
        end: if last { Some(sweep_end(payload)?) } else { None },
    })
}

fn sweep_end(payload: &Value) -> Option<SweepEnd> {
    let unavailable = match payload.get("unavailable").and_then(Value::as_str) {
        None => None,
        Some("onesync") => Some(EntitiesUnavailable::Onesync),
        Some("game") => Some(EntitiesUnavailable::Game),
        Some(_) => return None,
    };
    let counts = payload.get("counts");
    let count = |key| number::<u32>(counts.and_then(|counts| counts.get(key)), 0, MAX_COUNT);
    let micros = |key| {
        payload
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "microseconds, checked to be positive and capped"
                )]
                let micros = value.min(f64::from(u32::MAX)) as u32;
                micros
            })
    };
    Some(SweepEnd {
        unavailable,
        counts: EntityCounts {
            vehicles: count("vehicles"),
            peds: count("peds"),
            objects: count("objects"),
            player_peds: count("playerPeds").unwrap_or(0),
        },
        truncated: payload.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        work_us: micros("workUs"),
        list_us: micros("listUs"),
        frames: number(payload.get("frames"), 0, MAX_COUNT),
        every_ms: number(payload.get("everyMs"), 1, 3_600_000),
        spaced: payload.get("spaced").and_then(Value::as_bool).unwrap_or(false),
        resources: resources(payload.get("resources"))?,
    })
}

/// The names the rows point into. The bridge's JSON writes a list without any name as `{}` or as
/// `[]`.
fn resources(value: Option<&Value>) -> Option<Vec<String>> {
    let names = match value {
        None => return Some(Vec::new()),
        Some(Value::Object(fields)) if fields.is_empty() => return Some(Vec::new()),
        Some(Value::Array(names)) if names.len() <= MAX_RESOURCES => names,
        Some(_) => return None,
    };
    names
        .iter()
        .map(|name| {
            name.as_str()
                .filter(|name| !name.is_empty() && name.len() <= MAX_RESOURCE_NAME_BYTES)
                .map(str::to_owned)
        })
        .collect()
}

fn number<T: TryFrom<u64>>(value: Option<&Value>, min: u64, max: u64) -> Option<T> {
    value
        .and_then(Value::as_u64)
        .filter(|number| (min..=max).contains(number))
        .and_then(|number| T::try_from(number).ok())
}

/// Reads `netId,serial,kind,class,model,x,y,z,owner,bucket,population,resource,flags` rows,
/// separated by `;`, into `out`, which never holds more than [`MAX_ROWS`]. `resources` is how many
/// names the sweep lists.
pub fn read_rows(rows: &str, resources: usize, out: &mut Vec<EntityRow>) {
    for row in rows.split(';') {
        if out.len() >= MAX_ROWS {
            return;
        }
        if let Some(row) = read_row(row, resources) {
            out.push(row);
        }
    }
}

fn read_row(row: &str, resources: usize) -> Option<EntityRow> {
    let mut fields = [""; FIELDS];
    let mut given = row.split(',');
    for field in &mut fields {
        *field = given.next()?;
    }
    if given.next().is_some() {
        return None;
    }
    let whole = |index: usize, min: i64, max: i64| {
        fields[index]
            .parse::<i64>()
            .ok()
            .filter(|number| (min..=max).contains(number))
    };
    let coordinate = |index: usize| {
        fields[index]
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && f64::from(value.abs()) <= MAX_COORDINATE)
    };
    let kind = EntityKind::from_code(u8::try_from(whole(2, 1, 3)?).ok()?)?;
    let class = u8::try_from(whole(3, 0, 9)?).ok()?;
    // Only a vehicle has a class, and every vehicle has one.
    if (kind == EntityKind::Vehicle) != (class != 0) {
        return None;
    }
    let flags = u8::try_from(whole(12, 0, i64::from(OCCUPIED | RESOURCE_PENDING))?).ok()?;
    Some(EntityRow {
        net_id: u16::try_from(whole(0, 1, i64::from(u16::MAX))?).ok()?,
        serial: u32::try_from(whole(1, 1, i64::try_from(MAX_SERIAL).ok()?)?).ok()?,
        kind,
        class: VehicleClass::from_code(class),
        model: u32::try_from(whole(4, 0, i64::from(u32::MAX))?).ok()?,
        x: coordinate(5)?,
        y: coordinate(6)?,
        z: coordinate(7)?,
        owner: u16::try_from(whole(8, -1, i64::from(u16::MAX))?)
            .ok()
            .filter(|&owner| owner > 0),
        bucket: i32::try_from(whole(9, i64::from(i32::MIN), i64::from(i32::MAX))?).ok()?,
        population: u8::try_from(whole(10, 0, i64::from(MAX_POPULATION))?).ok()?,
        resource: u16::try_from(whole(11, 0, i64::try_from(resources).ok()?)?).ok()?,
        resource_pending: flags & RESOURCE_PENDING != 0,
        occupied: flags & OCCUPIED != 0,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const BRIDGE: &str = "sys_k3jx7qmw2pza";

    fn report(resource: &str, payload: Value) -> Option<EntityReportKind> {
        EntityReportKind::from_trace(
            &TraceKind::Script {
                resource: resource.to_owned(),
                payload,
            },
            BRIDGE,
        )
    }

    fn rows(text: &str, resources: usize) -> Vec<EntityRow> {
        let mut out = Vec::new();
        read_rows(text, resources, &mut out);
        out
    }

    #[test]
    fn a_part_of_a_sweep_keeps_its_rows_for_the_end_of_the_sweep() {
        let part = report(
            BRIDGE,
            json!({
                "type": "observer", "kind": "entities", "epoch": 4210, "seq": 7, "part": 2,
                "last": false, "rows": "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,1",
            }),
        );

        assert_eq!(
            part,
            Some(EntityReportKind::Sweep(SweepPart {
                epoch: 4210,
                seq: 7,
                part: 2,
                rows: "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,1".to_owned(),
                end: None,
            }))
        );
    }

    #[test]
    fn the_last_part_says_what_the_sweep_counted_and_cost() {
        let part = report(
            BRIDGE,
            json!({
                "type": "observer", "kind": "entities", "epoch": 4210, "seq": 7, "part": 3,
                "last": true, "rows": "",
                "counts": { "vehicles": 412, "objects": 9120, "playerPeds": 48 },
                "listed": 9884, "failed": 0, "truncated": true,
                "workUs": 12400, "listUs": 2900.5, "frames": 9, "everyMs": 30000, "spaced": true,
                "resources": ["qb-garages", "ox_inventory"],
            }),
        );

        let Some(EntityReportKind::Sweep(part)) = part else {
            panic!("not a sweep: {part:?}");
        };
        assert_eq!(
            part.end,
            Some(SweepEnd {
                unavailable: None,
                counts: EntityCounts {
                    vehicles: Some(412),
                    peds: None,
                    objects: Some(9120),
                    player_peds: 48,
                },
                truncated: true,
                work_us: Some(12400),
                list_us: Some(2900),
                frames: Some(9),
                every_ms: Some(30000),
                spaced: true,
                resources: vec!["qb-garages".to_owned(), "ox_inventory".to_owned()],
            })
        );
    }

    #[test]
    fn a_server_that_cannot_list_its_entities_says_why() {
        for (word, unavailable) in [
            ("onesync", EntitiesUnavailable::Onesync),
            ("game", EntitiesUnavailable::Game),
        ] {
            let part = report(
                BRIDGE,
                json!({
                    "type": "observer", "kind": "entities", "epoch": 0, "seq": 1, "part": 1,
                    "last": true, "rows": "", "unavailable": word,
                }),
            );

            let Some(EntityReportKind::Sweep(part)) = part else {
                panic!("not a sweep: {part:?}");
            };
            assert_eq!(part.end.and_then(|end| end.unavailable), Some(unavailable));
        }
    }

    #[test]
    fn a_sweep_that_names_no_resource_lists_none_however_the_bridge_wrote_that() {
        for names in [json!([]), json!({}), Value::Null] {
            let mut payload = json!({
                "type": "observer", "kind": "entities", "epoch": 1, "seq": 1, "part": 1,
                "last": true, "rows": "",
            });
            if !names.is_null() {
                payload["resources"] = names;
            }

            let Some(EntityReportKind::Sweep(part)) = report(BRIDGE, payload) else {
                panic!("not a sweep");
            };
            assert_eq!(part.end.map(|end| end.resources), Some(Vec::new()));
        }
    }

    #[test]
    fn a_report_that_is_not_shaped_as_the_bridge_shapes_them_is_dropped() {
        let part = |change: fn(&mut Value)| {
            let mut payload = json!({
                "type": "observer", "kind": "entities", "epoch": 1, "seq": 1, "part": 1,
                "last": true, "rows": "", "resources": ["chat"],
            });
            change(&mut payload);
            report(BRIDGE, payload)
        };

        assert!(part(|_| {}).is_some());
        assert_eq!(part(|payload| payload["rows"] = json!(12)), None);
        assert_eq!(
            part(|payload| payload["rows"] = json!("x".repeat(MAX_PART_BYTES + 1))),
            None
        );
        assert_eq!(part(|payload| payload["seq"] = json!(0)), None);
        assert_eq!(part(|payload| payload["part"] = json!(MAX_PARTS + 1)), None);
        assert_eq!(part(|payload| payload["epoch"] = json!(-1)), None);
        assert_eq!(part(|payload| payload["unavailable"] = json!("mood")), None);
        assert_eq!(part(|payload| payload["resources"] = json!(["chat", 4])), None);
        assert_eq!(part(|payload| payload["resources"] = json!([""])), None);
        assert_eq!(
            part(|payload| payload["resources"] = json!(["r".repeat(MAX_RESOURCE_NAME_BYTES + 1)])),
            None
        );
        assert_eq!(
            part(|payload| payload["resources"] = json!({ "1": "chat" })),
            None
        );
    }

    #[test]
    fn rows_read_every_field_of_an_entity() {
        let read = rows(
            "12,3,2,4,3078201489,412.6,-1208.3,32.1,4,0,7,2,1;\
             65535,2147483647,1,0,0,-0.0,0.0,-12.5,-1,-7,0,0,2;\
             7,1,3,0,4294967295,1.0,2.0,3.0,0,5,10,1,0",
            2,
        );

        assert_eq!(
            read,
            [
                EntityRow {
                    net_id: 12,
                    serial: 3,
                    kind: EntityKind::Vehicle,
                    class: Some(VehicleClass::Helicopter),
                    model: 3_078_201_489,
                    x: 412.6,
                    y: -1208.3,
                    z: 32.1,
                    owner: Some(4),
                    bucket: 0,
                    population: 7,
                    resource: 2,
                    resource_pending: false,
                    occupied: true,
                },
                EntityRow {
                    net_id: 65535,
                    serial: 2_147_483_647,
                    kind: EntityKind::Ped,
                    class: None,
                    model: 0,
                    x: -0.0,
                    y: 0.0,
                    z: -12.5,
                    owner: None,
                    bucket: -7,
                    population: 0,
                    resource: 0,
                    resource_pending: true,
                    occupied: false,
                },
                EntityRow {
                    net_id: 7,
                    serial: 1,
                    kind: EntityKind::Object,
                    class: None,
                    model: u32::MAX,
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                    owner: None,
                    bucket: 5,
                    population: 10,
                    resource: 1,
                    resource_pending: false,
                    occupied: false,
                },
            ]
        );
    }

    #[test]
    fn a_row_outside_its_ranges_is_left_out_alone() {
        let good = "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0";
        for bad in [
            "",
            "12,3,2,1",
            "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0,9",
            "0,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
            "65536,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
            "12,0,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
            "12,3,4,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
            // A vehicle without a class, and a ped with one.
            "12,3,2,0,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
            "12,3,1,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
            "12,3,2,10,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
            "12,3,2,1,4294967296,412.6,-1208.3,32.1,4,0,7,1,0",
            "12,3,2,1,-1,412.6,-1208.3,32.1,4,0,7,1,0",
            "12,3,2,1,3078201489,inf,-1208.3,32.1,4,0,7,1,0",
            "12,3,2,1,3078201489,nan,-1208.3,32.1,4,0,7,1,0",
            "12,3,2,1,3078201489,412.6,1e9,32.1,4,0,7,1,0",
            "12,3,2,1,3078201489,412.6,-1208.3,,4,0,7,1,0",
            "12,3,2,1,3078201489,412.6,-1208.3,32.1,-2,0,7,1,0",
            "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,2147483648,7,1,0",
            "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,11,1,0",
            // A resource the sweep does not list.
            "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,2,0",
            "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,4",
            "12,3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,x",
            "12, 3,2,1,3078201489,412.6,-1208.3,32.1,4,0,7,1,0",
        ] {
            let read = rows(&format!("{bad};{good}"), 1);

            assert_eq!(read.len(), 1, "{bad:?}");
            assert_eq!(read[0].net_id, 12);
        }
    }

    #[test]
    fn rows_stop_at_what_the_bridge_may_list() {
        let row = "12,3,3,0,1,1.0,2.0,3.0,-1,0,0,0,0";
        let part = vec![row; 600].join(";");
        let mut out = Vec::new();

        for _ in 0..40 {
            read_rows(&part, 0, &mut out);
        }

        assert_eq!(out.len(), MAX_ROWS);
    }

    #[test]
    fn a_deletion_is_answered_entity_by_entity() {
        let reply = report(
            BRIDGE,
            json!({
                "type": "observer", "kind": "entitiesDeleted", "request": 9,
                "outcomes": [
                    [12, "deleted"], [13, "occupied"], [14, "vanished"], [15, "unanswered"],
                    [0, "deleted"], ["16", "deleted"], [17],
                ],
            }),
        );

        assert_eq!(
            reply,
            Some(EntityReportKind::Deleted(DeleteReply {
                request: 9,
                outcomes: vec![
                    (12, EntityOutcome::Deleted),
                    (13, EntityOutcome::Occupied),
                    (14, EntityOutcome::Error),
                    (15, EntityOutcome::Error),
                    (17, EntityOutcome::Error),
                ],
            }))
        );
        assert_eq!(
            report(
                BRIDGE,
                json!({ "type": "observer", "kind": "entitiesDeleted", "outcomes": [[12, "deleted"]] })
            ),
            None
        );
    }

    #[test]
    fn only_the_bridge_of_this_start_reports_entities() {
        let sweep = json!({
            "type": "observer", "kind": "entities", "epoch": 1, "seq": 1, "part": 1, "last": true, "rows": "",
        });
        let deleted = json!({ "type": "observer", "kind": "entitiesDeleted", "request": 1, "outcomes": [] });

        assert!(report(BRIDGE, sweep.clone()).is_some());
        assert!(report(BRIDGE, deleted.clone()).is_some());
        for impostor in ["mischief", "observer", "sys_aaaaaaaaaaaa"] {
            assert_eq!(report(impostor, sweep.clone()), None, "{impostor}");
            assert_eq!(report(impostor, deleted.clone()), None, "{impostor}");
        }
        assert_eq!(report(BRIDGE, json!({ "type": "observer", "kind": "hb" })), None);
    }
}
