//! The server's entities as the bridge lists and deletes them: what `fake_entity` and
//! `fake_entities` put in the world, swept in parts while Observer asks, each entity under the
//! serial the bridge draws when it first sees it.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use serde_json::{Value, json};

/// The bridge's own limits, from `server.lua`.
const PART_ROWS: usize = 500;
const ROWS_MAX: usize = 20_000;
const LOOKUPS_MAX: usize = 300;
const BUDGET_US: u64 = 50_000;
const COSTLY_US: u64 = 25_000;
const COSTLY_LIST_US: u64 = 10_000;
const EVERY_MAX_MS: u64 = 60_000;
const DELETE_MAX: usize = 100;
/// The console commands that change the fake's world, which [`World::command`] takes.
pub const COMMANDS: [&str; 6] = [
    "fake_entity",
    "fake_entities",
    "fake_entity_gone",
    "fake_entity_stays",
    "fake_enter",
    "fake_possess",
];
const VEHICLE: u8 = 2;
const PED: u8 = 1;
const OBJECT: u8 = 3;
const TRAIN: u8 = 8;
const OCCUPIED: u8 = 1;
const RESOURCE_PENDING: u8 = 2;

#[derive(Debug, Clone)]
struct Entity {
    kind: u8,
    /// Of a vehicle, from 1; 0 otherwise.
    class: u8,
    model: u32,
    position: [f64; 3],
    owner: Option<u32>,
    bucket: i32,
    population: u8,
    resource: Option<String>,
    /// The player sitting in it.
    occupant: Option<u32>,
    /// The player whose own ped it became.
    player: Option<u32>,
    /// `DeleteEntity` leaves it there.
    stays: bool,
}

/// What the bridge keeps of an entity it saw.
struct Seen {
    serial: u32,
    kind: u8,
    model: u32,
    creator: Creator,
    sweep: u32,
}

/// The resource that created an entity, as far as the bridge looked.
enum Creator {
    NotLookedUp,
    None,
    Resource(String),
}

#[derive(Default)]
pub struct World {
    entities: BTreeMap<u16, Entity>,
    next_net_id: u16,
    seen: HashMap<u16, Seen>,
    next_serial: u32,
    /// The game timer when the bridge started.
    epoch: u64,
    seq: u32,
    sweeps: u32,
    spacing: u64,
}

/// What a sweep is told of the server.
pub struct Server {
    pub players: usize,
    pub onesync: bool,
    pub gta: bool,
    /// The cadence Observer asked for.
    pub every_ms: u64,
    /// What reading one entity costs, and listing them all, in microseconds.
    pub row_us: u64,
    pub list_us: Option<u64>,
}

impl World {
    /// The bridge started, or started again: it numbers the entities from 1 again.
    pub fn bridge_started(&mut self, timer_ms: u64) {
        self.seen.clear();
        self.next_serial = 0;
        self.epoch = timer_ms;
        self.seq = 0;
        self.sweeps = 0;
        self.spacing = 1;
    }

    /// Observer stopped asking: the next sweeps start at the cadence asked for.
    pub fn unspaced(&mut self) {
        self.spacing = 1;
    }

    pub fn cadence_ms(&self, every_ms: u64) -> u64 {
        (every_ms * self.spacing.max(1)).min(EVERY_MAX_MS)
    }

    /// `fake_entity <vehicle|ped|object|train|heli|boat> <x> <y> <z> [resource or -] [owner or -]
    /// [bucket]`: one entity, under the next net id.
    pub fn add(&mut self, words: &[String]) -> Option<u16> {
        let (kind, class) = match words.first()?.as_str() {
            "vehicle" => (VEHICLE, 1),
            "bike" => (VEHICLE, 2),
            "boat" => (VEHICLE, 3),
            "heli" => (VEHICLE, 4),
            "train" => (VEHICLE, TRAIN),
            "ped" => (PED, 0),
            "object" => (OBJECT, 0),
            _ => return None,
        };
        let coordinate = |index: usize| words.get(index)?.parse::<f64>().ok();
        let position = [coordinate(1)?, coordinate(2)?, coordinate(3)?];
        let named = |index: usize| words.get(index).filter(|word| *word != "-");
        let resource = named(4).cloned();
        let owner = named(5).and_then(|owner| owner.parse().ok());
        let bucket = words.get(6).and_then(|bucket| bucket.parse().ok()).unwrap_or(0);
        Some(self.insert(Entity {
            kind,
            class,
            model: model_of(kind, class, self.next_net_id),
            position,
            owner,
            bucket,
            population: if resource.is_some() { 7 } else { 5 },
            resource,
            occupant: None,
            player: None,
            stays: false,
        }))
    }

