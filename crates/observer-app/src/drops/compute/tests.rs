use observer_domain::changes::NumberChange;
use observer_domain::drops::stats;
use observer_domain::statistics::{EffectKind, EventDetail, TimelineEvent};
use observer_store::MetricsDatabase;
use observer_store::metrics::drops::{Grouping, NewSignature, Unclassified};
use observer_store::metrics::runs::{EndCause, RunKey, StartCause};
use observer_store::metrics::sessions::{self, Closure, NewSession};
use tempfile::TempDir;

use super::*;

/// Friday 2025-10-31 12:00 UTC.
const NOW_MS: i64 = 1_761_912_000_000;
const ADA: [u8; 16] = [1; 16];

struct Lab {
    _dir: TempDir,
    metrics: MetricsDatabase,
    next: i64,
}

impl Lab {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let metrics = MetricsDatabase::open(dir.path().join("metrics.sqlite3"))
            .await
            .unwrap();
        Self {
            _dir: dir,
            metrics,
            next: 1,
        }
    }

    async fn run(
        &self,
        generation: u32,
        started_ms: i64,
        ended: Option<(i64, EndCause)>,
        build: u32,
    ) -> RunKey {
        let run = RunKey {
            observer_started: NOW_MS - 400 * DAY_MS,
            generation,
        };
        self.metrics
            .write(move |tx| -> Result<(), StoreError> {
                runs::start(tx, run, started_ms, StartCause::Restart)?;
                runs::build(tx, run, build)?;
                if let Some((ended_ms, cause)) = ended {
                    runs::end(tx, run, ended_ms, cause)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        run
    }

    /// A session played from `seen_ms` for `length_ms`, ended in `category`.
    async fn play(
        &mut self,
        run: RunKey,
        seen_ms: i64,
        length_ms: i64,
        category: DropCategory,
        signature: Option<&'static str>,
        player: Option<[u8; 16]>,
    ) {
        let id = self.next;
        self.next += 1;
        self.metrics
            .write(move |tx| -> Result<(), StoreError> {
                sessions::insert(
                    tx,
                    &NewSession {
                        id,
                        run,
                        net_id: u32::try_from(id).unwrap(),
                        joined_ms: Some(seen_ms),
                        seen_ms,
                    },
                )?;
                sessions::close(
                    tx,
                    &Closure {
                        id,
                        ended_ms: seen_ms + length_ms,
                        length_ms: Some(length_ms),
                        category,
                        kind: (category == DropCategory::Kick).then_some(DropKind::Resource),
                        native: None,
                        resource: (category == DropCategory::Kick).then(|| "afk".to_owned()),
                        crash_text: signature.map(str::to_owned),
                        reason_text: None,
                        loading: None,
                        synthesized: false,
                    },
                )?;
                if let Some(player) = player {
                    tx.execute(
                        "UPDATE player_session SET player = ?1 WHERE id = ?2",
                        (player, id),
                    )?;
                }
                if let Some(signature) = signature {
                    let session = stored::unclassified(tx, 1000)?
                        .into_iter()
                        .find(|session| session.id == id)
                        .unwrap();
                    stored::classify(
                        tx,
                        &session,
                        Grouping::Signature(NewSignature {
                            signature,
                            form: "error",
                            module: None,
                            game_build: None,
                            label: None,
                            resource: None,
                            normaliser: 1,
                        }),
                        1,
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn read<T: Send + 'static>(
        &self,
        compute: impl FnOnce(&Connection) -> Result<T, StoreError> + Send + 'static,
    ) -> T {
        self.metrics.read(compute).await.unwrap()
    }
}

fn now() -> Timestamp {
    Timestamp::from_millisecond(NOW_MS).unwrap()
}

#[test]
fn a_day_compares_today_since_midnight_with_yesterdays_same_hours() {
    let paris = TimeZone::get("Europe/Paris").unwrap();

    let windows = Windows::of(DropsRange::Day, now(), &paris);

    // 13:00 in Paris: midnight there is 23:00 UTC the day before.
    assert_eq!(windows.now, (NOW_MS - 13 * HOUR_MS, NOW_MS));
    assert_eq!(windows.before, (NOW_MS - 37 * HOUR_MS, NOW_MS - DAY_MS));
    assert_eq!(windows.from_ms, NOW_MS - DAY_MS);
}

#[test]
fn a_week_compares_with_the_week_before() {
    let windows = Windows::of(DropsRange::Week, now(), &TimeZone::UTC);

    assert_eq!(windows.now, (NOW_MS - 7 * DAY_MS, NOW_MS));
    assert_eq!(windows.before, (NOW_MS - 14 * DAY_MS, NOW_MS - 7 * DAY_MS));
}

#[tokio::test]
async fn crashes_six_times_as_frequent_as_the_week_before_are_rising_and_one_player_is_named_as_their_source()
{
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 20 * DAY_MS, None, 3095).await;
    // The same play each week: 50 sessions of an hour; 30 crashes now, 5 before.
    for day in 0..14 {
        for slot in 0..5 {
            let seen = NOW_MS - (13 - day) * DAY_MS - 12 * HOUR_MS + slot * HOUR_MS;
            let recent = day >= 7;
            let crashing = if recent {
                slot < 3
            } else {
                slot == 0 && day % 2 == 0 && day < 10
            };
            let (category, signature) = if crashing {
                (DropCategory::Crash, Some("ERR_GFX_D3D_INIT"))
            } else {
                (DropCategory::Quit, None)
            };
            let player = (recent && crashing && slot == 0).then_some(ADA);
            lab.play(run, seen, HOUR_MS, category, signature, player).await;
        }
    }

    let summary = lab
        .read(|conn| summary(conn, DropsRange::Week, now(), &TimeZone::UTC, 0))
        .await
        .summary;

    let crashes = summary
        .tiles
        .iter()
        .find(|tile| tile.kind == TileKind::Crashes)
        .unwrap();
    assert_eq!(crashes.drops, 21);
    assert_eq!(crashes.trend.verdict, TrendVerdict::Rising);
    assert_eq!(crashes.spark.len(), 7 * 24);
    assert_eq!(crashes.rate, Some(60.0));
    // Ada made 7 of the 21 crashes: a third, more than 30 %.
    assert!(
        crashes
            .one_player_share
            .is_some_and(|share| (share - 1.0 / 3.0).abs() < 1e-9)
    );
    let share = summary
        .categories
        .iter()
        .find(|row| row.category == DropCategory::Crash)
        .unwrap()
        .share;
    assert!((share - 0.6).abs() < 1e-9);
}

#[tokio::test]
async fn a_chart_leaves_a_gap_where_nobody_played_and_draws_the_baseline_from_the_weeks_before() {
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 40 * DAY_MS, None, 3095).await;
    let hour = NOW_MS - 3 * HOUR_MS;
    let hour = hour - hour.rem_euclid(HOUR_MS);
    for week in 0..=4 {
        lab.play(
            run,
            hour - week * 7 * DAY_MS,
            HOUR_MS / 2,
            DropCategory::Crash,
            None,
            None,
        )
        .await;
    }

    let series = lab
        .read(|conn| {
            series(
                conn,
                DropsRange::Week,
                ChartCategory::Crash,
                now(),
                &TimeZone::UTC,
            )
        })
        .await;

    let index = usize::try_from((hour / 1000 - series.from) / 3600).unwrap();
    assert_eq!(series.rates.len(), 7 * 24);
    // One crash in half an hour of play: 200 per 100 player-hours.
    assert_eq!(series.rates[index], Some(200.0));
    assert_eq!(series.baseline[index], Some(200.0));
    assert_eq!(series.drops[index], 1);
    assert_eq!(series.rates[index - 1], None);
}

#[tokio::test]
async fn a_build_change_shows_the_rates_before_and_after_and_a_server_crash_its_players() {
    let mut lab = Lab::new().await;
    let old = lab
        .run(
            1,
            NOW_MS - 5 * DAY_MS,
            Some((NOW_MS - 2 * DAY_MS, EndCause::Stopped)),
            3095,
        )
        .await;
    let new = lab
        .run(
            2,
            NOW_MS - 2 * DAY_MS,
            Some((NOW_MS - DAY_MS, EndCause::Crashed)),
            3258,
        )
        .await;
    lab.play(old, NOW_MS - 3 * DAY_MS, HOUR_MS, DropCategory::Crash, None, None)
        .await;
    // Two players crash with the new signature: only then is it shown in the timeline.
    for player in [[1; 16], [2; 16]] {
        lab.play(
            new,
            NOW_MS - 2 * DAY_MS + HOUR_MS,
            HOUR_MS,
            DropCategory::Crash,
            Some("ERR_NEW"),
            Some(player),
        )
        .await;
    }
    lab.play(
        new,
        NOW_MS - DAY_MS - HOUR_MS,
        HOUR_MS,
        DropCategory::ServerCrash,
        None,
        None,
    )
    .await;

    let changes = lab
        .read(|conn| changes(conn, DropsRange::Week, now(), &TimeZone::UTC))
        .await;

    let build = changes
        .events
        .iter()
        .find(|event| event.kind == ChangeKind::Build)
        .unwrap();
    assert_eq!((build.build_from, build.build_to), (Some(3095), Some(3258)));
    assert_eq!(build.new_signatures, ["ERR_NEW"]);
    let crash = &build.before_after[0];
    assert_eq!(
        (crash.category, crash.trend.events_before, crash.trend.events_now),
        (DropCategory::Crash, 1, 2)
    );
    let server = changes
        .events
        .iter()
        .find(|event| event.kind == ChangeKind::ServerCrash)
        .unwrap();
    assert_eq!(server.players_dropped, Some(1));
    assert!(!changes.snapshots);
    assert!(changes.events.windows(2).all(|pair| pair[0].at >= pair[1].at));
}

#[tokio::test]
async fn drops_late_in_runs_raise_the_last_uptime_bin() {
    let mut lab = Lab::new().await;
    for generation in 1..=6 {
        let started = NOW_MS - 7 * DAY_MS + i64::from(generation) * DAY_MS;
        let run = lab.run(generation, started, None, 3095).await;
        for slot in 0..10 {
            lab.play(
                run,
                started + slot * HOUR_MS,
                HOUR_MS,
                DropCategory::Quit,
                None,
                None,
            )
            .await;
        }
        for slot in 12..20 {
            lab.play(
                run,
                started + slot * HOUR_MS,
                HOUR_MS / 2,
                DropCategory::Timeout,
                None,
                None,
            )
            .await;
        }
    }

    let uptime = lab
        .read(|conn| uptime(conn, DropsRange::Week, now(), &TimeZone::UTC, 0))
        .await;

    let last = uptime.bins.last().unwrap();
    assert_eq!(last.drops, 48);
    assert_eq!(last.verdict, UptimeVerdict::Higher);
    assert_eq!(uptime.bins[0].drops, 0);
}

#[tokio::test]
async fn the_rates_export_has_one_row_per_day_with_every_category() {
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 5 * DAY_MS, None, 3095).await;
    lab.play(run, NOW_MS - 2 * DAY_MS, HOUR_MS, DropCategory::Kick, None, None)
        .await;

    let days = lab
        .read(|conn| {
            day_rates(
                conn,
                NOW_MS - 3 * DAY_MS - 12 * HOUR_MS,
                NOW_MS,
                NOW_MS,
                &TimeZone::UTC,
            )
        })
        .await;

    assert_eq!(days.len(), 4);
    let kicked = days.iter().find(|day| day.played_ms > 0).unwrap();
    assert_eq!(kicked.day, "2025-10-29");
    assert!(kicked.drops.contains(&(DropCategory::Kick, 1)));
    assert_eq!(kicked.drops.len(), DropCategory::ALL.len());
}

#[test]
fn a_median_needs_two_values() {
    assert_eq!(median(&mut [3.0]), None);
    assert_eq!(median(&mut [3.0, 1.0]), Some(2.0));
    assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
    assert!(stats::per_hundred_hours(1, 0).is_none());
}

/// 90 days at about 300 players on average: 5,000 sessions a day, with crashes, timeouts and kicks.
/// `cargo test --release -p observer-app measure_a_season -- --ignored --nocapture`
#[tokio::test]
#[ignore = "a measurement, minutes long"]
#[expect(clippy::too_many_lines, reason = "a measurement reads best in one piece")]
async fn measure_a_season_of_drops() {
    let lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 91 * DAY_MS, None, 3095).await;
    let started = std::time::Instant::now();
    let codes = [
        "ERR_GFX_D3D_INIT",
        "ERR_MEM_EMBEDDEDALLOC_ALLOC",
        "ERR_SYS_PURE_CALL",
    ];
    for day in 0..90i64 {
        lab.metrics
            .write(move |tx| -> Result<(), StoreError> {
                for slot in 0..5000i64 {
                    let id = day * 5000 + slot + 1;
                    let seen = NOW_MS - (90 - day) * DAY_MS + slot * 17_280;
                    let category = match slot % 20 {
                        0 => DropCategory::Crash,
                        1 | 2 => DropCategory::Timeout,
                        3 => DropCategory::Kick,
                        _ => DropCategory::Quit,
                    };
                    sessions::insert(
                        tx,
                        &NewSession {
                            id,
                            run,
                            net_id: u32::try_from(id).unwrap(),
                            joined_ms: Some(seen),
                            seen_ms: seen,
                        },
                    )?;
                    sessions::close(
                        tx,
                        &Closure {
                            id,
                            ended_ms: seen + 90 * 60_000,
                            length_ms: Some(90 * 60_000),
                            category,
                            kind: (category == DropCategory::Kick).then_some(DropKind::Resource),
                            native: None,
                            resource: (category == DropCategory::Kick).then(|| format!("kicker{}", slot % 7)),
                            crash_text: None,
                            reason_text: None,
                            loading: Some(slot % 3 == 0),
                            synthesized: false,
                        },
                    )?;
                    if category == DropCategory::Crash {
                        let signature = codes[usize::try_from(slot % 3).unwrap()];
                        let session = Unclassified {
                            id,
                            category,
                            crash_text: None,
                            reason_text: None,
                            player: Some([u8::try_from(slot % 200).unwrap(); 16]),
                            ended_ms: seen + 90 * 60_000,
                        };
                        stored::classify(
                            tx,
                            &session,
                            Grouping::Signature(NewSignature {
                                signature,
                                form: "error",
                                module: None,
                                game_build: None,
                                label: None,
                                resource: None,
                                normaliser: 1,
                            }),
                            1,
                        )?;
                    }
                }
                Ok(())
            })
            .await
            .unwrap();
    }
    println!("450,000 sessions written in {:?}", started.elapsed());
    let time = |what: &str, started: std::time::Instant| println!("{what}: {:?}", started.elapsed());
    for pass in ["before any rollup", "after the rollups"] {
        for range in [DropsRange::Day, DropsRange::Week, DropsRange::Quarter] {
            let at = std::time::Instant::now();
            lab.read(move |conn| summary(conn, range, now(), &TimeZone::UTC, 0))
                .await;
            time(&format!("{pass}: summary {}", range.name()), at);
            let at = std::time::Instant::now();
            lab.read(move |conn| series(conn, range, ChartCategory::Unexpected, now(), &TimeZone::UTC))
                .await;
            time(&format!("{pass}: series {}", range.name()), at);
            let at = std::time::Instant::now();
            lab.read(move |conn| signatures(conn, range, now(), &TimeZone::UTC, 0))
                .await;
            time(&format!("{pass}: signatures {}", range.name()), at);
            let at = std::time::Instant::now();
            lab.read(move |conn| uptime(conn, range, now(), &TimeZone::UTC, 0))
                .await;
            time(&format!("{pass}: uptime {}", range.name()), at);
        }
        if pass == "before any rollup" {
            let at = std::time::Instant::now();
            let hours = lab.read(|conn| stored::pending_hours(conn, NOW_MS)).await;
            for chunk in hours.chunks(24) {
                let chunk = chunk.to_vec();
                lab.metrics
                    .write(move |tx| -> Result<(), StoreError> {
                        for hour in chunk {
                            stored::find_incidents(tx, hour * 1000, hour * 1000 + HOUR_MS)?;
                            stored::roll_up(tx, hour, NOW_MS)?;
                        }
                        Ok(())
                    })
                    .await
                    .unwrap();
            }
            time(&format!("{} hours rolled up", hours.len()), at);
        }
    }
}

#[tokio::test]
async fn the_servers_own_stops_and_crashes_get_no_player_rate_and_closures_without_report_no_unknown() {
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 3 * DAY_MS, None, 3095).await;
    for slot in 0..30 {
        lab.play(
            run,
            NOW_MS - DAY_MS + slot * 60_000,
            HOUR_MS,
            DropCategory::ServerCrash,
            None,
            None,
        )
        .await;
    }
    lab.metrics
        .write(|tx| -> Result<usize, StoreError> {
            Ok(tx.execute(
                "UPDATE player_session SET category = 'unknown', kind = 'lost' WHERE id <= 10",
                [],
            )?)
        })
        .await
        .unwrap();

    let summary = lab
        .read(|conn| summary(conn, DropsRange::Week, now(), &TimeZone::UTC, 0))
        .await
        .summary;

    let row = |category| {
        summary
            .categories
            .iter()
            .find(|row| row.category == category)
            .unwrap()
    };
    assert_eq!(row(DropCategory::ServerCrash).drops, 20);
    assert_eq!(
        (
            row(DropCategory::ServerCrash).rate,
            row(DropCategory::ServerCrash).trend
        ),
        (None, None)
    );
    assert_eq!(row(DropCategory::Unknown).drops, 0);
    assert_eq!(summary.coverage.lost, 10);
}

const GRACE: [u8; 16] = [2; 16];
const MINUTE_MS: i64 = 60_000;

fn wave_crashes(waves: &[insights::Wave]) -> Vec<u32> {
    waves
        .iter()
        .map(|wave| match wave.event.detail {
            EventDetail::CrashWave { crashes, .. } => crashes,
            _ => 0,
        })
        .collect()
}

#[tokio::test]
async fn crashes_of_one_signature_from_three_players_close_together_are_a_wave() {
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 40 * DAY_MS, None, 3095).await;
    let start = NOW_MS - 2 * HOUR_MS;
    // Five crashes in 24 minutes from Ada, Grace and a player not confirmed yet.
    for (minute, player) in [
        (0, Some(ADA)),
        (6, Some(GRACE)),
        (12, None),
        (18, Some(ADA)),
        (24, Some(GRACE)),
    ] {
        let ended = start + minute * MINUTE_MS;
        lab.play(
            run,
            ended - 30 * MINUTE_MS,
            30 * MINUTE_MS,
            DropCategory::Crash,
            Some("ERR_NEW"),
            player,
        )
        .await;
    }
    // Five crashes of another signature from one player are that player's, not the server's.
    for minute in 0..5 {
        let ended = start + minute * MINUTE_MS;
        lab.play(
            run,
            ended - 30 * MINUTE_MS,
            30 * MINUTE_MS,
            DropCategory::Crash,
            Some("ERR_ONE"),
            Some(ADA),
        )
        .await;
    }

    let (waves, incidents) = lab
        .read(|conn| insights::waves(conn, NOW_MS - DAY_MS, NOW_MS, NOW_MS))
        .await;

    assert_eq!(wave_crashes(&waves), [5]);
    let wave = &waves[0];
    assert_eq!(wave.start_ms, start);
    assert_eq!(wave.event.until, Some((start + 24 * MINUTE_MS) / 1000));
    let EventDetail::CrashWave { players, .. } = wave.event.detail else {
        unreachable!("a wave's detail")
    };
    assert_eq!(players, 3, "Ada and Grace once each, the unconfirmed player once");
    assert_eq!(incidents, Vec::<TimelineEvent>::new());
    let late = lab
        .read(|conn| insights::waves(conn, NOW_MS - HOUR_MS, NOW_MS, NOW_MS))
        .await;
    assert!(late.0.is_empty(), "the wave ended before the window");
    let cut = lab
        .read(move |conn| insights::waves(conn, start + 10 * MINUTE_MS, NOW_MS, NOW_MS))
        .await;
    assert_eq!(
        wave_crashes(&cut.0),
        [5],
        "a wave the window cuts is counted whole"
    );
}

