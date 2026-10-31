//! Reading one file: what kind it is, its keyed digest, the plain SHA-256 of what may show one, and
//! the text of what gets line diffs. A file that moves while it is read is read again, twice at
//! most; reads share a rate so that the game server keeps the disk.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use hmac::{Hmac, KeyInit, Mac};
use observer_domain::changes::{FileKind, LOCKFILES, extension, hidden};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::mask::{Masked, mask};
use super::tree::hex;
use super::walk::Stat;
use super::{Cancelled, Keys, ScanSettings};

const CHUNK: usize = 1024 * 1024;
const SNIFF: usize = 8 * 1024;
const ATTEMPTS: usize = 3;
/// A file written less than this before its read began may be written again within the same tick
/// of a coarse clock: it is read again next time.
const RACY: Duration = Duration::from_secs(2);
const LONG_LINE: u64 = 4096;
const DENSE_FILE: u64 = 64 * 1024;
const DENSE_LINE: u64 = 300;

/// What reading a file gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    pub kind: FileKind,
    /// The keyed digest of its bytes, hex; `None` when it could not be read.
    pub digest: Option<String>,
    /// The plain SHA-256 of a binary, encrypted or generated file.
    pub sha256: Option<String>,
    /// The text of a text file within the size limit, read in one settled pass and masked.
    pub masked: Option<Masked>,
    /// The metadata after the read.
    pub stat: Stat,
    pub bytes: u64,
    pub racy: bool,
    /// It changed on each of three reads: its digest is the last one's and no text is kept.
    pub unsettled: bool,
}

impl FileRead {
    pub fn unreadable(stat: Stat) -> Self {
        Self {
            kind: FileKind::Unreadable,
            digest: None,
            sha256: None,
            masked: None,
            stat,
            bytes: 0,
            racy: false,
            unsettled: false,
        }
    }
}

/// Reads the file at `location`, `path` from the data folder. `chain` for a file of the cfg exec
/// chain, whose text is masked as a cfg's.
pub fn read(
    path: &str,
    location: &Path,
    chain: bool,
    settings: &ScanSettings,
    keys: &Keys,
    rate: &mut Rate,
    cancel: &CancellationToken,
) -> Result<FileRead, Cancelled> {
    let hidden = hidden(path, &settings.hidden);
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let plan = Plan {
        path,
        chain,
        tag_key: &keys.tag,
        hidden,
        candidate: !hidden
            && extension(path).is_some_and(|extension| settings.extensions.contains(&extension))
            && !LOCKFILES.contains(&name.as_str()),
        name,
        settings,
    };
    let keep = if plan.candidate {
        settings.limits.max_text
    } else {
        0
    };
    let mut last = None;
    for _ in 0..ATTEMPTS {
        let Ok(before) = fs::metadata(location) else {
            return Ok(FileRead::unreadable(Stat::default()));
        };
        let started = SystemTime::now();
        let Ok(pass) = Pass::read(location, keys, keep, rate, cancel)? else {
            return Ok(FileRead::unreadable(Stat::of(&before)));
        };
        let Ok(after) = fs::metadata(location) else {
            return Ok(FileRead::unreadable(Stat::of(&before)));
        };
        let stat = Stat::of(&after);
        let settled = Stat::of(&before) == stat && pass.bytes == stat.size;
        let racy = jiff::Timestamp::try_from(started)
            .ok()
            .and_then(|started| i64::try_from(started.as_nanosecond()).ok())
            .is_none_or(|started| {
                stat.modified_ns > started - i64::try_from(RACY.as_nanos()).unwrap_or(i64::MAX)
            });
        if settled {
            return Ok(pass.finish(&plan, stat, racy, false));
        }
        last = Some((pass, stat));
    }
    let (pass, stat) = last.expect("ATTEMPTS is above zero, so a pass was kept");
    Ok(pass.finish(&plan, stat, true, true))
}

/// What a file's path says before it is read.
struct Plan<'a> {
    path: &'a str,
    chain: bool,
    tag_key: &'a [u8; 32],
    hidden: bool,
    /// Its extension is on the owner's list: it may be text.
    candidate: bool,
    name: String,
    settings: &'a ScanSettings,
}

/// One read of a file's bytes.
struct Pass {
    keyed: Hmac<Sha256>,
    plain: Sha256,
    bytes: u64,
    head: Vec<u8>,
    /// Every byte up to the text limit, plus one to tell a longer file.
    kept: Vec<u8>,
    longest_line: u64,
    line: u64,
    lines: u64,
}

