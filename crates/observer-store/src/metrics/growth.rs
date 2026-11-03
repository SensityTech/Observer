//! How FXServer's main process grew over the open run, from the minute aggregates.

use observer_domain::metrics::{GROWTH_WARM_UP, Growth, HANDLES_LEAK, PRIVATE_LEAK, RunGrowth};
use rusqlite::{Connection, OptionalExtension, params};

use super::Column;
use crate::database::StoreError;

/// The open run of the Observer whose metrics started at `observer_started` (Unix milliseconds),
/// fitted from the end of its warm-up to the newest closed minute before `now` (Unix seconds).
/// `None` without an open run.
pub fn read(conn: &Connection, observer_started: i64, now: i64) -> Result<Option<RunGrowth>, StoreError> {
    // The runs of earlier Observers were closed when this one opened the database.
    let started_ms: Option<i64> = conn
        .query_row(
            "SELECT started_ms FROM server_run WHERE observer_started = ?1 AND ended_ms IS NULL \
             ORDER BY generation DESC LIMIT 1",
            [observer_started],
            |row| row.get(0),
        )
        .optional()?;
    let Some(started_ms) = started_ms else {
        return Ok(None);
    };
    let started_at = started_ms.div_euclid(1000);
    let warm = started_at + GROWTH_WARM_UP;
    let from = warm + (60 - warm.rem_euclid(60)) % 60;
    let private = minutes(conn, Column::FxsPrivate, from, now)?;
    let handles = minutes(conn, Column::FxsHandles, from, now)?;
    let to = private
        .iter()
        .chain(&handles)
        .map(|&(ts, _, _)| ts + 60)
        .max()
        .unwrap_or(from);
    // Each minute stands at its middle, weighted by the samples behind its average.
    let points = |rows: &[(i64, f64, u32)]| -> Vec<(f64, f64, f64)> {
        rows.iter()
            .map(|&(ts, average, samples)| (real(ts + 30 - from), average, f64::from(samples)))
            .collect()
    };
    Ok(Some(RunGrowth {
        started_at,
        from,
        to,
        private: Growth::of(&points(&private), to - from, PRIVATE_LEAK),
        handles: Growth::of(&points(&handles), to - from, HANDLES_LEAK),
    }))
}

/// A run's private bytes fitted from the end of its warm-up to `until` (its end, or now, in Unix
/// seconds), and the most a minute of it averaged. Both `None` once its minutes are gone.
pub fn of_run(
    conn: &Connection,
    started_ms: i64,
    until: i64,
) -> Result<(Option<Growth>, Option<f64>), StoreError> {
    let started_at = started_ms.div_euclid(1000);
    let warm = started_at + GROWTH_WARM_UP;
    let from = warm + (60 - warm.rem_euclid(60)) % 60;
    let whole = minutes(
        conn,
        Column::FxsPrivate,
        started_at - started_at.rem_euclid(60),
        until,
    )?;
    let peak = whole.iter().map(|&(_, average, _)| average).reduce(f64::max);
    let fitted: Vec<(f64, f64, f64)> = whole
        .iter()
        .filter(|&&(ts, _, _)| ts >= from)
        .map(|&(ts, average, samples)| (real(ts + 30 - from), average, f64::from(samples)))
        .collect();
    let to = whole.last().map_or(from, |&(ts, _, _)| ts + 60);
    Ok((Growth::of(&fitted, to - from, PRIVATE_LEAK), peak))
}

/// The minutes from `from` that measured `column` and closed by `now`.
fn minutes(
    conn: &Connection,
    column: Column,
    from: i64,
    now: i64,
) -> Result<Vec<(i64, f64, u32)>, StoreError> {
    let name = column.name();
    let sql = format!(
        "SELECT ts, {name}_avg, {name}_n FROM sample_1m WHERE ts >= ?1 AND ts + 60 <= ?2 AND {name}_n > 0 \
         ORDER BY ts"
    );
    let mut statement = conn.prepare_cached(&sql)?;
    let rows = statement
        .query_map(params![from, now], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                u32::try_from(row.get::<_, i64>(2)?).unwrap_or(u32::MAX),
            ))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Seconds and counts within a run stay far below 2⁵³, where a float stops being exact.
#[allow(clippy::cast_precision_loss)]
fn real(value: i64) -> f64 {
    value as f64
}

#[cfg(test)]
mod tests {
    use observer_domain::server::ServerState;
    use rusqlite::Transaction;

    use super::*;
    use crate::metrics::memory;
    use crate::metrics::rollups::{self, Tier};
    use crate::metrics::runs::{self, EndCause, RunKey, StartCause};
    use crate::metrics::samples::tests::{sample, store};

    /// 2025-10-28T09:00:00Z, on a minute.
    const T: i64 = 1_761_642_000;
    const OBSERVER: i64 = (T - 600) * 1000;

    fn start(tx: &Transaction<'_>, observer_started: i64, generation: u32, at: i64) {
        let run = RunKey {
            observer_started,
            generation,
        };
        runs::start(tx, run, at * 1000, StartCause::Requested).unwrap();
    }

