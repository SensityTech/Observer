//! What the Statistics page reads from players' drops: how long sessions last, the crash waves and
//! bursts the timeline marks, whether a signature was quiet before a change, and what each
//! recorded change did to stability.

use std::collections::{BTreeMap, HashMap, HashSet};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use observer_domain::changes::NumberChange;
use observer_domain::drops::stats::{self, Comparison};
use observer_domain::drops::{CrashForm, DropCategory, DropKind, DropsRange};
use observer_domain::statistics::{
    EARLY_MINUTES, EFFECT_DAYS, EFFECTS_DAYS, EffectKind, EffectRow, EventDetail, SURVIVAL_BEFORE_DAYS,
    SURVIVAL_MINUTES, SURVIVAL_STEP, SessionSurvival, StabilityEffects, Survival, TimelineEvent,
    WAVE_BASELINE_MS, WAVE_LEAST_SPAN_MS, bursts, is_wave, survival,
};
use observer_store::metrics::drops::{self as stored, SignatureRow};
use observer_store::{Connection, StoreError};

use super::{
    DAY_MS, DISPERSION_DAYS, Hours, Pick, Side, at, build_changed, compare, kept_since, noise, tally, trend,
};

/// Changes this close together are one row of the effects: a start's snapshot and its run.
const SAME_CHANGE_MS: i64 = 10 * 60_000;
/// Resources a row names, the others counted.
const RESOURCES_NAMED: usize = 3;
/// How far around a window crashes are read, so that a wave its edges cut is counted whole.
const WAVE_REACH_MS: i64 = 3_600_000;

/// `GET /drops/sessions`: the sessions that started in the range against those of the 4 weeks
/// before it, both cut at the oldest session kept.
pub(in crate::drops) fn sessions(
    conn: &Connection,
    range: DropsRange,
    now: Timestamp,
    kept_from_ms: i64,
) -> Result<SessionSurvival, StoreError> {
    let now_ms = now.as_millisecond();
    let from_ms = now_ms - range.days() * DAY_MS;
    let kept = kept_since(conn, kept_from_ms)?;
    let before_from = (from_ms - SURVIVAL_BEFORE_DAYS * DAY_MS).max(kept).min(from_ms);
    Ok(SessionSurvival {
        range,
        now: survival_of(conn, from_ms.max(kept), now_ms, now)?,
        before: survival_of(conn, before_from, from_ms, now)?,
        kept_from: at(kept, now),
    })
}

fn survival_of(conn: &Connection, from_ms: i64, to_ms: i64, now: Timestamp) -> Result<Survival, StoreError> {
    let now_ms = now.as_millisecond();
    let mut observed = Vec::new();
    let mut ended = 0;
    let (mut crashes, mut early_crashes) = (0u32, 0u32);
    let minutes = |ms: i64| {
        #[allow(clippy::cast_precision_loss)]
        let minutes = ms.max(0) as f64 / 60_000.0;
        minutes
    };
    for session in stored::started(conn, from_ms, to_ms)? {
        let length = session
            .length_ms
            .or(session.ended_ms.map(|ended| ended - session.joined_ms))
            .unwrap_or(now_ms - session.joined_ms);
        // The server's stop or crash, and a session Observer lost track of, end no stay: the
        // player would have stayed.
        let by_player = session.ended_ms.is_some()
            && !matches!(
                session.category,
                Some(DropCategory::ServerStop | DropCategory::ServerCrash) | None
            )
            && session.kind != Some(DropKind::Lost);
        if by_player {
            ended += 1;
        }
        if session.category == Some(DropCategory::Crash) {
            crashes += 1;
            if minutes(length) < EARLY_MINUTES {
                early_crashes += 1;
            }
        }
        observed.push((minutes(length), by_player));
    }
    let marks: Vec<f64> = (0..=SURVIVAL_MINUTES / SURVIVAL_STEP)
        .map(|mark| f64::from(mark * SURVIVAL_STEP))
        .collect();
    let sessions = u32::try_from(observed.len()).unwrap_or(u32::MAX);
    let (points, median) = survival(&mut observed, &marks);
    let early = (sessions > 0).then(|| {
        let (left, _) = survival(&mut observed, &[EARLY_MINUTES]);
        1.0 - left.first().copied().unwrap_or(1.0)
    });
    Ok(Survival {
        from: at(from_ms, now),
        to: at(to_ms, now),
        sessions,
        ended,
        points,
        median,
        early,
        early_crashes: (crashes > 0).then(|| f64::from(early_crashes) / f64::from(crashes)),
    })
}