#[tokio::test]
async fn crashes_as_frequent_as_in_the_weeks_before_are_no_wave() {
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 40 * DAY_MS, None, 3095).await;
    // Every day of the four weeks before, the same five crashes in an hour of play.
    for day in 0..=28 {
        let start = NOW_MS - 2 * HOUR_MS - day * DAY_MS;
        for (minute, player) in [
            (0, Some(ADA)),
            (6, Some(GRACE)),
            (12, None),
            (18, Some(ADA)),
            (24, None),
        ] {
            let ended = start + minute * MINUTE_MS;
            lab.play(
                run,
                ended - 10 * MINUTE_MS,
                10 * MINUTE_MS,
                DropCategory::Crash,
                Some("ERR_DAILY"),
                player,
            )
            .await;
        }
    }

    let (waves, _) = lab
        .read(|conn| insights::waves(conn, NOW_MS - DAY_MS, NOW_MS, NOW_MS))
        .await;

    assert!(waves.is_empty(), "{waves:?}");
}

#[tokio::test]
async fn a_signature_is_quiet_before_a_change_only_without_a_crash_in_the_four_weeks_before() {
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 60 * DAY_MS, None, 3095).await;
    let change = NOW_MS - HOUR_MS;
    lab.play(
        run,
        change - 27 * DAY_MS,
        HOUR_MS,
        DropCategory::Crash,
        Some("ERR_SEEN"),
        Some(ADA),
    )
    .await;
    lab.play(
        run,
        change - 30 * DAY_MS,
        HOUR_MS,
        DropCategory::Crash,
        Some("ERR_OLD"),
        Some(ADA),
    )
    .await;
    let (seen, old) = lab
        .read(|conn| {
            let ids: Vec<i64> = conn
                .prepare("SELECT id FROM crash_signature ORDER BY signature DESC")?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            Ok((ids[0], ids[1]))
        })
        .await;

    let quiet = |signature| lab.read(move |conn| insights::quiet_before(conn, signature, change, NOW_MS));

    assert!(!quiet(seen).await, "a crash 27 days before");
    assert!(quiet(old).await, "the last crash 30 days before");
}

