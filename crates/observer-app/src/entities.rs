//! The server's entities: the list the bridge sweeps while a panel looks at it, what the panel
//! asks of that list, and the deletions. The list stays in memory and goes once nobody looks;
//! only a deletion leaves a trace, in the audit log.

mod query;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::Timestamp;

use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::entities::{
    CELLS, EntitiesLive, EntitiesSettings, EntityDeleteRequest, EntityDeletion, EntityFilters, EntityLimits,
    EntityMap, EntityOutcome, EntityPage, EntityResult, EntitySort, EntityTarget, MAX_DELETE, MAX_MAP_POINTS,
    VehicleClass,
};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::map::MapBounds;
use observer_domain::permissions::Permissions;
use observer_fxserver::bridge::BridgeAction;
use observer_fxserver::entities::{
    DeleteReply, EntityReport, EntityReportKind, EntityRow, MAX_PARTS, SweepEnd, SweepPart, read_rows,
};
use observer_fxserver::players::{PlayerReport, PlayerReportKind};
use observer_fxserver::supervisor::{State, Status, Supervisor};
use observer_store::Database;
use observer_store::entities as stored;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use self::query::Source;
use crate::audit::Record;
use crate::watching::{Watchers, Watching};
use crate::{Caller, Clock, Error, Players, Supervision};

/// How long one ask for the list lasts, and how often it is renewed while a panel looks.
const LEASE: Duration = Duration::from_secs(30);
const RENEW_EVERY: Duration = Duration::from_secs(10);
/// How long the server has to answer a deletion.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);
/// More than the bridge sends in a whole sweep.
const MAX_SWEEP_BYTES: usize = 4 * 1024 * 1024;

pub struct EntitiesSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
    /// Who is connected, for the owners' names; `None` while the players are not followed.
    pub players: Option<Players>,
}

/// The entities service. Clones share it.
#[derive(Clone)]
pub struct Entities {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    supervisor: Supervisor,
    players: Option<Players>,
    /// The last whole sweep, while someone looks.
    list: watch::Sender<Option<Arc<List>>>,
    live: watch::Sender<EntitiesLive>,
    watchers: Watchers,
    /// The cadence last asked of the bridge, in milliseconds.
    asked_ms: AtomicU32,
    /// One deletion at a time: a second one is refused, never queued behind a list that the
    /// first is changing.
    deleting: tokio::sync::Mutex<()>,
    awaited: Mutex<Option<Awaited>>,
    requests: AtomicU32,
}

/// The deletion the server has yet to answer.
struct Awaited {
    request: u32,
    generation: u32,
    answer: oneshot::Sender<Vec<(u16, EntityOutcome)>>,
}

/// A whole sweep, and its state as the panel was told it.
struct List {
    live: EntitiesLive,
    /// The start of FXServer and of its bridge the net ids and serials belong to.
    generation: u32,
    epoch: i64,
    rows: Vec<EntityRow>,
    /// The names the rows point into, from 1.
    resources: Vec<String>,
}

/// The parts of the sweep being received.
struct Gathering {
    generation: u32,
    epoch: i64,
    seq: u32,
    next: u32,
    parts: Vec<String>,
    bytes: usize,
}

pub struct EntitiesTask {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl EntitiesTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the entities' task failed");
        }
    }
}