/// A crash wave as the timeline and its links read it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Wave {
    pub event: TimelineEvent,
    pub signature: i64,
    pub start_ms: i64,
    /// A resource the crashes name: an asset's, or the blamed module's.
    pub names: Vec<String>,
}

/// The crash waves that reach into `from_ms` to `to_ms`, and the bursts of drops in it.
pub(in crate::drops) fn waves(
    conn: &Connection,
    from_ms: i64,
    to_ms: i64,
    now_ms: i64,
) -> Result<(Vec<Wave>, Vec<TimelineEvent>), StoreError> {
    // A wave the window's edges cut is counted whole: crashes are read from a little around it.
    let mut by_signature: BTreeMap<i64, Vec<stored::CrashRow>> = BTreeMap::new();
    for crash in stored::crashes(conn, from_ms - WAVE_REACH_MS, to_ms + WAVE_REACH_MS)? {
        by_signature.entry(crash.signature).or_default().push(crash);
    }
    let mut found = Vec::new();
    for (signature, crashes) in &by_signature {
        let times: Vec<i64> = crashes.iter().map(|crash| crash.ended_ms).collect();
        for burst in bursts(&times) {
            let wave = &crashes[burst];
            let (start_ms, end_ms) = (wave[0].ended_ms, wave[wave.len() - 1].ended_ms);
            if end_ms < from_ms || start_ms >= to_ms {
                continue;
            }
            // A player not yet confirmed counts once per session.
            let mut known = HashSet::new();
            let mut unconfirmed = 0;
            for crash in wave {
                match crash.player {
                    Some(player) => {
                        known.insert(player);
                    }
                    None => unconfirmed += 1,
                }
            }
            let players = known.len() + unconfirmed;
            if !is_wave(wave.len(), players, 0.0) {
                continue;
            }
            let expected = expected(conn, *signature, start_ms, end_ms, now_ms)?;
            if is_wave(wave.len(), players, expected) {
                found.push((*signature, wave[0].session, start_ms, end_ms, wave.len(), players));
            }
        }
    }
    let ids: Vec<i64> = found.iter().map(|wave| wave.0).collect();
    let rows: HashMap<i64, SignatureRow> = stored::signatures(conn, &ids)?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let players_ever = stored::players_by_subject(conn, DropCategory::Crash, None)?;
    let waves = found
        .into_iter()
        .map(|(signature, first, start_ms, end_ms, crashes, players)| {
            let row = rows.get(&signature);
            // As the drops page has it: a client's own text only once two players produced it.
            let shown = row.filter(|row| {
                CrashForm::parse(&row.form).is_some_and(|form| !form.free_text())
                    || players_ever.get(&signature).copied().unwrap_or(0) >= 2
            });
            let resource = row.and_then(|row| row.resource.clone());
            let names = resource
                .iter()
                .cloned()
                .chain(row.and_then(|row| row.module.clone()))
                .collect();
            Wave {
                event: TimelineEvent {
                    key: format!("wave:{signature}:{first}"),
                    at: start_ms.div_euclid(1000),
                    until: Some(end_ms.div_euclid(1000)),
                    detail: EventDetail::CrashWave {
                        signature_id: signature,
                        signature: shown.map(|row| row.signature.clone()),
                        label: shown.and_then(|row| row.label.clone()),
                        crashes: u32::try_from(crashes).unwrap_or(u32::MAX),
                        players: u32::try_from(players).unwrap_or(u32::MAX),
                        resource,
                    },
                },
                signature,
                start_ms,
                names,
            }
        })
        .collect();
    let incidents = stored::incidents(conn, from_ms, to_ms)?
        .into_iter()
        .map(|incident| TimelineEvent {
            key: format!("incident:{}", incident.id),
            at: incident.started_ms.div_euclid(1000),
            until: Some(incident.ended_ms.div_euclid(1000)),
            detail: EventDetail::Incident {
                drops: incident.drops,
                hitch_ms: incident.hitch_max_ms,
                players: incident.players,
            },
        })
        .collect();
    Ok((waves, incidents))
}

/// The crashes of `signature` the wave's span would have seen at its rate per hour played over
/// the weeks before it.
fn expected(
    conn: &Connection,
    signature: i64,
    start_ms: i64,
    end_ms: i64,
    now_ms: i64,
) -> Result<f64, StoreError> {
    let before = stored::window(conn, start_ms - WAVE_BASELINE_MS, start_ms, now_ms)?;
    let crashes: i64 = before
        .groups()
        .iter()
        .filter(|group| group.category == DropCategory::Crash && group.subject == signature)
        .map(|group| group.drops)
        .sum();
    if before.player_ms <= 0 {
        return Ok(0.0);
    }
    let during = stored::window(conn, start_ms, end_ms.max(start_ms + WAVE_LEAST_SPAN_MS), now_ms)?;
    #[allow(clippy::cast_precision_loss)]
    let expected = crashes as f64 * during.player_ms as f64 / before.player_ms as f64;
    Ok(expected)
}

