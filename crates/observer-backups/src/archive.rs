//! The file a dump becomes: gzip at level 6, written next to its final name and synced, with the
//! SHA-256 of the SQL and of the file taken on the way, and the dump itself looked at as it passes:
//! its header, its last line, and the tables a single transaction does not freeze. Reading the file
//! back proves what is on the disk is what was made.

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use memchr::memchr;
use observer_domain::backups::{BackupError, MAX_LOOSE_TABLES};
use sha2::{Digest, Sha256};

use crate::Failure;

/// How much of each line the inspection keeps: enough for `CREATE TABLE` and `) ENGINE=` lines.
const KEPT_PER_LINE: usize = 256;
/// The end of the dump kept to find its last line.
const KEPT_TAIL: usize = 1024;
const BUFFER: usize = 256 * 1024;
/// Engines whose tables `--single-transaction` does not freeze.
const LOOSE_ENGINES: [&str; 7] = [
    "MyISAM",
    "Aria",
    "MEMORY",
    "CSV",
    "ARCHIVE",
    "MRG_MyISAM",
    "BLACKHOLE",
];

/// What a dump looked like as it passed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inspection {
    /// The first line that is not MariaDB's sandbox comment names the dump's tool.
    pub header: bool,
    /// Its last line that is not empty is the one the tool writes after the last table.
    pub trailer: bool,
    pub loose_tables: Vec<String>,
    pub loose_table_count: u32,
}

impl Inspection {
    /// Why the dump is not a complete one, if it is not.
    pub fn problem(&self) -> Option<Failure> {
        if !self.header {
            Some(Failure::with(
                BackupError::DumpIncomplete,
                "the output does not start like a MariaDB or MySQL dump",
            ))
        } else if !self.trailer {
            Some(Failure::with(
                BackupError::DumpIncomplete,
                "the dump ended before its last line: it was cut short",
            ))
        } else {
            None
        }
    }
}

/// Reads a dump line by line as it streams, keeping only the start of each line.
#[derive(Debug, Default)]
pub struct Inspector {
    line: Vec<u8>,
    tail: Vec<u8>,
    header: Option<bool>,
    table: Option<String>,
    loose_tables: Vec<String>,
    loose_table_count: u32,
}

impl Inspector {
    pub fn feed(&mut self, mut bytes: &[u8]) {
        self.keep_tail(bytes);
        while let Some(end) = memchr(b'\n', bytes) {
            self.keep(&bytes[..end]);
            self.end_line();
            bytes = &bytes[end + 1..];
        }
        self.keep(bytes);
    }

    pub fn finish(mut self) -> Inspection {
        if !self.line.is_empty() {
            self.end_line();
        }
        let last = self
            .tail
            .split(|byte| *byte == b'\n')
            .map(trim_end)
            .rfind(|line| !line.is_empty());
        Inspection {
            header: self.header.unwrap_or(false),
            trailer: last.is_some_and(|line| line.starts_with(b"-- Dump completed")),
            loose_tables: self.loose_tables,
            loose_table_count: self.loose_table_count,
        }
    }

    fn keep(&mut self, part: &[u8]) {
        let room = KEPT_PER_LINE.saturating_sub(self.line.len());
        self.line.extend_from_slice(&part[..part.len().min(room)]);
    }

    fn keep_tail(&mut self, bytes: &[u8]) {
        if bytes.len() >= KEPT_TAIL {
            self.tail.clear();
            self.tail.extend_from_slice(&bytes[bytes.len() - KEPT_TAIL..]);
        } else {
            let excess = (self.tail.len() + bytes.len()).saturating_sub(KEPT_TAIL);
            self.tail.drain(..excess);
            self.tail.extend_from_slice(bytes);
        }
    }

