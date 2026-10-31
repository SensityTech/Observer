//! The drops' own task, every minute twenty seconds past, after the players' flush: closed sessions
//! get their signature or reason group, bursts become incidents, closed hours are rolled up, and
//! every hour the sessions past their retention go.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use jiff::SignedDuration;
use observer_domain::drops::DropCategory;
use observer_domain::drops::signature::{self, NORMALISER, Signature, SignatureForm};
use observer_store::StoreError;
use observer_store::Transaction;
use observer_store::metrics::drops::{
    self as stored, BURST_MS, Grouping, HOUR_MS, NewSignature, Unclassified,
};
use observer_store::metrics::sessions as stored_sessions;
use observer_store::players as stored_players;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::Inner;

const PAST_THE_MINUTE_MS: i64 = 20_000;
const FIRST_ROUND_AFTER: Duration = Duration::from_secs(5);
/// Sessions classified per transaction, and transactions per minute: a backlog never holds the
/// writer for long.
const CLASSIFY_BATCH: u32 = 200;
const CLASSIFY_ROUNDS: usize = 25;
/// Hours rolled up per transaction.
const HOURS_PER_WRITE: usize = 24;
/// One player's crashes make at most this many new signatures a day; then `text:other`.
const NEW_SIGNATURES_A_DAY: i64 = 5;
/// The unconfirmed sessions together make at most this many, and never of a client's free text.
const UNCONFIRMED_SIGNATURES_A_DAY: i64 = 20;
const OTHER: &str = "text:other";

pub(super) async fn run(inner: std::sync::Arc<Inner>, stop: CancellationToken) {
    // Observer's start, its first samples and the players' recovery go first.
    tokio::select! {
        () = stop.cancelled() => return,
        () = tokio::time::sleep(FIRST_ROUND_AFTER) => {}
    }
    // A newer normaliser classifies the sessions kept again.
    match inner.metrics.write(|tx| stored::outdated(tx, NORMALISER)).await {
        Ok(0) => {}
        Ok(sessions) => info!(
            sessions,
            normaliser = NORMALISER,
            "players' drops are classified again"
        ),
        Err(error) => warn!(%error, "players' drops could not be marked for a new classification"),
    }
    let mut expired_hour = None;
    let mut failing = false;
    loop {
        match tick(&inner, &mut expired_hour).await {
            Ok(()) => failing = false,
            Err(error) if !failing => {
                failing = true;
                warn!(%error, "players' drops could not be counted; Observer tries again every minute");
            }
            Err(_) => {}
        }
        let now_ms = inner.clock.now().as_millisecond();
        let mut next = now_ms.div_euclid(60_000) * 60_000 + PAST_THE_MINUTE_MS;
        if next <= now_ms {
            next += 60_000;
        }
        let wait = Duration::from_millis(u64::try_from(next - now_ms).unwrap_or(0));
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
        }
    }
}

pub(super) async fn tick(inner: &Inner, expired_hour: &mut Option<i64>) -> Result<(), StoreError> {
    classify(inner).await?;
    let now_ms = inner.clock.now().as_millisecond();
    let pending = inner
        .metrics
        .read(move |conn| stored::pending_hours(conn, now_ms))
        .await?;
    for hours in pending.chunks(HOURS_PER_WRITE) {
        let hours = hours.to_vec();
        inner
            .metrics
            .write(move |tx| -> Result<(), StoreError> {
                for hour in hours {
                    stored::find_incidents(tx, hour * 1000, hour * 1000 + HOUR_MS)?;
                    stored::roll_up(tx, hour, now_ms)?;
                }
                Ok(())
            })
            .await?;
    }
    // The hour under way: a burst is known before its hour closes.
    let hour_ms = stored::hour_of(now_ms) * 1000;
    inner
        .metrics
        .write(move |tx| stored::find_incidents(tx, hour_ms, now_ms - BURST_MS))
        .await?;

    let hour = now_ms.div_euclid(HOUR_MS);
    if *expired_hour != Some(hour) {
        *expired_hour = Some(hour);
        let retention = inner.db.read(stored_players::drops_settings).await?;
        let before_ms = (inner.clock.now()
            - SignedDuration::from_hours(24 * i64::from(retention.session_days)))
        .as_millisecond();
        // Who made a signature only matters for that day's quota.
        let producers_ms = now_ms - 2 * 24 * HOUR_MS;
        let expired = inner
            .metrics
            .write(move |tx| -> Result<usize, StoreError> {
                stored::count_crash_drops(tx, now_ms)?;
                stored::forget_producers(tx, producers_ms)?;
                stored_sessions::expire(tx, before_ms)
            })
            .await?;
        // A forget's checkpoint may have met a read: the journal is emptied here at the latest.
        if let Err(error) = inner.metrics.checkpoint().await {
            debug!(%error, "the statistics' journal could not be emptied; SQLite does it later");
        }
        if expired > 0 {
            info!(
                sessions = expired,
                "players' sessions past their retention deleted"
            );
        }
    }
    Ok(())
}