    /// `fake_entities <vehicles> <peds> <objects>`: that many of each, spread over the map, as a
    /// small busy server holds them: a third of the vehicles from a garage resource and owned by
    /// the first players, most objects from two resources, the rest the game's own.
    pub fn fill(&mut self, vehicles: usize, peds: usize, objects: usize) {
        for (kind, count) in [(VEHICLE, vehicles), (PED, peds), (OBJECT, objects)] {
            for index in 0..count {
                #[expect(clippy::cast_precision_loss, reason = "a test entity's place on a spiral")]
                let turn = index as f64;
                let angle = turn * 0.61 + f64::from(kind);
                let radius = 80.0 + 9.0 * turn.sqrt() * (1.0 + f64::from(kind));
                let position = [
                    radius * angle.cos() + 200.0,
                    radius * angle.sin() - 900.0,
                    30.0 + (turn % 7.0),
                ];
                let (class, resource, owner) = match kind {
                    VEHICLE => (
                        match index % 11 {
                            0 => 2,
                            5 => 4,
                            7 => 3,
                            _ => 1,
                        },
                        (index % 3 == 0).then(|| "qb-garages".to_owned()),
                        (index % 3 == 0).then(|| u32::try_from(index % 5 + 1).unwrap_or(1)),
                    ),
                    OBJECT => (
                        0,
                        match index % 4 {
                            0 => Some("ox_inventory".to_owned()),
                            1 => Some("qb-houses".to_owned()),
                            _ => None,
                        },
                        None,
                    ),
                    _ => (0, None, (index % 4 == 0).then_some(1)),
                };
                self.insert(Entity {
                    kind,
                    class,
                    model: model_of(kind, class, u16::try_from(index % 9).unwrap_or(0)),
                    position,
                    owner,
                    bucket: if index % 17 == 0 { 3 } else { 0 },
                    population: if resource.is_some() { 7 } else { 5 },
                    resource,
                    occupant: None,
                    player: None,
                    stays: false,
                });
            }
        }
    }

    /// One of the console commands that change the world; the others are not this module's.
    pub fn command(&mut self, name: &str, words: &[String]) {
        let number = |index: usize| words.get(index).and_then(|word| word.parse::<u32>().ok());
        let net_id = |index: usize| number(index).and_then(|net_id| u16::try_from(net_id).ok());
        match name {
            "fake_entity" => {
                self.add(words);
            }
            "fake_entities" => {
                let count = |index: usize| {
                    number(index)
                        .and_then(|count| usize::try_from(count).ok())
                        .unwrap_or(0)
                };
                self.fill(count(0), count(1), count(2));
            }
            "fake_entity_gone" => {
                if let Some(net_id) = net_id(0) {
                    self.remove(net_id);
                }
            }
            "fake_entity_stays" => {
                if let Some(net_id) = net_id(0) {
                    self.keep(net_id);
                }
            }
            "fake_enter" => {
                if let Some(player) = number(0) {
                    self.enter(player, net_id(1));
                }
            }
            "fake_possess" => {
                if let (Some(player), Some(net_id)) = (number(0), net_id(1)) {
                    self.possess(player, net_id);
                }
            }
            _ => {}
        }
    }

    fn insert(&mut self, entity: Entity) -> u16 {
        self.next_net_id += 1;
        self.entities.insert(self.next_net_id, entity);
        self.next_net_id
    }

