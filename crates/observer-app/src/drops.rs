//! Players' drops: a task that classifies closed sessions, finds bursts and rolls hours up, and what
//! the Player drops page reads. Statistics are cached for 30 seconds per route and parameters;
//! names, reason groups and a client's free text are added or hidden per caller afterwards, so an
//! answer built for a log reader never reaches a statistics viewer.

mod compute;
mod worker;

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use observer_domain::audit::AuditAction;
use observer_domain::drops::{
    ChartCategory, CrashDrops, DropCategory, DropChanges, DropKind, DropLog, DropLogRow, DropSeries,
    DropUptime, DropsRange, DropsSettings, DropsSummary, RepeatCrasher, ResourceTable, SignatureTable,
};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::permissions::Permissions;
use observer_domain::schedule::ScheduleStatus;
use observer_domain::statistics::{SessionSurvival, StabilityEffects, TimelineEvent};
use observer_store::metrics::drops::{self as stored, LogFilter, LogRow};
use observer_store::metrics::links;
use observer_store::players as stored_players;
use observer_store::{Database, MetricsDatabase, StoreError};
use serde_json::json;
use tokio::sync::{OnceCell, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub use self::compute::DayRates;
pub(crate) use self::compute::insights::{EffectChange, Wave};
use crate::audit::Record;
use crate::{Caller, Clock, Error, Metrics, Players, Servers};

const CACHED_FOR: Duration = Duration::from_secs(30);
const LOG_PAGE: u32 = 100;
const EXPORT_ROWS: u32 = 100_000;
/// The rates export covers at most this many days.
const EXPORT_DAYS: i64 = 400;

pub struct DropsSetup {
    pub db: Database,
    pub metrics: Metrics,
    pub clock: Clock,
    /// The schedule's status, whose time zone days start in.
    pub schedule: watch::Receiver<ScheduleStatus>,
    pub players: Players,
}

/// The drops service. Clones share it.
#[derive(Clone)]
pub struct Drops {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    metrics: MetricsDatabase,
    clock: Clock,
    schedule: watch::Receiver<ScheduleStatus>,
    players: Players,
    cache: Mutex<HashMap<String, Cached>>,
}

/// An answer, once computed, and when it was.
type Cached = Arc<OnceCell<(Instant, Arc<dyn Any + Send + Sync>)>>;

/// The drops' task; [`DropsTask::close`] ends it.
pub struct DropsTask {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl DropsTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the drops' task failed");
        }
    }
}

/// What the drop log is narrowed to, as the page asks it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogQuery {
    pub range: Option<DropsRange>,
    pub category: Option<DropCategory>,
    pub kind: Option<DropKind>,
    pub resource: Option<String>,
    pub signature: Option<i64>,
    pub run: Option<i64>,
    /// A name's start, a licence, or, with `PLAYER_IDENTIFIERS_VIEW`, another identifier.
    pub player: Option<String>,
    /// One profile, by its id.
    pub profile: Option<i64>,
    /// A profile and the licences strongly linked to it, with `PLAYER_IDENTIFIERS_VIEW`.
    pub group: Option<i64>,
    pub before: Option<String>,
}

impl LogQuery {
    /// The kinds of filter set, never their values: what the export's audit entry says.
    fn kinds(&self) -> Vec<&'static str> {
        [
            ("range", self.range.is_some()),
            ("category", self.category.is_some()),
            ("kind", self.kind.is_some()),
            ("resource", self.resource.is_some()),
            ("signature", self.signature.is_some()),
            ("run", self.run.is_some()),
            ("player", self.player.is_some()),
            ("profile", self.profile.is_some()),
            ("group", self.group.is_some()),
        ]
        .into_iter()
        .filter_map(|(kind, set)| set.then_some(kind))
        .collect()
    }
}

impl Drops {
    pub fn start(setup: DropsSetup) -> (Self, DropsTask) {
        let DropsSetup {
            db,
            metrics,
            clock,
            schedule,
            players,
        } = setup;
        let inner = Arc::new(Inner {
            db,
            metrics: metrics.database().clone(),
            clock,
            schedule,
            players,
            cache: Mutex::new(HashMap::new()),
        });
        let stop = CancellationToken::new();
        let task = tokio::spawn(worker::run(Arc::clone(&inner), stop.clone()));
        (Self { inner }, DropsTask { stop, task })
    }

