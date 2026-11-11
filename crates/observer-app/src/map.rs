//! The live map: the positions the bridge reports while a panel watches, joined with who is
//! connected, and the image the owner added. Positions stay in memory: nothing of them is written.

mod image;

use std::collections::HashMap;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use observer_domain::audit::AuditAction;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::map::{
    MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, MapImage, MapLive, MapPlayer, MapSettings, MapView,
};
use observer_fxserver::bridge::BridgeAction;
use observer_fxserver::players::{PlayerReport, PlayerReportKind, Position};
use observer_fxserver::supervisor::Supervisor;
use observer_store::Database;
use observer_store::map as stored;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub use self::image::ImageKind;
use self::image::image_size;
use crate::audit::Record;
use crate::host_file::{HostFileProblem, open_local};
use crate::secrets::{hex, random};
use crate::watching::{Watchers, Watching};
use crate::{Caller, Clock, Error, Players, Supervision};

/// How long one ask for positions lasts, and how often it is renewed while a panel watches.
const LEASE: Duration = Duration::from_secs(30);
const RENEW_EVERY: Duration = Duration::from_secs(10);
/// The folder of the data folder the image is kept in.
const FOLDER: &str = "map";

pub struct MapSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
    pub players: Players,
    pub data_dir: PathBuf,
}

/// The live map service. Clones share it.
#[derive(Clone)]
pub struct LiveMap {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    supervisor: Supervisor,
    players: Players,
    folder: PathBuf,
    live: watch::Sender<MapLive>,
    watchers: Watchers,
}

pub struct MapTask {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl MapTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the live map's task failed");
        }
    }
}

/// The positions of one run, gathered page by page.
#[derive(Default)]
struct Gathering {
    generation: u32,
    pages: Vec<Position>,
    onesync: bool,
}

