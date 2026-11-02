//! What the Statistics page reads from the metrics database beside the series: the frame time
//! distribution of two periods, players by day and hour, memory run after run, and the runs as
//! the timeline's events. Each runs on the database's reader.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use observer_domain::drops::DropsRange;
use observer_domain::metrics::{LEAK_POINTS, LEAK_SPAN, PRIVATE_LEAK};
use observer_domain::statistics::{
    EventDetail, FRAME_BUDGET_MS, FrameDistribution, FramePeriod, HOURS_DAYS, PlayerHours, RunEnd, RunFit,
    RunMemory, TimelineEvent, acceleration, quietest,
};
use observer_store::metrics::drops::{self as stored_drops, RunRow};
use observer_store::metrics::growth;
use observer_store::metrics::rollups::MINUTES_KEPT;
use observer_store::metrics::series::{self, Table, Window};
use observer_store::{Connection, StoreError};

const DAY: i64 = 86_400;

/// svMain's frames of the range ending at `now` (Unix seconds) and of the one before, on the
/// bounds that hold most of the newer period's ticks. A day reads the five-second slots, longer
/// ranges the closed hours.
pub(super) fn frames(
    conn: &Connection,
    range: DropsRange,
    now: i64,
) -> Result<FrameDistribution, StoreError> {
    let span = range.days() * DAY;
    let (to, table) = if range == DropsRange::Day {
        (now, Table::Raw)
    } else {
        (now - now.rem_euclid(3600), Table::Hour)
    };
    let known = series::tick_bounds(conn)?;
    let period = |from: i64| -> Result<BTreeMap<i64, Vec<u64>>, StoreError> {
        let window = Window {
            from,
            step: span,
            buckets: 1,
            table,
        };
        Ok(series::tick_grid(conn, &window, &known)?
            .pop()
            .unwrap_or_default())
    };
    let now_ticks = period(to - span)?;
    let before_ticks = period(to - 2 * span)?;
    let family = now_ticks
        .iter()
        .chain(&before_ticks)
        .max_by_key(|(_, ticks)| ticks.iter().sum::<u64>())
        .map(|(id, _)| *id);
    let bounds = family.and_then(|id| known.get(&id)).cloned().unwrap_or_default();
    let of = |ticks: &BTreeMap<i64, Vec<u64>>| -> Vec<u64> {
        family
            .and_then(|id| ticks.get(&id))
            .cloned()
            .unwrap_or_else(|| vec![0; bounds.len() + 1])
    };
    Ok(FrameDistribution {
        range,
        now: FramePeriod::of(to - span, to, &bounds, &of(&now_ticks)),
        before: FramePeriod::of(to - 2 * span, to - span, &bounds, &of(&before_ticks)),
        bounds,
        budget: FRAME_BUDGET_MS,
        since: series::first_frames(conn)?,
    })
}

/// Players on average by day of the week and hour of the day in `zone`, over the 28 days before
/// the hour that holds `now`.
pub(super) fn hours(conn: &Connection, zone: &TimeZone, now: i64) -> Result<PlayerHours, StoreError> {
    let to = now - now.rem_euclid(3600);
    let from = to - HOURS_DAYS * DAY;
    let mut cells = vec![(0.0, 0.0, 0u32); 168];
    let mut daily = vec![(0.0, 0.0); 24];
    for (ts, players, samples) in series::hourly_players(conn, from, to)? {
        let Ok(at) = Timestamp::from_second(ts) else {
            continue;
        };
        let local = at.to_zoned(zone.clone());
        let day = usize::try_from(local.weekday().to_monday_zero_offset()).unwrap_or_default();
        let hour = usize::try_from(local.hour()).unwrap_or_default();
        let cell = &mut cells[day * 24 + hour];
        cell.0 += players * samples;
        cell.1 += samples;
        cell.2 += 1;
        daily[hour].0 += players * samples;
        daily[hour].1 += samples;
    }
    let average = |(sum, weight): (f64, f64)| (weight > 0.0).then(|| sum / weight);
    let players: Vec<Option<f64>> = cells
        .iter()
        .map(|&(sum, weight, _)| average((sum, weight)))
        .collect();
    let daily: Vec<Option<f64>> = daily.into_iter().map(average).collect();
    Ok(PlayerHours {
        zone: zone.iana_name().unwrap_or("UTC").to_owned(),
        from,
        to,
        quietest: quietest(&players, &daily),
        players,
        measured: cells.iter().map(|&(_, _, hours)| hours).collect(),
        daily,
    })
}