    fn end_line(&mut self) {
        let line = trim_end(&self.line);
        if self.header.is_none() && !line.is_empty() && !line.starts_with(b"/*M!") {
            self.header = Some(line.starts_with(b"-- MariaDB dump") || line.starts_with(b"-- MySQL dump"));
        }
        if let Some(rest) = line.strip_prefix(b"CREATE TABLE `") {
            self.table = rest
                .iter()
                .position(|byte| *byte == b'`')
                .map(|end| String::from_utf8_lossy(&rest[..end]).into_owned());
        } else if let Some(rest) = line.strip_prefix(b") ENGINE=")
            && let Some(table) = self.table.take()
        {
            let engine = rest.split(|byte| *byte == b' ').next().unwrap_or_default();
            let loose = LOOSE_ENGINES
                .iter()
                .any(|known| known.as_bytes().eq_ignore_ascii_case(engine));
            if loose {
                self.loose_table_count += 1;
                if self.loose_tables.len() < MAX_LOOSE_TABLES {
                    self.loose_tables.push(table);
                }
            }
        }
        self.line.clear();
    }
}

fn trim_end(line: &[u8]) -> &[u8] {
    let end = line
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |last| last + 1);
    &line[..end]
}

/// The file a dump was written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub sql_bytes: u64,
    pub sql_sha256: [u8; 32],
    pub file_bytes: u64,
    pub file_sha256: [u8; 32],
    pub inspection: Inspection,
}

/// Writes the dump's chunks, as they come, gzipped into a new file at `path`, and syncs it.
pub fn write(chunks: impl Iterator<Item = Vec<u8>>, path: &Path) -> io::Result<Written> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let counted = Counted::new(BufWriter::with_capacity(BUFFER, file));
    let mut gzip = GzEncoder::new(counted, Compression::new(6));
    let mut inspector = Inspector::default();
    let mut sql = Sha256::new();
    let mut sql_bytes = 0u64;
    for chunk in chunks {
        inspector.feed(&chunk);
        sql.update(&chunk);
        sql_bytes += chunk.len() as u64;
        gzip.write_all(&chunk)?;
    }
    let counted = gzip.finish()?;
    let (writer, file_bytes, file_sha256) = counted.into_parts();
    let file = writer.into_inner().map_err(io::IntoInnerError::into_error)?;
    file.sync_all()?;
    Ok(Written {
        sql_bytes,
        sql_sha256: sql.finalize().into(),
        file_bytes,
        file_sha256,
        inspection: inspector.finish(),
    })
}

/// Reads the file at `path` back: its own digest, then the SQL inside, must be those written.
pub fn verify(path: &Path, written: &Written) -> Result<(), Failure> {
    let mut file = Hashed::new(BufReader::with_capacity(BUFFER, File::open(path)?));
    let mut sql = Hashed::new(MultiGzDecoder::new(&mut file));
    let unpacked = io::copy(&mut sql, &mut io::sink()).map_err(|error| {
        Failure::with(
            BackupError::VerifyFailed,
            format!("the file does not unpack: {error}"),
        )
    })?;
    let sql_sha256 = sql.finish();
    // Whatever follows the gzip stream, if anything, belongs to the file too.
    io::copy(&mut file, &mut io::sink())?;
    let file_sha256 = file.finish();
    if file_sha256 != written.file_sha256 || file.bytes != written.file_bytes {
        return Err(Failure::with(
            BackupError::VerifyFailed,
            "the file on the disk differs from the one written",
        ));
    }
    if sql_sha256 != written.sql_sha256 || unpacked != written.sql_bytes {
        return Err(Failure::with(
            BackupError::VerifyFailed,
            "the dump inside the file differs from the one read",
        ));
    }
    Ok(())
}

/// The SHA-256 of a file, read through.
pub fn digest(path: &Path) -> io::Result<([u8; 32], u64)> {
    let mut file = Hashed::new(BufReader::with_capacity(BUFFER, File::open(path)?));
    io::copy(&mut file, &mut io::sink())?;
    let bytes = file.bytes;
    Ok((file.finish(), bytes))
}

/// A writer that counts and hashes what goes through it.
struct Counted<W> {
    inner: W,
    hash: Sha256,
    bytes: u64,
}

impl<W: Write> Counted<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
            bytes: 0,
        }
    }

    fn into_parts(self) -> (W, u64, [u8; 32]) {
        (self.inner, self.bytes, self.hash.finalize().into())
    }
}