impl LiveMap {
    pub fn start(setup: MapSetup) -> (Self, MapTask) {
        let supervisor = setup.supervision.supervisor().clone();
        let reports = supervisor.player_reports();
        let map = Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                supervisor,
                players: setup.players,
                folder: setup.data_dir.join(FOLDER),
                live: watch::Sender::new(MapLive {
                    players: Vec::new(),
                    at: None,
                    waiting: 0,
                    onesync: true,
                }),
                watchers: Watchers::new(),
            }),
        };
        let stop = CancellationToken::new();
        let task = tokio::spawn(map.clone().run(stop.clone(), reports));
        (map, MapTask { stop, task })
    }

    /// The map as it stands and its changes, for as long as the guard is held: the server sends
    /// positions only while someone holds one.
    pub fn watch(&self) -> (MapLive, watch::Receiver<MapLive>, Watching) {
        let guard = self.inner.watchers.join();
        let mut live = self.inner.live.subscribe();
        let snapshot = live.borrow_and_update().clone();
        (snapshot, live, guard)
    }

    async fn run(self, stop: CancellationToken, mut reports: broadcast::Receiver<PlayerReport>) {
        let mut watchers = self.inner.watchers.subscribe();
        let mut renew = tokio::time::interval(RENEW_EVERY);
        let mut gathering = Gathering::default();
        let mut watched = false;
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                Ok(()) = watchers.changed() => {
                    let now_watched = *watchers.borrow_and_update() > 0;
                    if now_watched != watched {
                        watched = now_watched;
                        self.ask(watched).await;
                    }
                }
                _ = renew.tick() => {
                    if watched {
                        self.ask(true).await;
                    }
                    let now = self.inner.clock.now();
                    self.inner.live.send_if_modified(|live| forget_stale(live, now));
                }
                report = reports.recv() => match report {
                    Ok(report) => self.on_report(report, &mut gathering, watched).await,
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                },
            }
        }
    }

    /// Asks the bridge for positions for a while, or to stop.
    async fn ask(&self, watched: bool) {
        let every_ms = match self.settings().await {
            Ok(settings) => u32::from(settings.every_seconds) * 1000,
            Err(error) => {
                warn!(%error, "the live map's settings could not be read");
                return;
            }
        };
        let for_ms = if watched {
            u32::try_from(LEASE.as_millis()).unwrap_or(u32::MAX)
        } else {
            0
        };
        // A server not running has no position to send; the next renewal asks again.
        let _ = self
            .inner
            .supervisor
            .act(BridgeAction::Positions { every_ms, for_ms })
            .await;
    }

    async fn on_report(&self, report: PlayerReport, gathering: &mut Gathering, watched: bool) {
        if gathering.generation != report.generation {
            *gathering = Gathering {
                generation: report.generation,
                ..Gathering::default()
            };
            self.inner.live.send_modify(|live| {
                live.players.clear();
                live.at = None;
            });
        }
        match report.kind {
            PlayerReportKind::Positions(page) => {
                if page.page == 1 {
                    gathering.pages.clear();
                }
                gathering.pages.extend(page.players);
                gathering.onesync = page.onesync;
                if page.last {
                    let positions = std::mem::take(&mut gathering.pages);
                    let at = Timestamp::from_millisecond(report.at_ms).ok();
                    self.publish(positions, page.onesync, at).await;
                }
            }
            // The bridge started again: it forgot what it was asked.
            PlayerReportKind::Roster(page) if page.page == 1 && watched => self.ask(true).await,
            PlayerReportKind::Drop(dropped) => {
                self.inner.live.send_if_modified(|live| {
                    let before = live.players.len();
                    live.players.retain(|player| player.net_id != dropped.net_id);
                    live.players.len() != before
                });
            }
            _ => {}
        }
    }

    /// The positions joined with who is connected, by net id: names, labels and play time.
    async fn publish(&self, positions: Vec<Position>, onesync: bool, at: Option<Timestamp>) {
        let online = match self.inner.players.online().await {
            Ok(online) => online.players,
            Err(error) => {
                warn!(%error, "the connected players could not be read for the live map");
                return;
            }
        };
        let by_id: HashMap<u32, Position> = positions
            .into_iter()
            .map(|position| (position.net_id, position))
            .collect();
        let players: Vec<MapPlayer> = online
            .iter()
            .filter_map(|player| {
                let position = by_id.get(&player.net_id)?;
                Some(MapPlayer {
                    net_id: player.net_id,
                    player: player.player,
                    name: player.name.clone(),
                    label: player.label.clone(),
                    total_ms: player.total_ms,
                    x: position.x,
                    y: position.y,
                    z: position.z,
                    heading: position.heading,
                })
            })
            .collect();
        let waiting = u32::try_from(online.len().saturating_sub(players.len())).unwrap_or(u32::MAX);
        self.inner.live.send_replace(MapLive {
            players,
            at,
            waiting,
            onesync,
        });
    }

    pub async fn settings(&self) -> Result<MapSettings, Error> {
        Ok(self.inner.db.read(stored::settings).await?)
    }

    pub async fn view(&self) -> Result<MapView, Error> {
        let (settings, image) = self
            .inner
            .db
            .read(|conn| -> Result<_, Error> { Ok((stored::settings(conn)?, stored::image(conn)?)) })
            .await?;
        Ok(MapView {
            settings,
            image: image.map(|(image, _)| image),
        })
    }

    /// Changes the calibration, the thresholds or the cadence; a new cadence applies at the next ask.
    pub async fn change_settings(&self, caller: &Caller, settings: MapSettings) -> Result<(), Error> {
        if !settings.is_valid() {
            return Err(ErrorCode::InvalidMapSettings.into());
        }
        let before = self.settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::MapSettingsChanged)
            .details(json!({ "from": before, "to": settings }));
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                stored::save_settings(tx, &settings)?;
                Ok(record.write(tx, now)?)
            })
            .await?;
        if self.inner.watchers.any() {
            self.ask(true).await;
        }
        Ok(())
    }

    /// Copies a PNG, JPEG or WebP image from the host into the map folder, replacing the one there.
    /// The path stays out of the audit log.
    pub async fn set_image(&self, caller: &Caller, path: &str) -> Result<MapImage, Error> {
        let path = PathBuf::from(path);
        let folder = self.inner.folder.clone();
        let version = hex(&random::<8>()?);
        let copied = {
            let version = version.clone();
            tokio::task::spawn_blocking(move || copy_image(&path, &folder, &version))
                .await
                .map_err(|_| Error::from(ErrorCode::Internal))?
        };
        let (kind, width, height, bytes) = copied.map_err(|problem| {
            Error::from(ApiError::new(ErrorCode::InvalidMapImage).with("problem", problem))
        })?;
        let file = format!("map-{version}.{}", kind.extension());
        let image = MapImage {
            width,
            height,
            bytes,
            added_at: self.inner.clock.now(),
            added_by: caller.username.clone(),
            version,
        };
        let record = Record::by(caller, AuditAction::MapImageChanged).details(json!({
            "width": width,
            "height": height,
            "bytes": bytes,
        }));
        let now = image.added_at;
        let kept = (image.clone(), file.clone());
        let previous = self
            .inner
            .db
            .write(move |tx| -> Result<_, Error> {
                let previous = stored::image(tx)?;
                stored::save_image(tx, Some((&kept.0, &kept.1)))?;
                record.write(tx, now)?;
                Ok(previous)
            })
            .await?;
        if let Some((_, old)) = previous.filter(|(_, old)| *old != file) {
            let _ = std::fs::remove_file(self.inner.folder.join(old));
        }
        Ok(image)
    }

    pub async fn remove_image(&self, caller: &Caller) -> Result<(), Error> {
        let record = Record::by(caller, AuditAction::MapImageChanged).details(json!({ "removed": true }));
        let now = self.inner.clock.now();
        let previous = self
            .inner
            .db
            .write(move |tx| -> Result<_, Error> {
                let previous = stored::image(tx)?.ok_or(ErrorCode::NoMapImage)?;
                stored::save_image(tx, None)?;
                record.write(tx, now)?;
                Ok(previous)
            })
            .await?;
        let _ = std::fs::remove_file(self.inner.folder.join(previous.1));
        Ok(())
    }

    /// The image's file and its type, to serve to the panel.
    pub async fn image_file(&self) -> Result<(PathBuf, ImageKind), Error> {
        let (_, file) = self
            .inner
            .db
            .read(stored::image)
            .await?
            .ok_or(ErrorCode::NoMapImage)?;
        let kind = ImageKind::from_file(&file).ok_or(ErrorCode::NoMapImage)?;
        Ok((self.inner.folder.join(file), kind))
    }
}