#[tokio::test]
async fn how_long_players_stay_counts_their_own_ends_and_keeps_the_servers_out() {
    let mut lab = Lab::new().await;
    let run = lab.run(1, NOW_MS - 60 * DAY_MS, None, 3095).await;
    let week = NOW_MS - 3 * DAY_MS;
    lab.play(
        run,
        week,
        5 * MINUTE_MS,
        DropCategory::Crash,
        Some("ERR_EARLY"),
        Some(ADA),
    )
    .await;
    lab.play(
        run,
        week + HOUR_MS,
        20 * MINUTE_MS,
        DropCategory::Quit,
        None,
        None,
    )
    .await;
    lab.play(
        run,
        week + 2 * HOUR_MS,
        30 * MINUTE_MS,
        DropCategory::ServerStop,
        None,
        None,
    )
    .await;
    lab.play(
        run,
        week + 3 * HOUR_MS,
        60 * MINUTE_MS,
        DropCategory::Quit,
        None,
        None,
    )
    .await;
    // The weeks before: one session of two hours.
    lab.play(
        run,
        NOW_MS - 20 * DAY_MS,
        120 * MINUTE_MS,
        DropCategory::Quit,
        None,
        None,
    )
    .await;

    let survival = lab
        .read(|conn| insights::sessions(conn, DropsRange::Week, now(), NOW_MS - 90 * DAY_MS))
        .await;

    let now_side = &survival.now;
    assert_eq!((now_side.sessions, now_side.ended), (4, 3));
    assert_eq!(now_side.points.len(), 121);
    // After 5 minutes 1 of 4 left; after 20, 1 of the 3 then at risk; the server's stop at 30
    // takes nobody's share; after 60, the last.
    assert!((now_side.points[3] - 0.75).abs() < 1e-9, "{:?}", now_side.points);
    assert!((now_side.points[10] - 0.5).abs() < 1e-9);
    assert_eq!(now_side.points[30], 0.0);
    assert_eq!(now_side.median, Some(20.0));
    assert!((now_side.early.unwrap() - 0.25).abs() < 1e-9);
    assert_eq!(now_side.early_crashes, Some(1.0));
    assert_eq!(
        (survival.before.sessions, survival.before.median),
        (1, Some(120.0))
    );
}

