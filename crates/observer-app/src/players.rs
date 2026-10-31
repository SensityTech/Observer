//! The players: who connects and for how long, what they show, and why they leave. One task owns
//! the sessions of the run under way; it writes identities and play time to the state database,
//! then sessions to the metrics database, once a second, so that a restart's thousands of joins
//! and drops are two transactions rather than thousands.

mod identity;
mod import;
mod redact;
mod sessions;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::drops::{DropCategory, DropKind};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::permissions::Permissions;
use observer_domain::players::{
    Community, CommunityDay, CommunityTotals, Confirmation, ImportState, OnlinePlayer, OnlinePlayers,
    PlayDay, PlayerPage, PlayerProfile, PlayerSort, PlayersImport, PlayersSettings, Search, SearchRefusal,
};
use observer_domain::schedule::ScheduleStatus;
use observer_fxserver::players::PlayerReport;
use observer_fxserver::supervisor::{Cause, Event, EventKind, Readings, State, Status};
use observer_store::metrics::runs::{self, RunKey, StartCause};
use observer_store::metrics::sessions::{self as stored_sessions, Closure};
use observer_store::players::{self as stored, Credit, Imported, PlayerFilter, PlayerQuery, Seen};
use observer_store::{Database, MetricsDatabase, StoreError};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use self::identity::{Fingerprints, Keys, hex, split_days};
use self::sessions::{Identity, IdentityWrite, SessionWrite, Sessions};
use crate::audit::Record;
use crate::metrics::runs::Change;
use crate::{Caller, Clock, Error, Metrics, Servers, Supervision};

const FLUSH_EVERY: Duration = Duration::from_secs(1);
/// Sessions wait this long for their run before the wait is logged.
const RUN_MISSING: Duration = Duration::from_secs(10);
const UPKEEP_EVERY: Duration = Duration::from_hours(1);
/// Sessions are kept this long after they end.
const SESSION_DAYS: i64 = 90;
const PAGE: u32 = 50;
const DROPS_SHOWN: u32 = 20;
const ACTIVITY_DAYS: i64 = 90;
const REQUESTS: usize = 16;
/// An import writes this many players per transaction, so that logins never wait long.
const IMPORT_BATCH: usize = 200;
const IMPORT_EVERY: Duration = Duration::from_millis(100);
/// Licences forgotten on request, as keyed fingerprints and dates: outside the backups, so that a
/// restored backup cannot bring a forgotten player back.
const FORGOTTEN_FILE: &str = "players-forgotten";

pub struct PlayersSetup {
    pub db: Database,
    pub metrics: Metrics,
    pub clock: Clock,
    pub supervision: Supervision,
    /// The schedule's status, whose time zone days are dates in.
    pub schedule: watch::Receiver<ScheduleStatus>,
    pub data_dir: PathBuf,
}

/// The players service. Clones share it.
#[derive(Clone)]
pub struct Players {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    metrics: MetricsDatabase,
    clock: Clock,
    schedule: watch::Receiver<ScheduleStatus>,
    requests: mpsc::Sender<Request>,
    import: Arc<Mutex<Option<PlayersImport>>>,
}

/// The players' task; [`PlayersTask::close`] writes what it holds and ends it.
pub struct PlayersTask {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl PlayersTask {
    /// Call it once the supervision is closed: the server's stop closes the sessions.
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the players' task failed");
        }
    }
}

enum Request {
    Online(oneshot::Sender<Vec<Connected>>),
    Forget {
        player: i64,
        caller: Caller,
        reply: oneshot::Sender<Result<Option<[u8; 16]>, Error>>,
    },
    Import {
        parsed: import::Parsed,
        caller: Caller,
        reply: oneshot::Sender<Result<PlayersImport, Error>>,
    },
}

/// A session under way, as the panel may see it.
#[derive(Debug, Clone)]
struct Connected {
    net_id: u32,
    seen_ms: i64,
    confirmation: Confirmation,
    license: Option<String>,
    name: Option<String>,
    player: Option<i64>,
}

impl Players {
    /// Opens the service: closes the sessions an earlier Observer left open, crediting each once,
    /// applies the forgotten licences again, and starts the task.
    pub async fn start(setup: PlayersSetup) -> Result<(Self, PlayersTask), Error> {
        let PlayersSetup {
            db,
            metrics,
            clock,
            supervision,
            schedule,
            data_dir,
        } = setup;
        let fresh = random::<32>()?;
        let key = db.write(move |tx| stored::fingerprint_key(tx, fresh)).await?;
        let fingerprints = Fingerprints::new(key);
        let forgotten_file = data_dir.join(FORGOTTEN_FILE);
        let metrics_db = metrics.database().clone();
        let zone = zone_of(&schedule.borrow());

        recover(&db, &metrics_db, &zone, metrics.observer_started()).await?;
        apply_forgotten(&db, &metrics_db, &fingerprints, &forgotten_file).await?;
        let next_id = metrics_db.read(stored_sessions::next_id).await?;

        let (requests, requests_rx) = mpsc::channel(REQUESTS);
        let import = Arc::new(Mutex::new(None));
        let supervisor = supervision.supervisor();
        let stop = CancellationToken::new();
        let worker = Worker {
            db: db.clone(),
            metrics: metrics_db.clone(),
            clock: clock.clone(),
            fingerprints,
            forgotten_file,
            schedule: schedule.clone(),
            observer_started: metrics.observer_started(),
            sessions: {
                let mut keys = Keys::new(random::<32>()?);
                Sessions::new(next_id, Box::new(move || keys.next()))
            },
            pseudonyms: Keys::new(random::<32>()?),
            state: Vec::new(),
            credits: Vec::new(),
            later: Later::default(),
            failing: false,
            missing_run: None,
            last_list: None,
            status: supervisor.status(),
            importing: None,
            import: Arc::clone(&import),
        };
        let task = tokio::spawn(worker.run(
            stop.clone(),
            supervisor.player_reports(),
            supervisor.events(),
            supervisor.readings(),
            requests_rx,
        ));
        let players = Self {
            inner: Arc::new(Inner {
                db,
                metrics: metrics_db,
                clock,
                schedule,
                requests,
                import,
            }),
        };
        Ok((players, PlayersTask { stop, task }))
    }

