//! The switches of Observer's optional features: what the owner chose, what `observer.toml` holds
//! off, and what runs in this Observer. A feature that is off when Observer starts is never built.
//! Turning one off stops it at once: its routes and topics refuse, and the tasks handed to
//! [`Features::switched`] close. Turning one on takes effect when [`Feature::turn_on`] says.
//! SFTP alone also takes an address, from `observer.toml` or from the owner in the panel, and its
//! port follows both the switch and the address while Observer runs.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::features::{
    Applies, Feature, FeatureHold, FeatureList, FeatureReach, FeatureView, SftpListen,
};
use observer_store::Database;
use observer_store::features as store;
use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::audit::Record;
use crate::{Caller, Clock, Error};

/// How long turning a feature off waits for its tasks to close before it answers all the same.
const CLOSE_WAIT: Duration = Duration::from_secs(20);
/// How long a change waits to learn what became of SFTP's port before it answers all the same.
const PORT_WAIT: Duration = Duration::from_secs(10);

/// What [`Features::load`] works with.
pub struct FeaturesSetup {
    pub db: Database,
    pub clock: Clock,
    /// What `observer.toml` turns off: the panel cannot turn these on.
    pub host_off: BTreeSet<Feature>,
    /// Where SFTP listens, when `observer.toml` says: the panel then cannot change it.
    pub sftp: Option<SocketAddr>,
    /// Observer's data folder, which holds the features' files.
    pub data_dir: PathBuf,
}

/// SFTP's address, and what became of its port.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Sftp {
    /// What `observer.toml` names.
    pinned: Option<SocketAddr>,
    /// What the owner gave in the panel.
    chosen: Option<SocketAddr>,
    /// The address the port is open on.
    open: Option<SocketAddr>,
    /// The fingerprint of the host key served on the open port.
    fingerprint: Option<String>,
    /// Why the port did not open on the address in force.
    refused: Option<String>,
}

impl Sftp {
    fn listen(&self) -> Option<SocketAddr> {
        self.pinned.or(self.chosen)
    }