impl Entities {
    pub fn start(setup: EntitiesSetup) -> (Self, EntitiesTask) {
        let supervisor = setup.supervision.supervisor().clone();
        let reports = supervisor.entity_reports();
        let players = supervisor.player_reports();
        let status = supervisor.status();
        let entities = Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                supervisor,
                players: setup.players,
                list: watch::Sender::new(None),
                live: watch::Sender::new(EntitiesLive::default()),
                watchers: Watchers::new(),
                asked_ms: AtomicU32::new(0),
                deleting: tokio::sync::Mutex::new(()),
                awaited: Mutex::new(None),
                requests: AtomicU32::new(0),
            }),
        };
        let stop = CancellationToken::new();
        let task = tokio::spawn(entities.clone().run(stop.clone(), reports, players, status));
        (entities, EntitiesTask { stop, task })
    }

    /// The state of the list and its changes, for as long as the guard is held: the server lists
    /// its entities only while someone holds one.
    pub fn watch(&self) -> (EntitiesLive, watch::Receiver<EntitiesLive>, Watching) {
        let watching = self.inner.watchers.join();
        let mut live = self.inner.live.subscribe();
        let snapshot = live.borrow_and_update().clone();
        (snapshot, live, watching)
    }

    async fn run(
        self,
        stop: CancellationToken,
        mut reports: broadcast::Receiver<EntityReport>,
        mut players: broadcast::Receiver<PlayerReport>,
        mut status: watch::Receiver<Status>,
    ) {
        let mut watchers = self.inner.watchers.subscribe();
        let mut renew = tokio::time::interval(RENEW_EVERY);
        let mut gathering = None;
        let mut watched = false;
        loop {
            tokio::select! {
                () = stop.cancelled() => {
                    // Turned off while a panel looks: the server stops listing at once.
                    if watched {
                        self.ask(false).await;
                    }
                    self.forget();
                    self.unanswered();
                    return;
                }
                Ok(()) = watchers.changed() => {
                    let now_watched = *watchers.borrow_and_update() > 0;
                    if now_watched != watched {
                        watched = now_watched;
                        if !watched {
                            gathering = None;
                            self.forget();
                        }
                        self.ask(watched).await;
                    }
                }
                _ = renew.tick() => {
                    if watched {
                        self.ask(true).await;
                    }
                }
                report = reports.recv() => match report {
                    Ok(report) => self.on_report(report, &mut gathering, watched),
                    // Parts of a sweep are gone: the next one starts over.
                    Err(RecvError::Lagged(_)) => gathering = None,
                    Err(RecvError::Closed) => return,
                },
                report = players.recv() => match report {
                    // The bridge started again: it forgot what it was asked, and numbers the
                    // entities from 1 again.
                    Ok(PlayerReport { kind: PlayerReportKind::Roster(page), .. }) if page.page == 1 => {
                        gathering = None;
                        self.forget();
                        self.unanswered();
                        if watched {
                            self.ask(true).await;
                        }
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                },
                Ok(()) = status.changed() => {
                    let running = matches!(
                        status.borrow_and_update().state,
                        State::Starting | State::Online | State::Degraded | State::Hung
                    );
                    if !running {
                        gathering = None;
                        self.forget();
                        self.unanswered();
                    }
                }
            }
        }
    }

    /// Asks the bridge for the list for a while, or to stop.
    async fn ask(&self, watched: bool) {
        let every_ms = match self.settings().await {
            Ok(settings) => u32::from(settings.every_seconds) * 1000,
            Err(error) => {
                warn!(%error, "the entities' settings could not be read");
                return;
            }
        };
        self.inner.asked_ms.store(every_ms, Ordering::Relaxed);
        let for_ms = if watched {
            u32::try_from(LEASE.as_millis()).unwrap_or(u32::MAX)
        } else {
            0
        };
        // A server not running has nothing to list; the next renewal asks again.
        let _ = self
            .inner
            .supervisor
            .act(BridgeAction::Entities { every_ms, for_ms })
            .await;
    }

    /// Drops the list: nobody looks at it any more, or its net ids name nothing now.
    fn forget(&self) {
        self.inner.list.send_replace(None);
        self.inner.live.send_if_modified(|live| {
            let known = *live != EntitiesLive::default();
            if known {
                *live = EntitiesLive::default();
            }
            known
        });
    }

    /// The server that was to answer a deletion is gone.
    fn unanswered(&self) {
        self.awaited().take();
    }

    fn awaited(&self) -> std::sync::MutexGuard<'_, Option<Awaited>> {
        self.inner.awaited.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn on_report(&self, report: EntityReport, gathering: &mut Option<Gathering>, watched: bool) {
        match report.kind {
            EntityReportKind::Deleted(reply) => self.answered(report.generation, reply),
            // A sweep the bridge was still sending when the last panel left.
            EntityReportKind::Sweep(_) if !watched => {}
            EntityReportKind::Sweep(part) => {
                if let Some((gathered, end)) = gather(gathering, report.generation, part) {
                    self.publish(&gathered, end);
                }
            }
        }
    }

    fn answered(&self, generation: u32, reply: DeleteReply) {
        let mut awaited = self.awaited();
        if awaited
            .as_ref()
            .is_some_and(|awaited| awaited.request == reply.request && awaited.generation == generation)
            && let Some(awaited) = awaited.take()
        {
            let _ = awaited.answer.send(reply.outcomes);
        }
    }

    /// A whole sweep becomes the list.
    fn publish(&self, gathered: &Gathering, end: SweepEnd) {
        let at = Some(self.inner.clock.now());
        let asked_ms = self.inner.asked_ms.load(Ordering::Relaxed);
        if let Some(unavailable) = end.unavailable {
            self.inner.list.send_replace(None);
            self.inner.live.send_replace(EntitiesLive {
                at,
                unavailable: Some(unavailable),
                every_ms: asked_ms,
                ..EntitiesLive::default()
            });
            return;
        }
        let mut rows = Vec::new();
        for part in &gathered.parts {
            read_rows(part, end.resources.len(), &mut rows);
        }
        let live = EntitiesLive {
            run: Some(format!("{}-{}", gathered.generation, gathered.epoch)),
            sweep: gathered.seq,
            at,
            unavailable: None,
            counts: Some(end.counts),
            listed: u32::try_from(rows.len()).unwrap_or(u32::MAX),
            truncated: end.truncated,
            every_ms: end.every_ms.unwrap_or(asked_ms),
            spaced: end.spaced,
            work_us: end.work_us,
            list_us: end.list_us,
            frames: end.frames,
        };
        // The list first: whoever hears of the sweep reads it.
        self.inner.list.send_replace(Some(Arc::new(List {
            live: live.clone(),
            generation: gathered.generation,
            epoch: gathered.epoch,
            rows,
            resources: end.resources,
        })));
        self.inner.live.send_replace(live);
    }

    /// The list and its state, read together.
    fn current(&self) -> (Option<Arc<List>>, EntitiesLive) {
        let list = self.inner.list.borrow().clone();
        let live = match &list {
            Some(list) => list.live.clone(),
            None => self.inner.live.borrow().clone(),
        };
        (list, live)
    }

    /// The connected players' names by net id, as the live map shows them.
    async fn names(&self, shown: bool) -> HashMap<u16, String> {
        let Some(players) = self.inner.players.as_ref().filter(|_| shown) else {
            return HashMap::new();
        };
        match players.online().await {
            Ok(online) => online
                .players
                .into_iter()
                .filter_map(|player| {
                    let name = player.label.and_then(|label| label.name).or(player.name)?;
                    Some((u16::try_from(player.net_id).ok()?, name))
                })
                .collect(),
            Err(error) => {
                warn!(%error, "the connected players could not be read for the entities");
                HashMap::new()
            }
        }
    }

    /// One page of the entities the filters keep. Who owns an entity, and whether a player sits
    /// in one, is only told to who may see where players stand.
    pub async fn page(
        &self,
        caller: &Caller,
        filters: &EntityFilters,
        sort: EntitySort,
        descending: bool,
        page: u32,
    ) -> Result<EntityPage, Error> {
        let owners_shown = caller.permissions.contains(Permissions::PLAYERS_MAP);
        let names = self.names(owners_shown).await;
        let (list, live) = self.current();
        let (rows, resources): (&[EntityRow], &[String]) = match &list {
            Some(list) => (&list.rows, &list.resources),
            None => (&[], &[]),
        };
        let source = Source {
            rows,
            resources,
            names: &names,
            owners_shown,
        };
        let mut kept = source.filtered(filters);
        source.sort(&mut kept, sort, descending);
        let (page, items) = source.page(&kept, page);
        Ok(EntityPage {
            live,
            owners_shown,
            limits: EntityLimits::default(),
            matching: u32::try_from(kept.len()).unwrap_or(u32::MAX),
            page,
            items,
            facets: source.facets(&kept),
        })
    }

    /// The entities the filters keep, for the map, within the part of it the panel shows when it
    /// gives one: each entity, or cells of `cell` game units once there are too many to draw.
    pub async fn map(
        &self,
        caller: &Caller,
        filters: &EntityFilters,
        cell: u32,
        bounds: Option<MapBounds>,
    ) -> Result<EntityMap, Error> {
        if !CELLS.contains(&cell) || bounds.is_some_and(|bounds| !bounds.is_valid()) {
            return Err(ErrorCode::InvalidRequest.into());
        }
        let owners_shown = caller.permissions.contains(Permissions::PLAYERS_MAP);
        let names = self.names(owners_shown).await;
        let (list, live) = self.current();
        let (rows, resources): (&[EntityRow], &[String]) = match &list {
            Some(list) => (&list.rows, &list.resources),
            None => (&[], &[]),
        };
        let kept = Source {
            rows,
            resources,
            names: &names,
            owners_shown,
        }
        .filtered(filters);
        let kept = match &bounds {
            Some(bounds) => query::within(kept, bounds),
            None => kept,
        };
        let dense = kept.len() > MAX_MAP_POINTS;
        Ok(EntityMap {
            live,
            kinds: query::kinds(rows),
            matching: u32::try_from(kept.len()).unwrap_or(u32::MAX),
            points: if dense { Vec::new() } else { query::points(&kept) },
            cell: dense.then_some(cell),
            cells: if dense {
                query::cells(&kept, cell)
            } else {
                Vec::new()
            },
        })
    }

    /// Deletes entities the caller picked in the list named by the request, and says what became
    /// of each. Never a vehicle a player sits in, nor a train, and nothing from a list that is
    /// out of date or that the server's restart voided; the bridge checks each entity again.
    pub async fn delete(
        &self,
        caller: &Caller,
        request: EntityDeleteRequest,
    ) -> Result<EntityDeletion, Error> {
        let (entities, caller) = (self.clone(), caller.clone());
        // On a task of its own: a deletion sent to the server is recorded even when the panel that
        // asked for it goes away before the answer.
        tokio::spawn(async move { entities.delete_now(&caller, request).await })
            .await
            .map_err(|_| Error::from(ErrorCode::Internal))?
    }

    async fn delete_now(
        &self,
        caller: &Caller,
        request: EntityDeleteRequest,
    ) -> Result<EntityDeletion, Error> {
        if request.targets.is_empty() {
            return Err(ErrorCode::InvalidRequest.into());
        }
        if request.targets.len() > MAX_DELETE {
            return Err(ApiError::new(ErrorCode::TooManyEntities)
                .with("max", MAX_DELETE)
                .into());
        }
        let Ok(_one_at_a_time) = self.inner.deleting.try_lock() else {
            return Err(ErrorCode::EntitiesDeleting.into());
        };
        let list = self
            .inner
            .list
            .borrow()
            .clone()
            .ok_or(ErrorCode::EntitiesNotListed)?;
        if list.live.run.as_deref() != Some(request.run.as_str()) {
            return Err(ErrorCode::EntitiesChanged.into());
        }
        let now = self.inner.clock.now();
        if list.live.is_stale(now) {
            return Err(ErrorCode::EntitiesOutOfDate.into());
        }
        let (mut results, sent) = ruled_out(&list, &request.targets);
        if sent.is_empty() {
            return Ok(EntityDeletion { results });
        }

        let outcomes = self.ask_to_delete(&list, &sent, now).await?;
        if let Some(outcomes) = &outcomes {
            for result in results
                .iter_mut()
                .filter(|result| result.outcome == EntityOutcome::Unanswered)
            {
                result.outcome = outcomes
                    .iter()
                    .find(|(net_id, _)| *net_id == result.net_id)
                    .map_or(EntityOutcome::Error, |(_, outcome)| *outcome);
            }
        }
        let deleted: Vec<&EntityRow> = sent
            .iter()
            .filter(|row| {
                results
                    .iter()
                    .any(|result| result.net_id == row.net_id && result.outcome == EntityOutcome::Deleted)
            })
            .copied()
            .collect();
        // What an unanswered deletion asked for is told, since some of it may be gone.
        let told = if outcomes.is_some() { &deleted } else { &sent };
        let record = Record::by(caller, AuditAction::EntitiesDeleted)
            .outcome(if outcomes.is_some() {
                Outcome::Success
            } else {
                Outcome::Failure
            })
            .details(deletion_details(&results, deleted.len(), told, &list.resources));
        let net_ids: Vec<u16> = deleted.iter().map(|row| row.net_id).collect();
        self.without(&list, &net_ids);
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> { Ok(record.write(tx, now)?) })
            .await?;
        Ok(EntityDeletion { results })
    }

    /// Has the bridge delete `sent`, entities of `list`, and waits for what became of each: `None`
    /// when it does not answer in time.
    async fn ask_to_delete(
        &self,
        list: &List,
        sent: &[&EntityRow],
        now: Timestamp,
    ) -> Result<Option<Vec<(u16, EntityOutcome)>>, Error> {
        let request = self.inner.requests.fetch_add(1, Ordering::Relaxed) % 0x7fff_ffff + 1;
        let (answer, answered) = oneshot::channel();
        *self.awaited() = Some(Awaited {
            request,
            generation: list.generation,
            answer,
        });
        let wait_ms = i64::try_from(ANSWER_WITHIN.as_millis()).unwrap_or(i64::MAX);
        let action = BridgeAction::DeleteEntities {
            request,
            epoch: list.epoch,
            // The bridge refuses from the second that follows: before the wait for its answer
            // ends, so that a deletion told as unanswered deletes nothing afterwards.
            deadline: (now.as_millisecond() + wait_ms) / 1000 - 1,
            targets: sent
                .iter()
                .flat_map(|row| [u32::from(row.net_id), row.serial])
                .collect(),
        };
        if self.inner.supervisor.act(action).await.is_err() {
            self.unanswered();
            return Err(ErrorCode::ServerNotRunning.into());
        }
        let outcomes = tokio::time::timeout(ANSWER_WITHIN, answered)
            .await
            .ok()
            .and_then(Result::ok);
        self.unanswered();
        Ok(outcomes)
    }

    /// Takes deleted entities out of the list they were deleted from, so that it shows the
    /// deletion before the next sweep confirms it.
    fn without(&self, list: &Arc<List>, deleted: &[u16]) {
        if deleted.is_empty() {
            return;
        }
        let mut live = None;
        self.inner.list.send_if_modified(|current| {
            let Some(shown) = current.as_ref().filter(|shown| Arc::ptr_eq(shown, list)) else {
                return false;
            };
            let rows: Vec<EntityRow> = shown
                .rows
                .iter()
                .filter(|row| !deleted.contains(&row.net_id))
                .copied()
                .collect();
            let next = List {
                live: EntitiesLive {
                    listed: u32::try_from(rows.len()).unwrap_or(u32::MAX),
                    ..shown.live.clone()
                },
                generation: shown.generation,
                epoch: shown.epoch,
                rows,
                resources: shown.resources.clone(),
            };
            live = Some(next.live.clone());
            *current = Some(Arc::new(next));
            true
        });
        if let Some(live) = live {
            self.inner.live.send_replace(live);
        }
    }

    pub async fn settings(&self) -> Result<EntitiesSettings, Error> {
        Ok(self.inner.db.read(stored::settings).await?)
    }

    /// Changes how often the server lists its entities, which applies at the next ask.
    pub async fn change_settings(&self, caller: &Caller, settings: EntitiesSettings) -> Result<(), Error> {
        if !settings.is_valid() {
            return Err(ErrorCode::InvalidEntitiesSettings.into());
        }
        let before = self.settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::EntitiesSettingsChanged)
            .details(json!({ "from": before, "to": settings }));
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                stored::save_settings(tx, &settings)?;
                Ok(record.write(tx, now)?)
            })
            .await?;
        if self.inner.watchers.any() {
            self.ask(true).await;
        }
        Ok(())
    }
}