    /// `fake_entity_gone <net id>`: the game removed it by itself.
    pub fn remove(&mut self, net_id: u16) {
        self.entities.remove(&net_id);
    }

    /// `fake_entity_stays <net id>`: asked to delete it, the game keeps it.
    pub fn keep(&mut self, net_id: u16) {
        if let Some(entity) = self.entities.get_mut(&net_id) {
            entity.stays = true;
        }
    }

    /// `fake_enter <player> <net id>`: the player sits in the vehicle; without one, steps out.
    pub fn enter(&mut self, player: u32, net_id: Option<u16>) {
        for entity in self.entities.values_mut() {
            if entity.occupant == Some(player) {
                entity.occupant = None;
            }
        }
        if let Some(entity) = net_id.and_then(|net_id| self.entities.get_mut(&net_id)) {
            entity.occupant = Some(player);
        }
    }

    /// `fake_possess <player> <net id>`: the ped becomes the player's own.
    pub fn possess(&mut self, player: u32, net_id: u16) {
        if let Some(entity) = self.entities.get_mut(&net_id).filter(|entity| entity.kind == PED) {
            entity.player = Some(player);
        }
    }

    /// A sweep as the bridge reports it: its parts, in order.
    pub fn sweep(&mut self, server: &Server) -> Vec<Value> {
        self.seq += 1;
        let (epoch, seq) = (self.epoch, self.seq);
        let head = |part: usize, last: bool, rows: &[String]| {
            json!({
                "type": "observer", "kind": "entities", "epoch": epoch, "seq": seq,
                "part": part, "last": last, "rows": rows.join(";"),
            })
        };
        let unavailable = if !server.gta {
            Some("game")
        } else if !server.onesync {
            Some("onesync")
        } else {
            None
        };
        if let Some(unavailable) = unavailable {
            let mut report = head(1, true, &[]);
            report["unavailable"] = json!(unavailable);
            return vec![report];
        }
        self.sweeps += 1;

        let list_us = server.list_us.unwrap_or(100 + self.entities.len() as u64 / 10);
        let mut spent = list_us;
        let mut names: Vec<String> = Vec::new();
        let mut rows: Vec<String> = Vec::new();
        let mut parts = Vec::new();
        let (mut listed, mut lookups, mut truncated) = (0, 0, false);
        let listable: Vec<u16> = [VEHICLE, OBJECT, PED]
            .into_iter()
            .flat_map(|kind| {
                self.entities
                    .iter()
                    .filter(move |(_, entity)| entity.kind == kind && entity.player.is_none())
                    .map(|(net_id, _)| *net_id)
            })
            .collect();
        for net_id in listable {
            if listed >= ROWS_MAX || spent >= BUDGET_US {
                truncated = true;
                break;
            }
            spent += server.row_us;
            if let Some(row) = self.row(net_id, &mut lookups, &mut names) {
                rows.push(row);
                listed += 1;
            }
            if rows.len() >= PART_ROWS {
                parts.push(head(parts.len() + 1, false, &rows));
                rows.clear();
            }
        }
        if !truncated {
            let sweep = self.sweeps;
            self.seen.retain(|_, seen| seen.sweep == sweep);
        }

        let ceiling = EVERY_MAX_MS.div_ceil(server.every_ms.max(1));
        if spent > COSTLY_US || list_us > COSTLY_LIST_US {
            self.spacing = (self.spacing * 2).min(ceiling.max(1));
        } else if self.spacing > 1 && spent < COSTLY_US / 2 && list_us < COSTLY_LIST_US / 2 {
            self.spacing = (self.spacing / 2).max(1);
        }

        let mut last = head(parts.len() + 1, true, &rows);
        last["counts"] = self.counts(server.players);
        last["listed"] = json!(listed);
        last["failed"] = json!(0);
        last["truncated"] = json!(truncated);
        last["workUs"] = json!(spent);
        last["listUs"] = json!(list_us);
        last["frames"] = json!(1 + spent / 1000);
        last["everyMs"] = json!(self.cadence_ms(server.every_ms));
        last["spaced"] = json!(self.spacing > 1);
        last["resources"] = json!(names);
        parts.push(last);
        parts
    }

