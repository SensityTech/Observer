//! The MySQL family's driver, for MariaDB and MySQL alike: finding the engine's own tools on the
//! host, the option file that carries the password (never argv), the dump with
//! `--single-transaction` streamed into the archive, and the connection test.
//!
//! What a later engine brings is its own module of this shape: its tools, its credentials file,
//! its command, the start and end of a complete dump.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use observer_domain::backups::{BackupError, ClientFlavour, DatabaseConnection, DatabaseEngine};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::archive::{self, Written};
use crate::{Failure, redact};

const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const CHUNK: usize = 64 * 1024;
/// Chunks in flight between the tool and the file: a slow disk slows the dump instead of filling
/// the memory.
const CHUNKS_IN_FLIGHT: usize = 8;
const KEPT_STDERR: usize = 8 * 1024;
/// Variables the tools read that would change where they connect, or with what.
const CLEARED_VARIABLES: [&str; 4] = ["MYSQL_PWD", "MYSQL_HOST", "MYSQL_TCP_PORT", "MYSQL_UNIX_PORT"];
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A dump tool found on the host, with the client of the same folder that the test uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub dump: PathBuf,
    /// `mariadb` or `mysql` next to it, when there is one.
    pub shell: Option<PathBuf>,
    pub flavour: ClientFlavour,
    /// The version the tool prints, such as `11.7.2-MariaDB` or `8.0.36`.
    pub version: String,
}

impl Client {
    /// The tool's name and version, as a run records it.
    pub fn describe(&self) -> String {
        let name = self
            .dump
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        format!("{name} {}", self.version)
    }
}

/// The dump tools the engine prefers, in order.
fn dump_names(engine: DatabaseEngine) -> [&'static str; 2] {
    match engine {
        DatabaseEngine::MariaDb => ["mariadb-dump", "mysqldump"],
        DatabaseEngine::MySql => ["mysqldump", "mariadb-dump"],
    }
}

/// The folders searched when the settings name none: the `PATH`, then where the usual installs
/// put their tools.
pub fn search_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    {
        let program_files = std::env::var_os("ProgramFiles")
            .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
        for (parent, prefix) in [
            (program_files.clone(), "MariaDB "),
            (program_files.join("MySQL"), "MySQL Server "),
        ] {
            folders.extend(
                versioned(&parent, prefix)
                    .into_iter()
                    .map(|folder| folder.join("bin")),
            );
        }
        folders.push(PathBuf::from(r"C:\xampp\mysql\bin"));
        folders.extend(
            versioned(Path::new(r"C:\laragon\bin\mysql"), "")
                .into_iter()
                .map(|folder| folder.join("bin")),
        );
    }
    #[cfg(not(windows))]
    folders.extend(["/usr/bin", "/usr/local/bin", "/usr/local/mysql/bin"].map(PathBuf::from));
    folders
}

/// The folders of `parent` whose name starts with `prefix`, the newest version first.
#[cfg(windows)]
fn versioned(parent: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
        })
        .collect();
    found.sort();
    found.reverse();
    found
}