/// What the list already says of each entity asked for, once each, and the rows to ask the
/// server about: those are told as unanswered until it answers.
fn ruled_out<'a>(list: &'a List, targets: &[EntityTarget]) -> (Vec<EntityResult>, Vec<&'a EntityRow>) {
    let listed: HashMap<u16, &EntityRow> = list.rows.iter().map(|row| (row.net_id, row)).collect();
    let mut results: Vec<EntityResult> = Vec::new();
    let mut sent = Vec::new();
    for target in targets {
        if results.iter().any(|result| result.net_id == target.net_id) {
            continue;
        }
        let outcome = match listed.get(&target.net_id) {
            None => EntityOutcome::Gone,
            Some(row) if row.serial != target.serial || row.model != target.model => EntityOutcome::Changed,
            Some(row) if row.occupied => EntityOutcome::Occupied,
            Some(row) if row.class == Some(VehicleClass::Train) => EntityOutcome::Train,
            Some(row) => {
                sent.push(*row);
                EntityOutcome::Unanswered
            }
        };
        results.push(EntityResult {
            net_id: target.net_id,
            outcome,
        });
    }
    (results, sent)
}

/// What the audit log keeps of a deletion: how many entities were asked for and deleted, the
/// count of each other outcome, and the largest groups of `told`.
fn deletion_details(
    results: &[EntityResult],
    deleted: usize,
    told: &[&EntityRow],
    resources: &[String],
) -> serde_json::Value {
    let (groups, more) = query::groups(told, resources);
    let mut others = serde_json::Map::new();
    for result in results
        .iter()
        .filter(|result| result.outcome != EntityOutcome::Deleted)
    {
        let count = others.entry(result.outcome.as_str()).or_insert(json!(0));
        *count = json!(count.as_u64().unwrap_or(0) + 1);
    }
    let mut details = json!({ "asked": results.len(), "deleted": deleted });
    if !others.is_empty() {
        details["outcomes"] = others.into();
    }
    if !groups.is_empty() {
        details["entities"] = groups.into();
    }
    if more > 0 {
        details["more"] = more.into();
    }
    details
}

