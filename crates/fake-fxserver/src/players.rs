//! Players, as FXServer and the bridge show them: `/players.json` lists who joined, and with the
//! bridge started each join and drop is reported, and the players already there once it starts.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// The address every fake player connects from, which tests look for wherever Observer writes.
pub const ADDRESS: &str = "192.0.2.77";
/// The bridge sends at most this many players in one report.
const PAGE: usize = 100;

#[derive(Debug, Clone)]
pub struct Player {
    pub name: String,
    /// With the `ip:` one, as FXServer lists them.
    pub identifiers: Vec<String>,
    pub tokens: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Players {
    by_id: BTreeMap<u32, Player>,
    next_id: u32,
}

impl Players {
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// A new player: `name`, and the identifiers and tokens given, or a full set made from their id.
    pub fn join(
        &mut self,
        name: Option<&str>,
        identifiers: Option<&str>,
        tokens: Option<&str>,
    ) -> (u32, Player) {
        let player = self.coming(name, identifiers, tokens);
        (self.admit(player.clone()), player)
    }

    /// What the next player to join would show with these: a connecting player, before they join.
    /// The address shows IPv4-mapped for every other player, as dual-stack sockets give it.
    pub fn coming(&self, name: Option<&str>, identifiers: Option<&str>, tokens: Option<&str>) -> Player {
        let id = self.next_id + 1;
        let address = if id.is_multiple_of(2) {
            format!("ip:::ffff:{ADDRESS}")
        } else {
            format!("ip:{ADDRESS}")
        };
        let mut shown: Vec<String> = match identifiers {
            Some(list) => list.split(',').map(str::to_owned).collect(),
            None => vec![
                format!("license:{id:040x}"),
                format!("license2:{:040x}", u64::from(id) + 1_000_000),
                format!("steam:1100001{id:08x}"),
                format!("discord:{}", 100_000_000_000_000_000_u64 + u64::from(id)),
                format!("fivem:{id}"),
            ],
        };
        if !shown.iter().any(|identifier| identifier.starts_with("ip:")) {
            shown.push(address);
        }
        let tokens = match tokens {
            Some(list) => list.split(',').map(str::to_owned).collect(),
            None => (0..10 + id % 11)
                .map(|n| format!("{}:{:064x}", n % 5 + 2, u64::from(id) * 1000 + u64::from(n)))
                .collect(),
        };
        Player {
            name: name.map_or_else(|| format!("Player {id}"), str::to_owned),
            identifiers: shown,
            tokens,
        }
    }

    pub fn admit(&mut self, player: Player) -> u32 {
        self.next_id += 1;
        self.by_id.insert(self.next_id, player);
        self.next_id
    }

    pub fn leave(&mut self, id: u32) -> Option<Player> {
        self.by_id.remove(&id)
    }

    pub fn ids(&self) -> Vec<u32> {
        self.by_id.keys().copied().collect()
    }

    /// What `/players.json` lists, with the token: endpoints and every identifier.
    pub fn listed(&self) -> Vec<Value> {
        self.by_id
            .iter()
            .map(|(id, player)| {
                json!({
                    "endpoint": format!("{ADDRESS}:{}", 50_000 + id),
                    "id": id,
                    "identifiers": player.identifiers,
                    "name": player.name,
                    "ping": 30,
                })
            })
            .collect()
    }

    /// The bridge's `players` reports, a page per hundred players, one even when there are none.
    pub fn roster(&self, t: u64) -> Vec<Value> {
        let entries: Vec<Value> = self.by_id.iter().map(|(id, player)| entry(*id, player)).collect();
        let pages: Vec<&[Value]> = if entries.is_empty() {
            vec![&[]]
        } else {
            entries.chunks(PAGE).collect()
        };
        let count = pages.len();
        pages
            .into_iter()
            .enumerate()
            .map(|(index, players)| {
                json!({ "type": "observer", "kind": "players", "t": t, "page": index + 1,
                        "last": index + 1 == count, "players": players })
            })
            .collect()
    }
}

/// What the bridge reports of a player.
pub fn entry(id: u32, player: &Player) -> Value {
    json!({ "id": id, "identifiers": player.identifiers, "tokens": player.tokens, "name": player.name })
}

pub fn join_report(id: u32, player: &Player, t: u64) -> Value {
    let mut report = entry(id, player);
    report["type"] = json!("observer");
    report["kind"] = json!("join");
    report["t"] = json!(t);
    report
}

pub fn drop_report(id: u32, t: u64, category: u64, resource: &str, reason: &str) -> Value {
    json!({ "type": "observer", "kind": "drop", "id": id, "t": t, "reason": reason,
            "resource": resource, "category": category, "ped": true })
}