/// Leaves no one on the map once no position came for a lease: the server crashed or stopped without
/// telling who left.
fn forget_stale(live: &mut MapLive, now: Timestamp) -> bool {
    let stale = live
        .at
        .is_some_and(|at| now.duration_since(at) > jiff::SignedDuration::try_from(LEASE).unwrap_or_default());
    if !stale || live.players.is_empty() {
        return false;
    }
    live.players.clear();
    live.waiting = 0;
    true
}

/// The image's kind and size, once checked and copied to `map-<version>.<extension>` by way of a
/// temporary file.
fn copy_image(path: &Path, folder: &Path, version: &str) -> Result<(ImageKind, u32, u32, u64), &'static str> {
    let (mut file, metadata) = open_local(path).map_err(|problem| match problem {
        HostFileProblem::NotLocal => "notLocal",
        HostFileProblem::Missing => "missing",
        HostFileProblem::NotAFile | HostFileProblem::Unreadable => "unreadable",
    })?;
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err("tooLarge");
    }
    let mut head = vec![0; 64 * 1024];
    let read = file.read(&mut head).map_err(|_| "unreadable")?;
    head.truncate(read);
    let (kind, width, height) = image_size(&head).ok_or("notAnImage")?;
    if width == 0 || height == 0 {
        return Err("notAnImage");
    }
    if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err("tooLarge");
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| "unreadable")?;
    std::fs::create_dir_all(folder).map_err(|_| "unreadable")?;
    let target = folder.join(format!("map-{version}.{}", kind.extension()));
    let partial = folder.join(format!("map-{version}.partial"));
    let copied = (|| -> std::io::Result<u64> {
        let mut out = std::fs::File::create(&partial)?;
        let bytes = std::io::copy(&mut file.take(MAX_IMAGE_BYTES), &mut out)?;
        out.sync_all()?;
        std::fs::rename(&partial, &target)?;
        Ok(bytes)
    })();
    copied.map(|bytes| (kind, width, height, bytes)).map_err(|_| {
        let _ = std::fs::remove_file(&partial);
        "unreadable"
    })
}

#[cfg(test)]
mod tests;
