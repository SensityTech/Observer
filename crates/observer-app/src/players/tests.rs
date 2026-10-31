use std::net::SocketAddr;

use observer_domain::schedule::ScheduleSettings;
use observer_store::metrics::runs::{self, StartCause};
use observer_store::metrics::sessions::NewSession;
use tempfile::TempDir;

use super::*;
use crate::{Accounts, MetricsSetup, PasswordCost, Running, Setup};

const ADA: &str = "license:0123456789abcdef0123456789abcdef01234567";
/// A run of an earlier Observer, and when its last sample was taken.
const EARLIER: RunKey = RunKey {
    observer_started: 1_761_900_000_000,
    generation: 1,
};
const LAST_SAMPLE_MS: i64 = 1_761_900_000_000 + 2 * 3_600_000;

/// Observer's databases in a temporary folder, with the services the players service stands on.
struct Lab {
    dir: TempDir,
    db: Database,
    metrics: Metrics,
    supervision: Supervision,
    _running: Running,
}

impl Lab {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2026-10-31T10:00:00Z".parse().unwrap());
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
            clock,
        })
        .await
        .unwrap();
        Self {
            dir,
            db,
            metrics,
            supervision,
            _running: running,
        }
    }

    async fn players(&self) -> (Players, PlayersTask) {
        let status = ScheduleStatus {
            settings: ScheduleSettings::default(),
            version: None,
            zone: "Europe/Paris".to_owned(),
            next: None,
            upcoming: Vec::new(),
            one_off: None,
        };
        Players::start(PlayersSetup {
            db: self.db.clone(),
            metrics: self.metrics.clone(),
            clock: Clock::manual("2026-10-31T10:00:00Z".parse().unwrap()),
            supervision: self.supervision.clone(),
            schedule: watch::channel(status).1,
            data_dir: self.dir.path().join("data"),
        })
        .await
        .unwrap()
    }

    /// Ada as a confirmed join wrote her, an hour into a run of an earlier Observer that died with
    /// her session open: the run was closed as lost at its last sample, her session was not.
    async fn left_open(&self) -> i64 {
        let at = Timestamp::from_millisecond(EARLIER.observer_started + 3_600_000).unwrap();
        let player = self
            .db
            .write(move |tx| -> Result<i64, StoreError> {
                let (player, _) = stored::record_seen(
                    tx,
                    &Seen {
                        license: ADA,
                        name: Some("Ada"),
                        others: &[],
                        at,
                        pseudonym: [7; 16],
                    },
                )?;
                stored::open_credit(tx, [9; 16], player, at)?;
                Ok(player)
            })
            .await
            .unwrap();
        self.metrics
            .database()
            .write(move |tx| -> Result<(), StoreError> {
                runs::start(tx, EARLIER, EARLIER.observer_started, StartCause::Requested)?;
                stored_sessions::insert(
                    tx,
                    &NewSession {
                        id: 1,
                        run: EARLIER,
                        net_id: 3,
                        joined_ms: Some(at.as_millisecond()),
                        seen_ms: at.as_millisecond(),
                    },
                )?;
                stored_sessions::attach(tx, 1, [7; 16], [9; 16])?;
                tx.execute(
                    "UPDATE server_run SET ended_ms = ?1, end_cause = 'lost'",
                    [LAST_SAMPLE_MS],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        player
    }

    async fn play_ms(&self, player: i64) -> Option<i64> {
        self.db
            .read(move |conn| stored::profile(conn, player, ""))
            .await
            .unwrap()
            .map(|profile| profile.summary.play_ms)
    }
}

#[tokio::test]
async fn a_session_a_dead_observer_left_open_is_credited_once_and_closed_as_lost() {
    let lab = Lab::new().await;
    let player = lab.left_open().await;

    let (_, playing) = lab.players().await;
    playing.close().await;
    let credited = lab.play_ms(player).await;
    let (_, playing) = lab.players().await;
    playing.close().await;

    assert_eq!(credited, Some(3_600_000));
    assert_eq!(lab.play_ms(player).await, Some(3_600_000));
    let closed = lab
        .metrics
        .database()
        .read(|conn| {
            conn.query_row(
                "SELECT category, kind, ended_ms, length_ms, synthesized FROM player_session",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            )
            .map_err(StoreError::from)
        })
        .await
        .unwrap();
    assert_eq!(
        closed,
        (
            "unknown".to_owned(),
            "lost".to_owned(),
            LAST_SAMPLE_MS,
            3_600_000,
            true
        )
    );
    assert_eq!(lab.db.read(stored::open_records).await.unwrap(), []);
}

#[tokio::test]
async fn a_session_whose_credit_was_written_is_not_credited_again() {
    let lab = Lab::new().await;
    let player = lab.left_open().await;
    // Observer died between the two transactions: the state database credited and deleted the
    // record, the metrics database never closed the session.
    lab.db.write(|tx| stored::drop_record(tx, [9; 16])).await.unwrap();

    let (_, playing) = lab.players().await;
    playing.close().await;

    assert_eq!(lab.play_ms(player).await, Some(0));
    let open: i64 = lab
        .metrics
        .database()
        .read(|conn| {
            conn.query_row(
                "SELECT count(*) FROM player_session WHERE ended_ms IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
        })
        .await
        .unwrap();
    assert_eq!(open, 0);
}

#[tokio::test]
async fn a_forgotten_licence_a_restored_backup_brought_back_is_forgotten_again_at_start() {
    let lab = Lab::new().await;
    let (_, playing) = lab.players().await;
    playing.close().await;
    let player = lab.left_open().await;
    let key = lab
        .db
        .write(|tx| stored::fingerprint_key(tx, [0; 32]))
        .await
        .unwrap();
    let fingerprint = Fingerprints::new(key).license(ADA);
    fs::create_dir_all(lab.dir.path().join("data")).unwrap();
    append_forgotten(
        &lab.dir.path().join("data").join(FORGOTTEN_FILE),
        fingerprint,
        "2026-10-31T09:00:00Z".parse().unwrap(),
    )
    .unwrap();

    let (_, playing) = lab.players().await;
    playing.close().await;

    assert_eq!(lab.play_ms(player).await, None);
    let pseudonyms: i64 = lab
        .metrics
        .database()
        .read(|conn| {
            conn.query_row("SELECT count(player) FROM player_session", [], |row| row.get(0))
                .map_err(StoreError::from)
        })
        .await
        .unwrap();
    assert_eq!(pseudonyms, 0);
    assert_eq!(
        lab.db
            .read(stored::forgotten)
            .await
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        [&fingerprint]
    );
}

#[test]
fn the_forgotten_file_reads_back_what_was_written_and_skips_what_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FORGOTTEN_FILE);
    let at: Timestamp = "2026-10-31T09:00:00Z".parse().unwrap();

    append_forgotten(&file, [1; 16], at).unwrap();
    fs::write(&file, format!("{}garbage\n", fs::read_to_string(&file).unwrap())).unwrap();
    append_forgotten(&file, [2; 16], at).unwrap();

    assert_eq!(read_forgotten(&file).unwrap(), [([1; 16], at), ([2; 16], at)]);
    assert_eq!(read_forgotten(&dir.path().join("none")).unwrap(), []);
}
