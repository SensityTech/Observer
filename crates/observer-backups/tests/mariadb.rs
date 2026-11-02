//! A dump by the real `mariadb-dump` of a disposable MariaDB server, restored into a second
//! database and compared. It runs only with `OBSERVER_TEST_MARIADB_BIN` naming the `bin` folder of
//! an installation: its server is started on a data folder of its own in a temporary folder, with
//! no option file but its own, on a free port of the loopback. It never connects to the host's own
//! server.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use observer_backups::dump::{self, DumpRequest};
use observer_domain::backups::{DatabaseEngine, parse_connection_string};
use tokio_util::sync::CancellationToken;

const ROOT_PASSWORD: &str = "disposable-root";

struct Server {
    child: Child,
    port: u16,
    bin: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool(bin: &Path, name: &str) -> PathBuf {
    bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn free_port() -> u16 {
    let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert_ne!(port, 3306, "never the host's own server");
    port
}

fn start(bin: &Path, data: &Path) -> Server {
    let port = free_port();
    let installed = if cfg!(windows) {
        Command::new(tool(bin, "mariadb-install-db"))
            .arg(format!("--datadir={}", data.display()))
            .arg(format!("--password={ROOT_PASSWORD}"))
            .arg(format!("--port={port}"))
            .stdout(Stdio::null())
            .status()
            .unwrap()
    } else {
        Command::new(tool(bin, "mariadb-install-db"))
            .args(["--no-defaults", "--auth-root-authentication-method=normal"])
            .arg(format!("--datadir={}", data.display()))
            .stdout(Stdio::null())
            .status()
            .unwrap()
    };
    assert!(installed.success(), "the data folder could not be made");
    let mut server = Command::new(tool(bin, "mariadbd"));
    if cfg!(windows) {
        server.arg(format!("--defaults-file={}", data.join("my.ini").display()));
    } else {
        server
            .arg("--no-defaults")
            .arg(format!("--datadir={}", data.display()))
            .arg(format!("--socket={}", data.join("mysqld.sock").display()));
    }
    let child = server
        .arg(format!("--port={port}"))
        .arg("--bind-address=127.0.0.1")
        .arg("--skip-name-resolve")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = Server {
        child,
        port,
        bin: bin.to_path_buf(),
    };
    let started = Instant::now();
    while TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the server did not start"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    if !cfg!(windows) {
        server.sql(
            "ALTER USER 'root'@'localhost' IDENTIFIED BY 'disposable-root'",
            None,
            false,
        );
    }
    server
}

impl Server {
    /// Runs SQL as root, with or without the password a fresh Linux install does not have yet.
    fn sql(&self, sql: &str, database: Option<&str>, with_password: bool) -> String {
        let mut command = Command::new(tool(&self.bin, "mariadb"));
        command
            .arg("--no-defaults")
            .args([
                "--protocol=tcp",
                "--host=127.0.0.1",
                "--user=root",
                "--batch",
                "--skip-column-names",
            ])
            .arg(format!("--port={}", self.port))
            .arg(format!("--execute={sql}"));
        if with_password || cfg!(windows) {
            command.env("MYSQL_PWD", ROOT_PASSWORD);
        }
        if let Some(database) = database {
            command.arg(database);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn restore(&self, sql: &[u8], database: &str) {
        use std::io::Write;
        let mut child = Command::new(tool(&self.bin, "mariadb"))
            .arg("--no-defaults")
            .args(["--protocol=tcp", "--host=127.0.0.1", "--user=root"])
            .arg(format!("--port={}", self.port))
            .arg(database)
            .env("MYSQL_PWD", ROOT_PASSWORD)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(sql).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

const SCHEMA: &str = "
    CREATE TABLE users (identifier VARCHAR(60) PRIMARY KEY, name VARCHAR(50), money INT, avatar BLOB)
        ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
    CREATE TABLE player_logs (id INT AUTO_INCREMENT PRIMARY KEY, line TEXT) ENGINE=Aria;
    INSERT INTO users VALUES ('license:1', 'Zoé 🚓', 1500, X'00FF10'), ('license:2', 'O''Brien\\n', -3, NULL);
    INSERT INTO player_logs (line) VALUES ('joined'), ('left');
    CREATE TRIGGER users_money BEFORE UPDATE ON users FOR EACH ROW SET NEW.money = GREATEST(NEW.money, -100);
    CREATE PROCEDURE pay(who VARCHAR(60), amount INT) UPDATE users SET money = money + amount WHERE identifier = who;
";

const CONTENTS: &str = "SELECT identifier, name, money, HEX(avatar) FROM users ORDER BY identifier";

#[tokio::test]
async fn a_real_dump_restores_into_the_same_database() {
    let Some(bin) = std::env::var_os("OBSERVER_TEST_MARIADB_BIN").map(PathBuf::from) else {
        eprintln!("OBSERVER_TEST_MARIADB_BIN is not set: no real MariaDB here");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let server = start(&bin, &data);
    server.sql(
        "CREATE DATABASE es_extended CHARACTER SET utf8mb4; CREATE DATABASE restored CHARACTER SET utf8mb4",
        None,
        true,
    );
    server.sql(SCHEMA, Some("es_extended"), true);

    let client = dump::find(DatabaseEngine::MariaDb, Some(&bin)).await.unwrap();
    assert!(
        client.version.ends_with("-MariaDB") && !client.version.contains(['\\', '/']),
        "{}",
        client.version
    );
    let connection = parse_connection_string(&format!(
        "mysql://root:{ROOT_PASSWORD}@127.0.0.1:{}/es_extended?charset=utf8mb4",
        server.port
    ))
    .unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();

    let facts = dump::test(&client, &connection, &work, "test").await.unwrap();
    assert_eq!((facts.tables, facts.loose_tables), (2, 1));
    assert!(
        facts.server_version.contains("MariaDB"),
        "{}",
        facts.server_version
    );

    let output = work.join("1.partial");
    let written = dump::dump(DumpRequest {
        client: &client,
        connection: &connection,
        work: &work,
        name: "1",
        output: &output,
        progress: Arc::default(),
        cancel: CancellationToken::new(),
    })
    .await
    .unwrap();

    assert_eq!(written.inspection.loose_tables, ["player_logs"]);
    assert_eq!(
        std::fs::read_dir(&work).unwrap().count(),
        1,
        "the option file is gone"
    );
    let mut sql = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::MultiGzDecoder::new(std::fs::File::open(&output).unwrap()),
        &mut sql,
    )
    .unwrap();
    server.restore(&sql, "restored");
    assert_eq!(
        server.sql(CONTENTS, Some("restored"), true),
        server.sql(CONTENTS, Some("es_extended"), true)
    );
    assert_eq!(
        server.sql("SELECT COUNT(*) FROM player_logs", Some("restored"), true),
        "2\n"
    );
    server.sql("CALL pay('license:2', -500)", Some("restored"), true);
    assert_eq!(
        server.sql(
            "SELECT money FROM users WHERE identifier = 'license:2'",
            Some("restored"),
            true
        ),
        "-100\n"
    );

    let wrong = parse_connection_string(&format!(
        "mysql://root:not-it@127.0.0.1:{}/es_extended",
        server.port
    ))
    .unwrap();
    let refused = dump::dump(DumpRequest {
        client: &client,
        connection: &wrong,
        work: &work,
        name: "2",
        output: &work.join("2.partial"),
        progress: Arc::default(),
        cancel: CancellationToken::new(),
    })
    .await
    .unwrap_err();
    assert_eq!(refused.error, observer_domain::backups::BackupError::DumpFailed);
    assert!(
        refused
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("Access denied"),
        "{refused:?}"
    );
    assert!(!work.join("2.partial").exists());
}