/// Gives the closed sessions their signature or reason group, oldest first.
async fn classify(inner: &Inner) -> Result<usize, StoreError> {
    let mut classified = 0;
    for _ in 0..CLASSIFY_ROUNDS {
        let pending = inner
            .metrics
            .read(|conn| stored::unclassified(conn, CLASSIFY_BATCH))
            .await?;
        if pending.is_empty() {
            break;
        }
        let pseudonyms: Vec<[u8; 16]> = pending
            .iter()
            .filter_map(|session| session.player)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        // A reason's template puts `{player}` for the dropped player's own name.
        let names: HashMap<[u8; 16], String> = match inner.players.identities(pseudonyms).await {
            Ok(found) => found
                .into_iter()
                .filter_map(|(pseudonym, player)| Some((pseudonym, player.name?)))
                .collect(),
            Err(error) => {
                warn!(%error, "players' names could not be read for their drops' reasons");
                HashMap::new()
            }
        };
        classified += pending.len();
        inner
            .metrics
            .write(move |tx| -> Result<(), StoreError> {
                for session in &pending {
                    let name = session
                        .player
                        .and_then(|player| names.get(&player))
                        .map(String::as_str);
                    // A forget or a late drop since the read changed it: the next round reads it again.
                    if stored::unchanged(tx, session)? {
                        group(tx, session, name)?;
                    }
                }
                Ok(())
            })
            .await?;
    }
    Ok(classified)
}

fn group(tx: &Transaction<'_>, session: &Unclassified, name: Option<&str>) -> Result<(), StoreError> {
    match (session.category, &session.crash_text, &session.reason_text) {
        (DropCategory::Crash, Some(text), _) => {
            let mut found = signature::signature(text);
            if !stored::signature_known(tx, &found.signature)? {
                // A modified client can crash with any text: one player makes a few new
                // signatures in the day before their crash, an unconfirmed one none at all.
                let day_before = session.ended_ms - 24 * HOUR_MS;
                let allowed = match session.player {
                    Some(_) => {
                        stored::signatures_made(tx, session.player, day_before, session.ended_ms)?
                            < NEW_SIGNATURES_A_DAY
                    }
                    None => {
                        !found.form.free_text()
                            && stored::signatures_made(tx, None, day_before, session.ended_ms)?
                                < UNCONFIRMED_SIGNATURES_A_DAY
                    }
                };
                if !allowed {
                    found = other();
                }
            }
            stored::classify(
                tx,
                session,
                Grouping::Signature(NewSignature {
                    signature: &found.signature,
                    form: found.form.as_str(),
                    module: found.module.as_deref(),
                    game_build: found.game_build,
                    label: found.label.as_deref(),
                    resource: found.resource.as_deref(),
                    normaliser: NORMALISER,
                }),
                NORMALISER,
            )
        }
        (DropCategory::Kick | DropCategory::Unknown, _, Some(text)) => {
            let template = signature::template(text, name);
            stored::classify(tx, session, Grouping::Reason(&template), NORMALISER)
        }
        _ => stored::classify(tx, session, Grouping::None, NORMALISER),
    }
}

fn other() -> Signature {
    Signature {
        signature: OTHER.to_owned(),
        form: SignatureForm::Text,
        module: None,
        game_build: None,
        label: None,
        resource: None,
    }
}