fn executable(folder: &Path, name: &str) -> Option<PathBuf> {
    let path = folder.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

/// Finds the engine's dump tool in `folder`, or in [`search_folders`], and asks it its version.
pub async fn find(engine: DatabaseEngine, folder: Option<&Path>) -> Result<Client, Failure> {
    let folders = match folder {
        Some(folder) => vec![folder.to_path_buf()],
        None => search_folders(),
    };
    let found = dump_names(engine)
        .into_iter()
        .find_map(|name| folders.iter().find_map(|folder| executable(folder, name)));
    let Some(dump) = found else {
        return Err(Failure::new(BackupError::ClientMissing));
    };
    let output = tokio::time::timeout(VERSION_TIMEOUT, quiet(&dump).arg("--version").output())
        .await
        .map_err(|_| {
            Failure::with(
                BackupError::ClientMissing,
                format!("{} did not answer", dump.display()),
            )
        })?
        .map_err(|error| Failure::with(BackupError::ClientMissing, format!("{}: {error}", dump.display())))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (flavour, version) = parse_version(&text).ok_or_else(|| {
        Failure::with(
            BackupError::ClientMissing,
            format!("unexpected version: {}", text.trim()),
        )
    })?;
    let folder = dump.parent().map(Path::to_path_buf).unwrap_or_default();
    let shell = ["mariadb", "mysql"]
        .into_iter()
        .find_map(|name| executable(&folder, name));
    Ok(Client {
        dump,
        shell,
        flavour,
        version,
    })
}

/// Reads what `mariadb-dump --version` or `mysqldump --version` prints.
pub fn parse_version(text: &str) -> Option<(ClientFlavour, String)> {
    let line = text.lines().find(|line| !line.trim().is_empty())?;
    let words: Vec<&str> = line.split_whitespace().collect();
    // "mariadb-dump from 11.7.2-MariaDB, client 10.19 for Win64 (AMD64)" and the older
    // "mysqldump  Ver 10.19 Distrib 10.6.18-MariaDB, for debian-linux-gnu (x86_64)".
    if let Some(word) = words.iter().find(|word| word.contains("MariaDB")) {
        let version = word.trim_end_matches(',');
        return Some((ClientFlavour::MariaDb, version.to_owned()));
    }
    // "mysqldump  Ver 8.0.36 for Linux on x86_64 (MySQL Community Server - GPL)" and the older
    // "mysqldump  Ver 10.13 Distrib 5.7.44, for Linux (x86_64)".
    let after = |marker: &str| {
        words
            .iter()
            .position(|word| *word == marker)
            .and_then(|index| words.get(index + 1))
            .map(|word| word.trim_end_matches(',').to_owned())
    };
    let version = after("Distrib").or_else(|| after("Ver"))?;
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then_some((ClientFlavour::MySql, version))
}

/// The MySQL version a client is, when it is MySQL's own: `8.0.36` is `(8, 0)`.
fn mysql_version(client: &Client) -> Option<(u32, u32)> {
    if client.flavour != ClientFlavour::MySql {
        return None;
    }
    let mut parts = client.version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// The `[client]` section the tools read the connection from. Every value is quoted, with `\` and
/// `"` escaped; one with a control character cannot be written and is refused.
pub fn option_file(connection: &DatabaseConnection) -> Result<String, Failure> {
    let quoted = |value: &str| -> Result<String, Failure> {
        if value.chars().any(char::is_control) {
            return Err(Failure::with(
                BackupError::SourceUnreadable,
                "a connection value holds a control character",
            ));
        }
        Ok(format!(
            "\"{}\"",
            value.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    };
    let mut lines = vec![
        "[client]".to_owned(),
        format!("user={}", quoted(&connection.user)?),
        format!("password={}", quoted(connection.password.expose())?),
    ];
    if let Some(socket) = &connection.socket {
        lines.push("protocol=socket".to_owned());
        lines.push(format!("socket={}", quoted(socket)?));
    } else {
        // TCP even to `localhost`, as oxmysql connects: the account the server's resources use is
        // then the one the dump uses, and not one tied to a Unix socket.
        lines.push("protocol=tcp".to_owned());
        lines.push(format!("host={}", quoted(&connection.host)?));
        lines.push(format!("port={}", connection.port));
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// The option file of one run, removed when dropped.
#[derive(Debug)]
pub struct OptionFile {
    path: PathBuf,
}

impl OptionFile {
    /// Writes the file as `<work>/<name>.cnf`, new, readable by Observer's account only on Unix and
    /// under the data folder's own rights on Windows.
    pub fn write(work: &Path, name: &str, connection: &DatabaseConnection) -> Result<Self, Failure> {
        let text = option_file(connection)?;
        let path = work.join(format!("{name}.cnf"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&path)?;
        let written = file.write_all(text.as_bytes()).and_then(|()| file.sync_all());
        let option_file = Self { path };
        written?;
        Ok(option_file)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OptionFile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), %error, "a dump's option file could not be removed");
        }
    }
}

/// The dump's arguments after the option file: a consistent snapshot of `InnoDB` tables, rows
/// streamed, everything a restore needs, and the database alone so that it restores under any name.
pub fn dump_arguments(client: &Client, options: &Path, database: &str) -> Vec<OsString> {
    let mut defaults = OsString::from("--defaults-file=");
    defaults.push(options);
    let mut arguments = vec![defaults];
    arguments.extend(
        [
            "--single-transaction",
            "--quick",
            "--routines",
            "--triggers",
            "--events",
            "--hex-blob",
            "--no-tablespaces",
            "--default-character-set=utf8mb4",
            "--max-allowed-packet=1G",
        ]
        .map(OsString::from),
    );
    if let Some(version) = mysql_version(client) {
        // A dump with GTIDs needs SUPER to restore; and mysqldump 8 asks MariaDB for histograms
        // it does not keep.
        arguments.push("--set-gtid-purged=OFF".into());
        if version >= (8, 0) {
            arguments.push("--column-statistics=0".into());
        }
    }
    arguments.push("--".into());
    arguments.push(database.into());
    arguments
}

/// A command with no console window, no stdin, and none of the variables that would point the tool
/// elsewhere.
fn quiet(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).kill_on_drop(true);
    for variable in CLEARED_VARIABLES {
        command.env_remove(variable);
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// One dump to run.
#[derive(Debug)]
pub struct DumpRequest<'a> {
    pub client: &'a Client,
    pub connection: &'a DatabaseConnection,
    /// Where the option file is written for the time of the dump.
    pub work: &'a Path,
    /// The option file's name, unique to the run.
    pub name: &'a str,
    /// The new file the dump goes to, gzipped.
    pub output: &'a Path,
    /// The dump's bytes read so far.
    pub progress: Arc<AtomicU64>,
    pub cancel: CancellationToken,
}

/// Runs the dump into `output` and checks it: the tool's status, the dump's first and last lines,
/// then the file read back. On failure the output is removed.
pub async fn dump(request: DumpRequest<'_>) -> Result<Written, Failure> {
    let result = run_dump(&request).await;
    if result.is_err()
        && let Err(error) = tokio::fs::remove_file(request.output).await
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(path = %request.output.display(), %error, "a failed dump's file could not be removed");
    }
    result
}

async fn run_dump(request: &DumpRequest<'_>) -> Result<Written, Failure> {
    let connection = request.connection;
    let password = connection.password.expose().to_owned();
    let options = {
        let (work, name, connection) = (
            request.work.to_path_buf(),
            request.name.to_owned(),
            connection.clone(),
        );
        tokio::task::spawn_blocking(move || OptionFile::write(&work, &name, &connection))
            .await
            .map_err(io::Error::other)??
    };
    let arguments = dump_arguments(request.client, options.path(), &connection.database);
    let mut child = quiet(&request.client.dump)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Failure::with(BackupError::ClientMissing, error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("the dump's output is not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("the dump's errors are not piped"))?;
    let errors = tokio::spawn(capped(stderr));

    let (chunks, receiver) = mpsc::channel::<Vec<u8>>(CHUNKS_IN_FLIGHT);
    let output = request.output.to_path_buf();
    let writer = tokio::task::spawn_blocking(move || {
        let mut receiver = receiver;
        archive::write(std::iter::from_fn(|| receiver.blocking_recv()), &output)
    });

    let streamed = tokio::select! {
        streamed = stream(stdout, &chunks, &request.progress) => streamed,
        () = request.cancel.cancelled() => Err(Failure::new(BackupError::Cancelled)),
    };
    drop(chunks);
    let status = finish(&mut child, matches!(streamed, Ok(true))).await;
    let written = writer.await.map_err(io::Error::other)?;
    drop(options);
    let stderr = errors.await.unwrap_or_default();
    let read_out = streamed?;
    let status = status?;
    if !status.success() {
        let said = last_line(&stderr).unwrap_or_else(|| format!("the tool ended with {status}"));
        return Err(Failure::with(
            BackupError::DumpFailed,
            redact(&said, &[&password]),
        ));
    }
    let written = written?;
    if !read_out {
        return Err(Failure::with(BackupError::Io, "the file stopped taking the dump"));
    }
    if let Some(failure) = written.inspection.problem() {
        return Err(failure);
    }
    let checked = written.clone();
    let output = request.output.to_path_buf();
    tokio::task::spawn_blocking(move || archive::verify(&output, &checked))
        .await
        .map_err(io::Error::other)??;
    Ok(written)
}

/// Sends the tool's output to the writer until it ends, `true`, or the writer stops taking it,
/// `false`: its own error then says why.
async fn stream(
    mut stdout: impl AsyncRead + Unpin,
    chunks: &mpsc::Sender<Vec<u8>>,
    progress: &AtomicU64,
) -> Result<bool, Failure> {
    loop {
        let mut chunk = vec![0; CHUNK];
        let read = stdout.read(&mut chunk).await?;
        if read == 0 {
            return Ok(true);
        }
        chunk.truncate(read);
        progress.fetch_add(read as u64, Ordering::Relaxed);
        if chunks.send(chunk).await.is_err() {
            return Ok(false);
        }
    }
}

/// Waits for the tool, or kills it when the output was not read to its end.
async fn finish(child: &mut Child, read_out: bool) -> io::Result<ExitStatus> {
    if !read_out {
        child.kill().await?;
    }
    child.wait().await
}

/// The first bytes of a stream, the rest drained so that the tool never blocks on it.
async fn capped(mut stream: impl AsyncRead + Unpin) -> String {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 4096];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        let room = KEPT_STDERR.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..read.min(room)]);
    }
    String::from_utf8_lossy(&kept).into_owned()
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_owned)
}

