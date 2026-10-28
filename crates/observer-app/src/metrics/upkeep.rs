//! Every minute, ten seconds past: the closed minutes and hours get their aggregates, gaps
//! included; every hour the retention runs, and every day SQLite's optimizer.

use std::time::Duration;

use observer_store::metrics::rollups::{self, Tier};
use observer_store::{MetricsDatabase, StoreError};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::Clock;

const PAST_THE_MINUTE_MS: i64 = 10_000;
/// Aggregates computed per transaction, so that a long catch-up never holds the writer for long.
const BUCKETS_PER_WRITE: usize = 240;

pub(super) async fn run(db: MetricsDatabase, clock: Clock, stop: CancellationToken) {
    let mut expired_hour = None;
    let mut optimized_day = None;
    loop {
        let now = clock.now().as_second();
        if let Err(error) = roll_up(&db, now).await {
            warn!(%error, "the statistics could not be aggregated");
        }
        let hour = now.div_euclid(3600);
        if expired_hour != Some(hour) {
            expired_hour = Some(hour);
            match expire(&db, now).await {
                Ok(0) => {}
                Ok(rows) => info!(rows, "statistics past their retention deleted"),
                Err(error) => warn!(%error, "the statistics retention could not be applied"),
            }
        }
        let day = now.div_euclid(86_400);
        if optimized_day != Some(day) {
            optimized_day = Some(day);
            if let Err(error) = db.read(rollups::optimize).await {
                warn!(%error, "the metrics database could not be optimized");
            }
        }

        let now_ms = clock.now().as_millisecond();
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

/// The closed buckets that raw samples cover and that have no aggregate, and the newest closed
/// ones again, which a sample written late may have changed.
pub(super) async fn roll_up(db: &MetricsDatabase, now: i64) -> Result<(), StoreError> {
    for tier in [Tier::Minute, Tier::Hour] {
        let size = tier.seconds();
        let closed = now - now.rem_euclid(size);
        let mut buckets = db.read(move |conn| rollups::missing(conn, tier, closed)).await?;
        let again: &[i64] = match tier {
            Tier::Minute => &[closed - 2 * size, closed - size],
            Tier::Hour => &[closed - size],
        };
        for bucket in again {
            if !buckets.contains(bucket) {
                buckets.push(*bucket);
            }
        }
        for chunk in buckets.chunks(BUCKETS_PER_WRITE) {
            let chunk = chunk.to_vec();
            db.write(move |tx| rollups::compute(tx, tier, &chunk)).await?;
        }
    }
    Ok(())
}

async fn expire(db: &MetricsDatabase, now: i64) -> Result<usize, StoreError> {
    let rows = db.write(move |tx| rollups::expire(tx, now)).await?;
    db.read(rollups::vacuum).await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use observer_domain::server::ServerState;
    use observer_store::metrics::samples::{self, Sample};
    use observer_store::metrics::{Column, Values};

    use super::*;

    const T: i64 = 1_761_642_000;

    async fn store(db: &MetricsDatabase, ts: &[i64]) {
        let ts = ts.to_vec();
        db.write(move |tx| -> Result<(), StoreError> {
            for ts in ts {
                let mut values = Values::default();
                values.set(Column::Players, Some(1.0));
                samples::insert(
                    tx,
                    &Sample {
                        ts,
                        state: ServerState::Online,
                        values,
                    },
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn minutes(db: &MetricsDatabase) -> Vec<(i64, i64)> {
        db.read(|conn| -> Result<_, StoreError> {
            let mut statement = conn.prepare("SELECT ts, samples FROM sample_1m ORDER BY ts")?;
            Ok(statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_gap_is_caught_up_and_the_newest_closed_minutes_are_computed_again() {
        let dir = tempfile::tempdir().unwrap();
        let db = MetricsDatabase::open(dir.path().join("metrics.sqlite3"))
            .await
            .unwrap();
        // Samples three hours ago, before Observer stopped, and in the last minutes.
        store(&db, &[T - 3 * 3600, T - 3 * 3600 + 5, T - 120, T - 60]).await;
        roll_up(&db, T + 10).await.unwrap();
        assert_eq!(minutes(&db).await, [(T - 3 * 3600, 2), (T - 120, 1), (T - 60, 1)]);

        store(&db, &[T - 55]).await;
        roll_up(&db, T + 70).await.unwrap();

        assert_eq!(minutes(&db).await[2], (T - 60, 2), "the late sample is in");
        let hours: i64 = db
            .read(|conn| -> Result<_, StoreError> {
                Ok(conn.query_row("SELECT count(*) FROM sample_1h", [], |row| row.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(hours, 2);
        db.close().await.unwrap();
    }
}
