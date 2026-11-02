//! What the Statistics page reads across services: the timeline's events, with the links between
//! a crash wave and the change just before it, and what each recorded change did to stability.
//! Each part stays behind its own permission and its own feature; an answer says what it left out.

use std::collections::HashSet;

use jiff::Timestamp;
use observer_domain::changes::{EnvironmentFact, LiveChangeKind};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::features::Feature;
use observer_domain::permissions::Permissions;
use observer_domain::statistics::{
    EFFECT_DAYS, EFFECTS_DAYS, EffectKind, EventDetail, EventLink, EventSource, LINK_WINDOW_MS,
    LONGEST_WINDOW, LinkDismissal, StabilityEffects, TimelineEvent, TimelineEvents, link_key, linked,
};

use crate::changes::Recorded;
use crate::drops::EffectChange;
use crate::{Caller, Changes, Clock, Drops, Error, Features, Metrics};

/// A change event names at most this many resources.
const RESOURCES_NAMED: usize = 3;
/// The longest key a link can have: two event keys.
const LONGEST_LINK: usize = 256;

/// The statistics across services. Clones share them.
#[derive(Clone)]
pub struct Statistics {
    features: Features,
    /// `None` for each feature Observer started without.
    metrics: Option<Metrics>,
    drops: Option<Drops>,
    changes: Option<Changes>,
    clock: Clock,
}

/// A change the links can name: its event's key, when, and the resources it touched.
struct LinkedChange {
    key: String,
    at_ms: i64,
    resources: Vec<String>,
}

impl Statistics {
    pub fn new(
        features: Features,
        metrics: Option<Metrics>,
        drops: Option<Drops>,
        changes: Option<Changes>,
        clock: Clock,
    ) -> Self {
        Self {
            features,
            metrics,
            drops,
            changes,
            clock,
        }
    }

    /// The service of `feature` while it runs: one switched off since the start is still there.
    fn running<'a, T>(&self, feature: Feature, service: Option<&'a T>) -> Option<&'a T> {
        service.filter(|_| self.features.runs(feature))
    }

    fn drops(&self) -> Result<&Drops, Error> {
        self.running(Feature::Drops, self.drops.as_ref())
            .ok_or_else(|| self.features.refusal(Feature::Drops))
    }

    /// What happened from `from` to `to` (Unix seconds): the runs for every reader, what changed
    /// for `CHANGES_VIEW`, crash waves and bursts for `DROPS_VIEW`, and the links for both. What
    /// changed and the drops are left out too while their feature does not run.
    pub async fn events(&self, caller: &Caller, from: i64, to: i64) -> Result<TimelineEvents, Error> {
        if to <= from || to - from > LONGEST_WINDOW {
            return Err(ApiError::new(ErrorCode::InvalidRequest)
                .with("from", from)
                .with("to", to)
                .into());
        }
        let (from_ms, to_ms) = (from * 1000, to * 1000);
        let metrics = self
            .running(Feature::Metrics, self.metrics.as_ref())
            .ok_or_else(|| self.features.refusal(Feature::Metrics))?;
        let mut events = metrics.run_events(from_ms, to_ms).await?;
        let mut omitted = Vec::new();

        let mut changes = Vec::new();
        let recorder = self.running(Feature::Changes, self.changes.as_ref());
        if let Some(recorder) = recorder.filter(|_| caller.permissions.contains(Permissions::CHANGES_VIEW)) {
            // From a little earlier too: a wave at the start of the window may follow one.
            let recorded = recorder
                .recorded(instant(from_ms - LINK_WINDOW_MS), instant(to_ms))
                .await?;
            for (event, change) in change_events(recorded) {
                if (from_ms..to_ms).contains(&(event.at * 1000)) {
                    events.push(event);
                }
                changes.extend(change);
            }
            changes.sort_by_key(|change| change.at_ms);
        } else {
            omitted.push(EventSource::Changes);
        }

        let mut links = Vec::new();
        let drops = self.running(Feature::Drops, self.drops.as_ref());
        if let Some(drops) = drops.filter(|_| caller.permissions.contains(Permissions::DROPS_VIEW)) {
            let found = drops.waves(from_ms, to_ms).await?;
            let (waves, incidents) = &*found;
            events.extend(incidents.iter().cloned());
            events.extend(waves.iter().map(|wave| wave.event.clone()));
            let dismissed: HashSet<String> = if changes.is_empty() {
                HashSet::new()
            } else {
                drops.dismissed_links().await?
            };
            let times: Vec<i64> = changes.iter().map(|change| change.at_ms).collect();
            for wave in waves {
                let Some(change) = linked(&times, wave.start_ms).map(|index| &changes[index]) else {
                    continue;
                };
                if !drops.quiet_before(wave.signature, change.at_ms).await? {
                    continue;
                }
                let key = link_key(&change.key, &wave.event.key);
                links.push(EventLink {
                    dismissed: dismissed.contains(&key),
                    key,
                    change: change.key.clone(),
                    wave: wave.event.key.clone(),
                    after: (wave.start_ms - change.at_ms).div_euclid(1000),
                    names: wave
                        .names
                        .iter()
                        .find(|name| change.resources.contains(name))
                        .cloned(),
                });
            }
        } else {
            omitted.push(EventSource::Drops);
        }
        events.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.key.cmp(&b.key)));
        Ok(TimelineEvents {
            events,
            links,
            omitted,
        })
    }

    /// What each change of the last 90 days did to stability: builds for every drops viewer, the
    /// cfg, launch settings and resources too for `CHANGES_VIEW` while what changed is recorded.
    pub async fn effects(&self, caller: &Caller) -> Result<StabilityEffects, Error> {
        let drops = self.drops()?;
        let recorder = self
            .running(Feature::Changes, self.changes.as_ref())
            .filter(|_| caller.permissions.contains(Permissions::CHANGES_VIEW));
        let with_changes = recorder.is_some();
        let mut recorded = Vec::new();
        if let Some(recorder) = recorder {
            let now_ms = self.clock.now().as_millisecond();
            let from_ms = now_ms - (EFFECTS_DAYS + EFFECT_DAYS) * 86_400_000;
            for start in recorder.recorded(instant(from_ms), instant(now_ms)).await?.starts {
                let mut kinds: Vec<EffectKind> = start
                    .counts
                    .environment
                    .iter()
                    .map(|fact| match fact {
                        EnvironmentFact::Build => EffectKind::Build,
                        EnvironmentFact::GameBuild => EffectKind::GameBuild,
                        EnvironmentFact::Launch => EffectKind::Launch,
                        EnvironmentFact::Cfg => EffectKind::Cfg,
                    })
                    .collect();
                if !start.resources.is_empty() {
                    kinds.push(EffectKind::Resources);
                }
                recorded.push(EffectChange {
                    at_ms: start.at.as_millisecond(),
                    kinds,
                    build: start.build,
                    resources: start.resources,
                });
            }
        }
        drops.effects(recorded, with_changes).await
    }

    /// Says a link is not related, or takes that back. The route asks for `DROPS_VIEW`; the link
    /// also names a change, which is `CHANGES_VIEW`'s. The drops keep the links, so they must run.
    pub async fn dismiss(&self, caller: &Caller, dismissal: LinkDismissal) -> Result<(), Error> {
        if !caller.permissions.contains(Permissions::CHANGES_VIEW) {
            let key = Permissions::CHANGES_VIEW
                .iter()
                .next()
                .map_or("", |info| info.key);
            return Err(ApiError::new(ErrorCode::Forbidden).with("permission", key).into());
        }
        let link = dismissal.link.trim();
        if link.is_empty() || link.len() > LONGEST_LINK || !link.contains('>') {
            return Err(ApiError::new(ErrorCode::InvalidRequest).with("link", link).into());
        }
        self.drops()?
            .dismiss_link(caller, link.to_owned(), dismissal.dismissed)
            .await
    }
}

