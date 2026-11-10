//! What the server's resources count of themselves while Debug is on: events, state bag writes
//! and outgoing HTTP requests, totalled here over the last quarter of an hour and kept in memory.
//! A start of FXServer empties them. The hosts each resource called are the one thing written
//! down, so that a host called for the first time stands out.

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::AuditAction;
use observer_domain::debug::{
    BagKind, BagResourceRow, BagRow, BagTotals, DebugBags, DebugEvents, DebugHttp, DebugState, DebugStatus,
    DebugWindow, EventPlayerRow, EventResourceRow, EventRow, EventSender, EventTotals, EventWay, Flow,
    HttpRow, HttpTotals, LargestWrite, RefusedRow, TrafficPoint,
};
use observer_domain::error::ErrorCode;
use observer_domain::features::Feature;
use observer_fxserver::hook::{self, Report, Way};
use observer_fxserver::supervisor::{Event, EventKind, HookReport, Supervisor};
use observer_store::Database;
use observer_store::debug::{self as store, StoredHost};
use serde_json::json;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::audit::Record;
use crate::{Caller, Clock, Error, Features, Supervision};

/// The counts are kept in slices of this many seconds.
const SLICE_SECONDS: u32 = 5;
/// A quarter of an hour of them.
const SLICES: i64 = 180;
/// The rows one slice keeps apart of each list. Past it the rest is left out, and the page says
/// that it shows less than there was.
const SLICE_ROWS: usize = 4000;
/// The rows a page is given of each list.
const PAGE_ROWS: usize = 200;
const SENDERS: usize = 5;
const LARGEST: usize = 10;
const SILENT_NAMES: usize = 100;
const REPORTING: usize = 4000;
/// The hosts remembered, over every resource.
const MAX_HOSTS: usize = 5000;
/// How long every host is taken as known, from the first report Debug ever read.
const LEARNING: SignedDuration = SignedDuration::from_hours(24);

/// What [`DebugCounts::start`] works with.
pub struct DebugSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
    pub features: Features,
}

/// The Debug service. Clones share it.
#[derive(Clone)]
pub struct DebugCounts {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    supervisor: Supervisor,
    features: Features,
    counts: Mutex<Counts>,
    hosts: Mutex<Hosts>,
}

/// The task that reads the reports, until [`DebugTask::close`].
pub struct DebugTask {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl DebugTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the Debug task did not end cleanly");
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct EventSum {
    count: u64,
    bytes: u64,
    largest: u64,
    handler_us: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct BagSum {
    count: u64,
    bytes: u64,
    largest: u64,
    replicated: u64,
    replicated_bytes: u64,
}

type Name = Arc<str>;

/// What was counted in one slice of the window.
#[derive(Debug, Default)]
struct Slice {
    /// The slice's place in time: its seconds since the epoch over [`SLICE_SECONDS`].
    index: i64,
    /// By resource, way and event.
    events: HashMap<(Name, EventWay, Name), EventSum>,
    /// By event and client, for what clients sent.
    senders: HashMap<(Name, u32), Flow>,
    /// By resource, event and client.
    refused: HashMap<(Name, Name, Option<u32>), u64>,
    /// By resource, bag kind and key.
    bags: HashMap<(Name, BagKind, Name), BagSum>,
    largest: Vec<LargestWrite>,
    /// By resource, method and host.
    hosts: HashMap<(Name, Name, Name), Flow>,
    to_clients: u64,
    from_clients: u64,
    /// Less was kept than there was.
    cut: bool,
}

/// The counts of the start that runs.
#[derive(Debug, Default)]
struct Counts {
    generation: u32,
    since: Option<Timestamp>,
    reporting: BTreeSet<String>,
    /// What the bridge said is started, and at which start.
    started: (u32, Vec<String>),
    slices: VecDeque<Slice>,
    /// Reports were lost between FXServer and here.
    lagged: bool,
}

const fn way_of(way: Way) -> Option<EventWay> {
    match way {
        Way::Received => Some(EventWay::Local),
        Way::FromClient => Some(EventWay::FromClients),
        Way::FromEngine => Some(EventWay::FromEngine),
        Way::ToClient => Some(EventWay::ToClient),
        Way::ToAll => Some(EventWay::ToAll),
        // The resources that received it count it: counted here too, it would show twice.
        Way::Triggered => None,
    }
}

const fn kind_of(kind: hook::BagKind) -> BagKind {
    match kind {
        hook::BagKind::Global => BagKind::Global,
        hook::BagKind::Player => BagKind::Player,
        hook::BagKind::Entity => BagKind::Entity,
        hook::BagKind::LocalEntity => BagKind::LocalEntity,
        hook::BagKind::Other => BagKind::Other,
    }
}

/// The entry of `key`, unless the map is full and does not hold it.
fn row<'a, K: std::hash::Hash + Eq, V: Default>(
    map: &'a mut HashMap<K, V>,
    key: K,
    cut: &mut bool,
) -> Option<&'a mut V> {
    if map.len() >= SLICE_ROWS && !map.contains_key(&key) {
        *cut = true;
        return None;
    }
    Some(map.entry(key).or_default())
}

fn slice_of(at: Timestamp) -> i64 {
    at.as_second().div_euclid(i64::from(SLICE_SECONDS))
}