/// Whether `signature` had no crash in the weeks before `at_ms`.
pub(in crate::drops) fn quiet_before(
    conn: &Connection,
    signature: i64,
    at_ms: i64,
    now_ms: i64,
) -> Result<bool, StoreError> {
    let before = stored::window(conn, at_ms - WAVE_BASELINE_MS, at_ms, now_ms)?;
    Ok(!before
        .groups()
        .iter()
        .any(|group| group.category == DropCategory::Crash && group.subject == signature && group.drops > 0))
}

/// A change the effects compare around.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EffectChange {
    pub at_ms: i64,
    pub kinds: Vec<EffectKind>,
    pub build: Option<NumberChange>,
    pub resources: Vec<String>,
}

/// `GET /drops/effects`: each change of the last 90 days, its builds read from the runs and the
/// rest from `recorded`, the unexpected drop rate on the 7 days before against the 7 after, both
/// cut at the neighbouring changes and tested together.
pub(in crate::drops) fn effects(
    conn: &Connection,
    now: Timestamp,
    zone: &TimeZone,
    recorded: Vec<EffectChange>,
    with_changes: bool,
) -> Result<StabilityEffects, StoreError> {
    let now_ms = now.as_millisecond();
    let from_ms = now_ms - EFFECTS_DAYS * DAY_MS;
    let side_ms = EFFECT_DAYS * DAY_MS;
    let runs = stored::runs(conn, from_ms - side_ms, now_ms)?;
    let mut changes = recorded;
    for pair in runs.windows(2) {
        let (previous, run) = (&pair[0], &pair[1]);
        let builds_known = changes.iter().any(|change| {
            change.kinds.contains(&EffectKind::Build)
                && (change.at_ms - run.started_ms).abs() <= SAME_CHANGE_MS
        });
        if build_changed(previous.build, run.build) && !builds_known {
            changes.push(EffectChange {
                at_ms: run.started_ms,
                kinds: vec![EffectKind::Build],
                build: Some(NumberChange {
                    from: previous.build,
                    to: run.build,
                }),
                resources: Vec::new(),
            });
        }
    }
    changes.sort_by_key(|change| change.at_ms);
    let hours = Hours::read(conn, from_ms - side_ms - (DISPERSION_DAYS + 1) * DAY_MS, now_ms)?;
    let keep = |group: &stored::Group| Pick::Unexpected.takes(group);
    let mut measured: Vec<(usize, Comparison, Side, Side, i64, i64)> = Vec::new();
    for (index, change) in changes.iter().enumerate() {
        if change.at_ms < from_ms {
            continue;
        }
        let since = index.checked_sub(1).map_or(change.at_ms - side_ms, |previous| {
            changes[previous].at_ms.max(change.at_ms - side_ms)
        });
        let until = changes
            .get(index + 1)
            .map_or(change.at_ms + side_ms, |next| {
                next.at_ms.min(change.at_ms + side_ms)
            })
            .min(now_ms);
        let before = stored::window(conn, since, change.at_ms, now_ms)?;
        let after = stored::window(conn, change.at_ms, until, now_ms)?;
        let (before, after) = (
            Side::of(tally(&before, keep), before.player_ms),
            Side::of(tally(&after, keep), after.player_ms),
        );
        let comparison = compare(after, before, noise(&hours, zone, since, keep));
        measured.push((index, comparison, after, before, since, until));
    }
    let comparisons: Vec<Comparison> = measured.iter().map(|(_, comparison, ..)| *comparison).collect();
    let verdicts = stats::list_verdicts(&comparisons);
    let mut rows: Vec<EffectRow> = measured
        .into_iter()
        .zip(verdicts)
        .map(|((index, comparison, after, before, since, until), verdict)| {
            let change = &changes[index];
            EffectRow {
                at: at(change.at_ms, now),
                kinds: change.kinds.clone(),
                build: change.build,
                resources: change.resources.iter().take(RESOURCES_NAMED).cloned().collect(),
                more_resources: u32::try_from(change.resources.len().saturating_sub(RESOURCES_NAMED))
                    .unwrap_or(u32::MAX),
                trend: trend(&comparison, verdict, after, before),
                before_from: at(since, now),
                after_to: at(until, now),
                open: until >= now_ms && until < change.at_ms + side_ms,
            }
        })
        .collect();
    rows.reverse();
    Ok(StabilityEffects {
        from: at(from_ms, now),
        to: now,
        rows,
        changes: with_changes,
    })
}
