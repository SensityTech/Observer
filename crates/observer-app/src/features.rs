//! The switches of Observer's optional features: what the owner chose, what `observer.toml` holds
//! off, and what runs in this Observer. A feature that is off when Observer starts is never built.
//! Turning one off stops it at once: its routes and topics refuse, and the tasks handed to
//! [`Features::switched`] close. Turning one on takes effect when [`Feature::turn_on`] says.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use jiff::Timestamp;
use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::features::{Applies, Feature, FeatureHold, FeatureList, FeatureReach, FeatureView};
use observer_store::Database;
use observer_store::features as store;
use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::audit::Record;
use crate::{Caller, Clock, Error};

/// What [`Features::load`] works with.
pub struct FeaturesSetup {
    pub db: Database,
    pub clock: Clock,
    /// What `observer.toml` turns off: the panel cannot turn these on.
    pub host_off: BTreeSet<Feature>,
    /// Where SFTP listens, when `observer.toml` says.
    pub sftp: Option<SocketAddr>,
}

/// The feature switches. Clones share them.
#[derive(Clone)]
pub struct Features {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    host_off: BTreeSet<Feature>,
    sftp: Option<SocketAddr>,
    /// What the owner chose and when; a feature missing keeps its default, on.
    chosen: Mutex<BTreeMap<Feature, (bool, Timestamp)>>,
    /// What runs in this Observer: decided at its start, then narrowed by every feature turned
    /// off, and widened only by those whose switch applies at once.
    running: watch::Sender<BTreeSet<Feature>>,
    /// One change at a time, so that the store, the audit log and `running` agree.
    changing: AsyncMutex<()>,
}

/// The refusal of a route or a topic whose feature does not run.
pub fn feature_disabled(feature: Feature) -> Error {
    ApiError::new(ErrorCode::FeatureDisabled)
        .with("feature", feature.as_str())
        .into()
}

impl Features {
    /// Reads the owner's choices and decides what runs in this Observer.
    pub async fn load(setup: FeaturesSetup) -> Result<Self, Error> {
        let chosen: BTreeMap<_, _> = setup
            .db
            .read(store::switches)
            .await?
            .into_iter()
            .map(|switch| (switch.feature, (switch.enabled, switch.changed_at)))
            .collect();
        let inner = Inner {
            db: setup.db,
            clock: setup.clock,
            host_off: setup.host_off,
            sftp: setup.sftp,
            chosen: Mutex::new(chosen),
            running: watch::Sender::new(BTreeSet::new()),
            changing: AsyncMutex::new(()),
        };
        let mut running = BTreeSet::new();
        // A feature comes after the one it requires.
        for feature in Feature::ALL {
            if inner.enabled(feature) && inner.hold(feature, &running).is_none() {
                running.insert(feature);
            }
        }
        inner.running.send_replace(running);
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Whether `feature` runs in this Observer now.
    pub fn runs(&self, feature: Feature) -> bool {
        self.inner.running.borrow().contains(&feature)
    }

    /// What runs, as it changes.
    pub fn running(&self) -> watch::Receiver<BTreeSet<Feature>> {
        self.inner.running.subscribe()
    }

    pub fn view(&self) -> FeatureList {
        let running = self.inner.running.borrow().clone();
        let chosen = self.inner.chosen.lock().unwrap_or_else(PoisonError::into_inner);
        let features = Feature::ALL
            .into_iter()
            .map(|feature| {
                let listens = match (feature, self.inner.sftp) {
                    (Feature::Sftp, Some(listen)) => vec![listen.to_string()],
                    _ => Vec::new(),
                };
                FeatureView {
                    feature,
                    enabled: chosen.get(&feature).is_none_or(|&(enabled, _)| enabled),
                    running: running.contains(&feature),
                    held: self.inner.hold(feature, &running),
                    requires: feature.requires(),
                    turn_on: feature.turn_on(),
                    changed_at: chosen.get(&feature).map(|&(_, at)| at),
                    reach: FeatureReach::of(feature, listens),
                }
            })
            .collect();
        FeatureList { features }
    }

    /// Turns `feature` on or off for the owner, and audits it. Off applies at once, to the
    /// features that require it too; on, as the feature's switch says. `observer.toml` refuses
    /// to let a feature it holds off be turned on, and that refusal is audited.
    pub async fn set(&self, caller: &Caller, feature: Feature, enabled: bool) -> Result<FeatureList, Error> {
        let _changing = self.inner.changing.lock().await;
        let now = self.inner.clock.now();
        let record = Record::by(caller, AuditAction::FeatureChanged).target(feature.as_str());
        if enabled && self.inner.host_off.contains(&feature) {
            let refused = record
                .outcome(Outcome::Denied)
                .details(json!({ "enabled": true, "lock": "host" }));
            self.inner.db.write(move |tx| refused.write(tx, now)).await?;
            return Err(ApiError::new(ErrorCode::FeatureLocked)
                .with("feature", feature.as_str())
                .into());
        }
        if self.inner.enabled(feature) == enabled {
            return Ok(self.view());
        }
        let applies = if enabled { feature.turn_on() } else { Applies::Now };
        let record = record.details(json!({ "enabled": enabled, "applies": applies }));
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                store::set(tx, feature, enabled, now)?;
                record.write(tx, now)?;
                Ok(())
            })
            .await?;
        self.inner
            .chosen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(feature, (enabled, now));
        self.inner.running.send_if_modified(|running| {
            if enabled {
                applies == Applies::Now
                    && self.inner.hold(feature, running).is_none()
                    && running.insert(feature)
            } else {
                let before = running.len();
                running.retain(|&other| other != feature && other.requires() != Some(feature));
                running.len() != before
            }
        });
        Ok(self.view())
    }

    /// Hands over a feature's task: `close` runs as soon as the feature stops running, or when
    /// [`Switched::close`] says Observer stops, whichever comes first.
    pub fn switched<F>(&self, feature: Feature, close: impl FnOnce() -> F + Send + 'static) -> Switched
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stopping = CancellationToken::new();
        let running = self.running();
        let waiter = tokio::spawn({
            let stopping = stopping.clone();
            async move {
                let turned_off = tokio::select! {
                    () = turned_off(running, feature) => true,
                    () = stopping.cancelled() => false,
                };
                close().await;
                if turned_off {
                    info!(feature = feature.as_str(), "turned off; its tasks stopped");
                }
            }
        });
        Switched { stopping, waiter }
    }
}