/// A count of rows, for the wire.
fn small(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

impl Slice {
    /// Adds the events one resource counted, with who sent them and what was refused.
    fn events(&mut self, resource: &Name, report: &Report) {
        for event in &report.events {
            let Some(way) = way_of(event.way) else {
                continue;
            };
            match way {
                EventWay::ToClient | EventWay::ToAll => self.to_clients += event.bytes,
                EventWay::FromClients => self.from_clients += event.bytes,
                EventWay::Local | EventWay::FromEngine => {}
            }
            let key = (Arc::clone(resource), way, event.name.as_str().into());
            if let Some(sum) = row(&mut self.events, key, &mut self.cut) {
                sum.count += event.count;
                sum.bytes += event.bytes;
                sum.largest = sum.largest.max(event.largest);
                sum.handler_us += event.handler_us;
            }
        }
        for sender in &report.senders {
            let key = (sender.name.as_str().into(), sender.player);
            if let Some(flow) = row(&mut self.senders, key, &mut self.cut) {
                flow.count += sender.count;
                flow.bytes += sender.bytes;
            }
        }
        for refused in &report.refused {
            let key = (Arc::clone(resource), refused.name.as_str().into(), refused.player);
            if let Some(count) = row(&mut self.refused, key, &mut self.cut) {
                *count += refused.count;
            }
        }
    }

    /// Adds the state bag writes and the requests one resource counted.
    fn writes(&mut self, resource: &Name, report: Report) {
        for bag in report.bags {
            let key = (Arc::clone(resource), kind_of(bag.kind), bag.key.as_str().into());
            if let Some(sum) = row(&mut self.bags, key, &mut self.cut) {
                sum.count += bag.count;
                sum.bytes += bag.bytes;
                sum.largest = sum.largest.max(bag.largest);
                sum.replicated += bag.replicated;
                // A key goes out to clients or stays on the server: its writes share the bytes.
                sum.replicated_bytes += bag
                    .bytes
                    .saturating_mul(bag.replicated.min(bag.count))
                    .checked_div(bag.count)
                    .unwrap_or(0);
            }
        }
        self.largest
            .extend(report.largest.into_iter().map(|write| LargestWrite {
                bag: write.bag,
                key: write.key,
                resource: resource.to_string(),
                bytes: write.bytes,
                replicated: write.replicated,
            }));
        self.largest.sort_by_key(|write| Reverse(write.bytes));
        self.largest.truncate(LARGEST);
        for host in report.hosts {
            let key = (
                Arc::clone(resource),
                host.method.as_str().into(),
                host.host.as_str().into(),
            );
            if let Some(flow) = row(&mut self.hosts, key, &mut self.cut) {
                flow.count += host.count;
                flow.bytes += host.bytes;
            }
        }
    }
}

/// An event in one way, while its rows are added up.
#[derive(Default)]
struct EventSums {
    sum: EventSum,
    resources: HashMap<Name, Flow>,
}

/// Who sent each event, by client.
type Senders = HashMap<Name, HashMap<u32, Flow>>;

fn event_rows(rows: HashMap<(Name, EventWay), EventSums>, senders: &Senders) -> Vec<EventRow> {
    let mut events: Vec<EventRow> = rows
        .into_iter()
        .map(|((name, way), row)| {
            let mut by: Vec<(Name, Flow)> = row.resources.into_iter().collect();
            by.sort_by(|a, b| (b.1.bytes, b.1.count, &a.0).cmp(&(a.1.bytes, a.1.count, &b.0)));
            let mut sent: Vec<EventSender> = senders
                .get(&name)
                .filter(|_| way == EventWay::FromClients)
                .into_iter()
                .flatten()
                .map(|(&player, flow)| EventSender {
                    player,
                    count: flow.count,
                    bytes: flow.bytes,
                })
                .collect();
            sent.sort_by_key(|by| (Reverse(by.count), by.player));
            sent.truncate(SENDERS);
            EventRow {
                name: name.to_string(),
                way,
                resource_count: small(by.len()),
                resources: by.into_iter().take(3).map(|(name, _)| name.to_string()).collect(),
                count: row.sum.count,
                bytes: row.sum.bytes,
                largest: row.sum.largest,
                handler_us: row.sum.handler_us,
                senders: sent,
            }
        })
        .collect();
    events.sort_by(|a, b| (b.bytes, b.count, &a.name, a.way).cmp(&(a.bytes, a.count, &b.name, b.way)));
    events.truncate(PAGE_ROWS);
    events
}

/// What each client sent in all, with the event it sent most.
fn player_rows(senders: &Senders) -> Vec<EventPlayerRow> {
    let mut players: HashMap<u32, (Flow, (u64, &Name))> = HashMap::new();
    for (name, by_player) in senders {
        for (player, flow) in by_player {
            let (total, top) = players.entry(*player).or_insert((Flow::default(), (0, name)));
            total.count += flow.count;
            total.bytes += flow.bytes;
            if (flow.count, name) > *top {
                *top = (flow.count, name);
            }
        }
    }
    let mut rows: Vec<EventPlayerRow> = players
        .into_iter()
        .map(|(player, (flow, (_, top)))| EventPlayerRow {
            player,
            count: flow.count,
            bytes: flow.bytes,
            top: top.to_string(),
        })
        .collect();
    rows.sort_by_key(|row| (Reverse(row.count), row.player));
    rows.truncate(PAGE_ROWS);
    rows
}

impl Counts {
    /// Adds what one resource counted, at `now`. The counts of an earlier start are dropped at
    /// the first report of a new one.
    fn add(&mut self, now: Timestamp, generation: u32, report: Report) {
        if generation != self.generation {
            let started = std::mem::take(&mut self.started);
            *self = Self {
                generation,
                started,
                ..Self::default()
            };
        }
        self.since.get_or_insert(now);
        if self.reporting.len() < REPORTING {
            self.reporting.insert(report.resource.clone());
        }
        let resource: Name = report.resource.as_str().into();
        let slice = self.slice(now);
        slice.cut |= report.overflow > 0;
        slice.events(&resource, &report);
        slice.writes(&resource, report);
    }

    /// Drops what was counted, as the feature is turned off: counting starts afresh when it is
    /// turned on again, and no window holds the gap in between. Which resources report is a fact
    /// of the start, and stays: one that has nothing to count says so once and no more.
    fn forget(&mut self) {
        self.since = None;
        self.slices.clear();
        self.lagged = false;
    }

    /// The slice `now` falls in, the last one kept. Those older than a quarter of an hour go.
    fn slice(&mut self, now: Timestamp) -> &mut Slice {
        let index = slice_of(now).max(self.slices.back().map_or(i64::MIN, |last| last.index));
        if self.slices.back().is_none_or(|last| last.index != index) {
            self.slices.push_back(Slice {
                index,
                ..Slice::default()
            });
        }
        while self
            .slices
            .front()
            .is_some_and(|first| first.index <= index - SLICES)
        {
            self.slices.pop_front();
        }
        let last = self.slices.len() - 1;
        &mut self.slices[last]
    }

    /// The slices of `window`, as of `now`.
    fn window(&self, now: Timestamp, window: DebugWindow) -> impl Iterator<Item = &Slice> {
        let last = slice_of(now);
        let first = last - i64::from(window.seconds() / SLICE_SECONDS);
        self.slices
            .iter()
            .filter(move |slice| slice.index > first && slice.index <= last)
    }

    /// The seconds of `window` that were counted as of `now`, so that a rate is not spread over
    /// time nothing was counted in: the start may be younger than the window, and the window opens
    /// with a slice. The whole window where nothing was counted at all.
    fn covered(&self, now: Timestamp, window: DebugWindow) -> u32 {
        let Some(since) = self.since else {
            return window.seconds();
        };
        let slices = i64::from(window.seconds() / SLICE_SECONDS);
        let opens = (slice_of(now) - slices + 1) * i64::from(SLICE_SECONDS);
        let counted = now.as_second() - since.as_second().max(opens);
        u32::try_from(counted.max(1)).unwrap_or(window.seconds())
    }

    /// Whether the whole of `slice` was counted as of `now`. The one this start began in and the
    /// one that is not over hold less, and would draw as a dip in the traffic.
    fn whole(&self, slice: &Slice, now: Timestamp) -> bool {
        let opens = slice.index * i64::from(SLICE_SECONDS);
        let over = opens + i64::from(SLICE_SECONDS) <= now.as_second();
        over && self.since.is_some_and(|since| since.as_second() <= opens)
    }

    fn cut(&self, now: Timestamp, window: DebugWindow) -> bool {
        self.lagged || self.window(now, window).any(|slice| slice.cut)
    }

    /// Who sent each event from a client, over `window`.
    fn senders(&self, now: Timestamp, window: DebugWindow) -> Senders {
        let mut senders = Senders::new();
        for slice in self.window(now, window) {
            for ((name, player), flow) in &slice.senders {
                let by = senders
                    .entry(Arc::clone(name))
                    .or_default()
                    .entry(*player)
                    .or_default();
                by.count += flow.count;
                by.bytes += flow.bytes;
            }
        }
        senders
    }

    /// The events clients sent that no resource opened to them, with how many there were and
    /// from how many clients.
    fn refused(&self, now: Timestamp, window: DebugWindow) -> (Vec<RefusedRow>, u64, u32) {
        let mut refused: HashMap<&(Name, Name, Option<u32>), u64> = HashMap::new();
        for slice in self.window(now, window) {
            for (key, count) in &slice.refused {
                *refused.entry(key).or_default() += count;
            }
        }
        let total = refused.values().sum();
        let players: HashSet<u32> = refused.keys().filter_map(|(_, _, player)| *player).collect();
        let mut rows: Vec<RefusedRow> = refused
            .into_iter()
            .map(|((resource, name, player), count)| RefusedRow {
                name: name.to_string(),
                player: *player,
                resource: resource.to_string(),
                count,
            })
            .collect();
        rows.sort_by(|a, b| (b.count, &a.name, a.player).cmp(&(a.count, &b.name, b.player)));
        rows.truncate(PAGE_ROWS);
        (rows, total, small(players.len()))
    }

    fn events(&self, now: Timestamp, window: DebugWindow) -> Events {
        let mut rows: HashMap<(Name, EventWay), EventSums> = HashMap::new();
        let mut resources: HashMap<Name, EventResourceRow> = HashMap::new();
        let mut totals = EventTotals::default();
        let mut series = Vec::new();
        for slice in self.window(now, window) {
            if self.whole(slice, now) {
                series.push(TrafficPoint {
                    at: Timestamp::from_second(slice.index * i64::from(SLICE_SECONDS)).unwrap_or(now),
                    seconds: SLICE_SECONDS,
                    to_clients: slice.to_clients,
                    from_clients: slice.from_clients,
                });
            }
            for ((resource, way, name), sum) in &slice.events {
                let row = rows.entry((Arc::clone(name), *way)).or_default();
                row.sum.count += sum.count;
                row.sum.bytes += sum.bytes;
                row.sum.largest = row.sum.largest.max(sum.largest);
                row.sum.handler_us += sum.handler_us;
                let by = row.resources.entry(Arc::clone(resource)).or_default();
                by.count += sum.count;
                by.bytes += sum.bytes;
                let of = resources
                    .entry(Arc::clone(resource))
                    .or_insert_with(|| EventResourceRow {
                        resource: resource.to_string(),
                        to_clients: Flow::default(),
                        from_clients: Flow::default(),
                        local: 0,
                        handler_us: 0,
                    });
                of.handler_us += sum.handler_us;
                totals.handler_us += sum.handler_us;
                let flows = match way {
                    EventWay::ToClient | EventWay::ToAll => [&mut of.to_clients, &mut totals.to_clients],
                    EventWay::FromClients => [&mut of.from_clients, &mut totals.from_clients],
                    EventWay::Local | EventWay::FromEngine => {
                        of.local += sum.count;
                        totals.local += if *way == EventWay::Local { sum.count } else { 0 };
                        continue;
                    }
                };
                for flow in flows {
                    flow.count += sum.count;
                    flow.bytes += sum.bytes;
                }
            }
        }
        let senders = self.senders(now, window);
        let (refused, refused_count, refused_players) = self.refused(now, window);
        totals.refused = refused_count;
        totals.refused_players = refused_players;

        let mut resources: Vec<EventResourceRow> = resources.into_values().collect();
        resources.sort_by(|a, b| {
            let weight = |row: &EventResourceRow| row.to_clients.bytes + row.from_clients.bytes;
            (weight(b), b.handler_us, &a.resource).cmp(&(weight(a), a.handler_us, &b.resource))
        });
        resources.truncate(PAGE_ROWS);

        Events {
            totals,
            series,
            players: player_rows(&senders),
            rows: event_rows(rows, &senders),
            resources,
            refused,
        }
    }

    fn bags(&self, now: Timestamp, window: DebugWindow) -> Bags {
        #[derive(Default)]
        struct Row {
            sum: BagSum,
            resources: HashMap<Name, u64>,
        }
        let mut rows: HashMap<(Name, BagKind), Row> = HashMap::new();
        let mut resources: HashMap<Name, (BagSum, HashSet<(Name, BagKind)>)> = HashMap::new();
        let mut largest: Vec<LargestWrite> = Vec::new();
        let mut totals = BagTotals::default();
        for slice in self.window(now, window) {
            for ((resource, kind, key), sum) in &slice.bags {
                let row = rows.entry((Arc::clone(key), *kind)).or_default();
                let (of, keys) = resources.entry(Arc::clone(resource)).or_default();
                for into in [&mut row.sum, of] {
                    into.count += sum.count;
                    into.bytes += sum.bytes;
                    into.largest = into.largest.max(sum.largest);
                    into.replicated += sum.replicated;
                    into.replicated_bytes += sum.replicated_bytes;
                }
                *row.resources.entry(Arc::clone(resource)).or_default() += sum.bytes;
                keys.insert((Arc::clone(key), *kind));
                totals.writes += sum.count;
                totals.bytes += sum.bytes;
                totals.replicated_bytes += sum.replicated_bytes;
            }
            largest.extend(slice.largest.iter().cloned());
        }
        totals.keys = small(rows.len());
        totals.resources = small(resources.len());
        largest.sort_by(|a, b| (b.bytes, &a.bag, &a.key).cmp(&(a.bytes, &b.bag, &b.key)));
        largest.dedup_by(|a, b| a.bag == b.bag && a.key == b.key && a.resource == b.resource);
        largest.truncate(LARGEST);

        let mut keys: Vec<BagRow> = rows
            .into_iter()
            .map(|((key, kind), row)| {
                let mut by: Vec<(Name, u64)> = row.resources.into_iter().collect();
                by.sort_by(|a, b| (b.1, &a.0).cmp(&(a.1, &b.0)));
                BagRow {
                    key: key.to_string(),
                    kind,
                    resource_count: small(by.len()),
                    resources: by.into_iter().take(3).map(|(name, _)| name.to_string()).collect(),
                    count: row.sum.count,
                    bytes: row.sum.bytes,
                    largest: row.sum.largest,
                    replicated: row.sum.replicated,
                }
            })
            .collect();
        keys.sort_by(|a, b| (b.bytes, b.count, &a.key, a.kind).cmp(&(a.bytes, a.count, &b.key, b.kind)));
        keys.truncate(PAGE_ROWS);

        let mut resources: Vec<BagResourceRow> = resources
            .into_iter()
            .map(|(resource, (sum, keys))| BagResourceRow {
                resource: resource.to_string(),
                count: sum.count,
                bytes: sum.bytes,
                replicated_bytes: sum.replicated_bytes,
                keys: small(keys.len()),
            })
            .collect();
        resources.sort_by(|a, b| (b.bytes, b.count, &a.resource).cmp(&(a.bytes, a.count, &b.resource)));
        resources.truncate(PAGE_ROWS);

        Bags {
            totals,
            keys,
            resources,
            largest,
        }
    }

    /// The requests of `window`, by resource, method and host.
    fn requests(&self, now: Timestamp, window: DebugWindow) -> Vec<(Name, Name, Name, Flow)> {
        let mut rows: HashMap<&(Name, Name, Name), Flow> = HashMap::new();
        for slice in self.window(now, window) {
            for (key, flow) in &slice.hosts {
                let row = rows.entry(key).or_default();
                row.count += flow.count;
                row.bytes += flow.bytes;
            }
        }
        rows.into_iter()
            .map(|((resource, method, host), flow)| {
                (Arc::clone(resource), Arc::clone(method), Arc::clone(host), flow)
            })
            .collect()
    }
}

struct Events {
    totals: EventTotals,
    series: Vec<TrafficPoint>,
    rows: Vec<EventRow>,
    resources: Vec<EventResourceRow>,
    players: Vec<EventPlayerRow>,
    refused: Vec<RefusedRow>,
}

struct Bags {
    totals: BagTotals,
    keys: Vec<BagRow>,
    resources: Vec<BagResourceRow>,
    largest: Vec<LargestWrite>,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    first: Timestamp,
    known: bool,
}

/// The hosts each resource called, as stored.
#[derive(Debug, Default)]
struct Hosts {
    seen: HashMap<(String, String), Seen>,
    learning_until: Option<Timestamp>,
}

impl Hosts {
    /// Takes note of the hosts of a report at `now`, and answers what has to be stored: the hosts
    /// seen for the first time, and the end of the day of learning when this report opens it.
    fn note(&mut self, now: Timestamp, report: &Report) -> (Vec<StoredHost>, Option<Timestamp>) {
        let opened = self
            .learning_until
            .is_none()
            .then(|| now.saturating_add(LEARNING).unwrap_or(now));
        self.learning_until = self.learning_until.or(opened);
        let learning = self.learning_until.is_some_and(|until| now < until);
        let mut fresh = Vec::new();
        for host in &report.hosts {
            let key = (report.resource.clone(), host.host.clone());
            if self.seen.len() >= MAX_HOSTS || self.seen.contains_key(&key) {
                continue;
            }
            self.seen.insert(
                key,
                Seen {
                    first: now,
                    known: learning,
                },
            );
            fresh.push(StoredHost {
                resource: report.resource.clone(),
                host: host.host.clone(),
                first_seen: now,
                known: learning,
            });
        }
        (fresh, opened)
    }
}

impl DebugCounts {
    /// Reads the hosts stored and starts following the reports. Nothing is counted while the
    /// feature is off: the reports of a server started when it was on are dropped.
    pub async fn start(setup: DebugSetup) -> Result<(Self, DebugTask), Error> {
        let supervisor = setup.supervision.supervisor().clone();
        let (stored, learning_until) = setup
            .db
            .read(|conn| Ok::<_, Error>((store::hosts(conn)?, store::learning_until(conn)?)))
            .await?;
        let hosts = Hosts {
            seen: stored
                .into_iter()
                .map(|host| {
                    let seen = Seen {
                        first: host.first_seen,
                        known: host.known,
                    };
                    ((host.resource, host.host), seen)
                })
                .collect(),
            learning_until,
        };
        let reports = supervisor.hook_reports();
        let events = supervisor.events();
        let counts = Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                supervisor,
                features: setup.features,
                counts: Mutex::new(Counts::default()),
                hosts: Mutex::new(hosts),
            }),
        };
        let stop = CancellationToken::new();
        let task = tokio::spawn(counts.clone().run(stop.clone(), reports, events));
        Ok((counts, DebugTask { stop, task }))
    }

    async fn run(
        self,
        stop: CancellationToken,
        mut reports: broadcast::Receiver<HookReport>,
        mut events: broadcast::Receiver<Event>,
    ) {
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                report = reports.recv() => match report {
                    Ok(report) => self.take(report).await,
                    Err(RecvError::Lagged(_)) => self.counts().lagged = true,
                    Err(RecvError::Closed) => return,
                },
                event = events.recv() => match event {
                    Ok(Event { generation, kind: EventKind::Resources { started, .. }, .. }) => {
                        self.counts().started = (generation, started);
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                },
            }
        }
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, Counts> {
        self.inner.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn hosts(&self) -> std::sync::MutexGuard<'_, Hosts> {
        self.inner.hosts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn take(&self, report: HookReport) {
        if !self.inner.features.runs(Feature::Debug) {
            self.counts().forget();
            return;
        }
        let now = self.inner.clock.now();
        let (fresh, opened) = self.hosts().note(now, &report.report);
        self.counts().add(now, report.generation, report.report);
        if fresh.is_empty() && opened.is_none() {
            return;
        }
        let stored = self
            .inner
            .db
            .write(move |tx| -> Result<(), Error> {
                if let Some(until) = opened {
                    store::set_learning_until(tx, until)?;
                }
                for host in &fresh {
                    store::add(tx, host)?;
                }
                Ok(())
            })
            .await;
        if let Err(error) = stored {
            warn!(%error, "the hosts the resources called could not be stored");
        }
    }

    /// Where the counting stands, for the line under every Debug page's title.
    pub fn status(&self) -> DebugStatus {
        let (generation, running, bridge) = {
            let server = self.inner.supervisor.status();
            let server = server.borrow();
            (server.generation, server.pid.is_some(), server.bridge.clone())
        };
        let hooked = self
            .inner
            .supervisor
            .hooked()
            .filter(|hooked| running && hooked.generation == generation);
        let counts = self.counts();
        let of_this_start = counts.generation == generation;
        let silent: Vec<String> = match &counts.started {
            (started, names) if *started == generation && hooked.is_some() => names
                .iter()
                // The bridge is Observer's own, and counts nothing.
                .filter(|name| bridge.as_ref() != Some(*name))
                .filter(|name| !(of_this_start && counts.reporting.contains(*name)))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        DebugStatus {
            state: match &hooked {
                Some(hooked) if hooked.refused.is_some() => DebugState::Refused,
                Some(_) => DebugState::Counting,
                None => DebugState::Waiting,
            },
            running,
            since: counts.since.filter(|_| of_this_start && hooked.is_some()),
            reporting: if of_this_start && hooked.is_some() {
                small(counts.reporting.len())
            } else {
                0
            },
            silent_count: small(silent.len()),
            silent: silent.into_iter().take(SILENT_NAMES).collect(),
            refused: hooked.and_then(|hooked| hooked.refused),
        }
    }

    pub fn events(&self, window: DebugWindow) -> DebugEvents {
        let status = self.status();
        let now = self.inner.clock.now();
        let counts = self.counts();
        let events = counts.events(now, window);
        DebugEvents {
            status,
            seconds: counts.covered(now, window),
            totals: events.totals,
            series: events.series,
            events: events.rows,
            resources: events.resources,
            players: events.players,
            refused: events.refused,
            truncated: counts.cut(now, window),
        }
    }

    pub fn bags(&self, window: DebugWindow) -> DebugBags {
        let status = self.status();
        let now = self.inner.clock.now();
        let counts = self.counts();
        let bags = counts.bags(now, window);
        DebugBags {
            status,
            seconds: counts.covered(now, window),
            totals: bags.totals,
            keys: bags.keys,
            resources: bags.resources,
            largest: bags.largest,
            truncated: counts.cut(now, window),
        }
    }

    pub fn http(&self, window: DebugWindow) -> DebugHttp {
        let status = self.status();
        let now = self.inner.clock.now();
        let (requests, truncated, seconds) = {
            let counts = self.counts();
            (
                counts.requests(now, window),
                counts.cut(now, window),
                counts.covered(now, window),
            )
        };
        let hosts = self.hosts();
        let mut totals = HttpTotals::default();
        let (mut called, mut callers, mut fresh) = (HashSet::new(), HashSet::new(), HashSet::new());
        let mut rows: Vec<HttpRow> = requests
            .into_iter()
            .map(|(resource, method, host, flow)| {
                let seen = hosts.seen.get(&(resource.to_string(), host.to_string()));
                let new = seen.is_some_and(|seen| !seen.known);
                totals.requests += flow.count;
                totals.bytes += flow.bytes;
                called.insert(Arc::clone(&host));
                callers.insert(Arc::clone(&resource));
                if new {
                    fresh.insert((Arc::clone(&resource), Arc::clone(&host)));
                }
                HttpRow {
                    host: host.to_string(),
                    method: method.to_string(),
                    resource: resource.to_string(),
                    count: flow.count,
                    bytes: flow.bytes,
                    first_called: seen.map(|seen| seen.first),
                    new,
                }
            })
            .collect();
        totals.hosts = small(called.len());
        totals.resources = small(callers.len());
        totals.new_hosts = small(fresh.len());
        rows.sort_by(|a, b| {
            (b.new, b.count, &a.host, &a.resource, &a.method).cmp(&(
                a.new,
                a.count,
                &b.host,
                &b.resource,
                &b.method,
            ))
        });
        rows.truncate(PAGE_ROWS);
        DebugHttp {
            status,
            seconds,
            totals,
            requests: rows,
            learning_until: hosts.learning_until,
            truncated,
        }
    }

    /// The owner knows why `resource` calls `host`: it stops showing as new, and that is audited.
    pub async fn know_host(&self, caller: &Caller, resource: String, host: String) -> Result<(), Error> {
        let now = self.inner.clock.now();
        let record = Record::by(caller, AuditAction::DebugHostKnown)
            .target(resource.as_str())
            .details(json!({ "host": host }));
        let key = (resource, host);
        let stored = key.clone();
        let known = self
            .inner
            .db
            .write(move |tx| -> Result<bool, Error> {
                if !store::know(tx, &stored.0, &stored.1)? {
                    return Ok(false);
                }
                record.write(tx, now)?;
                Ok(true)
            })
            .await?;
        if !known {
            return Err(ErrorCode::NotFound.into());
        }
        if let Some(seen) = self.hosts().seen.get_mut(&key) {
            seen.known = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use observer_fxserver::hook::{BagCount, EventCount, HostCount, RefusedCount, SenderCount};

    use super::*;

    fn at(time: &str) -> Timestamp {
        format!("2026-11-10T{time}Z").parse().unwrap()
    }

    fn event(way: Way, name: &str, count: u64, bytes: u64, largest: u64, handler_us: u64) -> EventCount {
        EventCount {
            way,
            name: name.to_owned(),
            count,
            bytes,
            largest,
            handler_us,
        }
    }

    fn voice() -> Report {
        Report {
            resource: "pma-voice".to_owned(),
            events: vec![
                event(Way::ToClient, "voice:talking", 186, 7626, 44, 0),
                event(Way::FromClient, "voice:talking", 24, 912, 40, 480),
                event(Way::Received, "playerDropped", 1, 48, 48, 30),
                event(Way::Triggered, "voice:internal", 5, 50, 10, 0),
            ],
            senders: vec![
                SenderCount {
                    name: "voice:talking".to_owned(),
                    player: 41,
                    count: 19,
                    bytes: 722,
                },
                SenderCount {
                    name: "voice:talking".to_owned(),
                    player: 7,
                    count: 5,
                    bytes: 190,
                },
            ],
            refused: vec![RefusedCount {
                name: "bank:give".to_owned(),
                player: Some(41),
                count: 3,
            }],
            bags: vec![BagCount {
                kind: hook::BagKind::Player,
                key: "proximity".to_owned(),
                count: 28,
                bytes: 252,
                largest: 9,
                replicated: 28,
            }],
            largest: vec![hook::LargestWrite {
                bag: "player:41".to_owned(),
                key: "proximity".to_owned(),
                bytes: 9,
                replicated: true,
            }],
            hosts: vec![HostCount {
                method: "POST".to_owned(),
                host: "discord.com".to_owned(),
                count: 2,
                bytes: 800,
            }],
            overflow: 0,
        }
    }

    #[test]
    fn a_rate_is_taken_over_the_seconds_counted_rather_than_over_the_whole_window() {
        let mut counts = Counts::default();
        assert_eq!(counts.covered(at("21:50:10"), DebugWindow::FiveMinutes), 300);
        counts.add(at("21:50:02"), 3, voice());
        counts.add(at("21:50:07"), 3, voice());
        counts.add(at("21:50:08"), 3, voice());

        // Eight seconds after the first report, five minutes hold eight seconds of counts.
        assert_eq!(counts.covered(at("21:50:10"), DebugWindow::FiveMinutes), 8);
        // The traffic is drawn slice by slice, once a slice is over, and never the one this start
        // began in: neither holds five seconds of counts.
        let drawn = |now| counts.events(at(now), DebugWindow::FiveMinutes).series;
        assert_eq!(drawn("21:50:09"), []);
        assert_eq!(
            drawn("21:50:10"),
            [TrafficPoint {
                at: at("21:50:05"),
                seconds: 5,
                to_clients: 15_252,
                from_clients: 1824,
            }]
        );
        // Later, a window is whole but for the part of its last slice still to come.
        assert_eq!(counts.covered(at("21:58:03"), DebugWindow::Minute), 58);
        assert_eq!(counts.covered(at("21:58:05"), DebugWindow::Minute), 55);
        // Eight minutes and seven seconds in, a quarter of an hour holds no more than that.
        assert_eq!(counts.covered(at("21:58:09"), DebugWindow::QuarterHour), 487);
    }

    #[test]
    fn turned_off_and_on_again_the_counting_starts_afresh_for_the_resources_already_known() {
        let mut counts = Counts::default();
        counts.add(at("21:50:01"), 3, voice());
        let idle = Report {
            resource: "hud".to_owned(),
            ..Report::default()
        };
        counts.add(at("21:50:01"), 3, idle);

        counts.forget();
        counts.add(at("21:52:00"), 3, voice());

        assert_eq!(counts.since, Some(at("21:52:00")));
        assert_eq!(counts.covered(at("21:52:30"), DebugWindow::FiveMinutes), 30);
        let events = counts.events(at("21:52:30"), DebugWindow::FiveMinutes);
        assert_eq!(
            events.totals.to_clients.count, 186,
            "what was counted before is gone"
        );
        // A resource with nothing to count reports once, at its start: it is still known.
        assert!(counts.reporting.contains("hud"));
    }

    #[test]
    fn events_are_totalled_by_way_with_who_sent_them_and_what_was_refused() {
        let mut counts = Counts::default();
        counts.add(at("21:50:01"), 3, voice());
        counts.add(at("21:50:02"), 3, voice());
        counts.add(
            at("21:50:02"),
            3,
            Report {
                resource: "hud".to_owned(),
                events: vec![event(Way::Received, "playerDropped", 1, 48, 96, 10)],
                ..Report::default()
            },
        );

        let events = counts.events(at("21:50:03"), DebugWindow::Minute);

        assert_eq!(
            (events.totals.to_clients.count, events.totals.to_clients.bytes),
            (372, 15_252)
        );
        assert_eq!(
            (events.totals.from_clients.count, events.totals.from_clients.bytes),
            (48, 1824)
        );
        assert_eq!(
            events.totals.local, 3,
            "what a resource triggers is counted where it lands"
        );
        assert_eq!(events.totals.handler_us, 2 * 510 + 10);
        assert_eq!((events.totals.refused, events.totals.refused_players), (6, 1));
        let names: Vec<_> = events
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row.way, row.count))
            .collect();
        assert_eq!(
            names,
            [
                ("voice:talking", EventWay::ToClient, 372),
                ("voice:talking", EventWay::FromClients, 48),
                ("playerDropped", EventWay::Local, 3),
            ]
        );
        let from_clients = &events.rows[1];
        assert_eq!(from_clients.resources, ["pma-voice"]);
        assert_eq!(
            from_clients
                .senders
                .iter()
                .map(|by| (by.player, by.count))
                .collect::<Vec<_>>(),
            [(41, 38), (7, 10)]
        );
        assert_eq!(events.rows[0].senders, Vec::new());
        let dropped = &events.rows[2];
        assert_eq!((dropped.resource_count, dropped.largest), (2, 96));
        assert_eq!(dropped.resources, ["pma-voice", "hud"], "by what they carried");
        assert_eq!(
            events.players[0],
            EventPlayerRow {
                player: 41,
                count: 38,
                bytes: 1444,
                top: "voice:talking".to_owned(),
            }
        );
        assert_eq!(
            events.refused,
            [RefusedRow {
                name: "bank:give".to_owned(),
                player: Some(41),
                resource: "pma-voice".to_owned(),
                count: 6,
            }]
        );
        assert_eq!(events.resources[0].resource, "pma-voice");
        assert_eq!(events.resources[1].local, 1);
    }

    #[test]
    fn a_window_holds_its_last_minutes_and_a_new_start_empties_the_counts() {
        let mut counts = Counts::default();
        counts.add(at("21:40:00"), 3, voice());
        counts.add(at("21:52:00"), 3, voice());
        counts.add(at("21:54:30"), 3, voice());
        let now = at("21:54:58");

        let sent = |window| counts.events(now, window).totals.to_clients.count;

        assert_eq!(sent(DebugWindow::Minute), 186);
        assert_eq!(sent(DebugWindow::FiveMinutes), 372);
        assert_eq!(sent(DebugWindow::QuarterHour), 558);
        assert_eq!(counts.since, Some(at("21:40:00")));
        // Sixteen minutes on, the first report is past every window, and dropped.
        counts.add(at("21:56:00"), 3, voice());
        assert_eq!(counts.slices.front().unwrap().index, slice_of(at("21:52:00")));

        counts.started = (4, vec!["pma-voice".to_owned()]);
        counts.add(at("21:57:00"), 4, voice());

        assert_eq!(counts.slices.len(), 1);
        assert_eq!(counts.since, Some(at("21:57:00")));
        assert_eq!(
            counts.started.0, 4,
            "what the bridge said of the new start is kept"
        );
    }

    #[test]
    fn state_bags_are_totalled_by_key_with_what_went_out_to_clients() {
        let mut counts = Counts::default();
        counts.add(at("21:50:01"), 3, voice());
        counts.add(
            at("21:50:01"),
            3,
            Report {
                resource: "garage".to_owned(),
                bags: vec![
                    BagCount {
                        kind: hook::BagKind::Entity,
                        key: "fuel".to_owned(),
                        count: 4,
                        bytes: 1240,
                        largest: 2100,
                        replicated: 4,
                    },
                    BagCount {
                        kind: hook::BagKind::Global,
                        key: "weather".to_owned(),
                        count: 2,
                        bytes: 128,
                        largest: 64,
                        replicated: 0,
                    },
                ],
                largest: vec![hook::LargestWrite {
                    bag: "entity:1042".to_owned(),
                    key: "fuel".to_owned(),
                    bytes: 2100,
                    replicated: true,
                }],
                ..Report::default()
            },
        );

        let bags = counts.bags(at("21:50:03"), DebugWindow::FiveMinutes);

        assert_eq!(
            bags.totals,
            BagTotals {
                writes: 34,
                bytes: 1620,
                replicated_bytes: 1492,
                keys: 3,
                resources: 2,
            }
        );
        assert_eq!(
            bags.keys
                .iter()
                .map(|row| (row.key.as_str(), row.kind, row.replicated))
                .collect::<Vec<_>>(),
            [
                ("fuel", BagKind::Entity, 4),
                ("proximity", BagKind::Player, 28),
                ("weather", BagKind::Global, 0),
            ]
        );
        assert_eq!(bags.resources[0].resource, "garage");
        assert_eq!(
            (bags.resources[0].keys, bags.resources[0].replicated_bytes),
            (2, 1240)
        );
        assert_eq!(bags.largest[0].bag, "entity:1042");
        assert_eq!(bags.largest[0].resource, "garage");
        assert_eq!(bags.largest.len(), 2);
    }

    #[test]
    fn a_slice_keeps_so_many_rows_and_says_when_there_were_more() {
        let mut counts = Counts::default();
        let many = (0..SLICE_ROWS + 10)
            .map(|index| event(Way::Received, &format!("event-{index}"), 1, 1, 1, 0))
            .collect();
        let now = at("21:50:01");

        counts.add(
            now,
            3,
            Report {
                resource: "noisy".to_owned(),
                events: many,
                ..Report::default()
            },
        );

        assert_eq!(counts.slices[0].events.len(), SLICE_ROWS);
        assert!(counts.cut(now, DebugWindow::Minute));
        assert_eq!(counts.events(now, DebugWindow::Minute).rows.len(), PAGE_ROWS);
        let mut whole = Counts::default();
        whole.add(now, 3, voice());
        assert!(!whole.cut(now, DebugWindow::Minute));
        whole.add(
            now,
            3,
            Report {
                overflow: 4,
                ..voice()
            },
        );
        assert!(
            whole.cut(now, DebugWindow::Minute),
            "the resource itself kept less"
        );
    }

    #[test]
    fn a_host_is_known_while_observer_learns_and_new_once_the_day_is_over() {
        let mut hosts = Hosts::default();
        let first = at("21:35:00");

        let (stored, opened) = hosts.note(first, &voice());

        let until = first.saturating_add(LEARNING).unwrap();
        assert_eq!(opened, Some(until));
        assert_eq!(
            stored,
            [StoredHost {
                resource: "pma-voice".to_owned(),
                host: "discord.com".to_owned(),
                first_seen: first,
                known: true,
            }]
        );
        assert_eq!(
            hosts.note(at("21:36:00"), &voice()),
            (Vec::new(), None),
            "seen already"
        );

        let later = until.saturating_add(SignedDuration::from_mins(5)).unwrap();
        let elsewhere = Report {
            hosts: vec![HostCount {
                method: "POST".to_owned(),
                host: "paste.example".to_owned(),
                count: 1,
                bytes: 2300,
            }],
            ..voice()
        };
        let (stored, opened) = hosts.note(later, &elsewhere);

        assert_eq!(opened, None);
        assert_eq!(
            (stored[0].host.as_str(), stored[0].known),
            ("paste.example", false)
        );
        let key = ("pma-voice".to_owned(), "paste.example".to_owned());
        assert_eq!(hosts.seen[&key].first, later);
    }
}
