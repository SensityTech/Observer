use observer_domain::backups::SftpAuth;

use super::*;
use crate::database::memory;

const SERVER: &str = "default";

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn local() -> DestinationTarget {
    DestinationTarget::Local {
        folder: "/var/backups/fivem".to_owned(),
    }
}

fn add(conn: &Connection, name: &str, target: &DestinationTarget) -> i64 {
    insert_destination(
        conn,
        &DestinationRow {
            server_id: SERVER,
            name,
            enabled: true,
            target,
            public_key: None,
            retention: Retention::default(),
            at: at("2026-11-02T08:00:00Z"),
        },
    )
    .unwrap()
    .unwrap()
}

fn start(conn: &Connection) -> i64 {
    insert_run(
        conn,
        &NewRun {
            server_id: SERVER,
            trigger: BackupTrigger::Manual,
            actor: Some("owner"),
            engine: DatabaseEngine::MariaDb,
            started_at: at("2026-11-02T09:00:00Z"),
        },
    )
    .unwrap()
}

fn succeed(conn: &Connection, run: i64) {
    let finished = FinishedRun {
        at: at("2026-11-02T09:00:30Z"),
        outcome: BackupOutcome::Succeeded,
        sql_bytes: Some(1_000_000),
        file_bytes: Some(120_000),
        sql_sha256: Some(&"a".repeat(64)),
        file_sha256: Some(&"b".repeat(64)),
        server_started: false,
        loose_tables: &["player_logs".to_owned()],
        loose_table_count: 1,
        error: None,
        error_detail: None,
    };
    assert!(finish_run(conn, run, &finished).unwrap());
}

#[test]
fn settings_are_saved_under_their_version() {
    let conn = memory();
    let mut settings = BackupSettings {
        times: vec!["04:00".into()],
        source: SourceKind::Manual,
        manual: Some(ManualSource {
            host: "127.0.0.1".into(),
            port: 3307,
            user: "backup".into(),
            database: "es_extended".into(),
        }),
        ..BackupSettings::default()
    };

    assert_eq!(settings_of(&conn), None);
    assert_eq!(
        save_settings(&conn, SERVER, &settings, None, at("2026-11-02T08:00:00Z")).unwrap(),
        Some(1)
    );
    assert_eq!(
        save_settings(&conn, SERVER, &settings, None, at("2026-11-02T08:00:00Z")).unwrap(),
        None
    );

    settings.before_restarts = BeforeRestarts::AllRestarts;
    assert_eq!(
        save_settings(&conn, SERVER, &settings, Some(2), at("2026-11-02T08:01:00Z")).unwrap(),
        None
    );
    assert_eq!(
        save_settings(&conn, SERVER, &settings, Some(1), at("2026-11-02T08:01:00Z")).unwrap(),
        Some(2)
    );
    assert_eq!(
        settings_of(&conn),
        Some(StoredBackupSettings { settings, version: 2 })
    );
}

fn settings_of(conn: &Connection) -> Option<StoredBackupSettings> {
    settings(conn, SERVER).unwrap()
}

#[test]
fn the_source_password_is_stored_apart_and_replaced() {
    let conn = memory();
    let settings = BackupSettings::default();
    save_settings(&conn, SERVER, &settings, None, at("2026-11-02T08:00:00Z")).unwrap();

    assert!(!has_password(&conn, SERVER).unwrap());
    set_password(&conn, SERVER, &Secret::new("first")).unwrap();
    set_password(&conn, SERVER, &Secret::new("second")).unwrap();

    assert!(has_password(&conn, SERVER).unwrap());
    assert_eq!(password(&conn, SERVER).unwrap(), Some(Secret::new("second")));
}