    async fn connected(&self) -> Vec<Connected> {
        let (reply, answer) = oneshot::channel();
        if self.inner.requests.send(Request::Online(reply)).await.is_err() {
            return Vec::new();
        }
        answer.await.unwrap_or_default()
    }

    /// Who is connected now, with their play time so far.
    pub async fn online(&self) -> Result<OnlinePlayers, Error> {
        let connected = self.connected().await;
        let licenses: Vec<String> = connected
            .iter()
            .filter_map(|session| session.license.clone())
            .collect();
        let totals = self
            .inner
            .db
            .read(move |conn| stored::by_license(conn, &licenses))
            .await?;
        let now_ms = Timestamp::now().as_millisecond();
        let mut players: Vec<OnlinePlayer> = connected
            .into_iter()
            .map(|session| {
                let session_ms = (now_ms - session.seen_ms).max(0);
                let profile = session.license.as_ref().and_then(|license| totals.get(license));
                OnlinePlayer {
                    net_id: session.net_id,
                    confirmation: session.confirmation,
                    player: session.player.or_else(|| profile.map(|(id, _)| *id)),
                    name: session.name,
                    license: session.license,
                    since: Timestamp::from_millisecond(session.seen_ms).unwrap_or(Timestamp::UNIX_EPOCH),
                    session_ms,
                    total_ms: profile.map(|(_, total)| total + session_ms),
                }
            })
            .collect();
        players.sort_by_key(|player| player.net_id);
        Ok(OnlinePlayers { players })
    }

    /// A page of the players list. A search by an identifier other than the licence needs
    /// `PLAYER_IDENTIFIERS_VIEW`, checked on what the search is before anything is looked up.
    pub async fn page(
        &self,
        search: Option<&str>,
        sort: PlayerSort,
        after: Option<&str>,
        held: Permissions,
    ) -> Result<PlayerPage, Error> {
        let filter = match search.map(Search::parse).transpose() {
            Err(SearchRefusal::Address) => return Err(ErrorCode::AddressSearch.into()),
            Err(SearchRefusal::Malformed) => return Err(ErrorCode::InvalidSearch.into()),
            Ok(None | Some(None)) => PlayerFilter::All,
            Ok(Some(Some(Search::Name(name)))) => PlayerFilter::Name(name),
            Ok(Some(Some(Search::License(license)))) => PlayerFilter::License(license),
            Ok(Some(Some(Search::Identifier(identifier)))) => {
                if !held.contains(Permissions::PLAYER_IDENTIFIERS_VIEW) {
                    return Err(ApiError::new(ErrorCode::Forbidden)
                        .with("permission", "playerIdentifiersView")
                        .into());
                }
                PlayerFilter::Identifier(identifier.kind, identifier.value)
            }
        };
        let after = after.map(parse_cursor).transpose()?;
        let (_, today) = self.today();
        let query = PlayerQuery {
            filter,
            sort,
            recent_from: days_before(&today, 29),
            after,
            limit: PAGE + 1,
        };
        let mut rows = self.inner.db.read(move |conn| stored::page(conn, &query)).await?;
        let next = (rows.len() > PAGE as usize).then(|| {
            rows.truncate(PAGE as usize);
            rows.last().map(|(row, key)| format!("{key}.{}", row.id))
        });
        let online: HashSet<String> = self
            .connected()
            .await
            .into_iter()
            .filter(|session| session.confirmation == Confirmation::Confirmed)
            .filter_map(|session| session.license)
            .collect();
        Ok(PlayerPage {
            players: rows
                .into_iter()
                .map(|(mut row, _)| {
                    row.online = online.contains(&row.license);
                    row
                })
                .collect(),
            next: next.flatten(),
        })
    }