    fn zone(&self) -> TimeZone {
        TimeZone::get(&self.inner.schedule.borrow().zone).unwrap_or(TimeZone::UTC)
    }

    /// Forgets every cached answer: after a player is forgotten, none of theirs is served again.
    pub fn clear(&self) {
        self.inner
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The answer cached under `key`, or a new one computed once on the metrics database's reader,
    /// however many callers ask for it meanwhile. It reads one snapshot of the database, and the
    /// sessions kept since `kept_from` as the retention has them.
    async fn cached<T, F>(&self, key: &str, compute: F) -> Result<Arc<T>, Error>
    where
        T: Send + Sync + 'static,
        F: FnOnce(&observer_store::Connection, Timestamp, &TimeZone, i64) -> Result<T, StoreError>
            + Send
            + 'static,
    {
        let retention = self.inner.db.read(stored_players::drops_settings).await?;
        let key = format!("{key}:{}", retention.session_days);
        let cell = {
            let mut cache = self.inner.cache.lock().unwrap_or_else(PoisonError::into_inner);
            // One being computed stays, so that its callers keep waiting for it.
            cache.retain(|_, cell| cell.get().is_none_or(|(at, _)| at.elapsed() < CACHED_FOR));
            Arc::clone(cache.entry(key).or_insert_with(|| Arc::new(OnceCell::new())))
        };
        let value = cell
            .get_or_try_init(|| async {
                let now = self.inner.clock.now();
                let zone = self.zone();
                let kept_from_ms = now.as_millisecond() - i64::from(retention.session_days) * 86_400_000;
                let value = self
                    .inner
                    .metrics
                    .read(move |conn| -> Result<T, StoreError> {
                        let snapshot = conn.unchecked_transaction()?;
                        let value = compute(&snapshot, now, &zone, kept_from_ms)?;
                        snapshot.commit()?;
                        Ok(value)
                    })
                    .await?;
                Ok::<_, Error>((Instant::now(), Arc::new(value) as Arc<dyn Any + Send + Sync>))
            })
            .await?;
        Arc::clone(&value.1)
            .downcast::<T>()
            .map_err(|_| ErrorCode::Internal.into())
    }

    /// The tiles, the breakdown and the repeat crashers, named only for `PLAYERS_VIEW`.
    pub async fn summary(&self, range: DropsRange, held: Permissions) -> Result<DropsSummary, Error> {
        let base = self
            .cached(
                &format!("summary:{}", range.name()),
                move |conn, now, zone, kept| compute::summary(conn, range, now, zone, kept),
            )
            .await?;
        let mut summary = base.summary.clone();
        let names = if held.contains(Permissions::PLAYERS_VIEW) {
            self.inner
                .players
                .identities(base.repeat.iter().map(|(pseudonym, _)| *pseudonym).collect())
                .await?
        } else {
            HashMap::new()
        };
        summary.repeat_crashers = base
            .repeat
            .iter()
            .map(|(pseudonym, crashes)| RepeatCrasher {
                crashes: *crashes,
                player: names.get(pseudonym).cloned(),
            })
            .collect();
        Ok(summary)
    }

    pub async fn series(&self, range: DropsRange, category: ChartCategory) -> Result<DropSeries, Error> {
        let series = self
            .cached(
                &format!("series:{}:{}", range.name(), category.name()),
                move |conn, now, zone, _| compute::series(conn, range, category, now, zone),
            )
            .await?;
        Ok((*series).clone())
    }

    /// The crash signatures. What a client wrote reaches statistics viewers only once two players
    /// wrote it: a signature of any form, since a modified client can write any, its rollups, and
    /// each example's own text.
    pub async fn signatures(&self, range: DropsRange, held: Permissions) -> Result<SignatureTable, Error> {
        let base = self
            .cached(
                &format!("signatures:{}", range.name()),
                move |conn, now, zone, kept| compute::signatures(conn, range, now, zone, kept),
            )
            .await?;
        let mut table = base.table.clone();
        if !held.contains(Permissions::PLAYERS_VIEW) {
            table.modules.clone_from(&base.shared_modules);
            table.builds.clone_from(&base.shared_builds);
            for entry in &mut table.signatures {
                if base.players_ever.get(&entry.id).copied().unwrap_or(0) < 2 {
                    entry.signature = None;
                    entry.label = None;
                    entry.module = None;
                    entry.game_build = None;
                    entry.resource = None;
                    entry.examples.clear();
                    continue;
                }
                let players = base.example_players.get(&entry.id).cloned().unwrap_or_default();
                let mut players = players.into_iter();
                entry
                    .examples
                    .retain(|_| players.next().is_some_and(|players| players >= 2));
            }
        }
        Ok(table)
    }

    /// The kicking resources; their reason groups only for `PLAYERS_VIEW`.
    pub async fn resources(&self, range: DropsRange, held: Permissions) -> Result<ResourceTable, Error> {
        let base = self
            .cached(
                &format!("resources:{}", range.name()),
                move |conn, now, zone, kept| compute::resources(conn, range, now, zone, kept),
            )
            .await?;
        let mut table = (*base).clone();
        if !held.contains(Permissions::PLAYERS_VIEW) {
            for entry in &mut table.resources {
                entry.reasons = None;
            }
        }
        Ok(table)
    }

    pub async fn changes(&self, range: DropsRange) -> Result<DropChanges, Error> {
        let changes = self
            .cached(&format!("changes:{}", range.name()), move |conn, now, zone, _| {
                compute::changes(conn, range, now, zone)
            })
            .await?;
        Ok((*changes).clone())
    }

    pub async fn uptime(&self, range: DropsRange) -> Result<DropUptime, Error> {
        let uptime = self
            .cached(
                &format!("uptime:{}", range.name()),
                move |conn, now, zone, kept| compute::uptime(conn, range, now, zone, kept),
            )
            .await?;
        Ok((*uptime).clone())
    }

    /// How long players stay: the sessions that started in the range against those of the 4
    /// weeks before.
    pub async fn sessions(&self, range: DropsRange) -> Result<SessionSurvival, Error> {
        let survival = self
            .cached(
                &format!("sessions:{}", range.name()),
                move |conn, now, _, kept| compute::insights::sessions(conn, range, now, kept),
            )
            .await?;
        Ok((*survival).clone())
    }

    /// The crash waves that reach into `from_ms` to `to_ms`, and the bursts of drops in it.
    pub(crate) async fn waves(
        &self,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Arc<(Vec<Wave>, Vec<TimelineEvent>)>, Error> {
        self.cached(&format!("waves:{from_ms}:{to_ms}"), move |conn, now, _, _| {
            compute::insights::waves(conn, from_ms, to_ms, now.as_millisecond())
        })
        .await
    }

    /// Whether `signature` had no crash in the weeks before `at_ms`.
    pub(crate) async fn quiet_before(&self, signature: i64, at_ms: i64) -> Result<bool, Error> {
        let now_ms = self.inner.clock.now().as_millisecond();
        Ok(self
            .inner
            .metrics
            .read(move |conn| compute::insights::quiet_before(conn, signature, at_ms, now_ms))
            .await?)
    }

    /// What each change of the last 90 days did to stability: the builds of the runs, and
    /// `recorded` when `with_changes`.
    pub(crate) async fn effects(
        &self,
        recorded: Vec<EffectChange>,
        with_changes: bool,
    ) -> Result<StabilityEffects, Error> {
        let key = format!(
            "effects:{with_changes}:{}:{}",
            recorded.len(),
            recorded.last().map_or(0, |change| change.at_ms)
        );
        let effects = self
            .cached(&key, move |conn, now, zone, _| {
                compute::insights::effects(conn, now, zone, recorded, with_changes)
            })
            .await?;
        Ok((*effects).clone())
    }

    /// The links someone said are not related.
    pub(crate) async fn dismissed_links(&self) -> Result<std::collections::HashSet<String>, Error> {
        Ok(self.inner.metrics.read(links::dismissed).await?)
    }

    /// Records that the caller says the link is not related, or takes that back, audited.
    pub(crate) async fn dismiss_link(
        &self,
        caller: &Caller,
        link: String,
        dismissed: bool,
    ) -> Result<(), Error> {
        let action = if dismissed {
            AuditAction::StatisticsLinkDismissed
        } else {
            AuditAction::StatisticsLinkRestored
        };
        let record = Record::by(caller, action).target(&link);
        let now = self.inner.clock.now();
        let by = caller.username.clone();
        let changed = self
            .inner
            .metrics
            .write(move |tx| -> Result<bool, StoreError> {
                if dismissed {
                    links::dismiss(tx, &link, &by, now.as_millisecond())?;
                    Ok(true)
                } else {
                    links::restore(tx, &link)
                }
            })
            .await?;
        if changed {
            self.inner
                .db
                .write(move |tx| -> Result<(), StoreError> { record.write(tx, now) })
                .await?;
        }
        Ok(())
    }

    /// A page of the drop log, newest first. Needs `PLAYERS_VIEW`, which the route checks; a
    /// search by an identifier other than the licence, and a linked group, need
    /// `PLAYER_IDENTIFIERS_VIEW`, checked before anything is looked up.
    pub async fn log(&self, query: &LogQuery, held: Permissions) -> Result<DropLog, Error> {
        let mut filter = self.filter(query, held).await?;
        filter.before = query.before.as_deref().map(parse_cursor).transpose()?;
        let mut rows = self
            .inner
            .metrics
            .read(move |conn| stored::log(conn, &filter, LOG_PAGE + 1))
            .await?;
        let next = (rows.len() > LOG_PAGE as usize).then(|| {
            rows.truncate(LOG_PAGE as usize);
            rows.last().map(|row| format!("{}.{}", row.ended_ms, row.id))
        });
        Ok(DropLog {
            rows: self.rows(rows).await?,
            next: next.flatten(),
        })
    }

    /// The whole filtered log for the CSV export, audited with the kinds of its filters only.
    pub async fn export_log(
        &self,
        caller: &Caller,
        query: &LogQuery,
        held: Permissions,
    ) -> Result<Vec<DropLogRow>, Error> {
        let filter = self.filter(query, held).await?;
        let rows = self
            .inner
            .metrics
            .read(move |conn| stored::log(conn, &filter, EXPORT_ROWS))
            .await?;
        let rows = self.rows(rows).await?;
        let record = Record::by(caller, AuditAction::PlayerDropsExported)
            .target(Servers::DEFAULT)
            .details(json!({ "filters": query.kinds(), "rows": rows.len() }));
        let now = self.inner.clock.now();
        self.inner.db.write(move |tx| record.write(tx, now)).await?;
        Ok(rows)
    }

    /// Daily drops and play from `from` to `to`, both days of the schedule's time zone, audited.
    pub async fn export_rates(
        &self,
        caller: &Caller,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
    ) -> Result<Vec<DayRates>, Error> {
        let zone = self.zone();
        let start = from
            .to_zoned(zone.clone())
            .map_err(|_| ErrorCode::InvalidRequest)?;
        let end = to
            .tomorrow()
            .and_then(|day| day.to_zoned(zone.clone()))
            .map_err(|_| ErrorCode::InvalidRequest)?;
        let (from_ms, to_ms) = (
            start.timestamp().as_millisecond(),
            end.timestamp().as_millisecond(),
        );
        if to_ms <= from_ms || to_ms - from_ms > EXPORT_DAYS * 86_400_000 {
            return Err(ErrorCode::InvalidRequest.into());
        }
        let now_ms = self.inner.clock.now().as_millisecond();
        let days = self
            .inner
            .metrics
            .read(move |conn| -> Result<_, StoreError> {
                let snapshot = conn.unchecked_transaction()?;
                let days = compute::day_rates(&snapshot, from_ms, to_ms, now_ms, &zone)?;
                snapshot.commit()?;
                Ok(days)
            })
            .await?;
        let record = Record::by(caller, AuditAction::DropRatesExported)
            .target(Servers::DEFAULT)
            .details(json!({ "from": from.to_string(), "to": to.to_string(), "days": days.len() }));
        let now = self.inner.clock.now();
        self.inner.db.write(move |tx| record.write(tx, now)).await?;
        Ok(days)
    }

    /// The players a crash of FXServer dropped, for Diagnostics: the run FXServer's process
    /// started at `spawned` with this generation.
    pub async fn crash(&self, spawned: Timestamp, generation: u32) -> Result<Option<CrashDrops>, Error> {
        let spawned_ms = spawned.as_millisecond();
        let run = self
            .inner
            .metrics
            .read(move |conn| stored::run_spawned(conn, spawned_ms, generation))
            .await?;
        Ok(run.map(|run| CrashDrops {
            run: run.id,
            players: run.crash_drops,
        }))
    }

    pub async fn settings(&self) -> Result<DropsSettings, Error> {
        Ok(self.inner.db.read(stored_players::drops_settings).await?)
    }

    pub async fn change_settings(&self, caller: &Caller, settings: DropsSettings) -> Result<(), Error> {
        if !settings.check() {
            return Err(ApiError::new(ErrorCode::InvalidDropsSettings)
                .with("min", DropsSettings::LEAST_DAYS)
                .with("max", DropsSettings::MOST_DAYS)
                .into());
        }
        let before = self.settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::DropsRetentionChanged)
            .target(Servers::DEFAULT)
            .details(json!({ "from": before, "to": settings }));
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), StoreError> {
                stored_players::save_drops_settings(tx, &settings)?;
                record.write(tx, now)
            })
            .await?;
        Ok(())
    }

    /// The store's filter for a log query. Whom it names is resolved through the players service,
    /// with its rules: never an address, other identifiers only with `PLAYER_IDENTIFIERS_VIEW`.
    async fn filter(&self, query: &LogQuery, held: Permissions) -> Result<LogFilter, Error> {
        let players = match (&query.group, &query.player, &query.profile) {
            (Some(group), _, _) => Some(self.inner.players.group_pseudonyms(*group, held).await?),
            (None, Some(search), _) => Some(self.inner.players.pseudonyms_matching(search, held).await?),
            (None, None, Some(profile)) => Some(
                self.inner
                    .players
                    .pseudonym_of(*profile)
                    .await?
                    .into_iter()
                    .collect(),
            ),
            (None, None, None) => None,
        };
        let resource = match &query.resource {
            Some(name) => {
                let name = name.clone();
                // An unknown resource matches nothing rather than everything.
                Some(
                    self.inner
                        .metrics
                        .read(move |conn| stored::resource_id(conn, &name))
                        .await?
                        .unwrap_or(-1),
                )
            }
            None => None,
        };
        let now_ms = self.inner.clock.now().as_millisecond();
        let range = query.range.unwrap_or(DropsRange::Week);
        Ok(LogFilter {
            from_ms: now_ms - range.days() * 86_400_000,
            to_ms: now_ms + 1,
            category: query.category,
            kind: query.kind,
            resource,
            signature: query.signature,
            run: query.run,
            players,
            before: None,
        })
    }

    async fn rows(&self, rows: Vec<LogRow>) -> Result<Vec<DropLogRow>, Error> {
        let mut pseudonyms: Vec<[u8; 16]> = rows.iter().filter_map(|row| row.player).collect();
        pseudonyms.sort_unstable();
        pseudonyms.dedup();
        let players = self.inner.players.identities(pseudonyms).await?;
        let mut ids: Vec<i64> = rows.iter().filter_map(|row| row.signature).collect();
        ids.sort_unstable();
        ids.dedup();
        let signatures: HashMap<i64, (String, Option<String>)> = self
            .inner
            .metrics
            .read(move |conn| stored::signatures(conn, &ids))
            .await?
            .into_iter()
            .map(|row| (row.id, (row.signature, row.label)))
            .collect();
        Ok(rows
            .into_iter()
            .map(|row| {
                let signature = row.signature.and_then(|id| signatures.get(&id));
                DropLogRow {
                    id: row.id,
                    at: Timestamp::from_millisecond(row.ended_ms).unwrap_or(Timestamp::UNIX_EPOCH),
                    player: row.player.and_then(|player| players.get(&player)).cloned(),
                    category: row.category,
                    kind: row.kind,
                    resource: row.resource,
                    signature_id: row.signature,
                    signature: signature.map(|(signature, _)| signature.clone()),
                    label: signature.and_then(|(_, label)| label.clone()),
                    reason: row.reason_text,
                    length_ms: row.length_ms,
                    loading: row.loading,
                    run: row.run,
                    generation: row.generation,
                    synthesized: row.synthesized,
                    incident: row.incident.is_some(),
                }
            })
            .collect())
    }
}

/// `ended_ms.id`, as [`Drops::log`] hands it out.
fn parse_cursor(text: &str) -> Result<(i64, i64), Error> {
    text.split_once('.')
        .and_then(|(ended, id)| Some((ended.parse().ok()?, id.parse().ok()?)))
        .ok_or_else(|| ErrorCode::InvalidRequest.into())
}

#[cfg(test)]
mod tests;