/// The recorded changes as events, each with what a link reads of it: a resource stopped while
/// the server ran is shown, but runs nothing a crash could come from.
fn change_events(recorded: Recorded) -> Vec<(TimelineEvent, Option<LinkedChange>)> {
    let mut events = Vec::new();
    for start in recorded.starts {
        let key = format!("start:{}", start.id);
        let at_ms = start.at.as_millisecond();
        let detail = EventDetail::Changes {
            counts: start.counts,
            build: start.build,
            resources: start.resources.iter().take(RESOURCES_NAMED).cloned().collect(),
        };
        events.push((
            event(&key, at_ms, None, detail),
            Some(LinkedChange {
                key,
                at_ms,
                resources: start.resources,
            }),
        ));
    }
    for (id, change) in recorded.live {
        let key = format!("live:{id}");
        let at_ms = change.at.as_millisecond();
        let linked = (change.change != LiveChangeKind::Stopped).then(|| LinkedChange {
            key: key.clone(),
            at_ms,
            resources: vec![change.name.clone()],
        });
        let detail = EventDetail::Resource {
            name: change.name,
            change: change.change,
            by: change.by,
        };
        events.push((event(&key, at_ms, None, detail), linked));
    }
    for writes in recorded.writes {
        let key = format!("files:{}", writes.first);
        let at_ms = writes.at.as_millisecond();
        let until = writes.until.as_second();
        let detail = EventDetail::Files {
            files: writes.files,
            resources: writes.resources.clone(),
            by: writes.by,
        };
        events.push((
            event(&key, at_ms, Some(until), detail),
            Some(LinkedChange {
                key,
                at_ms,
                resources: writes.resources,
            }),
        ));
    }
    events
}

fn event(key: &str, at_ms: i64, until: Option<i64>, detail: EventDetail) -> TimelineEvent {
    TimelineEvent {
        key: key.to_owned(),
        at: at_ms.div_euclid(1000),
        until,
        detail,
    }
}

fn instant(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH)
}