/// The runs that overlap the range ending at `now`, each with its memory fitted while its minutes
/// are kept, and how each run's growth compares with the one before.
pub(super) fn runs(conn: &Connection, range: DropsRange, now: i64) -> Result<RunMemory, StoreError> {
    let from = now - range.days() * DAY;
    let fitted_from = now - MINUTES_KEPT;
    let mut runs = Vec::new();
    for run in stored_drops::runs(conn, from * 1000, now * 1000)? {
        let until = run.ended_ms.map_or(now, |ended| ended.div_euclid(1000));
        if until < from {
            continue;
        }
        let (private, peak) = if run.started_ms.div_euclid(1000) >= fitted_from {
            growth::of_run(conn, run.started_ms, until)?
        } else {
            (None, None)
        };
        runs.push(RunFit {
            started_at: run.started_ms.div_euclid(1000),
            ended_at: run.ended_ms.map(|ended| ended.div_euclid(1000)),
            end: run.end_cause.as_deref().and_then(RunEnd::parse),
            build: run.build,
            private,
            peak,
        });
    }
    // Only runs long enough for a verdict say how fast they grew.
    let growths: Vec<f64> = runs
        .iter()
        .filter_map(|run| run.private.as_ref())
        .filter(|growth| growth.points >= LEAK_POINTS && f64::from(growth.points) * 60.0 >= real(LEAK_SPAN))
        .map(|growth| growth.per_hour)
        .collect();
    Ok(RunMemory {
        range,
        from,
        to: now,
        acceleration: acceleration(&growths, PRIVATE_LEAK.floor),
        runs,
        fitted_from,
    })
}

/// The runs' starts, restarts, stops and crashes from `from_ms` to before `to_ms`, oldest first.
pub(super) fn run_events(
    conn: &Connection,
    from_ms: i64,
    to_ms: i64,
) -> Result<Vec<TimelineEvent>, StoreError> {
    let runs = stored_drops::runs(conn, from_ms, to_ms)?;
    let mut events = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        let next = runs.get(index + 1);
        // A crash and the recovery after it are one event: the crash, back online at the next
        // run's ready time.
        let recovered = |run: &RunRow| run.start_cause == "recovery";
        if (from_ms..to_ms).contains(&run.started_ms) && !recovered(run) {
            let detail = if run.start_cause == "restart" {
                EventDetail::Restart { build: run.build }
            } else {
                EventDetail::Start { build: run.build }
            };
            events.push(event(format!("run:{}", run.id), run.started_ms, detail));
        }
        let Some(ended) = run.ended_ms.filter(|ended| (from_ms..to_ms).contains(ended)) else {
            continue;
        };
        let detail = match run.end_cause.as_deref() {
            Some(cause @ ("crashed" | "hung")) => EventDetail::Crash {
                hang: cause == "hung",
                back_online_at: next
                    .filter(|next| recovered(next))
                    .map(|next| next.started_ms.div_euclid(1000)),
                players_dropped: Some(run.crash_drops),
            },
            // A restart's stop is the restart itself.
            Some("stopped") if next.is_some_and(|next| next.start_cause == "restart") => continue,
            Some("stopped") => EventDetail::Stop,
            _ => continue,
        };
        events.push(event(format!("end:{}", run.id), ended, detail));
    }
    events.sort_by_key(|event| event.at);
    Ok(events)
}

fn event(key: String, at_ms: i64, detail: EventDetail) -> TimelineEvent {
    TimelineEvent {
        key,
        at: at_ms.div_euclid(1000),
        until: None,
        detail,
    }
}

/// Seconds stay far below 2⁵³, where a float stops being exact.
#[allow(clippy::cast_precision_loss)]
fn real(value: i64) -> f64 {
    value as f64
}

#[cfg(test)]
mod tests {
    use observer_store::MetricsDatabase;
    use observer_store::metrics::runs::{self, EndCause, RunKey, StartCause};

    use super::*;

    const START_MS: i64 = 1_761_897_600_000;
    const MINUTE_MS: i64 = 60_000;

    #[tokio::test]
    async fn a_restart_is_one_event_and_a_crash_carries_the_recovery_that_followed_it() {
        let dir = tempfile::tempdir().unwrap();
        let db = MetricsDatabase::open(dir.path().join("metrics.sqlite3"))
            .await
            .unwrap();
        let key = |generation| RunKey {
            observer_started: START_MS,
            generation,
        };
        db.write(move |tx| -> Result<(), StoreError> {
            runs::start(tx, key(1), START_MS, StartCause::Requested)?;
            runs::end(tx, key(1), START_MS + 60 * MINUTE_MS, EndCause::Stopped)?;
            runs::start(tx, key(2), START_MS + 61 * MINUTE_MS, StartCause::Restart)?;
            runs::end(tx, key(2), START_MS + 90 * MINUTE_MS, EndCause::Crashed)?;
            runs::start(tx, key(3), START_MS + 91 * MINUTE_MS, StartCause::Recovery)?;
            runs::end(tx, key(3), START_MS + 120 * MINUTE_MS, EndCause::Stopped)?;
            Ok(())
        })
        .await
        .unwrap();

        let events = db
            .read(|conn| run_events(conn, START_MS, START_MS + 180 * MINUTE_MS))
            .await
            .unwrap();

        let kinds: Vec<(i64, &EventDetail)> = events
            .iter()
            .map(|event| ((event.at * 1000 - START_MS) / MINUTE_MS, &event.detail))
            .collect();
        assert_eq!(
            kinds,
            [
                (0, &EventDetail::Start { build: None }),
                (61, &EventDetail::Restart { build: None }),
                (
                    90,
                    &EventDetail::Crash {
                        hang: false,
                        back_online_at: Some((START_MS + 91 * MINUTE_MS) / 1000),
                        players_dropped: Some(0),
                    }
                ),
                (120, &EventDetail::Stop),
            ]
        );
    }
}