impl Inner {
    fn enabled(&self, feature: Feature) -> bool {
        self.chosen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&feature)
            .is_none_or(|&(enabled, _)| enabled)
    }

    /// Why `feature` cannot run beside `running`, whatever the owner chose.
    fn hold(&self, feature: Feature, running: &BTreeSet<Feature>) -> Option<FeatureHold> {
        if self.host_off.contains(&feature) {
            Some(FeatureHold::Host)
        } else if feature == Feature::Sftp && self.sftp.is_none() {
            Some(FeatureHold::NotConfigured)
        } else if feature
            .requires()
            .is_some_and(|required| !running.contains(&required))
        {
            Some(FeatureHold::Requires)
        } else {
            None
        }
    }
}

/// Resolves once `feature` no longer runs.
async fn turned_off(mut running: watch::Receiver<BTreeSet<Feature>>, feature: Feature) {
    while running.borrow_and_update().contains(&feature) {
        if running.changed().await.is_err() {
            // The switches are gone with Observer's stop, which closes the task in its turn.
            std::future::pending::<()>().await;
        }
    }
}

/// A feature's task, closed as soon as the feature is turned off, or by [`Switched::close`].
pub struct Switched {
    stopping: CancellationToken,
    waiter: JoinHandle<()>,
}

impl Switched {
    /// Closes the task if the feature is still on, and waits until it is closed.
    pub async fn close(self) {
        self.stopping.cancel();
        if let Err(error) = self.waiter.await {
            warn!(%error, "a feature's task did not close");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use observer_domain::permissions::Permissions;
    use observer_store::StoreError;
    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;
    use crate::Client;

    struct Lab {
        _dir: TempDir,
        db: Database,
        clock: Clock,
    }

    impl Lab {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
                .await
                .unwrap();
            Self {
                _dir: dir,
                db,
                clock: Clock::manual("2026-11-02T09:00:00Z".parse().unwrap()),
            }
        }

        /// The switches as Observer loads them at its start, with SFTP configured.
        async fn features(&self, host_off: &[Feature]) -> Features {
            Features::load(FeaturesSetup {
                db: self.db.clone(),
                clock: self.clock.clone(),
                host_off: host_off.iter().copied().collect(),
                sftp: Some(SocketAddr::from(([0, 0, 0, 0], 40126))),
            })
            .await
            .unwrap()
        }

        /// The audit entries of feature changes, oldest first: target, outcome and details.
        async fn audited(&self) -> Vec<(String, String, Value)> {
            self.db
                .read(|conn| -> Result<_, StoreError> {
                    let mut statement = conn.prepare(
                        "SELECT target, outcome, details FROM audit_log WHERE action = 'feature_changed'
                         ORDER BY id",
                    )?;
                    let rows = statement.query_map([], |row| {
                        let details: String = row.get(2)?;
                        Ok((row.get(0)?, row.get(1)?, serde_json::from_str(&details).unwrap()))
                    })?;
                    Ok(rows.collect::<Result<Vec<_>, _>>()?)
                })
                .await
                .unwrap()
        }
    }

    fn owner() -> Caller {
        Caller {
            session_id: 1,
            user_id: 1,
            username: "ada".to_owned(),
            is_owner: true,
            permissions: Permissions::NONE,
            must_change_password: false,
            reauthenticated_at: None,
            client: Client {
                ip: IpAddr::from([127, 0, 0, 1]),
                user_agent: None,
            },
        }
    }

    fn view(list: &FeatureList, feature: Feature) -> FeatureView {
        list.features
            .iter()
            .find(|view| view.feature == feature)
            .unwrap()
            .clone()
    }

    #[tokio::test]
    async fn every_feature_runs_by_default_and_sftp_waits_for_its_address() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;
        assert!(Feature::ALL.into_iter().all(|feature| features.runs(feature)));

        let unconfigured = Features::load(FeaturesSetup {
            db: lab.db.clone(),
            clock: lab.clock.clone(),
            host_off: BTreeSet::new(),
            sftp: None,
        })
        .await
        .unwrap();

        assert!(!unconfigured.runs(Feature::Sftp));
        let sftp = view(&unconfigured.view(), Feature::Sftp);
        assert!(sftp.enabled);
        assert_eq!(sftp.held, Some(FeatureHold::NotConfigured));
    }