impl Pass {
    fn read(
        location: &Path,
        keys: &Keys,
        keep: u64,
        rate: &mut Rate,
        cancel: &CancellationToken,
    ) -> Result<io::Result<Self>, Cancelled> {
        let mut file = match File::open(location) {
            Ok(file) => file,
            Err(error) => return Ok(Err(error)),
        };
        let mut pass = Self {
            keyed: <Hmac<Sha256> as KeyInit>::new_from_slice(&keys.content)
                .expect("HMAC takes a key of any length"),
            plain: Sha256::new(),
            bytes: 0,
            head: Vec::new(),
            kept: Vec::new(),
            longest_line: 0,
            line: 0,
            lines: 0,
        };
        let mut buffer = vec![0; CHUNK];
        loop {
            let read = match file.read(&mut buffer) {
                Ok(0) => return Ok(Ok(pass)),
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Ok(Err(error)),
            };
            rate.take(read as u64, cancel)?;
            pass.add(&buffer[..read], keep);
        }
    }

    fn add(&mut self, bytes: &[u8], keep: u64) {
        self.keyed.update(bytes);
        self.plain.update(bytes);
        self.bytes += bytes.len() as u64;
        if self.head.len() < SNIFF {
            let more = (SNIFF - self.head.len()).min(bytes.len());
            self.head.extend_from_slice(&bytes[..more]);
        }
        let room = usize::try_from((keep + 1).saturating_sub(self.kept.len() as u64)).unwrap_or(usize::MAX);
        if keep > 0 && room > 0 {
            self.kept.extend_from_slice(&bytes[..room.min(bytes.len())]);
        }
        let mut start = 0;
        for end in memchr::memchr_iter(b'\n', bytes) {
            self.line += (end - start) as u64;
            self.longest_line = self.longest_line.max(self.line);
            self.line = 0;
            self.lines += 1;
            start = end + 1;
        }
        self.line += (bytes.len() - start) as u64;
        self.longest_line = self.longest_line.max(self.line);
    }

    fn finish(self, plan: &Plan<'_>, stat: Stat, racy: bool, unsettled: bool) -> FileRead {
        let digest = Some(hex(&self.keyed.finalize().into_bytes()));
        let sha256 = Some(hex(&self.plain.finalize()));
        let mut read = FileRead {
            kind: FileKind::Binary,
            digest,
            sha256: None,
            masked: None,
            stat,
            bytes: self.bytes,
            racy,
            unsettled,
        };
        if plan.hidden {
            read.kind = FileKind::Hidden;
            return read;
        }
        let lines = self.lines + u64::from(self.line > 0);
        let dense = self.bytes > DENSE_FILE && self.bytes / lines.max(1) > DENSE_LINE;
        read.kind = if self.head.starts_with(b"FXAP") {
            FileKind::Encrypted
        } else if LOCKFILES.contains(&plan.name.as_str()) {
            FileKind::Generated
        } else if !plan.candidate {
            FileKind::Binary
        } else {
            let whole = self.bytes <= plan.settings.limits.max_text;
            let sniffed = if whole { &self.kept[..] } else { &self.head[..] };
            match decode(sniffed, !whole) {
                None => FileKind::Binary,
                Some(_) if self.longest_line > LONG_LINE || dense => FileKind::Generated,
                Some(text) => {
                    if whole && !unsettled {
                        read.masked = Some(mask(&text, plan.path, plan.chain, plan.tag_key));
                    }
                    FileKind::Text
                }
            }
        };
        if read.kind.shows_hash() {
            read.sha256 = sha256;
        }
        read
    }
}

/// The text of `bytes`: UTF-8, UTF-16 with a byte order mark, or bytes that are neither read as
/// Latin-1, as long as it holds no NUL and few control characters. `None` for anything else.
/// `partial` when `bytes` is only the start of the file, which may end inside a character.
pub fn decode(bytes: &[u8], partial: bool) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        utf8(rest, partial)?
    } else if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        utf16(rest, partial, u16::from_le_bytes)?
    } else if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        utf16(rest, partial, u16::from_be_bytes)?
    } else {
        utf8(bytes, partial).unwrap_or_else(|| bytes.iter().map(|byte| char::from(*byte)).collect())
    };
    let (mut chars, mut controls) = (0, 0);
    for c in text.chars() {
        if c == '\0' {
            return None;
        }
        chars += 1;
        if c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{c}' | '\u{1b}') {
            controls += 1;
        }
    }
    (controls * 100 <= chars).then_some(text)
}