    /// A player's profile. Identifiers and linked licences only with `PLAYER_IDENTIFIERS_VIEW`.
    pub async fn profile(&self, id: i64, held: Permissions) -> Result<PlayerProfile, Error> {
        let (zone, today) = self.today();
        let recent_from = days_before(&today, 29);
        let week_from = days_before(&today, 6);
        let activity_from = days_before(&today, ACTIVITY_DAYS - 1);
        let identifiers = held.contains(Permissions::PLAYER_IDENTIFIERS_VIEW);
        let (profile, names, mut days, shown, linked) = {
            let activity_from = activity_from.clone();
            let recent_from = recent_from.clone();
            self.inner
                .db
                .read(move |conn| -> Result<_, StoreError> {
                    let Some(profile) = stored::profile(conn, id, &recent_from)? else {
                        return Ok(None);
                    };
                    let names = stored::names(conn, id)?;
                    let days = stored::days(conn, id, &activity_from)?;
                    let shown = identifiers.then(|| stored::identifiers(conn, id)).transpose()?;
                    let linked = identifiers.then(|| stored::links(conn, id)).transpose()?;
                    Ok(Some((profile, names, days, shown, linked)))
                })
                .await?
                .ok_or(ErrorCode::PlayerNotFound)?
        };
        let pseudonym = profile.pseudonym;
        let since_ms =
            (self.inner.clock.now() - SignedDuration::from_hours(24 * SESSION_DAYS)).as_millisecond();
        let (drops, server_crash_rate) = self
            .inner
            .metrics
            .read(move |conn| -> Result<_, StoreError> {
                Ok((
                    stored_sessions::player_drops(conn, pseudonym, DROPS_SHOWN)?,
                    stored_sessions::crash_rate(conn, since_ms)?,
                ))
            })
            .await?;
        let in_progress = self.connected().await.into_iter().find(|session| {
            session.confirmation == Confirmation::Confirmed
                && session.license.as_deref() == Some(profile.summary.license.as_str())
        });
        let now_ms = Timestamp::now().as_millisecond();
        let in_progress_ms = in_progress
            .as_ref()
            .map_or(0, |session| (now_ms - session.seen_ms).max(0));
        if let Some(session) = &in_progress {
            for (day, ms) in split_days(session.seen_ms, now_ms, &zone) {
                match days.iter_mut().find(|known| known.day == day) {
                    Some(known) => known.play_ms += ms,
                    None if day >= activity_from => days.push(PlayDay { day, play_ms: ms }),
                    None => {}
                }
            }
        }
        let last7_ms = days
            .iter()
            .filter(|day| day.day >= week_from)
            .map(|day| day.play_ms)
            .sum();
        let last30_ms = days
            .iter()
            .filter(|day| day.day >= recent_from)
            .map(|day| day.play_ms)
            .sum();
        let summary = profile.summary;
        Ok(PlayerProfile {
            id: summary.id,
            name: summary.name,
            license: summary.license,
            first_seen: summary.first_seen,
            last_seen: summary.last_seen,
            imported_at: profile.imported_at,
            online: in_progress.is_some(),
            play_ms: summary.play_ms,
            imported_ms: summary.imported_ms,
            in_progress_ms,
            last7_ms,
            last30_ms,
            sessions: summary.sessions,
            longest_ms: profile.longest_ms,
            crashes: profile.crashes,
            timeouts: profile.timeouts,
            kicks: profile.kicks,
            crash_rate: (summary.play_ms > 0)
                .then(|| stored_sessions::per_hundred_hours(profile.crashes, summary.play_ms)),
            server_crash_rate,
            names,
            days,
            drops,
            identifiers: shown,
            linked,
            zone: zone_name(&self.inner.schedule.borrow()),
            today,
        })
    }

    /// Unique players, new players and play time per day over the last 30 days.
    pub async fn community(&self) -> Result<Community, Error> {
        let (zone, today) = self.today();
        let from = days_before(&today, 29);
        let since = start_of(&from, &zone);
        let (played, firsts, week, previous) = {
            let (from, to) = (from.clone(), today.clone());
            let (week_from, previous_from, previous_to) = (
                days_before(&today, 6),
                days_before(&today, 13),
                days_before(&today, 7),
            );
            self.inner
                .db
                .read(move |conn| -> Result<_, StoreError> {
                    Ok((
                        stored::days_played(conn, &from, &to)?,
                        stored::first_visits(conn, since)?,
                        stored::unique_players(conn, &week_from, &to)?,
                        stored::unique_players(conn, &previous_from, &previous_to)?,
                    ))
                })
                .await?
        };
        let mut new_players: HashMap<String, i64> = HashMap::new();
        for first in firsts {
            *new_players
                .entry(first.to_zoned(zone.clone()).date().to_string())
                .or_default() += 1;
        }
        let played: HashMap<String, (i64, i64)> = played
            .into_iter()
            .map(|(day, players, ms)| (day, (players, ms)))
            .collect();
        let days: Vec<CommunityDay> = (0..30)
            .rev()
            .map(|back| {
                let day = days_before(&today, back);
                let (players, play_ms) = played.get(&day).copied().unwrap_or_default();
                CommunityDay {
                    new_players: new_players.get(&day).copied().unwrap_or_default(),
                    players,
                    play_ms,
                    day,
                }
            })
            .collect();
        let totals = |range: &[CommunityDay], players| CommunityTotals {
            players,
            new_players: range.iter().map(|day| day.new_players).sum(),
            play_ms: range.iter().map(|day| day.play_ms).sum(),
        };
        Ok(Community {
            zone: zone_name(&self.inner.schedule.borrow()),
            week: totals(&days[23..], week),
            previous_week: totals(&days[16..23], previous),
            today,
            days,
        })
    }

    pub async fn settings(&self) -> Result<PlayersSettings, Error> {
        Ok(self.inner.db.read(stored::settings).await?)
    }