    /// Samples every five seconds over `minutes` from `T`, private bytes and handles given per
    /// minute, then the minutes rolled up.
    fn minutes_of(conn: &mut Connection, minutes: i64, value: impl Fn(i64) -> (f64, f64)) {
        let samples: Vec<_> = (0..minutes * 12)
            .map(|slot| {
                let (private, handles) = value(slot / 12);
                sample(
                    T + slot * 5,
                    ServerState::Online,
                    &[(Column::FxsPrivate, private), (Column::FxsHandles, handles)],
                )
            })
            .collect();
        store(conn, &samples);
        let buckets: Vec<i64> = (0..minutes).map(|minute| T + minute * 60).collect();
        let tx = conn.transaction().unwrap();
        rollups::compute(&tx, Tier::Minute, &buckets).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn the_warm_up_is_left_out_and_the_minutes_after_it_are_fitted() {
        let mut conn = memory();
        // Loading for thirty minutes, then 1 GB growing 1 MB a minute and 2 handles a minute.
        minutes_of(&mut conn, 150, |minute| {
            if minute < 30 {
                (3e9, 5000.0)
            } else {
                (
                    1e9 + 1_048_576.0 * real(minute - 30),
                    400.0 + 2.0 * real(minute - 30),
                )
            }
        });
        let tx = conn.transaction().unwrap();
        start(&tx, OBSERVER, 1, T - 20);
        tx.commit().unwrap();

        let growth = read(&conn, OBSERVER, T + 150 * 60).unwrap().unwrap();

        assert_eq!(growth.started_at, T - 20);
        assert_eq!(
            growth.from,
            T + 30 * 60,
            "the warm-up's end, rounded up to the minute"
        );
        assert_eq!(growth.to, T + 150 * 60);
        let private = growth.private.unwrap();
        assert_eq!(private.points, 120);
        assert!((private.per_hour - 60.0 * 1_048_576.0).abs() < 1.0, "{private:?}");
        assert!(
            (private.start - (1e9 - 1_048_576.0 / 2.0)).abs() < 1.0,
            "{private:?}"
        );
        assert!((private.fit - 1.0).abs() < 1e-9);
        assert!(private.leak, "60 MB an hour over two hours");
        let handles = growth.handles.unwrap();
        assert!((handles.per_hour - 120.0).abs() < 1e-6);
        assert!(handles.leak);
    }

    #[test]
    fn each_minute_weighs_as_much_as_the_samples_behind_it() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        start(&tx, OBSERVER, 1, T - 1800);
        tx.commit().unwrap();
        minutes_of(&mut conn, 4, |minute| {
            (if minute == 3 { 2e9 } else { 1e9 }, 100.0)
        });
        let full = read(&conn, OBSERVER, T + 3600).unwrap().unwrap().private.unwrap();

        // The last minute down to one sample: its jump counts for less.
        conn.execute("DELETE FROM sample_5s WHERE ts > ?1", [T + 180])
            .unwrap();
        let tx = conn.transaction().unwrap();
        rollups::compute(&tx, Tier::Minute, &[T + 180]).unwrap();
        tx.commit().unwrap();
        let light = read(&conn, OBSERVER, T + 3600).unwrap().unwrap().private.unwrap();

        assert_eq!((full.points, light.points), (4, 4));
        assert!(light.per_hour < full.per_hour / 2.0, "{full:?} {light:?}");
    }

    #[test]
    fn only_the_open_run_of_this_observer_is_fitted() {
        let mut conn = memory();
        minutes_of(&mut conn, 60, |_| (1e9, 400.0));
        assert_eq!(read(&conn, OBSERVER, T + 3600).unwrap(), None, "no run");

        let tx = conn.transaction().unwrap();
        start(&tx, OBSERVER - 86_400_000, 4, T - 3600);
        let ended = RunKey {
            observer_started: OBSERVER,
            generation: 1,
        };
        start(&tx, OBSERVER, 1, T - 3600);
        runs::end(&tx, ended, (T - 60) * 1000, EndCause::Stopped).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            read(&conn, OBSERVER, T + 3600).unwrap(),
            None,
            "another Observer's run, and an ended one"
        );

        let tx = conn.transaction().unwrap();
        start(&tx, OBSERVER, 2, T - 1800);
        tx.commit().unwrap();
        let growth = read(&conn, OBSERVER, T + 3600).unwrap().unwrap();
        assert_eq!(growth.started_at, T - 1800);
        assert_eq!(growth.private.unwrap().per_hour, 0.0);
    }

    #[test]
    fn a_run_still_warming_up_has_no_growth_yet() {
        let mut conn = memory();
        minutes_of(&mut conn, 10, |_| (1e9, 400.0));
        let tx = conn.transaction().unwrap();
        start(&tx, OBSERVER, 1, T);
        tx.commit().unwrap();

        let growth = read(&conn, OBSERVER, T + 600).unwrap().unwrap();

        assert_eq!((growth.from, growth.to), (T + 1800, T + 1800));
        assert_eq!((growth.private, growth.handles), (None, None));
    }
}