fn utf8(bytes: &[u8], partial: bool) -> Option<String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        // Cut in the middle of a character by the sniff's limit.
        Err(error) if partial && error.error_len().is_none() => {
            Some(String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned())
        }
        Err(_) => None,
    }
}

fn utf16(bytes: &[u8], partial: bool, unit: fn([u8; 2]) -> u16) -> Option<String> {
    let even = if partial { bytes.len() & !1 } else { bytes.len() };
    if even % 2 != 0 {
        return None;
    }
    let units = bytes[..even].as_chunks::<2>().0.iter().map(|pair| unit(*pair));
    char::decode_utf16(units).collect::<Result<_, _>>().ok()
}

/// The pace reads keep: at most `per_second` bytes each second, 0 for no limit.
#[derive(Debug)]
pub struct Rate {
    per_second: u64,
    since: Instant,
    bytes: u64,
}

impl Rate {
    pub fn new(per_second: u64) -> Self {
        Self {
            per_second,
            since: Instant::now(),
            bytes: 0,
        }
    }

    /// Counts `bytes` read and sleeps while reads are ahead of the pace.
    pub fn take(&mut self, bytes: u64, cancel: &CancellationToken) -> Result<(), Cancelled> {
        if cancel.is_cancelled() {
            return Err(Cancelled);
        }
        if self.per_second == 0 {
            return Ok(());
        }
        // A pause between reads gives no credit beyond the current second.
        if self.since.elapsed() >= Duration::from_secs(1) {
            self.since = Instant::now();
            self.bytes = 0;
        }
        self.bytes += bytes;
        let due = Duration::from_nanos(
            u64::try_from(u128::from(self.bytes) * 1_000_000_000 / u128::from(self.per_second))
                .unwrap_or(u64::MAX),
        );
        while let Some(ahead) = due
            .checked_sub(self.since.elapsed())
            .filter(|ahead| !ahead.is_zero())
        {
            if cancel.is_cancelled() {
                return Err(Cancelled);
            }
            std::thread::sleep(ahead.min(Duration::from_millis(50)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::{Keys, ScanSettings};

    fn settings() -> ScanSettings {
        ScanSettings::default()
    }

    fn text(read: &FileRead) -> Option<String> {
        read.masked.as_ref().map(|masked| masked.text.clone())
    }

    fn read_bytes(name: &str, bytes: &[u8]) -> FileRead {
        let dir = tempfile::tempdir().unwrap();
        let location = dir.path().join(name);
        fs::write(&location, bytes).unwrap();
        let path = format!("resources/r/{name}");
        read(
            &path,
            &location,
            false,
            &settings(),
            &Keys::fixed(),
            &mut Rate::new(0),
            &CancellationToken::new(),
        )
        .unwrap()
    }

    #[test]
    fn utf8_text_is_kept_with_its_line_endings() {
        let read = read_bytes("client.lua", b"local a = 1\r\nlocal b = 2\n");

        assert_eq!(read.kind, FileKind::Text);
        assert_eq!(text(&read).as_deref(), Some("local a = 1\r\nlocal b = 2\n"));
        assert_eq!(read.sha256, None);
        assert!(read.racy, "just written");
    }

    #[test]
    fn text_is_masked_as_it_is_read() {
        let read = read_bytes(
            "config.lua",
            b"Config.Token = 'abc'
",
        );

        assert_eq!(
            text(&read).as_deref(),
            Some(
                "Config.Token = '[REDACTED]'
"
            )
        );
        assert_eq!(read.masked.unwrap().redactions.len(), 1);
    }

    #[test]
    fn utf16_with_a_bom_is_read_as_text() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("a = 'é'\n".encode_utf16().flat_map(u16::to_le_bytes));

        let read = read_bytes("config.lua", &bytes);

        assert_eq!(text(&read).as_deref(), Some("a = 'é'\n"));
    }

    #[test]
    fn latin1_without_nul_is_read_as_text() {
        let read = read_bytes("config.lua", b"Config.Label = 'Caf\xe9'\n");

        assert_eq!(text(&read).as_deref(), Some("Config.Label = 'Café'\n"));
    }

    #[test]
    fn a_nul_or_an_unlisted_extension_makes_a_binary() {
        let nul = read_bytes("data.json", b"{\0}");
        let unlisted = read_bytes("model.ydr", b"plain text");

        for read in [nul, unlisted] {
            assert_eq!(read.kind, FileKind::Binary);
            assert_eq!(read.masked, None);
            assert_eq!(read.sha256.as_ref().map(String::len), Some(64));
        }
    }

    #[test]
    fn the_escrow_header_comes_before_the_text_sniff() {
        let read = read_bytes("client.lua", b"FXAP\x01\x02local a = 1");

        assert_eq!(read.kind, FileKind::Encrypted);
        assert!(read.sha256.is_some());
    }

    #[test]
    fn minified_files_and_lockfiles_are_generated() {
        let long_line = read_bytes("app.js", &vec![b'a'; 5000]);
        let dense = read_bytes("style.css", vec!["x".repeat(400); 200].join("\n").as_bytes());
        let lock = read_bytes("package-lock.json", b"{}\n");

        for read in [long_line, dense, lock] {
            assert_eq!(read.kind, FileKind::Generated);
            assert_eq!(read.masked, None);
            assert!(read.sha256.is_some());
        }
    }

    #[test]
    fn a_hidden_file_keeps_only_its_keyed_digest() {
        let read = read_bytes(".env", b"TOKEN=abc\n");

        assert_eq!(read.kind, FileKind::Hidden);
        assert_eq!(read.masked, None);
        assert_eq!(read.sha256, None);
        assert!(read.digest.is_some());
    }

    #[test]
    fn a_text_past_the_limit_has_no_contents() {
        let dir = tempfile::tempdir().unwrap();
        let location = dir.path().join("big.lua");
        fs::write(&location, "local a = 1\n".repeat(20)).unwrap();
        let mut settings = settings();
        settings.limits.max_text = 100;

        let read = read(
            "resources/r/big.lua",
            &location,
            false,
            &settings,
            &Keys::fixed(),
            &mut Rate::new(0),
            &CancellationToken::new(),
        )
        .unwrap();

        assert_eq!(read.kind, FileKind::Text);
        assert_eq!(read.masked, None);
        assert_eq!(read.bytes, 240);
    }

    #[test]
    fn the_digest_is_keyed() {
        let dir = tempfile::tempdir().unwrap();
        let location = dir.path().join("a.lua");
        fs::write(&location, "x").unwrap();
        let other = Keys {
            content: [7; 32],
            ..Keys::fixed()
        };
        let digest = |keys: &Keys| {
            read(
                "resources/r/a.lua",
                &location,
                false,
                &settings(),
                keys,
                &mut Rate::new(0),
                &CancellationToken::new(),
            )
            .unwrap()
            .digest
        };

        assert_ne!(digest(&Keys::fixed()), digest(&other));
        assert_ne!(digest(&Keys::fixed()).unwrap(), hex(&Sha256::digest(b"x")));
    }

    #[test]
    fn a_missing_file_is_unreadable() {
        let read = read(
            "resources/r/gone.lua",
            Path::new("definitely/not/here.lua"),
            false,
            &settings(),
            &Keys::fixed(),
            &mut Rate::new(0),
            &CancellationToken::new(),
        )
        .unwrap();

        assert_eq!(read.kind, FileKind::Unreadable);
        assert_eq!(read.digest, None);
    }

    #[test]
    fn decoding_tells_text_from_binary() {
        assert_eq!(decode(b"abc", false).as_deref(), Some("abc"));
        assert_eq!(decode(b"\xEF\xBB\xBFabc", false).as_deref(), Some("abc"));
        assert_eq!(decode(b"a\0c", false), None);
        assert_eq!(decode(&[0x01; 10], false), None);
        // A UTF-8 character cut by the sniff's limit.
        assert_eq!(decode("aé".as_bytes()[..2].as_ref(), true).as_deref(), Some("a"));
        // An unpaired surrogate.
        assert_eq!(decode(&[0xFF, 0xFE, 0x00, 0xD8], false), None);
    }

    #[test]
    fn the_rate_sleeps_once_reads_get_ahead() {
        let mut rate = Rate::new(1000);
        let started = Instant::now();

        rate.take(200, &CancellationToken::new()).unwrap();

        assert!(started.elapsed() >= Duration::from_millis(150));
    }

    #[test]
    fn a_cancelled_rate_stops_the_read() {
        let cancel = CancellationToken::new();
        cancel.cancel();

        assert_eq!(Rate::new(0).take(1, &cancel), Err(Cancelled));
    }
}