    #[tokio::test]
    async fn turning_a_feature_off_stops_it_at_once_closes_its_task_and_is_audited() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;
        let (closing, closed) = tokio::sync::oneshot::channel();
        let _switched = features.switched(Feature::Sftp, move || async move {
            let _ = closing.send(());
        });

        let list = features.set(&owner(), Feature::Sftp, false).await.unwrap();

        assert!(!features.runs(Feature::Sftp));
        let sftp = view(&list, Feature::Sftp);
        assert!(!sftp.enabled && !sftp.running);
        assert_eq!(sftp.changed_at, Some(lab.clock.now()));
        tokio::time::timeout(std::time::Duration::from_secs(5), closed)
            .await
            .expect("the task closes without waiting for Observer's stop")
            .unwrap();
        assert_eq!(
            lab.audited().await,
            [(
                "sftp".to_owned(),
                "success".to_owned(),
                json!({ "enabled": false, "applies": "now" })
            )]
        );
    }

    #[tokio::test]
    async fn the_task_of_a_feature_left_on_closes_only_when_observer_stops() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;
        let (closing, closed) = tokio::sync::oneshot::channel();
        let switched = features.switched(Feature::Metrics, move || async move {
            let _ = closing.send(());
        });

        features.set(&owner(), Feature::Sftp, false).await.unwrap();
        tokio::task::yield_now().await;
        let mut closed = closed;
        assert!(
            closed.try_recv().is_err(),
            "another feature went off, not this one"
        );

        switched.close().await;
        assert!(closed.try_recv().is_ok());
    }

    #[tokio::test]
    async fn a_feature_the_host_holds_off_never_runs_and_cannot_be_turned_on_from_the_panel() {
        let lab = Lab::new().await;
        let features = lab.features(&[Feature::Sftp]).await;
        assert!(!features.runs(Feature::Sftp));

        let refused = features.set(&owner(), Feature::Sftp, true).await.unwrap_err();

        assert_eq!(refused.code(), Some(ErrorCode::FeatureLocked));
        assert!(!features.runs(Feature::Sftp));
        let sftp = view(&features.view(), Feature::Sftp);
        assert_eq!(sftp.held, Some(FeatureHold::Host));
        assert_eq!(sftp.changed_at, None, "nothing was stored");
        assert_eq!(
            lab.audited().await,
            [(
                "sftp".to_owned(),
                "denied".to_owned(),
                json!({ "enabled": true, "lock": "host" })
            )]
        );
        let reloaded = lab.features(&[Feature::Sftp]).await;
        assert!(!reloaded.runs(Feature::Sftp));
    }

    #[tokio::test]
    async fn turning_players_off_stops_drops_which_stays_held_until_players_runs_again() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;

        features.set(&owner(), Feature::Players, false).await.unwrap();

        assert!(!features.runs(Feature::Drops));
        let drops = view(&features.view(), Feature::Drops);
        assert!(drops.enabled);
        assert_eq!(drops.held, Some(FeatureHold::Requires));
        let restarted = lab.features(&[]).await;
        assert!(!restarted.runs(Feature::Players) && !restarted.runs(Feature::Drops));
    }

    #[tokio::test]
    async fn tools_come_back_at_once_and_the_others_at_the_next_start() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;
        for feature in [Feature::Tools, Feature::Diagnostics] {
            features.set(&owner(), feature, false).await.unwrap();
            features.set(&owner(), feature, true).await.unwrap();
        }

        assert!(features.runs(Feature::Tools));
        assert!(!features.runs(Feature::Diagnostics));
        let diagnostics = view(&features.view(), Feature::Diagnostics);
        assert!(diagnostics.enabled && diagnostics.held.is_none());
        assert!(lab.features(&[]).await.runs(Feature::Diagnostics));
        let applies: Vec<Value> = lab
            .audited()
            .await
            .into_iter()
            .map(|(_, _, details)| details["applies"].clone())
            .collect();
        assert_eq!(applies, ["now", "now", "now", "nextStart"]);
    }

    #[tokio::test]
    async fn setting_a_switch_to_what_it_is_changes_nothing_and_audits_nothing() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;

        features.set(&owner(), Feature::Metrics, true).await.unwrap();

        assert_eq!(view(&features.view(), Feature::Metrics).changed_at, None);
        assert_eq!(lab.audited().await, []);
    }
}