    /// Changes the retentions; the owner's alone, which the route checks.
    pub async fn change_settings(&self, caller: &Caller, settings: PlayersSettings) -> Result<(), Error> {
        if settings.check().is_err() {
            return Err(ErrorCode::InvalidPlayersSettings.into());
        }
        let before = self.settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::PlayersRetentionChanged)
            .target(Servers::DEFAULT)
            .details(json!({ "from": before, "to": settings }));
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), StoreError> {
                stored::save_settings(tx, &settings)?;
                record.write(tx, now)
            })
            .await?;
        Ok(())
    }

    /// Forgets a player for an erasure request: their profile, play time, identifiers others do
    /// not hold, names, and the texts and pseudonym of their sessions. The audit entry names the
    /// licence's keyed fingerprint, never the licence.
    pub async fn forget(&self, caller: &Caller, player: i64) -> Result<(), Error> {
        let (reply, answer) = oneshot::channel();
        self.inner
            .requests
            .send(Request::Forget {
                player,
                caller: caller.clone(),
                reply,
            })
            .await
            .map_err(|_| Error::from(ErrorCode::ShuttingDown))?;
        answer
            .await
            .map_err(|_| Error::from(ErrorCode::ShuttingDown))??
            .ok_or(ErrorCode::PlayerNotFound)?;
        Ok(())
    }

    /// Takes txAdmin's players database in: read and checked whole first, then written in the
    /// background, a few hundred players at a time. The path stays out of the audit log.
    pub async fn import(&self, caller: &Caller, path: &str) -> Result<PlayersImport, Error> {
        if self
            .import_status()
            .is_some_and(|import| import.state == ImportState::Writing)
        {
            return Err(ErrorCode::JobRunning.into());
        }
        let path = PathBuf::from(path);
        let now = self.inner.clock.now();
        let parsed = tokio::task::spawn_blocking(move || import::read(&path, now))
            .await
            .map_err(|_| Error::from(ErrorCode::Internal))?
            .map_err(|problem| {
                ApiError::new(ErrorCode::InvalidPlayersFile)
                    .with("problem", serde_json::to_value(problem).unwrap_or_default())
            })?;
        let (reply, answer) = oneshot::channel();
        self.inner
            .requests
            .send(Request::Import {
                parsed,
                caller: caller.clone(),
                reply,
            })
            .await
            .map_err(|_| Error::from(ErrorCode::ShuttingDown))?;
        answer.await.map_err(|_| Error::from(ErrorCode::ShuttingDown))?
    }

    /// The import under way, or the last one since Observer started.
    pub fn import_status(&self) -> Option<PlayersImport> {
        self.inner
            .import
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn today(&self) -> (TimeZone, String) {
        let zone = zone_of(&self.inner.schedule.borrow());
        let today = self.inner.clock.now().to_zoned(zone.clone()).date().to_string();
        (zone, today)
    }
}

/// What was written to the state database and waits for the metrics one: the run the sessions
/// belong to, the sessions in the order they were decided, then their pseudonyms.
#[derive(Default)]
struct Later {
    run: Option<(RunKey, i64, StartCause)>,
    sessions: Vec<SessionWrite>,
    attaches: Vec<(i64, [u8; 16], [u8; 16])>,
}

impl Later {
    fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.attaches.is_empty()
    }
}

struct Worker {
    db: Database,
    metrics: MetricsDatabase,
    clock: Clock,
    fingerprints: Fingerprints,
    forgotten_file: PathBuf,
    schedule: watch::Receiver<ScheduleStatus>,
    observer_started: i64,
    sessions: Sessions,
    pseudonyms: Keys,
    /// Identities to write, then sessions to credit, in the state database.
    state: Vec<IdentityWrite>,
    credits: Vec<Credit>,
    later: Later,
    failing: bool,
    /// Since when the sessions wait for their run to be written.
    missing_run: Option<Instant>,
    last_list: Option<Instant>,
    status: watch::Receiver<Status>,
    importing: Option<Importing>,
    import: Arc<Mutex<Option<PlayersImport>>>,
}

/// The players of an import still to write.
struct Importing {
    records: VecDeque<import::Record>,
    caller: Caller,
    status: PlayersImport,
}