#[test]
fn destinations_keep_their_names_unique_and_their_secrets_apart() {
    let conn = memory();
    let sftp = DestinationTarget::Sftp {
        host: "backup.example.net".into(),
        port: 22,
        user: "fivem".into(),
        folder: "dumps".into(),
        auth: SftpAuth::Password,
        host_key: None,
    };
    let first = add(&conn, "Offsite", &sftp);
    let row = DestinationRow {
        server_id: SERVER,
        name: "Offsite",
        enabled: true,
        target: &local(),
        public_key: None,
        retention: Retention::default(),
        at: at("2026-11-02T08:00:00Z"),
    };
    assert_eq!(insert_destination(&conn, &row).unwrap(), None);
    set_destination_secret(&conn, first, &Secret::new("hunter2")).unwrap();

    let stored = destination(&conn, SERVER, first).unwrap().unwrap();

    assert_eq!(stored.target, sftp);
    assert!(stored.has_secret);
    assert_eq!(stored.version, 1);
    assert_eq!(
        destination_secret(&conn, first).unwrap(),
        Some(Secret::new("hunter2"))
    );
    assert_eq!(destination(&conn, "other", first).unwrap(), None);
}

#[test]
fn a_destination_changes_over_its_version_and_never_onto_another_name() {
    let conn = memory();
    let first = add(&conn, "Here", &local());
    add(&conn, "There", &local());
    let mut row = DestinationRow {
        server_id: SERVER,
        name: "There",
        enabled: false,
        target: &local(),
        public_key: None,
        retention: Retention {
            keep_last: None,
            keep_days: Some(30),
        },
        at: at("2026-11-02T08:05:00Z"),
    };

    assert_eq!(
        update_destination(&conn, first, 1, &row).unwrap(),
        Saved::NameTaken
    );
    row.name = "Here again";
    assert_eq!(
        update_destination(&conn, first, 2, &row).unwrap(),
        Saved::Conflict
    );
    assert_eq!(update_destination(&conn, first, 1, &row).unwrap(), Saved::Done);

    let stored = destination(&conn, SERVER, first).unwrap().unwrap();
    assert_eq!(stored.name, "Here again");
    assert_eq!(stored.version, 2);
    assert_eq!(stored.retention.keep_days, Some(30));
    assert!(!stored.enabled);
}

#[test]
fn a_run_ends_once_with_its_file_and_copies() {
    let conn = memory();
    let here = add(&conn, "Here", &local());
    let run = start(&conn);
    set_run_source(
        &conn,
        run,
        &RunSource {
            database: "es_extended",
            host: "localhost:3306",
            client: "mariadb-dump 11.7.2-MariaDB",
            file_name: "es_extended-20261102T090000Z-1.sql.gz",
        },
    )
    .unwrap();
    let copy = insert_copy(
        &conn,
        run,
        here,
        "es_extended-20261102T090000Z-1.sql.gz",
        at("2026-11-02T09:00:20Z"),
    )
    .unwrap();
    copy_stored(&conn, copy, 120_000, at("2026-11-02T09:00:25Z")).unwrap();
    succeed(&conn, run);

    let stored = super::run(&conn, SERVER, run).unwrap().unwrap();

    assert_eq!(stored.outcome, Some(BackupOutcome::Succeeded));
    assert_eq!(stored.database.as_deref(), Some("es_extended"));
    assert_eq!(stored.loose_tables, ["player_logs"]);
    assert_eq!(stored.copies.len(), 1);
    assert_eq!(stored.copies[0].state, CopyState::Stored);
    assert_eq!(stored.copies[0].destination, "Here");
    assert_eq!(stored.copies[0].bytes, Some(120_000));
    let destination = destination(&conn, SERVER, here).unwrap().unwrap();
    assert_eq!((destination.stored, destination.stored_bytes), (1, 120_000));

    let again = FinishedRun {
        at: at("2026-11-02T09:01:00Z"),
        outcome: BackupOutcome::Failed,
        sql_bytes: None,
        file_bytes: None,
        sql_sha256: None,
        file_sha256: None,
        server_started: false,
        loose_tables: &[],
        loose_table_count: 0,
        error: Some(BackupError::Io),
        error_detail: None,
    };
    assert!(!finish_run(&conn, run, &again).unwrap());
}

