//! Bans: what the staff decides, kept in the state database, and what the bridge enforces. The
//! admission asks them about each player the bridge holds at `playerConnecting`; their own task
//! checks every player once they joined, and keeps who is connected, so that a ban made while its
//! player plays drops them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::AuditAction;
use observer_domain::bans::{
    self as rules, Ban, BanFilter, BanPage, BanRequest, BanSearch, BansSettings, LiftRequest,
    MAX_BAN_MINUTES, MAX_BANNED_IDENTIFIERS, MAX_LIFT_REASON_CHARS, MAX_MESSAGE_CHARS, MAX_REASON_CHARS,
    MAX_TOKEN_MATCHES, RefusalValues,
};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::permissions::Permissions;
use observer_domain::players::{Identifiers, Search, SearchRefusal, clean_name};
use observer_domain::schedule::ScheduleStatus;
use observer_fxserver::bridge::{BanEvent, BridgeAction};
use observer_fxserver::players::{PlayerReport, PlayerReportKind, Shown};
use observer_fxserver::supervisor::Supervisor;
use observer_store::Database;
use observer_store::bans::{self as stored, BanMatch, BanQuery, Candidate, Lifted, NewBan};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::audit::Record;
use crate::secrets::random;
use crate::{Caller, Clock, Error, Supervision};

const PAGE: u32 = 50;

pub struct BansSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
    /// The schedule's status, whose time zone the refusal message writes dates in.
    pub schedule: watch::Receiver<ScheduleStatus>,
}

/// The bans service. Clones share it.
#[derive(Clone)]
pub struct Bans {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    supervisor: Supervisor,
    schedule: watch::Receiver<ScheduleStatus>,
    connected: Mutex<Connected>,
}

/// The players of the run under way, as the bridge reported them.
#[derive(Default)]
struct Connected {
    generation: u32,
    players: HashMap<u32, Seen>,
}

/// What a player showed, in the form bans hold it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Seen {
    held: Vec<(String, String)>,
    pub(crate) name: Option<String>,
    pub(crate) license: Option<String>,
}

impl Seen {
    pub(crate) fn of(shown: &Shown) -> Self {
        let identifiers = Identifiers::parse(
            shown.identifiers.iter().map(String::as_str),
            shown.tokens.iter().map(String::as_str),
        );
        Self {
            held: held(&identifiers),
            name: shown.name.as_deref().and_then(clean_name),
            license: identifiers.license,
        }
    }
}

/// The identifiers as a ban stores them: the licence without its prefix, the others as the
/// players keep them.
fn held(identifiers: &Identifiers) -> Vec<(String, String)> {
    let license = identifiers
        .license
        .as_deref()
        .and_then(|license| license.strip_prefix("license:"));
    license
        .map(|hex| (stored::LICENSE.to_owned(), hex.to_owned()))
        .into_iter()
        .chain(
            identifiers
                .others
                .iter()
                .map(|other| (other.kind.as_str().to_owned(), other.value.clone())),
        )
        .collect()
}

/// The bans' task; [`BansTask::close`] ends it.
pub struct BansTask {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl BansTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the bans' task failed");
        }
    }
}

