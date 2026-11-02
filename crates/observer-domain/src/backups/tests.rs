use super::*;

const FINE: [(DestinationField, FieldProblem); 0] = [];
const NO_ISSUES: [BackupSettingsIssue; 0] = [];

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn local(folder: &str) -> DestinationDraft {
    DestinationDraft {
        name: "This host".to_owned(),
        enabled: true,
        target: DestinationTarget::Local {
            folder: folder.to_owned(),
        },
        retention: Retention::default(),
        secret: None,
    }
}

fn s3(endpoint: &str) -> DestinationDraft {
    DestinationDraft {
        name: "Bucket".to_owned(),
        enabled: true,
        target: DestinationTarget::S3 {
            endpoint: endpoint.to_owned(),
            region: "auto".to_owned(),
            bucket: "sensity-backups".to_owned(),
            prefix: "database/".to_owned(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_owned(),
            path_style: true,
        },
        retention: Retention::default(),
        secret: Some(Secret::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY")),
    }
}

fn absolute() -> &'static str {
    if cfg!(windows) {
        r"D:\backups"
    } else {
        "/var/backups/fivem"
    }
}

fn problems(draft: &DestinationDraft, has_secret: bool) -> Vec<(DestinationField, FieldProblem)> {
    draft
        .issues(has_secret)
        .into_iter()
        .map(|issue| (issue.field, issue.problem))
        .collect()
}

#[test]
fn the_defaults_back_up_before_scheduled_restarts_from_the_cfg() {
    let settings = BackupSettings::default();

    assert_eq!(settings.before_restarts, BeforeRestarts::Scheduled);
    assert_eq!(settings.source, SourceKind::Cfg);
    assert_eq!(settings.restart_wait_seconds, 60);
    assert_eq!(settings.issues(false), NO_ISSUES);
}

#[test]
fn times_follow_the_restart_schedule_s_rules() {
    let settings = BackupSettings {
        times: vec!["03:00".into(), "3:00".into(), "03:00".into(), "03:20".into()],
        ..BackupSettings::default()
    };

    let issues = settings.issues(false);

    assert_eq!(
        issues,
        [
            BackupSettingsIssue {
                field: BackupField::Time,
                index: Some(1),
                problem: FieldProblem::InvalidTime,
            },
            BackupSettingsIssue {
                field: BackupField::Time,
                index: Some(2),
                problem: FieldProblem::Duplicate,
            },
            BackupSettingsIssue {
                field: BackupField::Time,
                index: Some(3),
                problem: FieldProblem::TooClose,
            },
        ]
    );
}

#[test]
fn a_manual_source_needs_its_fields_and_a_password() {
    let settings = BackupSettings {
        source: SourceKind::Manual,
        manual: Some(ManualSource {
            host: "db host".into(),
            port: 0,
            user: String::new(),
            database: "es_extended".into(),
        }),
        restart_wait_seconds: 601,
        client_folder: Some("bin".into()),
        ..BackupSettings::default()
    };

    let fields: Vec<_> = settings
        .issues(false)
        .into_iter()
        .map(|issue| (issue.field, issue.problem))
        .collect();

    assert_eq!(
        fields,
        [
            (BackupField::RestartWaitSeconds, FieldProblem::OutOfRange),
            (BackupField::ClientFolder, FieldProblem::NotAbsolute),
            (BackupField::Host, FieldProblem::Invalid),
            (BackupField::Port, FieldProblem::OutOfRange),
            (BackupField::User, FieldProblem::Required),
            (BackupField::Password, FieldProblem::Required),
        ]
    );
}

#[test]
fn a_manual_source_kept_while_the_cfg_is_used_needs_no_password() {
    let settings = BackupSettings {
        manual: Some(ManualSource {
            host: "127.0.0.1".into(),
            port: 3306,
            user: "backup".into(),
            database: "es_extended".into(),
        }),
        ..BackupSettings::default()
    };

    assert_eq!(settings.issues(false), NO_ISSUES);
}

#[test]
fn a_local_destination_takes_an_absolute_folder_and_no_secret() {
    assert_eq!(problems(&local(absolute()), false), FINE);
    assert_eq!(
        problems(&local("backups"), false),
        [(DestinationField::Folder, FieldProblem::NotAbsolute)]
    );
    let climbing = format!(
        "{}{}..{}server-data",
        absolute(),
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    );
    assert_eq!(
        problems(&local(&climbing), false),
        [(DestinationField::Folder, FieldProblem::Invalid)]
    );

    let mut with_secret = local(absolute());
    with_secret.secret = Some(Secret::new("hunter2"));
    assert_eq!(
        problems(&with_secret, false),
        [(DestinationField::Secret, FieldProblem::NotAccepted)]
    );
}

#[test]
fn an_s3_destination_needs_https_except_to_this_host() {
    assert_eq!(problems(&s3("https://abc.r2.cloudflarestorage.com"), false), FINE);
    assert_eq!(problems(&s3("http://127.0.0.1:9000/"), false), FINE);
    assert_eq!(problems(&s3("http://localhost:9000"), false), FINE);
    assert_eq!(
        problems(&s3("http://minio.lan:9000"), false),
        [(DestinationField::Endpoint, FieldProblem::NotEncrypted)]
    );
    for invalid in [
        "s3.amazonaws.com",
        "https://",
        "https://host/bucket",
        "https://user@host",
    ] {
        assert_eq!(
            problems(&s3(invalid), false),
            [(DestinationField::Endpoint, FieldProblem::Invalid)],
            "{invalid}"
        );
    }
}

#[test]
fn a_secret_is_required_once_and_kept_after() {
    let mut draft = s3("https://s3.eu-west-3.amazonaws.com");
    draft.secret = None;

    assert_eq!(
        problems(&draft, false),
        [(DestinationField::Secret, FieldProblem::Required)]
    );
    assert_eq!(problems(&draft, true), FINE);

    draft.secret = Some(Secret::new(""));
    assert_eq!(
        problems(&draft, true),
        [(DestinationField::Secret, FieldProblem::Required)]
    );
}

#[test]
fn an_sftp_key_destination_makes_its_own_key() {
    let draft = DestinationDraft {
        name: "Offsite".into(),
        enabled: true,
        target: DestinationTarget::Sftp {
            host: "backup.example.net".into(),
            port: 22,
            user: "fivem".into(),
            folder: "dumps".into(),
            auth: SftpAuth::Key,
            host_key: None,
        },
        retention: Retention {
            keep_last: None,
            keep_days: Some(30),
        },
        secret: None,
    };

    assert_eq!(problems(&draft, false), FINE);
}

#[test]
fn a_retention_keeps_something_within_its_ranges() {
    let mut draft = local(absolute());
    draft.retention = Retention {
        keep_last: None,
        keep_days: None,
    };
    assert_eq!(
        problems(&draft, false),
        [(DestinationField::Retention, FieldProblem::Required)]
    );

    draft.retention = Retention {
        keep_last: Some(0),
        keep_days: Some(3651),
    };
    assert_eq!(
        problems(&draft, false),
        [
            (DestinationField::KeepLast, FieldProblem::OutOfRange),
            (DestinationField::KeepDays, FieldProblem::OutOfRange),
        ]
    );
}

fn copies(times: &[&str]) -> Vec<StoredCopy> {
    times
        .iter()
        .zip(1..)
        .map(|(time, id)| StoredCopy {
            id,
            stored_at: at(time),
        })
        .collect()
}

#[test]
fn retention_by_count_lets_the_oldest_go() {
    let stored = copies(&[
        "2026-11-01T06:00:00Z",
        "2026-11-01T18:00:00Z",
        "2026-11-02T06:00:00Z",
        "2026-11-02T18:00:00Z",
    ]);
    let retention = Retention {
        keep_last: Some(2),
        keep_days: None,
    };

    assert_eq!(expired(&stored, retention, at("2026-11-03T00:00:00Z")), [2, 1]);
}

#[test]
fn retention_keeps_what_either_rule_keeps() {
    let stored = copies(&[
        "2026-10-01T06:00:00Z",
        "2026-10-30T06:00:00Z",
        "2026-11-02T06:00:00Z",
    ]);
    let retention = Retention {
        keep_last: Some(1),
        keep_days: Some(7),
    };

    assert_eq!(expired(&stored, retention, at("2026-11-03T00:00:00Z")), [1]);
}

#[test]
fn retention_never_lets_the_newest_copy_go() {
    let stored = copies(&["2025-01-01T06:00:00Z", "2025-01-02T06:00:00Z"]);
    let retention = Retention {
        keep_last: None,
        keep_days: Some(1),
    };

    assert_eq!(expired(&stored, retention, at("2026-11-03T00:00:00Z")), [1]);
    assert_eq!(
        expired(&stored[..1], retention, at("2026-11-03T00:00:00Z")),
        Vec::<i64>::new()
    );
}

#[test]
fn an_address_connection_string_is_read_with_its_escapes() {
    let connection =
        parse_connection_string("mysql://fivem:p%40ss%3Aword@db.local:3307/es_extended?charset=utf8mb4")
            .unwrap();

    assert_eq!(connection.host, "db.local");
    assert_eq!(connection.port, 3307);
    assert_eq!(connection.user, "fivem");
    assert_eq!(connection.password.expose(), "p@ss:word");
    assert_eq!(connection.database, "es_extended");
    assert_eq!(connection.socket, None);
}

#[test]
fn an_address_without_port_or_password_takes_the_defaults() {
    let connection = parse_connection_string("mariadb://root@localhost/qbcore").unwrap();

    assert_eq!((connection.host.as_str(), connection.port), ("localhost", 3306));
    assert_eq!(connection.password.expose(), "");

    let v6 = parse_connection_string("mysql://root:pw@[::1]:3310/qbcore").unwrap();
    assert_eq!((v6.host.as_str(), v6.port), ("::1", 3310));
}

#[test]
fn key_value_pairs_are_read_with_their_synonyms() {
    let connection = parse_connection_string(
        "Server=127.0.0.1; Port=3306; User Id=fivem; Pwd=a=b; Initial Catalog=es_extended; charset=utf8mb4",
    )
    .unwrap();

    assert_eq!(connection.host, "127.0.0.1");
    assert_eq!(connection.user, "fivem");
    assert_eq!(connection.password.expose(), "a=b");
    assert_eq!(connection.database, "es_extended");

    let socket =
        parse_connection_string("user=fivem;password=x;db=qb;socketPath=/run/mysqld/mysqld.sock").unwrap();
    assert_eq!(socket.socket.as_deref(), Some("/run/mysqld/mysqld.sock"));
    assert_eq!(socket.host, "localhost");
}

#[test]
fn a_connection_string_without_database_or_with_a_bad_port_is_refused() {
    assert_eq!(
        parse_connection_string("mysql://root:pw@localhost/"),
        Err(ConnectionProblem::NoDatabase)
    );
    assert_eq!(
        parse_connection_string("host=localhost;user=root"),
        Err(ConnectionProblem::NoDatabase)
    );
    assert_eq!(
        parse_connection_string("mysql://root@localhost:99999/es"),
        Err(ConnectionProblem::Port)
    );
    assert_eq!(
        parse_connection_string("es_extended"),
        Err(ConnectionProblem::Shape)
    );
}

#[test]
fn a_connection_prints_without_its_password() {
    let connection = parse_connection_string("mysql://root:hunter2@localhost/es").unwrap();

    let printed = format!("{connection:?}");

    assert!(!printed.contains("hunter2"), "{printed}");
    assert_eq!(format!("{:?}", connection.password), "[REDACTED]");
}

#[test]
fn file_names_keep_safe_characters_and_say_when() {
    assert_eq!(
        file_name("es_extended", at("2026-11-02T04:00:01Z"), 42),
        "es_extended-20261102T040001Z-42.sql.gz"
    );
    assert_eq!(
        file_name("../qb core", at("2026-11-02T04:00:01Z"), 7),
        "___qb_core-20261102T040001Z-7.sql.gz"
    );
}

#[test]
fn details_lose_their_control_characters_and_their_tail() {
    assert_eq!(detail("Access denied\r\n"), "Access denied");
    assert_eq!(detail(&"x".repeat(600)).len(), MAX_DETAIL_CHARS);
}

#[test]
fn names_on_the_wire_and_in_the_store_round_trip() {
    for value in BackupError::ALL {
        assert_eq!(BackupError::from_name(value.as_str()), Some(value));
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::Value::String(value.as_str().to_owned())
        );
    }
    for value in BackupOutcome::ALL {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::Value::String(value.as_str().to_owned())
        );
    }
    for value in BeforeRestarts::ALL {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::Value::String(value.as_str().to_owned())
        );
    }
}