    /// What the game counts of each kind, every player's own ped among the peds.
    fn counts(&self, players: usize) -> Value {
        let player_peds = u32::try_from(players).unwrap_or(u32::MAX);
        let mut counts = [0, 0, player_peds];
        for entity in self.entities.values().filter(|entity| entity.player.is_none()) {
            let place = match entity.kind {
                VEHICLE => 0,
                OBJECT => 1,
                _ => 2,
            };
            counts[place] += 1;
        }
        json!({ "vehicles": counts[0], "objects": counts[1], "peds": counts[2], "playerPeds": player_peds })
    }

    /// One entity's row, under the serial drawn when the bridge first saw it. Its creating
    /// resource is looked up once, a few hundred entities a sweep.
    fn row(&mut self, net_id: u16, lookups: &mut usize, names: &mut Vec<String>) -> Option<String> {
        let entity = self.entities.get(&net_id)?;
        let known = self
            .seen
            .get(&net_id)
            .is_some_and(|seen| seen.kind == entity.kind && seen.model == entity.model);
        if !known {
            self.next_serial += 1;
            self.seen.insert(
                net_id,
                Seen {
                    serial: self.next_serial,
                    kind: entity.kind,
                    model: entity.model,
                    creator: Creator::NotLookedUp,
                    sweep: 0,
                },
            );
        }
        let seen = self.seen.get_mut(&net_id)?;
        seen.sweep = self.sweeps;
        if matches!(seen.creator, Creator::NotLookedUp) && *lookups < LOOKUPS_MAX {
            *lookups += 1;
            seen.creator = entity.resource.clone().map_or(Creator::None, Creator::Resource);
        }
        let mut flags = 0;
        if entity.kind == VEHICLE && entity.occupant.is_some() {
            flags |= OCCUPIED;
        }
        let resource = match &seen.creator {
            Creator::NotLookedUp => {
                flags |= RESOURCE_PENDING;
                0
            }
            Creator::None => 0,
            Creator::Resource(name) => names.iter().position(|known| known == name).map_or_else(
                || {
                    names.push(name.clone());
                    names.len()
                },
                |place| place + 1,
            ),
        };
        let [x, y, z] = entity.position;
        let mut row = String::new();
        let _ = write!(
            row,
            "{net_id},{},{},{},{},{x:.1},{y:.1},{z:.1},{},{},{},{resource},{flags}",
            seen.serial,
            entity.kind,
            entity.class,
            entity.model,
            entity.owner.map_or(-1, i64::from),
            entity.bucket,
            entity.population,
        );
        Some(row)
    }

    /// The bridge's answer to `deleteEntities`, at `now` in Unix seconds, or none for a request it
    /// does not take.
    pub fn delete(&mut self, payload: &Value, now: i64) -> Option<Value> {
        let request = payload["request"].as_u64()?;
        let deadline = payload["deadline"].as_i64()?;
        let targets: Vec<u64> = payload["targets"]
            .as_array()?
            .iter()
            .map(Value::as_u64)
            .collect::<Option<_>>()?;
        if targets.is_empty() || !targets.len().is_multiple_of(2) || targets.len() / 2 > DELETE_MAX {
            return None;
        }
        let epoch = payload["epoch"].as_u64();
        let outcomes: Vec<Value> = targets
            .chunks(2)
            .map(|pair| {
                let net_id = u16::try_from(pair[0]).unwrap_or(0);
                let outcome = if epoch != Some(self.epoch) {
                    "changed"
                } else if now > deadline {
                    "late"
                } else {
                    self.delete_one(net_id, pair[1])
                };
                json!([net_id, outcome])
            })
            .collect();
        Some(
            json!({ "type": "observer", "kind": "entitiesDeleted", "request": request, "outcomes": outcomes }),
        )
    }