impl<W: Write> Write for Counted<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.hash.update(&buffer[..written]);
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A reader that counts and hashes what comes through it.
struct Hashed<R> {
    inner: R,
    hash: Sha256,
    bytes: u64,
}

impl<R: Read> Hashed<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
            bytes: 0,
        }
    }

    fn finish(&mut self) -> [u8; 32] {
        std::mem::take(&mut self.hash).finalize().into()
    }
}

impl<R: Read> Read for Hashed<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hash.update(&buffer[..read]);
        self.bytes += read as u64;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "/*M!999999\\- enable the sandbox mode */ \n\
        -- MariaDB dump 10.19-11.7.2-MariaDB, for Win64 (AMD64)\n\
        --\n\
        -- Host: 127.0.0.1    Database: es_extended\n\
        CREATE TABLE `users` (\n\
          `identifier` varchar(60) NOT NULL\n\
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;\n\
        INSERT INTO `users` VALUES ('license:abc');\n\
        CREATE TABLE `player_logs` (\n\
          `id` int(11) NOT NULL\n\
        ) ENGINE=Aria DEFAULT CHARSET=utf8mb4 PAGE_CHECKSUM=1;\n\
        /*!50001 CREATE TABLE `v_users` (\n\
          `identifier` tinyint NOT NULL\n\
        ) ENGINE=MyISAM */;\n\
        -- Dump completed on 2026-11-02  4:00:01\n";

    fn inspect_in_pieces(text: &str, piece: usize) -> Inspection {
        let mut inspector = Inspector::default();
        for chunk in text.as_bytes().chunks(piece) {
            inspector.feed(chunk);
        }
        inspector.finish()
    }

    #[test]
    fn a_complete_dump_names_its_tables_without_transactions_whatever_the_chunks() {
        for piece in [1, 7, 64, 4096] {
            let inspection = inspect_in_pieces(DUMP, piece);

            assert!(inspection.header, "{piece}");
            assert!(inspection.trailer, "{piece}");
            assert_eq!(inspection.loose_tables, ["player_logs"], "{piece}");
            assert_eq!(inspection.loose_table_count, 1);
            assert_eq!(inspection.problem(), None);
        }
    }

    #[test]
    fn a_dump_cut_short_or_from_elsewhere_is_refused() {
        let cut = &DUMP[..DUMP.find("-- Dump completed").unwrap()];
        assert_eq!(
            inspect_in_pieces(cut, 64).problem().map(|failure| failure.error),
            Some(BackupError::DumpIncomplete)
        );
        assert!(!inspect_in_pieces("mysqldump: Got error: 1045\n", 64).header);
        assert!(inspect_in_pieces("-- MySQL dump 10.13  Distrib 8.0.36\r\n-- Dump completed\r\n", 3).trailer);
    }

    #[test]
    fn a_written_file_unpacks_to_the_dump_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("1.partial");
        let chunks = DUMP.as_bytes().chunks(10).map(<[u8]>::to_vec);

        let written = write(chunks, &path).unwrap();

        assert_eq!(written.sql_bytes, DUMP.len() as u64);
        assert_eq!(written.sql_sha256, <[u8; 32]>::from(Sha256::digest(DUMP)));
        assert_eq!(digest(&path).unwrap(), (written.file_sha256, written.file_bytes));
        let mut unpacked = String::new();
        MultiGzDecoder::new(File::open(&path).unwrap())
            .read_to_string(&mut unpacked)
            .unwrap();
        assert_eq!(unpacked, DUMP);
        assert_eq!(verify(&path, &written), Ok(()));
    }

    #[test]
    fn a_file_changed_after_writing_fails_its_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("1.partial");
        let written = write([DUMP.as_bytes().to_vec()].into_iter(), &path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();

        let failure = verify(&path, &written).unwrap_err();

        assert_eq!(failure.error, BackupError::VerifyFailed);
    }

    #[test]
    fn a_file_is_never_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("1.partial");
        std::fs::write(&path, b"someone else's").unwrap();

        let error = write(std::iter::empty(), &path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }
}