impl Bans {
    /// Opens the service and starts its task, before FXServer boots so that no player gets past
    /// it.
    pub fn start(setup: BansSetup) -> (Self, BansTask) {
        let supervisor = setup.supervision.supervisor().clone();
        let reports = supervisor.player_reports();
        let bans = Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                supervisor,
                schedule: setup.schedule,
                connected: Mutex::new(Connected::default()),
            }),
        };
        let stop = CancellationToken::new();
        let task = tokio::spawn(bans.clone().run(stop.clone(), reports));
        (bans, BansTask { stop, task })
    }

    async fn run(self, stop: CancellationToken, mut reports: broadcast::Receiver<PlayerReport>) {
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                report = reports.recv() => match report {
                    Ok(report) => self.on_report(report).await,
                    Err(RecvError::Lagged(missed)) => {
                        warn!(missed, "the bans fell behind the bridge's reports; joined players are checked again");
                    }
                    Err(RecvError::Closed) => return,
                },
            }
        }
    }

    async fn on_report(&self, report: PlayerReport) {
        {
            let mut connected = self.connected();
            if connected.generation != report.generation {
                connected.generation = report.generation;
                connected.players.clear();
            }
        }
        match report.kind {
            PlayerReportKind::Join(joined) => self.joined(joined.net_id, &joined.shown).await,
            PlayerReportKind::Roster(page) => {
                for (net_id, shown) in &page.players {
                    self.joined(*net_id, shown).await;
                }
            }
            PlayerReportKind::Drop(dropped) => {
                self.connected().players.remove(&dropped.net_id);
            }
            // The admission answers a connecting player, after asking the bans.
            PlayerReportKind::Connecting(_)
            | PlayerReportKind::Labels(_)
            | PlayerReportKind::Positions(_) => {}
        }
    }

    /// A player who got in, checked again: the verdict may not have come in time, or a ban was
    /// made since.
    async fn joined(&self, net_id: u32, shown: &Shown) {
        let seen = Seen::of(shown);
        self.connected().players.insert(net_id, seen.clone());
        if let Some((message, event)) = self.refusal(&seen, Some(net_id)).await {
            self.act(BridgeAction::Drop {
                player: net_id,
                message,
                event,
            })
            .await;
        }
    }

    /// The message and the event for a player a ban refuses, counted on the ban; `None` when no
    /// ban does, or when the database cannot tell, so that a failing disk never locks everyone out.
    pub(crate) async fn refusal(&self, seen: &Seen, player: Option<u32>) -> Option<(String, BanEvent)> {
        let shown = seen.held.clone();
        let now = self.inner.clock.now();
        let found = self
            .inner
            .db
            .read(move |conn| -> Result<_, Error> {
                let settings = stored::settings(conn)?;
                let refusing = stored::candidates(conn, &shown, now)?
                    .into_iter()
                    .find(|candidate| {
                        rules::refuses(candidate.identifiers, candidate.tokens, settings.token_matches)
                    });
                Ok(refusing.map(|candidate| (candidate, settings)))
            })
            .await;
        let (candidate, settings) = match found {
            Ok(found) => found?,
            Err(error) => {
                warn!(%error, "a player could not be checked against the bans and gets in");
                return None;
            }
        };
        let id = candidate.id;
        if let Err(error) = self.inner.db.write(move |tx| stored::count_refusal(tx, id)).await {
            warn!(%error, "a refusal could not be counted on its ban");
        }
        Some(self.refused(&candidate, &settings, seen, player))
    }

    fn refused(
        &self,
        candidate: &Candidate,
        settings: &BansSettings,
        seen: &Seen,
        player: Option<u32>,
    ) -> (String, BanEvent) {
        let ends = self.ends_text(candidate.ends_at);
        let message = rules::refusal(
            &settings.message,
            &RefusalValues {
                reason: &candidate.reason,
                ends: &ends,
                code: &candidate.code,
                name: seen.name.as_deref().unwrap_or_default(),
                by: &candidate.by,
            },
        );
        let event = BanEvent {
            ban: candidate.code.clone(),
            reason: candidate.reason.clone(),
            ends_at: candidate.ends_at.map(Timestamp::as_millisecond),
            by: candidate.by.clone(),
            name: seen.name.clone(),
            license: seen.license.clone(),
            player,
        };
        (message, event)
    }

    /// When a ban ends, as the refusal message writes it: in the schedule's zone, named.
    fn ends_text(&self, ends_at: Option<Timestamp>) -> String {
        let Some(ends_at) = ends_at else {
            return "never".to_owned();
        };
        let zone = self.inner.schedule.borrow().zone.clone();
        let local = ends_at.to_zoned(TimeZone::get(&zone).unwrap_or(TimeZone::UTC));
        format!("{} ({zone})", local.strftime("%Y-%m-%d %H:%M"))
    }

    async fn act(&self, action: BridgeAction) {
        // A server that stopped meanwhile has no one left to answer or drop.
        let _ = self.inner.supervisor.act(action.fitted()).await;
    }

    fn connected(&self) -> std::sync::MutexGuard<'_, Connected> {
        self.inner
            .connected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Bans a player Observer knows, with every identifier and token their profile links to, or
    /// identifiers typed by hand, or both; drops the connected players it refuses.
    pub async fn ban(&self, caller: &Caller, request: BanRequest) -> Result<Ban, Error> {
        let invalid = |field: &str| Error::from(ApiError::new(ErrorCode::InvalidBan).with("field", field));
        let reason = rules::clean_line(&request.reason, MAX_REASON_CHARS).ok_or_else(|| invalid("reason"))?;
        if request
            .minutes
            .is_some_and(|minutes| !(1..=MAX_BAN_MINUTES).contains(&minutes))
        {
            return Err(invalid("minutes"));
        }
        if request.identifiers.len() > MAX_BANNED_IDENTIFIERS {
            return Err(invalid("identifiers"));
        }
        let typed = rules::parse_banned(&request.identifiers);
        let now = self.inner.clock.now();
        let ends_at = request
            .minutes
            .map(|minutes| now + SignedDuration::from_mins(i64::from(minutes)));
        let player = request.player;
        let caller_record = Record::by(caller, AuditAction::PlayerBanned);
        let (by_user, by_name) = (caller.user_id, caller.username.clone());
        let connected: Vec<(u32, Seen)> = self
            .connected()
            .players
            .iter()
            .map(|(id, seen)| (*id, seen.clone()))
            .collect();
        let (id, code, dropped) = self
            .inner
            .db
            .write(move |tx| -> Result<_, Error> {
                let identity = match player {
                    Some(id) => Some(stored::identity(tx, id)?.ok_or(ErrorCode::PlayerNotFound)?),
                    None => None,
                };
                let (held, license, name) = holding(&typed, identity);
                if held.is_empty() {
                    return Err(invalid("identifiers"));
                }
                let mut code = rules::ban_code(random()?);
                while stored::code_taken(tx, &code)? {
                    code = rules::ban_code(random()?);
                }
                let id = stored::insert(
                    tx,
                    &NewBan {
                        code: &code,
                        player,
                        name: name.as_deref(),
                        license: license.as_deref(),
                        reason: &reason,
                        by_user: Some(by_user),
                        by_name: &by_name,
                        at: now,
                        ends_at,
                        held: &held,
                    },
                )?;
                let token_matches = stored::settings(tx)?.token_matches;
                let dropped: Vec<(u32, Seen)> = connected
                    .into_iter()
                    .filter(|(_, seen)| {
                        let (identifiers, tokens) = overlap(&held, &seen.held);
                        rules::refuses(identifiers, tokens, token_matches)
                    })
                    .collect();
                let tokens = held.iter().filter(|(kind, _)| kind == stored::TOKEN).count();
                caller_record
                    .target(code.clone())
                    .details(json!({
                        "player": player,
                        "license": license,
                        "reason": reason,
                        "minutes": request.minutes,
                        "identifiers": held.len() - tokens,
                        "tokens": tokens,
                        "dropped": dropped.len(),
                    }))
                    .write(tx, now)?;
                Ok((id, code, dropped))
            })
            .await?;
        let ban = self.by_code(&code).await?;
        if dropped.is_empty() {
            return Ok(ban);
        }
        self.drop_banned(&ban, id, &dropped).await?;
        self.by_code(&code).await
    }

    /// Drops the connected players a new ban refuses, each counted on it.
    async fn drop_banned(&self, ban: &Ban, id: i64, dropped: &[(u32, Seen)]) -> Result<(), Error> {
        let settings = self.settings().await?;
        let candidate = Candidate {
            id,
            code: ban.code.clone(),
            reason: ban.reason.clone(),
            by: ban.by.clone(),
            ends_at: ban.ends_at,
            identifiers: 0,
            tokens: 0,
        };
        for (net_id, seen) in dropped {
            let (message, event) = self.refused(&candidate, &settings, seen, Some(*net_id));
            self.act(BridgeAction::Drop {
                player: *net_id,
                message,
                event,
            })
            .await;
        }
        let count = dropped.len();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                for _ in 0..count {
                    stored::count_refusal(tx, id)?;
                }
                Ok(())
            })
            .await
    }

    async fn by_code(&self, code: &str) -> Result<Ban, Error> {
        let code = code.to_owned();
        let now = self.inner.clock.now();
        self.inner
            .db
            .read(move |conn| stored::by_code(conn, &code, now))
            .await?
            .ok_or_else(|| ErrorCode::BanNotFound.into())
    }

    /// Lifts an active ban; it stays in the list with who lifted it and why.
    pub async fn lift(&self, caller: &Caller, code: &str, request: LiftRequest) -> Result<(), Error> {
        let code = rules::parse_ban_code(code).ok_or(ErrorCode::BanNotFound)?;
        let reason =
            match request.reason.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(text) => Some(rules::clean_line(text, MAX_LIFT_REASON_CHARS).ok_or_else(|| {
                    Error::from(ApiError::new(ErrorCode::InvalidBan).with("field", "reason"))
                })?),
            };
        let record = Record::by(caller, AuditAction::BanLifted)
            .target(code.clone())
            .details(json!({ "reason": reason }));
        let by = caller.username.clone();
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                match stored::lift(tx, &code, &by, reason.as_deref(), now)? {
                    Lifted::Done => Ok(record.write(tx, now)?),
                    Lifted::NotFound => Err(ErrorCode::BanNotFound.into()),
                    Lifted::NotActive => Err(ErrorCode::BanNotActive.into()),
                }
            })
            .await
    }

    /// A page of the list, latest first. A search by an identifier other than the licence needs
    /// `PLAYER_IDENTIFIERS_VIEW`, as the players' search does.
    pub async fn page(
        &self,
        filter: BanFilter,
        search: Option<&str>,
        after: Option<&str>,
        held: Permissions,
    ) -> Result<BanPage, Error> {
        let search = match search.map(BanSearch::parse).transpose() {
            Err(SearchRefusal::Address) => return Err(ErrorCode::AddressSearch.into()),
            Err(SearchRefusal::Malformed) => return Err(ErrorCode::InvalidSearch.into()),
            Ok(None | Some(None)) => BanMatch::All,
            Ok(Some(Some(BanSearch::Code(code)))) => BanMatch::Code(code),
            Ok(Some(Some(BanSearch::Text(text)))) => BanMatch::Text(text),
            Ok(Some(Some(BanSearch::Player(Search::License(license))))) => BanMatch::License(license),
            Ok(Some(Some(BanSearch::Player(Search::Identifier(identifier))))) => {
                if !held.contains(Permissions::PLAYER_IDENTIFIERS_VIEW) {
                    return Err(ApiError::new(ErrorCode::Forbidden)
                        .with("permission", "playerIdentifiersView")
                        .into());
                }
                BanMatch::Identifier(identifier.kind.as_str().to_owned(), identifier.value)
            }
            Ok(Some(Some(BanSearch::Player(Search::Name(name))))) => BanMatch::Text(name),
        };
        let after = after
            .map(|after| {
                after
                    .parse::<i64>()
                    .map_err(|_| Error::from(ErrorCode::InvalidRequest))
            })
            .transpose()?;
        let query = BanQuery {
            filter,
            search,
            after,
            limit: PAGE + 1,
            now: self.inner.clock.now(),
        };
        let now = query.now;
        let (mut rows, counts) = self
            .inner
            .db
            .read(move |conn| -> Result<_, Error> {
                Ok((stored::page(conn, &query)?, stored::counts(conn, now)?))
            })
            .await?;
        let next = (rows.len() > PAGE as usize).then(|| {
            rows.truncate(PAGE as usize);
            rows.last().map(|(_, id)| id.to_string())
        });
        Ok(BanPage {
            bans: rows.into_iter().map(|(ban, _)| ban).collect(),
            next: next.flatten(),
            counts,
            zone: self.inner.schedule.borrow().zone.clone(),
        })
    }

    /// The bans made from a player's profile, latest first.
    pub async fn of_player(&self, player: i64) -> Result<Vec<Ban>, Error> {
        let now = self.inner.clock.now();
        Ok(self
            .inner
            .db
            .read(move |conn| stored::of_player(conn, player, now))
            .await?)
    }

    pub async fn settings(&self) -> Result<BansSettings, Error> {
        Ok(self.inner.db.read(stored::settings).await?)
    }

    /// Changes the refusal message or the tokens a match needs, from the next connection.
    pub async fn change_settings(&self, caller: &Caller, settings: BansSettings) -> Result<(), Error> {
        let settings = settings.checked().ok_or_else(|| {
            Error::from(
                ApiError::new(ErrorCode::InvalidBansSettings)
                    .with("maxMessage", MAX_MESSAGE_CHARS)
                    .with("maxTokens", MAX_TOKEN_MATCHES),
            )
        })?;
        let before = self.settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::BansSettingsChanged)
            .details(json!({ "from": before, "to": settings }));
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                stored::save_settings(tx, &settings)?;
                Ok(record.write(tx, now)?)
            })
            .await
    }
}

/// What a ban holds, its licence and its name: the identifiers typed by hand, and those of the
/// player's profile when it is made from one.
fn holding(
    typed: &Identifiers,
    identity: Option<stored::Identity>,
) -> (Vec<(String, String)>, Option<String>, Option<String>) {
    let mut held = held(typed);
    let mut license = typed.license.clone();
    let mut name = None;
    if let Some(identity) = identity {
        if let Some(hex) = identity.license.strip_prefix("license:") {
            held.push((stored::LICENSE.to_owned(), hex.to_owned()));
        }
        held.extend(identity.others);
        license = Some(identity.license);
        name = Some(identity.name);
    }
    held.sort();
    held.dedup();
    (held, license, name)
}

/// How many of a ban's identifiers, tokens apart, a player shows.
fn overlap(held: &[(String, String)], shown: &[(String, String)]) -> (u32, u32) {
    shown
        .iter()
        .filter(|entry| held.contains(entry))
        .fold((0, 0), |(identifiers, tokens), (kind, _)| {
            if kind == stored::TOKEN {
                (identifiers, tokens + 1)
            } else {
                (identifiers + 1, tokens)
            }
        })
}

#[cfg(test)]
mod tests;
