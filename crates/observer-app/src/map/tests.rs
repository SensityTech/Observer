use std::net::{IpAddr, SocketAddr};

use observer_domain::map::{MapBounds, MapImagery};
use observer_domain::permissions::Permissions;
use observer_domain::schedule::{ScheduleSettings, ScheduleStatus};
use tempfile::TempDir;

use super::*;
use crate::{
    Accounts, Client, Metrics, MetricsSetup, PasswordCost, PlayersSetup, PlayersTask, Running, Servers, Setup,
};

/// The state database in a temporary folder, with the services the live map stands on.
struct Lab {
    dir: TempDir,
    db: Database,
    clock: Clock,
    supervision: Supervision,
    players: Players,
    _playing: PlayersTask,
    _running: Running,
}

impl Lab {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2026-11-09T20:00:00Z".parse().unwrap());
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let data_dir = dir.path().join("data");
        let (supervision, running) = Supervision::start(
            Setup {
                servers: Servers::new(db.clone(), clock.clone()),
                db: db.clone(),
                clock: clock.clone(),
                accounts,
                data_dir: data_dir.clone(),
                panel: SocketAddr::from(([127, 0, 0, 1], 41975)),
                artifacts: data_dir.join("artifacts"),
                console_logs: data_dir.join("logs/console"),
                log: None,
            },
            |_| {},
        )
        .await
        .unwrap();
        let metrics = Metrics::open(MetricsSetup {
            path: dir.path().join("metrics.sqlite3"),
            clock: clock.clone(),
        })
        .await
        .unwrap();
        let status = ScheduleStatus {
            settings: ScheduleSettings::default(),
            version: None,
            zone: "Europe/Paris".to_owned(),
            next: None,
            upcoming: Vec::new(),
            one_off: None,
        };
        let (players, playing) = Players::start(PlayersSetup {
            db: db.clone(),
            metrics,
            clock: clock.clone(),
            supervision: supervision.clone(),
            schedule: watch::channel(status).1,
            data_dir: data_dir.clone(),
        })
        .await
        .unwrap();
        Self {
            dir,
            db,
            clock,
            supervision,
            players,
            _playing: playing,
            _running: running,
        }
    }

    fn map(&self) -> (LiveMap, MapTask) {
        LiveMap::start(MapSetup {
            db: self.db.clone(),
            clock: self.clock.clone(),
            supervision: self.supervision.clone(),
            players: self.players.clone(),
            data_dir: self.dir.path().join("data"),
        })
    }

    /// A PNG header of `width` by `height`, written on the host, followed by some bytes.
    fn png(&self, name: &str, width: u32, height: u32) -> String {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png.extend_from_slice(&[7; 1000]);
        let path = self.dir.path().join(name);
        std::fs::write(&path, png).unwrap();
        path.to_string_lossy().into_owned()
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

fn problem(error: Error) -> serde_json::Value {
    match error {
        Error::Refused(error) => error.params["problem"].clone(),
        other => panic!("not a refusal: {other:?}"),
    }
}

#[tokio::test]
async fn an_image_on_the_host_is_copied_in_and_replaces_the_one_before() {
    let lab = Lab::new().await;
    let (map, task) = lab.map();
    let first = lab.png("atlas.png", 8192, 8192);
    let second = lab.png("satellite.png", 4096, 4096);

    let added = map.set_image(&owner(), &first).await.unwrap();
    let (kept, kind) = map.image_file().await.unwrap();
    let replaced = map.set_image(&owner(), &second).await.unwrap();
    let (now_kept, _) = map.image_file().await.unwrap();

    assert_eq!((added.width, added.height, added.bytes), (8192, 8192, 1024));
    assert_eq!(added.added_by, "ada");
    assert_eq!(kind, ImageKind::Png);
    assert!(!kept.exists(), "the first copy stays behind");
    assert!(now_kept.exists());
    assert_ne!(added.version, replaced.version);
    assert_eq!(map.view().await.unwrap().image, Some(replaced));
    map.remove_image(&owner()).await.unwrap();
    assert!(!now_kept.exists());
    assert_eq!(map.view().await.unwrap().image, None);
    task.close().await;
}

#[tokio::test]
async fn what_is_not_a_local_image_of_a_sane_size_is_refused() {
    let lab = Lab::new().await;
    let (map, task) = lab.map();
    let text = lab.dir.path().join("notes.txt");
    std::fs::write(&text, "not an image").unwrap();
    let huge = lab.png("huge.png", 20_000, 100);

    assert_eq!(
        problem(map.set_image(&owner(), "relative.png").await.unwrap_err()),
        "notLocal"
    );
    assert_eq!(
        problem(
            map.set_image(&owner(), &lab.dir.path().join("none.png").to_string_lossy())
                .await
                .unwrap_err()
        ),
        "missing"
    );
    assert_eq!(
        problem(
            map.set_image(&owner(), &text.to_string_lossy())
                .await
                .unwrap_err()
        ),
        "notAnImage"
    );
    assert_eq!(
        problem(map.set_image(&owner(), &huge).await.unwrap_err()),
        "tooLarge"
    );
    assert!(map.image_file().await.is_err());
    task.close().await;
}

#[tokio::test]
async fn settings_are_checked_before_they_are_kept() {
    let lab = Lab::new().await;
    let (map, task) = lab.map();
    let placed = MapSettings {
        imagery: MapImagery::Image,
        bounds: Some(MapBounds {
            west: -5640.0,
            east: 6470.0,
            south: -4140.0,
            north: 7970.0,
        }),
        new_hours: 5,
        veteran_hours: 50,
        every_seconds: 1,
    };

    let refused = map
        .change_settings(
            &owner(),
            MapSettings {
                veteran_hours: 5,
                ..placed.clone()
            },
        )
        .await;
    map.change_settings(&owner(), placed.clone()).await.unwrap();

    assert!(matches!(refused, Err(Error::Refused(error)) if error.code == ErrorCode::InvalidMapSettings));
    assert_eq!(map.settings().await.unwrap(), placed);
    task.close().await;
}

#[test]
fn players_go_from_the_map_once_no_position_came_for_a_lease() {
    let at: Timestamp = "2026-11-09T20:00:00Z".parse().unwrap();
    let player = MapPlayer {
        net_id: 3,
        player: Some(7),
        name: Some("kaito".to_owned()),
        label: None,
        total_ms: Some(1),
        x: 1.0,
        y: 2.0,
        z: 3.0,
        heading: 0.0,
    };
    let mut live = MapLive {
        players: vec![player],
        at: Some(at),
        waiting: 1,
        onesync: true,
    };

    assert!(!forget_stale(&mut live, at + jiff::SignedDuration::from_secs(29)));
    assert_eq!(live.players.len(), 1);
    assert!(forget_stale(&mut live, at + jiff::SignedDuration::from_secs(31)));
    assert_eq!((live.players.len(), live.waiting), (0, 0));
    assert!(!forget_stale(&mut live, at + jiff::SignedDuration::from_secs(60)));
}

#[tokio::test]
async fn the_map_counts_who_watches_it_until_they_leave() {
    let lab = Lab::new().await;
    let (map, task) = lab.map();

    let (snapshot, _, first) = map.watch();
    let (_, _, second) = map.watch();
    assert_eq!(*map.inner.watchers.subscribe().borrow(), 2);
    drop(first);
    drop(second);

    assert_eq!(*map.inner.watchers.subscribe().borrow(), 0);
    assert_eq!(snapshot.players, []);
    assert!(snapshot.onesync);
    task.close().await;
}