/// What the database server says of itself and of the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub server_version: String,
    pub tables: u32,
    pub bytes: u64,
    pub loose_tables: u32,
}

const FACTS: &str = "SELECT VERSION(), \
    (SELECT COUNT(*) FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE'), \
    (SELECT COALESCE(SUM(DATA_LENGTH + INDEX_LENGTH), 0) FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE()), \
    (SELECT COUNT(*) FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE' \
        AND ENGINE NOT IN ('InnoDB', 'ROCKSDB'))";

/// Connects with the client next to the dump tool and reads the server's version and the
/// database's size: the credentials, the network and the database's name, all checked at once.
pub async fn test(
    client: &Client,
    connection: &DatabaseConnection,
    work: &Path,
    name: &str,
) -> Result<Facts, Failure> {
    let Some(shell) = &client.shell else {
        return Err(Failure::with(
            BackupError::ClientMissing,
            "no mariadb or mysql client next to the dump tool",
        ));
    };
    let password = connection.password.expose().to_owned();
    let options = {
        let (work, name, connection) = (work.to_path_buf(), name.to_owned(), connection.clone());
        tokio::task::spawn_blocking(move || OptionFile::write(&work, &name, &connection))
            .await
            .map_err(io::Error::other)??
    };
    let mut defaults = OsString::from("--defaults-file=");
    defaults.push(options.path());
    let output = quiet(shell)
        .arg(defaults)
        .args(["--batch", "--skip-column-names", "--connect-timeout=10"])
        .arg(format!("--execute={FACTS}"))
        .arg("--")
        .arg(&connection.database)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let output = tokio::time::timeout(TEST_TIMEOUT, output)
        .await
        .map_err(|_| Failure::new(BackupError::TimedOut))??;
    drop(options);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let said = last_line(&stderr).unwrap_or_else(|| format!("the client ended with {}", output.status));
        return Err(Failure::with(
            BackupError::DumpFailed,
            redact(&said, &[&password]),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let fields: Vec<&str> = text.trim_end().split('\t').collect();
    let [version, tables, bytes, loose] = fields.as_slice() else {
        return Err(Failure::with(
            BackupError::DumpFailed,
            format!("unexpected answer: {}", text.trim()),
        ));
    };
    Ok(Facts {
        server_version: (*version).to_owned(),
        tables: tables.parse().unwrap_or_default(),
        bytes: bytes.parse().unwrap_or_default(),
        loose_tables: loose.parse().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use observer_domain::backups::{Secret, parse_connection_string};

    use super::*;

    fn client(flavour: ClientFlavour, version: &str) -> Client {
        Client {
            dump: PathBuf::from("mysqldump"),
            shell: None,
            flavour,
            version: version.to_owned(),
        }
    }

    #[test]
    fn versions_are_read_from_both_tools_and_their_old_forms() {
        let cases = [
            (
                "mariadb-dump from 11.7.2-MariaDB, client 10.19 for Win64 (AMD64)\r\n",
                (ClientFlavour::MariaDb, "11.7.2-MariaDB"),
            ),
            (
                "mysqldump  Ver 10.19 Distrib 10.6.18-MariaDB, for debian-linux-gnu (x86_64)\n",
                (ClientFlavour::MariaDb, "10.6.18-MariaDB"),
            ),
            (
                "mysqldump  Ver 8.0.36 for Linux on x86_64 (MySQL Community Server - GPL)\n",
                (ClientFlavour::MySql, "8.0.36"),
            ),
            (
                "mysqldump  Ver 10.13 Distrib 5.7.44, for Linux (x86_64)\n",
                (ClientFlavour::MySql, "5.7.44"),
            ),
        ];
        for (text, (flavour, version)) in cases {
            assert_eq!(parse_version(text), Some((flavour, version.to_owned())), "{text}");
        }
        assert_eq!(parse_version("pg_dump (PostgreSQL) 16.2\n"), None);
    }

    #[test]
    fn mysql_s_own_dump_gets_the_options_it_needs_and_mariadb_s_does_not() {
        let options = Path::new("work/1.cnf");
        let mariadb = dump_arguments(&client(ClientFlavour::MariaDb, "11.7.2-MariaDB"), options, "es");
        let mysql8 = dump_arguments(&client(ClientFlavour::MySql, "8.0.36"), options, "es");
        let mysql57 = dump_arguments(&client(ClientFlavour::MySql, "5.7.44"), options, "es");

        let has = |arguments: &[OsString], wanted: &str| arguments.iter().any(|argument| argument == wanted);
        assert_eq!(mariadb[0], OsString::from("--defaults-file=work/1.cnf"));
        assert!(has(&mariadb, "--single-transaction"));
        assert!(!has(&mariadb, "--set-gtid-purged=OFF"));
        assert!(has(&mysql8, "--set-gtid-purged=OFF") && has(&mysql8, "--column-statistics=0"));
        assert!(has(&mysql57, "--set-gtid-purged=OFF") && !has(&mysql57, "--column-statistics=0"));
        assert_eq!(mariadb.last(), Some(&OsString::from("es")));
        assert_eq!(mariadb[mariadb.len() - 2], OsString::from("--"));
    }

    #[test]
    fn the_option_file_quotes_every_value_and_forces_tcp() {
        let connection = parse_connection_string("mysql://fivem:p%22a%5Css@127.0.0.1:3307/es").unwrap();

        let text = option_file(&connection).unwrap();

        assert_eq!(
            text,
            "[client]\nuser=\"fivem\"\npassword=\"p\\\"a\\\\ss\"\nprotocol=tcp\nhost=\"127.0.0.1\"\nport=3307\n"
        );
    }

    #[test]
    fn a_socket_connection_goes_through_the_socket() {
        let connection =
            parse_connection_string("user=fivem;password=x;db=qb;socketPath=/run/mysqld/mysqld.sock")
                .unwrap();

        let text = option_file(&connection).unwrap();

        assert!(
            text.contains("protocol=socket\nsocket=\"/run/mysqld/mysqld.sock\"\n"),
            "{text}"
        );
        assert!(!text.contains("host="));
    }

    #[test]
    fn a_value_with_a_control_character_is_refused_rather_than_mangled() {
        let mut connection = parse_connection_string("mysql://fivem@localhost/es").unwrap();
        connection.password = Secret::new("line\nbreak");

        assert_eq!(
            option_file(&connection).unwrap_err().error,
            BackupError::SourceUnreadable
        );
    }

    #[test]
    fn the_option_file_is_new_and_gone_once_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let connection = parse_connection_string("mysql://fivem:hunter2@localhost/es").unwrap();

        let file = OptionFile::write(dir.path(), "1", &connection).unwrap();
        let path = file.path().to_path_buf();
        assert!(std::fs::read_to_string(&path).unwrap().contains("hunter2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(OptionFile::write(dir.path(), "1", &connection).is_err());
        drop(file);

        assert!(!path.exists());
    }
}