#[tokio::test]
async fn a_change_is_compared_with_the_week_before_and_a_rise_is_told_from_noise() {
    let mut lab = Lab::new().await;
    let change = NOW_MS - 7 * DAY_MS;
    let before = lab
        .run(1, NOW_MS - 50 * DAY_MS, Some((change, EndCause::Stopped)), 3095)
        .await;
    let after = lab.run(2, change, None, 3258).await;
    // Ten hours of play a day; one crash a day before the change, eight a day after.
    for day in 0..42 {
        let at = NOW_MS - (42 - day) * DAY_MS + 8 * HOUR_MS;
        let run = if at < change { before } else { after };
        let crashes = if at < change { 1 } else { 8 };
        for slot in 0..10 {
            let (category, signature) = if slot < crashes {
                (DropCategory::Crash, Some("ERR_BUILD"))
            } else {
                (DropCategory::Quit, None)
            };
            lab.play(run, at + slot * HOUR_MS, HOUR_MS, category, signature, None)
                .await;
        }
    }

    let effects = lab
        .read(|conn| insights::effects(conn, now(), &TimeZone::UTC, Vec::new(), false))
        .await;

    assert!(!effects.changes);
    assert_eq!(effects.rows.len(), 1);
    let row = &effects.rows[0];
    assert_eq!(row.kinds, [EffectKind::Build]);
    assert_eq!(
        row.build,
        Some(NumberChange {
            from: Some(3095),
            to: Some(3258)
        })
    );
    assert_eq!(row.trend.verdict, TrendVerdict::Rising);
    assert!(row.trend.now.unwrap() > 6.0 * row.trend.before.unwrap());
    assert!(!row.open, "the week after has passed");
}