    /// Whether the port is where the address in force wants it, or was refused there.
    fn settled(&self) -> bool {
        self.refused.is_some() || self.open == self.listen()
    }
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
    sftp: watch::Sender<Sftp>,
    /// The address in force as it changes, for whoever keeps the port.
    sftp_listen: watch::Sender<Option<SocketAddr>>,
    data_dir: PathBuf,
    /// What the owner chose and when; a feature missing keeps its default.
    chosen: Mutex<BTreeMap<Feature, (bool, Timestamp)>>,
    /// What runs in this Observer: decided at its start, then narrowed by every feature turned
    /// off, and widened only by those whose switch applies at once.
    running: watch::Sender<BTreeSet<Feature>>,
    /// One change at a time, so that the store, the audit log and `running` agree.
    changing: AsyncMutex<()>,
    /// The tasks handed over and not closed yet, by feature.
    open: Arc<watch::Sender<BTreeMap<Feature, usize>>>,
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
        let sftp = Sftp {
            pinned: setup.sftp,
            chosen: setup.db.read(store::sftp_listen).await?,
            ..Sftp::default()
        };
        let inner = Inner {
            db: setup.db,
            clock: setup.clock,
            host_off: setup.host_off,
            sftp_listen: watch::Sender::new(sftp.listen()),
            sftp: watch::Sender::new(sftp),
            data_dir: setup.data_dir,
            chosen: Mutex::new(chosen),
            running: watch::Sender::new(BTreeSet::new()),
            changing: AsyncMutex::new(()),
            open: Arc::new(watch::Sender::new(BTreeMap::new())),
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

    /// The refusal of a route of `feature` while it does not run, with what holds it if anything
    /// does: the panel then tells the owner where to look.
    pub fn refusal(&self, feature: Feature) -> Error {
        let refusal = ApiError::new(ErrorCode::FeatureDisabled).with("feature", feature.as_str());
        let running = self.inner.running.borrow().clone();
        match self.inner.hold(feature, &running) {
            Some(held) => refusal.with("held", json!(held)).into(),
            None => refusal.into(),
        }
    }

    /// Every feature's switch, and what it reaches with the size of its files now.
    pub async fn view(&self) -> FeatureList {
        let data_dir = self.inner.data_dir.clone();
        let sizes: BTreeMap<&str, Option<u64>> = tokio::task::spawn_blocking(move || {
            Feature::ALL
                .iter()
                .flat_map(|feature| feature.files())
                .map(|&path| (path, size_on_disk(&data_dir, path)))
                .collect()
        })
        .await
        .unwrap_or_default();
        let running = self.inner.running.borrow().clone();
        let listen = self.inner.sftp.borrow().listen();
        let chosen = self.inner.chosen.lock().unwrap_or_else(PoisonError::into_inner);
        let features = Feature::ALL
            .into_iter()
            .map(|feature| {
                let listens = match (feature, listen) {
                    (Feature::Sftp, Some(listen)) => vec![listen.to_string()],
                    _ => Vec::new(),
                };
                FeatureView {
                    feature,
                    enabled: chosen.get(&feature).is_none_or(|&(enabled, _)| enabled),
                    running: running.contains(&feature),
                    held: self.inner.hold(feature, &running),
                    requires: feature.requires().to_vec(),
                    turn_on: feature.turn_on(),
                    turn_off: feature.turn_off(),
                    changed_at: chosen.get(&feature).map(|&(_, at)| at),
                    reach: FeatureReach::of(feature, listens, |path| sizes.get(path).copied().flatten()),
                }
            })
            .collect();
        FeatureList { features }
    }

    /// Turns `feature` on or off for the owner, and audits it. Off applies at once, to the
    /// features that require it too, and answers once their tasks closed; on, as the feature's
    /// switch says. `observer.toml` refuses to let a feature it holds off be turned on, and that
    /// refusal is audited.
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
            return Ok(self.view().await);
        }
        let applies = if enabled {
            feature.turn_on()
        } else {
            feature.turn_off()
        };
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
        if feature == Feature::Sftp {
            // The switch turned on again gives a refused port another try.
            self.inner
                .sftp
                .send_if_modified(|sftp| sftp.refused.take().is_some());
        }
        let mut stopped = Vec::new();
        self.inner.running.send_if_modified(|running| {
            if enabled {
                applies != Applies::ObserverRestart
                    && self.inner.hold(feature, running).is_none()
                    && running.insert(feature)
            } else {
                stopped = running
                    .iter()
                    .copied()
                    .filter(|&other| other == feature || other.requires().contains(&feature))
                    .collect();
                running.retain(|other| !stopped.contains(other));
                !stopped.is_empty()
            }
        });
        // The answer then means what it says: the port is closed, the tasks are over.
        if !stopped.is_empty()
            && tokio::time::timeout(CLOSE_WAIT, closed(self.inner.open.subscribe(), &stopped))
                .await
                .is_err()
        {
            warn!(
                feature = feature.as_str(),
                "turned off, but its tasks have not stopped yet"
            );
        }
        if enabled && feature == Feature::Sftp && self.runs(Feature::Sftp) {
            self.port_settled().await;
        }
        Ok(self.view().await)
    }

    /// Where SFTP listens, who says so, and whether its port is open.
    pub fn sftp(&self) -> SftpListen {
        let sftp = self.inner.sftp.borrow();
        let listen = sftp.listen();
        let open = listen.is_some() && sftp.open == listen;
        SftpListen {
            listen: listen.map(|listen| listen.to_string()),
            pinned: sftp.pinned.is_some(),
            open,
            fingerprint: sftp.fingerprint.clone().filter(|_| open),
            refused: sftp.refused.clone(),
        }
    }

    /// Gives SFTP its address for the owner, or takes it back, and audits it. The port follows
    /// at once while the feature is on, and the answer says what became of it. The same address
    /// given again tries a refused port anew. `observer.toml` refuses when it names the address.
    pub async fn set_sftp_listen(
        &self,
        caller: &Caller,
        listen: Option<SocketAddr>,
    ) -> Result<SftpListen, Error> {
        let _changing = self.inner.changing.lock().await;
        let (previous, refused) = {
            let sftp = self.inner.sftp.borrow();
            if sftp.pinned.is_some() {
                return Err(ApiError::new(ErrorCode::SftpListenPinned).into());
            }
            (sftp.chosen, sftp.refused.is_some())
        };
        if listen.is_some_and(|listen| listen.port() == 0) {
            return Err(ApiError::new(ErrorCode::InvalidSftpListen)
                .with("reason", "port")
                .into());
        }
        if previous == listen && !refused {
            return Ok(self.sftp());
        }
        if previous != listen {
            let now = self.inner.clock.now();
            let record = Record::by(caller, AuditAction::SftpListenChanged)
                .target(Feature::Sftp.as_str())
                .details(json!({
                    "listen": listen.map(|listen| listen.to_string()),
                    "previous": previous.map(|previous| previous.to_string()),
                }));
            self.inner
                .db
                .write(move |tx| -> Result<(), Error> {
                    store::set_sftp_listen(tx, listen)?;
                    record.write(tx, now)?;
                    Ok(())
                })
                .await?;
        }
        self.inner.sftp.send_modify(|sftp| {
            sftp.chosen = listen;
            sftp.refused = None;
        });
        let mut closing = false;
        self.inner.running.send_if_modified(|running| {
            if self.inner.enabled(Feature::Sftp) && self.inner.hold(Feature::Sftp, running).is_none() {
                running.insert(Feature::Sftp)
            } else {
                closing = running.remove(&Feature::Sftp);
                closing
            }
        });
        self.inner.sftp_listen.send_replace(listen);
        if closing {
            if tokio::time::timeout(CLOSE_WAIT, closed(self.inner.open.subscribe(), &[Feature::Sftp]))
                .await
                .is_err()
            {
                warn!("SFTP lost its address, but its port has not closed yet");
            }
        } else if self.runs(Feature::Sftp) {
            self.port_settled().await;
        }
        Ok(self.sftp())
    }

    /// The address SFTP listens on, as it changes. Whoever keeps the port follows it together
    /// with [`Features::running`], and says what became of the port.
    pub fn sftp_listen(&self) -> watch::Receiver<Option<SocketAddr>> {
        self.inner.sftp_listen.subscribe()
    }

    /// The port is open on `listen`, where clients are shown the host key of `fingerprint`.
    pub fn sftp_opened(&self, listen: SocketAddr, fingerprint: String) {
        self.inner.sftp.send_modify(|sftp| {
            sftp.open = Some(listen);
            sftp.fingerprint = Some(fingerprint);
        });
    }

    /// The port is closed.
    pub fn sftp_closed(&self) {
        self.inner.sftp.send_if_modified(|sftp| {
            sftp.fingerprint = None;
            sftp.open.take().is_some()
        });
    }

    /// `listen` did not open. SFTP stops running until its address is given again or its switch
    /// is turned on again, which is when the port is tried anew.
    pub fn sftp_refused(&self, listen: SocketAddr, error: String) {
        let in_force = self.inner.sftp.send_if_modified(|sftp| {
            if sftp.listen() != Some(listen) {
                return false;
            }
            sftp.open = None;
            sftp.fingerprint = None;
            sftp.refused = Some(error);
            true
        });
        if in_force {
            self.inner
                .running
                .send_if_modified(|running| running.remove(&Feature::Sftp));
        }
    }

    /// Waits a while for the port to open or be refused on the address in force. Nobody keeps
    /// the port in a build without SFTP's server, and then there is nothing to wait for.
    async fn port_settled(&self) {
        if self.inner.sftp_listen.receiver_count() == 0 {
            return;
        }
        let mut sftp = self.inner.sftp.subscribe();
        if tokio::time::timeout(PORT_WAIT, sftp.wait_for(Sftp::settled))
            .await
            .is_err()
        {
            warn!("SFTP's port has neither opened nor been refused yet");
        }
    }

    /// Hands over a feature's task: `close` runs as soon as the feature stops running, or when
    /// [`Switched::close`] says Observer stops, whichever comes first.
    pub fn switched<F>(&self, feature: Feature, close: impl FnOnce() -> F + Send + 'static) -> Switched
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stopping = CancellationToken::new();
        let running = self.running();
        let open = Arc::clone(&self.inner.open);
        open.send_modify(|open| *open.entry(feature).or_default() += 1);
        let waiter = tokio::spawn({
            let stopping = stopping.clone();
            async move {
                let turned_off = tokio::select! {
                    // Whoever keeps the task may close it as it sees the feature stop running:
                    // it is still the switch that ended it, and the log says so.
                    biased;
                    () = turned_off(running, feature) => true,
                    () = stopping.cancelled() => false,
                };
                close().await;
                if turned_off {
                    info!(feature = feature.as_str(), "turned off; its tasks stopped");
                }
                open.send_modify(|open| {
                    if let Entry::Occupied(mut count) = open.entry(feature) {
                        *count.get_mut() -= 1;
                        if *count.get() == 0 {
                            count.remove();
                        }
                    }
                });
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
            return Some(FeatureHold::Host);
        }
        if feature == Feature::Sftp {
            let sftp = self.sftp.borrow();
            if sftp.listen().is_none() {
                return Some(FeatureHold::NotConfigured);
            }
            if sftp.refused.is_some() {
                return Some(FeatureHold::PortRefused);
            }
        }
        feature
            .requires()
            .iter()
            .any(|required| !running.contains(required))
            .then_some(FeatureHold::Requires)
    }
}

/// What `path` holds on disk under `data_dir`: a database with its journal, or the files of a
/// folder. `None` when it is not there.
fn size_on_disk(data_dir: &Path, path: &str) -> Option<u64> {
    let full = data_dir.join(path);
    let metadata = fs::metadata(&full).ok()?;
    if metadata.is_dir() {
        return Some(folder_size(&full));
    }
    let mut journal = full.into_os_string();
    journal.push("-wal");
    Some(metadata.len() + fs::metadata(journal).map_or(0, |journal| journal.len()))
}

/// The files under `folder`, links left out so that none is counted twice.
fn folder_size(folder: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.metadata() {
            Ok(metadata) if metadata.is_dir() => folder_size(&entry.path()),
            Ok(metadata) if metadata.is_file() => metadata.len(),
            _ => 0,
        })
        .sum()
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

/// Resolves once no task of `features` is left open.
async fn closed(mut open: watch::Receiver<BTreeMap<Feature, usize>>, features: &[Feature]) {
    while open
        .borrow_and_update()
        .keys()
        .any(|feature| features.contains(feature))
    {
        if open.changed().await.is_err() {
            return;
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
        dir: TempDir,
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
                dir,
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
                data_dir: self.dir.path().join("data"),
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
            data_dir: lab.dir.path().join("data"),
        })
        .await
        .unwrap();

        assert!(!unconfigured.runs(Feature::Sftp));
        let sftp = view(&unconfigured.view().await, Feature::Sftp);
        assert!(sftp.enabled);
        assert_eq!(sftp.held, Some(FeatureHold::NotConfigured));
    }

    #[tokio::test]
    async fn turning_a_feature_off_stops_it_at_once_closes_its_task_and_is_audited() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;
        let (closing, mut closed) = tokio::sync::oneshot::channel();
        let _switched = features.switched(Feature::Sftp, move || async move {
            // A listener that takes a while to let its connections go.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = closing.send(());
        });

        let list = features.set(&owner(), Feature::Sftp, false).await.unwrap();

        assert!(
            closed.try_recv().is_ok(),
            "the answer comes once the task closed, without waiting for Observer's stop"
        );
        assert!(!features.runs(Feature::Sftp));
        let sftp = view(&list, Feature::Sftp);
        assert!(!sftp.enabled && !sftp.running);
        assert_eq!(sftp.changed_at, Some(lab.clock.now()));
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
        let sftp = view(&features.view().await, Feature::Sftp);
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
    async fn a_refusal_names_the_feature_and_what_holds_it() {
        let lab = Lab::new().await;
        let features = lab.features(&[Feature::Diagnostics]).await;
        features.set(&owner(), Feature::Players, false).await.unwrap();

        let params = |feature| match features.refusal(feature) {
            Error::Refused(refusal) => (refusal.code, Value::Object(refusal.params)),
            other => panic!("{other:?}"),
        };

        assert_eq!(
            params(Feature::Diagnostics),
            (
                ErrorCode::FeatureDisabled,
                json!({ "feature": "diagnostics", "held": "host" })
            )
        );
        assert_eq!(params(Feature::Players).1, json!({ "feature": "players" }));
        assert_eq!(
            params(Feature::Drops).1,
            json!({ "feature": "drops", "held": "requires" })
        );
    }

    #[tokio::test]
    async fn turning_players_off_stops_drops_which_stays_held_until_players_runs_again() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;

        features.set(&owner(), Feature::Players, false).await.unwrap();

        assert!(!features.runs(Feature::Drops));
        let drops = view(&features.view().await, Feature::Drops);
        assert!(drops.enabled);
        assert_eq!(drops.held, Some(FeatureHold::Requires));
        let restarted = lab.features(&[]).await;
        assert!(!restarted.runs(Feature::Players) && !restarted.runs(Feature::Drops));
    }

    #[tokio::test]
    async fn turning_the_statistics_off_stops_the_drops_drawn_from_the_runs_they_record() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;

        features.set(&owner(), Feature::Metrics, false).await.unwrap();

        assert!(features.runs(Feature::Players));
        assert!(!features.runs(Feature::Drops));
        let drops = view(&features.view().await, Feature::Drops);
        assert_eq!(drops.requires, [Feature::Players, Feature::Metrics]);
        assert_eq!(drops.held, Some(FeatureHold::Requires));
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
        let diagnostics = view(&features.view().await, Feature::Diagnostics);
        assert!(diagnostics.enabled && diagnostics.held.is_none());
        assert!(lab.features(&[]).await.runs(Feature::Diagnostics));
        let applies: Vec<Value> = lab
            .audited()
            .await
            .into_iter()
            .map(|(_, _, details)| details["applies"].clone())
            .collect();
        assert_eq!(applies, ["now", "nextServerStart", "now", "observerRestart"]);
    }

    #[tokio::test]
    async fn the_view_weighs_each_database_with_its_journal_and_each_folder_with_its_files() {
        let lab = Lab::new().await;
        let data = lab.dir.path().join("data");
        fs::create_dir_all(data.join("crashes/2026-11-02")).unwrap();
        fs::write(data.join("metrics.sqlite3"), [0; 4096]).unwrap();
        fs::write(data.join("metrics.sqlite3-wal"), [0; 1000]).unwrap();
        fs::write(data.join("crashes/2026-11-02/dump.dmp"), [0; 300]).unwrap();
        fs::write(data.join("crashes/report.json"), [0; 20]).unwrap();

        let list = lab.features(&[]).await.view().await;

        let files = |feature| view(&list, feature).reach.files;
        assert_eq!(files(Feature::Metrics)[0].bytes, Some(5096));
        assert_eq!(files(Feature::Drops)[0].bytes, Some(5096));
        assert_eq!(files(Feature::Diagnostics)[0].bytes, Some(320));
        assert_eq!(files(Feature::Changes)[0].path, "changes.sqlite3");
        assert_eq!(files(Feature::Changes)[0].bytes, None, "never created");
    }

    #[tokio::test]
    async fn setting_a_switch_to_what_it_is_changes_nothing_and_audits_nothing() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;

        features.set(&owner(), Feature::Metrics, true).await.unwrap();

        assert_eq!(view(&features.view().await, Feature::Metrics).changed_at, None);
        assert_eq!(lab.audited().await, []);
    }

    /// The switches as Observer loads them when `observer.toml` leaves SFTP's address to the panel.
    async fn unpinned(lab: &Lab) -> Features {
        Features::load(FeaturesSetup {
            db: lab.db.clone(),
            clock: lab.clock.clone(),
            host_off: BTreeSet::new(),
            sftp: None,
            data_dir: lab.dir.path().join("data"),
        })
        .await
        .unwrap()
    }

    /// Stands for whoever keeps the port: each address it is asked to open, `answer` opens or
    /// refuses.
    fn keeper(
        features: &Features,
        answer: impl Fn(SocketAddr) -> Result<(), String> + Send + 'static,
    ) -> JoinHandle<()> {
        let features = features.clone();
        let mut running = features.running();
        let mut listen = features.sftp_listen();
        tokio::spawn(async move {
            let mut open = None;
            loop {
                let on = running.borrow_and_update().contains(&Feature::Sftp);
                let at = *listen.borrow_and_update();
                let wanted = if on { at } else { None };
                if open != wanted {
                    if open.take().is_some() {
                        features.sftp_closed();
                    }
                    match wanted.map(|at| (at, answer(at))) {
                        Some((at, Ok(()))) => {
                            open = Some(at);
                            features.sftp_opened(at, "SHA256:host-key".to_owned());
                        }
                        Some((at, Err(error))) => features.sftp_refused(at, error),
                        None => {}
                    }
                }
                tokio::select! {
                    changed = running.changed() => if changed.is_err() { return },
                    changed = listen.changed() => if changed.is_err() { return },
                }
            }
        })
    }

    fn address(listen: &str) -> SocketAddr {
        listen.parse().unwrap()
    }

    #[tokio::test]
    async fn the_owner_gives_sftp_its_address_which_outlives_observer_and_takes_it_back() {
        let lab = Lab::new().await;
        let features = unpinned(&lab).await;
        let _keeper = keeper(&features, |_| Ok(()));

        let given = features
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:40126")))
            .await
            .unwrap();

        assert_eq!(
            given,
            SftpListen {
                listen: Some("127.0.0.1:40126".to_owned()),
                pinned: false,
                open: true,
                fingerprint: Some("SHA256:host-key".to_owned()),
                refused: None,
            }
        );
        assert!(features.runs(Feature::Sftp));
        let sftp = view(&features.view().await, Feature::Sftp);
        assert_eq!(sftp.turn_on, Applies::Now);
        assert_eq!(sftp.reach.listens, ["127.0.0.1:40126"]);
        let restarted = unpinned(&lab).await;
        assert!(restarted.runs(Feature::Sftp));
        assert_eq!(restarted.sftp().listen.as_deref(), Some("127.0.0.1:40126"));

        let taken_back = features.set_sftp_listen(&owner(), None).await.unwrap();

        assert_eq!((taken_back.listen, taken_back.open), (None, false));
        assert_eq!(taken_back.fingerprint, None);
        assert!(!features.runs(Feature::Sftp));
        let sftp = view(&features.view().await, Feature::Sftp);
        assert_eq!(sftp.held, Some(FeatureHold::NotConfigured));
        assert!(!unpinned(&lab).await.runs(Feature::Sftp));
    }

    #[tokio::test]
    async fn a_new_address_moves_the_port_and_the_same_one_changes_nothing() {
        let lab = Lab::new().await;
        let features = unpinned(&lab).await;
        let _keeper = keeper(&features, |_| Ok(()));
        let listened = |features: &Features| features.sftp().listen;
        features
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:40126")))
            .await
            .unwrap();

        let same = features
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:40126")))
            .await
            .unwrap();
        let moved = features
            .set_sftp_listen(&owner(), Some(address("0.0.0.0:2022")))
            .await
            .unwrap();

        assert!(same.open);
        assert!(
            moved.open,
            "the answer waits for the port to open on the new address"
        );
        assert_eq!(listened(&features).as_deref(), Some("0.0.0.0:2022"));
        let changes = lab
            .db
            .read(|conn| -> Result<i64, StoreError> {
                Ok(conn.query_row(
                    "SELECT count(*) FROM audit_log WHERE action = 'sftp_listen_changed'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(changes, 2, "the same address given again is no change");
    }

    #[tokio::test]
    async fn a_port_that_does_not_open_holds_sftp_until_its_address_is_given_again() {
        let lab = Lab::new().await;
        let features = unpinned(&lab).await;
        let tries = Arc::new(Mutex::new(Vec::new()));
        let _keeper = keeper(&features, {
            let tries = Arc::clone(&tries);
            move |at| {
                tries.lock().unwrap().push(at.port());
                if at.port() == 40126 {
                    Err("the port is taken".to_owned())
                } else {
                    Ok(())
                }
            }
        });

        let refused = features
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:40126")))
            .await
            .unwrap();

        assert_eq!(
            refused.listen.as_deref(),
            Some("127.0.0.1:40126"),
            "the address is kept"
        );
        assert_eq!(
            (refused.open, refused.refused.as_deref()),
            (false, Some("the port is taken"))
        );
        assert!(!features.runs(Feature::Sftp));
        let sftp = view(&features.view().await, Feature::Sftp);
        assert_eq!(sftp.held, Some(FeatureHold::PortRefused));

        let again = features
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:40126")))
            .await
            .unwrap();
        let elsewhere = features
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:40127")))
            .await
            .unwrap();

        assert_eq!(again.refused.as_deref(), Some("the port is taken"));
        assert!(elsewhere.open && elsewhere.refused.is_none());
        assert!(features.runs(Feature::Sftp));
        assert_eq!(*tries.lock().unwrap(), [40126, 40126, 40127]);
    }

    #[tokio::test]
    async fn the_switch_of_sftp_opens_and_closes_its_port_while_observer_runs() {
        let lab = Lab::new().await;
        let features = lab.features(&[]).await;
        let _keeper = keeper(&features, |_| Ok(()));
        // The keeper opens the port of a feature that runs from Observer's start.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !features.sftp().open {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        features.set(&owner(), Feature::Sftp, false).await.unwrap();
        tokio::task::yield_now().await;
        let off = features.sftp();
        let list = features.set(&owner(), Feature::Sftp, true).await.unwrap();

        assert!(!off.open);
        assert!(view(&list, Feature::Sftp).running);
        assert!(
            features.sftp().open,
            "turned on, it listens without a restart of Observer"
        );
    }

    #[tokio::test]
    async fn the_address_observer_toml_names_is_not_the_panels_and_a_port_takes_a_number() {
        let lab = Lab::new().await;
        let pinned = lab.features(&[]).await;

        let refused = pinned
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:2022")))
            .await
            .unwrap_err();
        let no_port = unpinned(&lab)
            .await
            .set_sftp_listen(&owner(), Some(address("127.0.0.1:0")))
            .await
            .unwrap_err();

        assert_eq!(refused.code(), Some(ErrorCode::SftpListenPinned));
        let shown = pinned.sftp();
        assert_eq!(
            (shown.listen.as_deref(), shown.pinned),
            (Some("0.0.0.0:40126"), true)
        );
        assert_eq!(no_port.code(), Some(ErrorCode::InvalidSftpListen));
    }
}