#[test]
fn runs_left_unfinished_are_ended_as_interrupted_with_their_uploads() {
    let conn = memory();
    let here = add(&conn, "Here", &local());
    let done = start(&conn);
    succeed(&conn, done);
    let left = start(&conn);
    let copy = insert_copy(&conn, left, here, "x.sql.gz", at("2026-11-02T09:00:20Z")).unwrap();

    let ended = end_unfinished_runs(&conn, at("2026-11-02T10:00:00Z")).unwrap();

    assert_eq!(
        ended,
        [Interrupted {
            id: left,
            trigger: BackupTrigger::Manual,
            actor: Some("owner".into()),
            database: None,
        }]
    );
    let interrupted = run(&conn, SERVER, left).unwrap().unwrap();
    assert_eq!(interrupted.outcome, Some(BackupOutcome::Interrupted));
    assert_eq!(interrupted.copies[0].state, CopyState::Failed);
    assert_eq!(interrupted.copies[0].error, Some(BackupError::Interrupted));
    assert_eq!(copy, interrupted.copies[0].id);
}

#[test]
fn pages_of_runs_go_back_from_the_newest() {
    let conn = memory();
    let ids: Vec<i64> = (0..5).map(|_| start(&conn)).collect();

    let first = runs(&conn, SERVER, None, 2).unwrap();
    let second = runs(&conn, SERVER, Some(first[1].id), 2).unwrap();

    assert_eq!(
        first.iter().map(|run| run.id).collect::<Vec<_>>(),
        [ids[4], ids[3]]
    );
    assert_eq!(
        second.iter().map(|run| run.id).collect::<Vec<_>>(),
        [ids[2], ids[1]]
    );
}

#[test]
fn deleted_copies_leave_the_retention_and_old_runs_without_copies_are_forgotten() {
    let conn = memory();
    let here = add(&conn, "Here", &local());
    let run = start(&conn);
    let copy = insert_copy(&conn, run, here, "x.sql.gz", at("2026-11-02T09:00:20Z")).unwrap();
    copy_stored(&conn, copy, 10, at("2026-11-02T09:00:25Z")).unwrap();
    succeed(&conn, run);

    assert_eq!(stored_copies(&conn, here).unwrap().len(), 1);
    assert_eq!(forget_runs(&conn, SERVER, at("2027-06-01T00:00:00Z")).unwrap(), 0);

    copy_deleted(&conn, copy, DeleteReason::Retention, at("2026-11-03T09:00:00Z")).unwrap();

    assert_eq!(stored_copies(&conn, here).unwrap(), []);
    let kept = super::copy(&conn, SERVER, copy).unwrap().unwrap();
    assert_eq!(kept.state, CopyState::Deleted);
    assert_eq!(forget_runs(&conn, SERVER, at("2026-11-02T09:00:00Z")).unwrap(), 0);
    assert_eq!(forget_runs(&conn, SERVER, at("2027-06-01T00:00:00Z")).unwrap(), 1);
}

#[test]
fn deleting_a_destination_forgets_its_secret_and_copies() {
    let conn = memory();
    let here = add(&conn, "Here", &local());
    set_destination_secret(&conn, here, &Secret::new("x")).unwrap();
    let run = start(&conn);
    insert_copy(&conn, run, here, "x.sql.gz", at("2026-11-02T09:00:20Z")).unwrap();

    assert!(delete_destination(&conn, SERVER, here).unwrap());

    assert_eq!(destination_secret(&conn, here).unwrap(), None);
    assert_eq!(super::run(&conn, SERVER, run).unwrap().unwrap().copies, []);
    assert!(!delete_destination(&conn, SERVER, here).unwrap());
}