impl Worker {
    async fn run(
        mut self,
        stop: CancellationToken,
        mut reports: broadcast::Receiver<PlayerReport>,
        mut events: broadcast::Receiver<Event>,
        mut readings: watch::Receiver<Readings>,
        mut requests: mpsc::Receiver<Request>,
    ) {
        let mut flush = tokio::time::interval(FLUSH_EVERY);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut upkeep = tokio::time::interval(UPKEEP_EVERY);
        let mut import_batch = tokio::time::interval(IMPORT_EVERY);
        upkeep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => break,
                report = reports.recv() => match report {
                    Ok(report) => self.sessions.report(&report, &self.fingerprints),
                    Err(RecvError::Lagged(missed)) => {
                        warn!(missed, "the players service missed reports from the bridge; the player list fills the gaps");
                    }
                    Err(RecvError::Closed) => break,
                },
                event = events.recv() => match event {
                    Ok(event) => self.event(&event),
                    Err(RecvError::Lagged(missed)) => {
                        warn!(missed, "the players service missed supervisor events; it follows the supervisor's status");
                        self.resync();
                    }
                    Err(RecvError::Closed) => break,
                },
                changed = readings.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    self.listed(&readings);
                }
                request = requests.recv() => match request {
                    Some(request) => self.request(request).await,
                    None => break,
                },
                _ = flush.tick() => self.flush().await,
                _ = upkeep.tick() => self.upkeep().await,
                _ = import_batch.tick(), if self.importing.is_some() => self.import_batch().await,
            }
        }
        if self.importing.is_some() {
            self.finish_import(false).await;
        }
        // The server's stop just before is part of the run.
        while let Ok(report) = reports.try_recv() {
            self.sessions.report(&report, &self.fingerprints);
        }
        while let Ok(event) = events.try_recv() {
            self.event(&event);
        }
        self.flush().await;
        if !self.state.is_empty() || !self.credits.is_empty() || !self.later.is_empty() {
            warn!("some sessions of players could not be written before Observer stopped");
        }
    }

    fn event(&mut self, event: &Event) {
        if let Some(Change::Started { at_ms, cause }) = Change::of(event) {
            self.later.run = Some((self.run_key(event.generation), at_ms, cause));
        }
        match &event.kind {
            EventKind::Spawned { .. } => {
                self.sessions
                    .spawned(self.run_key(event.generation), Instant::now());
            }
            EventKind::State {
                to: State::Stopping,
                cause,
                ..
            } => {
                if let Some(kind) = stop_kind(cause) {
                    self.sessions.stopping(event.generation, kind);
                }
            }
            EventKind::State {
                cause: Cause::Hang { .. },
                ..
            } => self.sessions.hung(event.generation),
            EventKind::Exited { expected, .. } => {
                self.sessions
                    .exited(event.generation, event.at_ms, *expected, Instant::now());
            }
            _ => {}
        }
    }

    fn run_key(&self, generation: u32) -> RunKey {
        RunKey {
            observer_started: self.observer_started,
            generation,
        }
    }

    /// After missed events, follows the run the supervisor says is under way: a run whose end was
    /// missed closes its sessions where they were last seen.
    fn resync(&mut self) {
        let (generation, running) = {
            let status = self.status.borrow();
            (status.generation, status.pid.is_some())
        };
        let followed = self.sessions.generation();
        if running && followed == Some(generation) {
            return;
        }
        if running {
            self.sessions.spawned(self.run_key(generation), Instant::now());
        } else if followed.is_some() {
            self.sessions.end_lost(Instant::now());
        }
    }

    fn listed(&mut self, readings: &watch::Receiver<Readings>) {
        let fresh = {
            let readings = readings.borrow();
            readings.run.as_ref().and_then(|run| {
                let list = run.players.as_ref()?;
                (self.last_list != Some(list.at)).then(|| (run.generation, list.at, list.value.clone()))
            })
        };
        let Some((generation, at, list)) = fresh else {
            return;
        };
        self.last_list = Some(at);
        let age = i64::try_from(at.elapsed().as_millis()).unwrap_or(0);
        let list_ms = Timestamp::now().as_millisecond() - age;
        self.sessions.listed(generation, &list, list_ms, at);
    }

    async fn request(&mut self, request: Request) {
        match request {
            Request::Online(reply) => {
                let _ = reply.send(self.connected());
            }
            Request::Forget {
                player,
                caller,
                reply,
            } => {
                let _ = reply.send(self.forget(player, &caller).await);
            }
            Request::Import {
                parsed,
                caller,
                reply,
            } => {
                let _ = reply.send(self.start_import(parsed, caller).await);
            }
        }
    }

    /// Starts writing an import, without the players whose licence was forgotten: the file of
    /// forgotten licences is taken in first, as txAdmin's database still holds every one of them.
    async fn start_import(&mut self, parsed: import::Parsed, caller: Caller) -> Result<PlayersImport, Error> {
        if self.importing.is_some() {
            return Err(ErrorCode::JobRunning.into());
        }
        apply_forgotten(&self.db, &self.metrics, &self.fingerprints, &self.forgotten_file).await?;
        let forgotten = self.db.read(stored::forgotten).await?;
        let read = parsed.records.len() as u64 + parsed.invalid;
        let (records, skipped): (Vec<_>, Vec<_>) = parsed
            .records
            .into_iter()
            .partition(|record| !forgotten.contains_key(&self.fingerprints.license(&record.license)));
        let status = PlayersImport {
            state: ImportState::Writing,
            started_at: self.clock.now(),
            finished_at: None,
            bytes: parsed.bytes,
            read,
            written: 0,
            imported: 0,
            updated: 0,
            forgotten: skipped.len() as u64,
            invalid: parsed.invalid,
        };
        self.publish_import(&status);
        self.importing = Some(Importing {
            records: records.into(),
            caller,
            status: status.clone(),
        });
        Ok(status)
    }

    async fn import_batch(&mut self) {
        let Some(importing) = self.importing.as_mut() else {
            return;
        };
        let count = importing.records.len().min(IMPORT_BATCH);
        let batch: Vec<import::Record> = importing.records.drain(..count).collect();
        let pseudonyms: Vec<[u8; 16]> = batch.iter().map(|_| self.pseudonyms.next()).collect();
        let at = self.clock.now();
        let written = self
            .db
            .write(move |tx| -> Result<u64, StoreError> {
                let mut new = 0;
                for (record, pseudonym) in batch.iter().zip(pseudonyms) {
                    let imported = Imported {
                        license: &record.license,
                        name: record.name.as_deref(),
                        play_ms: record.play_ms,
                        joined: record.joined,
                        last: record.last,
                        others: &record.others,
                        imported_at: at,
                        pseudonym,
                    };
                    new += u64::from(stored::import(tx, &imported)?);
                }
                Ok(new)
            })
            .await;
        match written {
            Ok(new) => {
                let Some(importing) = self.importing.as_mut() else {
                    return;
                };
                importing.status.written += count as u64;
                importing.status.imported += new;
                importing.status.updated += count as u64 - new;
                let status = importing.status.clone();
                let done = importing.records.is_empty();
                self.publish_import(&status);
                if done {
                    self.finish_import(true).await;
                }
            }
            Err(error) => {
                warn!(%error, "the import of txAdmin's players stopped; running it again finishes it");
                self.finish_import(false).await;
            }
        }
    }

    /// Ends the import, applies the retentions at once to what came in already expired, and audits
    /// it with its counts only.
    async fn finish_import(&mut self, finished: bool) {
        let Some(mut importing) = self.importing.take() else {
            return;
        };
        if finished {
            self.upkeep().await;
        }
        let now = self.clock.now();
        importing.status.state = if finished {
            ImportState::Finished
        } else {
            ImportState::Failed
        };
        importing.status.finished_at = Some(now);
        self.publish_import(&importing.status);
        let status = &importing.status;
        let record = Record::by(&importing.caller, AuditAction::PlayersImported)
            .target(Servers::DEFAULT)
            .outcome(if finished {
                Outcome::Success
            } else {
                Outcome::Failure
            })
            .details(json!({
                "bytes": status.bytes,
                "read": status.read,
                "written": status.written,
                "imported": status.imported,
                "updated": status.updated,
                "forgotten": status.forgotten,
                "invalid": status.invalid,
            }));
        if let Err(error) = self.db.write(move |tx| record.write(tx, now)).await {
            warn!(%error, "cannot audit the import of txAdmin's players");
        }
    }

    fn publish_import(&self, status: &PlayersImport) {
        *self.import.lock().unwrap_or_else(PoisonError::into_inner) = Some(status.clone());
    }

    fn connected(&self) -> Vec<Connected> {
        self.sessions
            .open()
            .map(|open| {
                let (confirmation, license, name, player) = match &open.identity {
                    Identity::Confirming(_) => (Confirmation::Confirming, None, None, None),
                    Identity::Confirmed {
                        license,
                        name,
                        player,
                        ..
                    } => (
                        Confirmation::Confirmed,
                        Some(license.clone()),
                        name.clone(),
                        *player,
                    ),
                    Identity::Unconfirmed | Identity::Anonymous => {
                        (Confirmation::Unconfirmed, None, None, None)
                    }
                };
                Connected {
                    net_id: open.net_id,
                    seen_ms: open.seen_ms,
                    confirmation,
                    license,
                    name,
                    player,
                }
            })
            .collect()
    }

    /// Writes what the sessions decided: identities and play time first, in the state database,
    /// which alone says what was credited; then the sessions, in the metrics database, in the
    /// order they were decided.
    async fn flush(&mut self) {
        let writes = self.sessions.take_writes();
        let zone = zone_of(&self.schedule.borrow());
        self.state.extend(writes.identities);
        for write in writes.sessions {
            if let SessionWrite::Close {
                closure,
                credit: Some(key),
            } = &write
            {
                self.credits.push(credit_for(*key, closure, &zone));
            }
            self.later.sessions.push(write);
        }

        if self.flush_identities().await {
            self.flush_sessions().await;
        }
    }

    /// Identities, crediting records and credits, in one state transaction. `false` while they
    /// cannot be written: the sessions wait, since a session is closed only once credited.
    async fn flush_identities(&mut self) -> bool {
        if !self.state.is_empty() || !self.credits.is_empty() {
            let identities = self.state.clone();
            let credits = self.credits.clone();
            let pseudonyms: Vec<[u8; 16]> = identities.iter().map(|_| self.pseudonyms.next()).collect();
            let written = self
                .db
                .write(move |tx| -> Result<_, StoreError> {
                    let mut identified = Vec::with_capacity(identities.len());
                    for (identity, pseudonym) in identities.iter().zip(pseudonyms) {
                        let at =
                            Timestamp::from_millisecond(identity.seen_ms).unwrap_or(Timestamp::UNIX_EPOCH);
                        let (player, pseudonym) = stored::record_seen(
                            tx,
                            &Seen {
                                license: &identity.license,
                                name: identity.name.as_deref(),
                                others: &identity.others,
                                at,
                                pseudonym,
                            },
                        )?;
                        stored::open_credit(tx, identity.credit, player, at, identity.session)?;
                        identified.push((identity.session, player, pseudonym, identity.credit));
                    }
                    for credit in &credits {
                        stored::credit(tx, credit)?;
                    }
                    Ok(identified)
                })
                .await;
            match written {
                Ok(identified) => {
                    self.state.clear();
                    self.credits.clear();
                    for (session, player, pseudonym, credit) in identified {
                        self.sessions.identified(session, player);
                        self.later.attaches.push((session, pseudonym, credit));
                    }
                }
                Err(error) => {
                    self.failed(
                        &error,
                        "the players' identities and play time could not be written",
                    );
                    return false;
                }
            }
        }
        true
    }

    async fn flush_sessions(&mut self) {
        if self.later.is_empty() {
            return;
        }
        let run = self.later.run;
        let sessions = self.later.sessions.clone();
        let attaches = self.later.attaches.clone();
        let written = self
            .metrics
            .write(move |tx| -> Result<(), FlushError> {
                if let Some((key, started_ms, cause)) = run {
                    runs::start(tx, key, started_ms, cause)?;
                }
                for write in &sessions {
                    match write {
                        SessionWrite::Open(session) => {
                            if !stored_sessions::insert(tx, session)? {
                                return Err(FlushError::RunMissing);
                            }
                        }
                        SessionWrite::Close { closure, .. } => stored_sessions::close(tx, closure)?,
                        SessionWrite::Reclassify(closure) => stored_sessions::reclassify(tx, closure)?,
                    }
                }
                for (session, pseudonym, credit) in &attaches {
                    stored_sessions::attach(tx, *session, *pseudonym, *credit)?;
                }
                Ok(())
            })
            .await;
        match written {
            Ok(()) => {
                self.later.sessions.clear();
                self.later.attaches.clear();
                self.failing = false;
                self.missing_run = None;
            }
            Err(FlushError::RunMissing) => {
                let since = *self.missing_run.get_or_insert_with(Instant::now);
                if since.elapsed() > RUN_MISSING && !self.failing {
                    self.failing = true;
                    warn!("the players' sessions wait for a run of FXServer the statistics never recorded");
                }
            }
            Err(FlushError::Store(error)) => {
                self.failed(&error, "the players' sessions could not be written");
            }
        }
    }

    fn failed(&mut self, error: &StoreError, what: &str) {
        if !self.failing {
            self.failing = true;
            warn!(%error, "{what}; Observer tries again every second");
        }
    }

    /// Forgets a player. The metrics database goes first: blanking a pseudonym can be done again,
    /// while the player's row, once deleted, is the key to it. The deletion and its audit entry
    /// share one transaction.
    async fn forget(&mut self, player: i64, caller: &Caller) -> Result<Option<[u8; 16]>, Error> {
        self.flush().await;
        if !self.state.is_empty() || !self.credits.is_empty() {
            return Err(ErrorCode::Internal.into());
        }
        let Some((license, pseudonym)) = self
            .db
            .read(move |conn| {
                Ok::<_, StoreError>(
                    stored::profile(conn, player, "")?
                        .map(|profile| (profile.summary.license, profile.pseudonym)),
                )
            })
            .await?
        else {
            return Ok(None);
        };
        self.sessions.forget(&license);
        self.later
            .attaches
            .retain(|(_, attached, _)| *attached != pseudonym);
        if let Some(importing) = self.importing.as_mut() {
            let before = importing.records.len();
            importing.records.retain(|record| record.license != license);
            importing.status.forgotten += (before - importing.records.len()) as u64;
        }
        let fingerprint = self.fingerprints.license(&license);
        let at = self.clock.now();
        append_forgotten(&self.forgotten_file, fingerprint, at)?;
        let record = Record::by(caller, AuditAction::PlayerForgotten).target(hex(&fingerprint));
        forget_everywhere(
            &self.db,
            &self.metrics,
            player,
            fingerprint,
            pseudonym,
            at,
            Some(record),
        )
        .await?;
        info!(fingerprint = %hex(&fingerprint), "a player was forgotten");
        Ok(Some(fingerprint))
    }

    async fn upkeep(&mut self) {
        let now = self.clock.now();
        let zone = zone_of(&self.schedule.borrow());
        let result = async {
            let retention = self.db.read(stored::settings).await?;
            let today = now.to_zoned(zone).date().to_string();
            let day_cutoff = days_before(&today, i64::from(retention.day_days));
            let expired = self
                .db
                .write(move |tx| stored::expire(tx, now, &retention, &day_cutoff))
                .await?;
            let before_ms = (now - SignedDuration::from_hours(24 * SESSION_DAYS)).as_millisecond();
            let sessions = self
                .metrics
                .write(move |tx| stored_sessions::expire(tx, before_ms))
                .await?;
            Ok::<_, StoreError>((expired, sessions))
        }
        .await;
        match result {
            Ok((expired, sessions)) => {
                let counts = self.sessions.counts;
                info!(
                    links = expired.links,
                    names = expired.names,
                    days = expired.days,
                    players = expired.players,
                    sessions,
                    dropped_identifiers = counts.dropped_identifiers,
                    unconfirmed = counts.unconfirmed,
                    clamped = counts.clamped,
                    rosters_given_up = counts.rosters_given_up,
                    "players' upkeep"
                );
            }
            Err(error) => warn!(%error, "the players' upkeep failed"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum FlushError {
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The run of a session is not written yet: the whole batch waits for the next flush.
    #[error("the run is not recorded yet")]
    RunMissing,
}

fn credit_for(key: [u8; 16], closure: &Closure, zone: &TimeZone) -> Credit {
    let length_ms = closure.length_ms.unwrap_or(0).max(0);
    Credit {
        key,
        length_ms,
        ended: Timestamp::from_millisecond(closure.ended_ms).unwrap_or(Timestamp::UNIX_EPOCH),
        days: split_days(closure.ended_ms - length_ms, closure.ended_ms, zone),
        crashed: closure.category == DropCategory::Crash,
        timed_out: closure.category == DropCategory::Timeout,
        kicked: closure.category == DropCategory::Kick,
    }
}

fn stop_kind(cause: &Cause) -> Option<DropKind> {
    match cause {
        Cause::Requested { .. } => Some(DropKind::Stop),
        Cause::Restart { .. } => Some(DropKind::Restart),
        Cause::Scheduled { .. } => Some(DropKind::Scheduled),
        Cause::Shutdown => Some(DropKind::Shutdown),
        Cause::CrashLoop { .. } => Some(DropKind::CrashLoop),
        Cause::ConsoleQuit { .. } => Some(DropKind::Console),
        _ => None,
    }
}

/// Closes the sessions an earlier Observer left open, at the end of their run. A session whose
/// crediting record is still in the state database is credited first, once; the record goes
/// whether it was or not.
async fn recover(
    db: &Database,
    metrics: &MetricsDatabase,
    zone: &TimeZone,
    observer_started: i64,
) -> Result<(), Error> {
    let left = metrics.read(stored_sessions::left_open).await?;
    let records = db.read(stored::open_records).await?;
    if left.is_empty() && records.is_empty() {
        return Ok(());
    }
    let closures: HashMap<i64, Closure> = left
        .iter()
        .map(|session| {
            let ended_ms = stored_sessions::lost_end(session, observer_started);
            let closure = Closure {
                id: session.id,
                ended_ms,
                length_ms: Some(ended_ms - session.seen_ms),
                category: DropCategory::Unknown,
                kind: Some(DropKind::Lost),
                native: None,
                resource: None,
                crash_text: None,
                reason_text: None,
                loading: None,
                synthesized: true,
            };
            (session.id, closure)
        })
        .collect();
    // A record names its session: one whose key never reached the metrics database, because
    // Observer died in between, is still credited.
    let credits: Vec<Credit> = records
        .iter()
        .filter_map(|record| {
            closures
                .get(&record.session)
                .map(|closure| credit_for(record.key, closure, zone))
        })
        .collect();
    let keys: Vec<[u8; 16]> = records.iter().map(|record| record.key).collect();
    db.write(move |tx| -> Result<(), StoreError> {
        for credit in &credits {
            stored::credit(tx, credit)?;
        }
        for key in keys {
            stored::drop_record(tx, key)?;
        }
        Ok(())
    })
    .await?;
    let closed = closures.len();
    metrics
        .write(move |tx| -> Result<(), StoreError> {
            for closure in closures.values() {
                stored_sessions::close(tx, closure)?;
            }
            Ok(())
        })
        .await?;
    if closed > 0 {
        info!(
            closed,
            "players' sessions left open by an earlier Observer were closed"
        );
    }
    Ok(())
}

/// Takes the forgotten licences of the file back into the database, and forgets again any of them
/// a restored backup brought back.
async fn apply_forgotten(
    db: &Database,
    metrics: &MetricsDatabase,
    fingerprints: &Fingerprints,
    file: &Path,
) -> Result<(), Error> {
    let listed = read_forgotten(file)?;
    db.write(move |tx| -> Result<(), StoreError> {
        for (fingerprint, at) in listed {
            stored::remember_forgotten(tx, fingerprint, at)?;
        }
        Ok(())
    })
    .await?;
    let (forgotten, players) = db
        .read(|conn| Ok::<_, StoreError>((stored::forgotten(conn)?, stored::licenses(conn)?)))
        .await?;
    for (player, license, last_seen) in players {
        let fingerprint = fingerprints.license(&license);
        if forgotten.get(&fingerprint).is_some_and(|at| last_seen <= *at) {
            let pseudonym = db
                .read(move |conn| stored::profile(conn, player, ""))
                .await?
                .map(|profile| profile.pseudonym);
            if let Some(pseudonym) = pseudonym {
                let at = forgotten[&fingerprint];
                forget_everywhere(db, metrics, player, fingerprint, pseudonym, at, None).await?;
                info!(fingerprint = %hex(&fingerprint), "a forgotten player a backup brought back was forgotten again");
            }
        }
    }
    Ok(())
}

async fn forget_everywhere(
    db: &Database,
    metrics: &MetricsDatabase,
    player: i64,
    fingerprint: [u8; 16],
    pseudonym: [u8; 16],
    at: Timestamp,
    record: Option<Record>,
) -> Result<(), Error> {
    metrics
        .write(move |tx| stored_sessions::forget(tx, pseudonym))
        .await?;
    metrics.checkpoint().await?;
    db.write(move |tx| -> Result<(), StoreError> {
        stored::forget(tx, player, fingerprint, at)?;
        if let Some(record) = record {
            record.write(tx, at)?;
        }
        Ok(())
    })
    .await?;
    db.read(stored::checkpoint).await?;
    Ok(())
}

fn read_forgotten(file: &Path) -> Result<Vec<([u8; 16], Timestamp)>, Error> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut listed = Vec::new();
    for line in text.lines() {
        let Some((fingerprint, at)) = line.split_once(' ') else {
            continue;
        };
        let (Some(fingerprint), Ok(at)) = (unhex(fingerprint), at.parse::<Timestamp>()) else {
            warn!("a line of {} is malformed and was skipped", file.display());
            continue;
        };
        listed.push((fingerprint, at));
    }
    Ok(listed)
}

fn append_forgotten(file: &Path, fingerprint: [u8; 16], at: Timestamp) -> Result<(), Error> {
    let mut opened = OpenOptions::new().create(true).append(true).open(file)?;
    writeln!(opened, "{} {at}", hex(&fingerprint))?;
    opened.sync_all()?;
    Ok(())
}

fn unhex(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 {
        return None;
    }
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(bytes)
}

fn parse_cursor(text: &str) -> Result<(i64, i64), Error> {
    text.split_once('.')
        .and_then(|(key, id)| Some((key.parse().ok()?, id.parse().ok()?)))
        .ok_or_else(|| ErrorCode::InvalidRequest.into())
}

fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(Error::Random)?;
    Ok(bytes)
}

fn zone_of(status: &ScheduleStatus) -> TimeZone {
    TimeZone::get(&status.zone).unwrap_or(TimeZone::UTC)
}

fn zone_name(status: &ScheduleStatus) -> String {
    status.zone.clone()
}

/// The day `back` days before `day`, both `YYYY-MM-DD`.
fn days_before(day: &str, back: i64) -> String {
    day.parse::<jiff::civil::Date>()
        .ok()
        .and_then(|date| date.checked_sub(jiff::Span::new().days(back)).ok())
        .map_or_else(|| day.to_owned(), |date| date.to_string())
}

fn start_of(day: &str, zone: &TimeZone) -> Timestamp {
    day.parse::<jiff::civil::Date>()
        .ok()
        .and_then(|date| date.to_zoned(zone.clone()).ok())
        .map_or(Timestamp::UNIX_EPOCH, |zoned| zoned.timestamp())
}

#[cfg(test)]
mod tests;