    fn delete_one(&mut self, net_id: u16, serial: u64) -> &'static str {
        let Some(entity) = self.entities.get(&net_id) else {
            return "gone";
        };
        let listed = self.seen.get(&net_id).is_some_and(|seen| {
            u64::from(seen.serial) == serial && seen.kind == entity.kind && seen.model == entity.model
        });
        if !listed {
            return "changed";
        }
        if entity.player.is_some() {
            return "player";
        }
        if entity.kind == VEHICLE && entity.occupant.is_some() {
            return "occupied";
        }
        if entity.class == TRAIN {
            return "train";
        }
        if entity.stays {
            return "unconfirmed";
        }
        self.entities.remove(&net_id);
        self.seen.remove(&net_id);
        "deleted"
    }
}

/// A model's hash that depends on what the entity is: a few distinct ones of each kind.
fn model_of(kind: u8, class: u8, variant: u16) -> u32 {
    const VEHICLES: [u32; 4] = [0xB779_A091, 0x3C4E_2113, 0x9F05_F101, 0x5C23_AF9B];
    const PEDS: [u32; 3] = [0x705E_61F2, 0x9C9E_FFD8, 0x1DF5_A2C3];
    const OBJECTS: [u32; 5] = [0x0A1B_2C3D, 0x6B79_5EBC, 0xE0FA_F6D2, 0x4C80_EB0E, 0x3ADB_3357];
    let variant = usize::from(variant);
    match kind {
        VEHICLE if class == TRAIN => 0x3D6A_AA9B,
        VEHICLE if class == 4 => 0x2C63_4FBD,
        VEHICLE => VEHICLES[variant % VEHICLES.len()],
        PED => PEDS[variant % PEDS.len()],
        _ => OBJECTS[variant % OBJECTS.len()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    fn server(players: usize) -> Server {
        Server {
            players,
            onesync: true,
            gta: true,
            every_ms: 15_000,
            row_us: 2,
            list_us: None,
        }
    }

    fn rows(parts: &[Value]) -> Vec<String> {
        parts
            .iter()
            .flat_map(|part| part["rows"].as_str().unwrap().split(';'))
            .filter(|row| !row.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_sweep_lists_vehicles_objects_then_peds_under_serials_that_last() {
        let mut world = World::default();
        world.bridge_started(4210);
        world.add(&words("ped 10 20 30"));
        world.add(&words("vehicle 412.64 -1208.31 32.1 qb-garages 4 2"));
        world.add(&words("object 1 2 3 ox_inventory"));

        let first = world.sweep(&server(2));
        let again = world.sweep(&server(2));

        assert_eq!(
            rows(&first),
            [
                "2,1,2,1,1011753235,412.6,-1208.3,32.1,4,2,7,1,0",
                "3,2,3,0,3774543570,1.0,2.0,3.0,-1,0,7,2,0",
                "1,3,1,0,1885233650,10.0,20.0,30.0,-1,0,5,0,0",
            ]
        );
        assert_eq!(rows(&again), rows(&first));
        let last = first.last().unwrap();
        assert_eq!(
            (last["epoch"].as_u64(), last["seq"].as_u64()),
            (Some(4210), Some(1))
        );
        assert_eq!(again.last().unwrap()["seq"], 2);
        assert_eq!(
            last["counts"],
            json!({ "vehicles": 1, "objects": 1, "peds": 3, "playerPeds": 2 })
        );
        assert_eq!(last["resources"], json!(["qb-garages", "ox_inventory"]));
        assert_eq!(last["everyMs"], 15_000);
        assert_eq!(last["truncated"], false);
    }

    #[test]
    fn a_large_world_comes_in_parts_and_looks_resources_up_a_few_hundred_a_sweep() {
        let mut world = World::default();
        world.bridge_started(1);
        world.fill(0, 0, 1200);

        let first = world.sweep(&server(0));
        let second = world.sweep(&server(0));

        assert_eq!(first.len(), 3);
        assert_eq!(first[0]["last"], false);
        assert_eq!(first[2]["last"], true);
        assert_eq!(rows(&first).len(), 1200);
        let pending = |parts: &[Value]| rows(parts).iter().filter(|row| row.ends_with(",2")).count();
        assert_eq!(pending(&first), 900);
        assert_eq!(pending(&second), 600);
    }

    #[test]
    fn a_costly_sweep_is_cut_short_and_spaces_the_next_ones_out() {
        let mut world = World::default();
        world.bridge_started(1);
        world.fill(0, 0, 4000);
        let costly = Server {
            row_us: 20,
            ..server(0)
        };

        let sweep = world.sweep(&costly);

        let last = sweep.last().unwrap();
        assert_eq!(last["truncated"], true);
        assert_eq!(last["counts"]["objects"], 4000);
        assert!(rows(&sweep).len() < 4000);
        assert_eq!(last["spaced"], true);
        assert_eq!(last["everyMs"], 30_000);
        world.sweep(&costly);
        assert_eq!(world.cadence_ms(15_000), 60_000);
        world.sweep(&costly);
        assert_eq!(world.cadence_ms(15_000), 60_000, "never past a minute");
        world.sweep(&server(0));
        assert_eq!(
            world.cadence_ms(15_000),
            30_000,
            "a cheap sweep brings them closer again"
        );
    }

    #[test]
    fn a_server_without_onesync_or_another_game_lists_nothing_and_says_why() {
        let mut world = World::default();
        world.bridge_started(1);
        world.add(&words("ped 10 20 30"));

        for (server, why) in [
            (
                Server {
                    onesync: false,
                    ..server(0)
                },
                "onesync",
            ),
            (
                Server {
                    gta: false,
                    ..server(0)
                },
                "game",
            ),
        ] {
            let sweep = world.sweep(&server);

            assert_eq!(sweep.len(), 1);
            assert_eq!(sweep[0]["unavailable"], why);
            assert_eq!(sweep[0]["rows"], "");
        }
    }

    #[test]
    fn a_deletion_checks_each_entity_again() {
        let mut world = World::default();
        world.bridge_started(4210);
        for line in [
            "vehicle 1 1 1",
            "vehicle 2 2 2",
            "train 3 3 3",
            "ped 4 4 4",
            "object 5 5 5",
            "object 6 6 6",
            "object 7 7 7",
        ] {
            world.add(&words(line));
        }
        world.sweep(&server(1));
        world.enter(1, Some(2));
        world.possess(1, 4);
        world.remove(6);
        world.keep(7);
        // Serials follow the sweep's order: vehicles 1 to 3, objects 4 to 6, the ped 7.
        let ask = |targets: Value, epoch: u64, deadline: i64| json!({ "request": 9, "epoch": epoch, "deadline": deadline, "targets": targets });

        let reply = world
            .delete(
                &ask(json!([1, 1, 2, 2, 3, 3, 4, 7, 5, 9, 6, 5, 7, 6]), 4210, 100),
                100,
            )
            .unwrap();

        assert_eq!(reply["request"], 9);
        assert_eq!(
            reply["outcomes"],
            json!([
                [1, "deleted"],
                [2, "occupied"],
                [3, "train"],
                [4, "player"],
                [5, "changed"],
                [6, "gone"],
                [7, "unconfirmed"],
            ])
        );
        assert_eq!(
            world.delete(&ask(json!([5, 4]), 4210, 99), 100).unwrap()["outcomes"],
            json!([[5, "late"]])
        );
        assert_eq!(
            world.delete(&ask(json!([5, 4]), 1, 100), 100).unwrap()["outcomes"],
            json!([[5, "changed"]])
        );
        assert_eq!(
            world.delete(&ask(json!([5, 4]), 4210, 100), 100).unwrap()["outcomes"],
            json!([[5, "deleted"]])
        );
        assert!(world.delete(&ask(json!([]), 4210, 100), 100).is_none());
        assert!(world.delete(&ask(json!([5]), 4210, 100), 100).is_none());
    }
}