/// Adds a part to the sweep being received, and gives the sweep back once its last part came. A
/// part lost or repeated spoils its sweep: only the first part of another one starts over.
fn gather(
    gathering: &mut Option<Gathering>,
    generation: u32,
    part: SweepPart,
) -> Option<(Gathering, SweepEnd)> {
    let continues = gathering.as_ref().is_some_and(|gathering| {
        gathering.generation == generation
            && gathering.epoch == part.epoch
            && gathering.seq == part.seq
            && gathering.next == part.part
    });
    if !continues {
        *gathering = (part.part == 1).then(|| Gathering {
            generation,
            epoch: part.epoch,
            seq: part.seq,
            next: 1,
            parts: Vec::new(),
            bytes: 0,
        });
    }
    let current = gathering.as_mut()?;
    current.bytes += part.rows.len();
    current.parts.push(part.rows);
    current.next += 1;
    // Past either, no part that could follow would make a sweep the bridge sends.
    let whole = part.end.filter(|_| current.bytes <= MAX_SWEEP_BYTES);
    if whole.is_none() && (current.bytes > MAX_SWEEP_BYTES || current.next > MAX_PARTS) {
        *gathering = None;
    }
    let end = whole?;
    gathering.take().map(|gathered| (gathered, end))
}

#[cfg(test)]
mod tests;
