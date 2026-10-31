//! txAdmin's players database, read and checked whole before anything is written: one malformed
//! file is refused rather than half imported. Only players are read; notes, bans, warnings and
//! whitelist entries are skipped as they stream past.
//!
//! The file is read as a stream, each player turned into a [`Record`] as it comes, its strings
//! and lists capped: what Observer holds stays in proportion to the players it keeps, whatever the
//! file holds. The folder it is in can be written by FXServer's resources.

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use jiff::Timestamp;
use observer_domain::players::{IdentifierKind, Identifiers, PlayersFileProblem, clean_name};
use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

/// More than any server's players database: txAdmin keeps about 1 KiB per player.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PLAYERS: usize = 1_000_000;
/// Identifiers and tokens kept per player; FiveM shows about twenty.
const MAX_IDS: usize = 64;
/// No identifier, token or licence is longer; a name longer is cut by [`clean_name`] anyway.
const MAX_STRING_BYTES: usize = 1024;
/// A century, in minutes: more is not play time.
const MAX_MINUTES: f64 = 100.0 * 365.25 * 24.0 * 60.0;
/// The versions whose players have `ids` and `hwids`.
const VERSIONS: [u64; 2] = [4, 5];

/// A player of the file, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Record {
    pub(super) license: String,
    pub(super) name: Option<String>,
    pub(super) play_ms: i64,
    pub(super) joined: Timestamp,
    pub(super) last: Timestamp,
    pub(super) others: Vec<(IdentifierKind, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Parsed {
    pub(super) bytes: u64,
    pub(super) records: Vec<Record>,
    /// Players without a valid licence.
    pub(super) invalid: u64,
}

/// Reads the file at `path`, a regular local file of at most 512 MiB, and checks every player.
/// Dates are clamped to `now`, so that one in the future cannot make a profile or a link immortal.
pub(super) fn read(path: &Path, now: Timestamp) -> Result<Parsed, PlayersFileProblem> {
    if !path.is_absolute() || remote(path) {
        return Err(PlayersFileProblem::NotLocal);
    }
    let (file, metadata) = open(path)?;
    if metadata.len() > MAX_BYTES {
        return Err(PlayersFileProblem::TooLarge);
    }
    let mut source = serde_json::Deserializer::from_reader(BufReader::new(file.take(MAX_BYTES)));
    let database = DatabaseSeed { now }.deserialize(&mut source).map_err(|error| {
        if error.is_io() {
            PlayersFileProblem::Unreadable
        } else if error.to_string().starts_with(TOO_MANY) {
            PlayersFileProblem::TooLarge
        } else {
            PlayersFileProblem::NotPlayersDatabase
        }
    })?;
    source.end().map_err(|_| PlayersFileProblem::NotPlayersDatabase)?;
    let (version, records, invalid) = database;
    match version {
        Some(version) if VERSIONS.contains(&version) => Ok(Parsed {
            bytes: metadata.len(),
            records,
            invalid,
        }),
        Some(_) => Err(PlayersFileProblem::UnsupportedVersion),
        None => Err(PlayersFileProblem::NotPlayersDatabase),
    }
}

/// A UNC or device path: on Windows, opening one reaches another host, or a pipe that could act
/// as Observer.
fn remote(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.starts_with(r"\\") || text.starts_with("//")
}

const TOO_MANY: &str = "more players than";

/// The top-level object: its `version`, and its `players` turned into records as they come.
struct DatabaseSeed {
    now: Timestamp,
}

impl<'de> DeserializeSeed<'de> for DatabaseSeed {
    type Value = (Option<u64>, Vec<Record>, u64);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for DatabaseSeed {
    type Value = (Option<u64>, Vec<Record>, u64);

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("txAdmin's players database")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut version = None;
        let mut players = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "version" => version = Some(map.next_value::<u64>()?),
                "players" => players = Some(map.next_value_seed(PlayersSeed { now: self.now })?),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let (records, invalid) = players.ok_or_else(|| serde::de::Error::missing_field("players"))?;
        Ok((version, records, invalid))
    }
}

struct PlayersSeed {
    now: Timestamp,
}

impl<'de> DeserializeSeed<'de> for PlayersSeed {
    type Value = (Vec<Record>, u64);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for PlayersSeed {
    type Value = (Vec<Record>, u64);

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a list of players")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut records = Vec::new();
        let mut invalid = 0;
        while let Some(entry) = seq.next_element::<Entry>()? {
            if records.len() + usize::try_from(invalid).unwrap_or(usize::MAX) >= MAX_PLAYERS {
                return Err(serde::de::Error::custom(format!("{TOO_MANY} {MAX_PLAYERS}")));
            }
            match record(&entry, self.now) {
                Some(record) => records.push(record),
                None => invalid += 1,
            }
        }
        Ok((records, invalid))
    }
}

/// A player as txAdmin writes it, each field taken only if it has the expected type.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    #[serde(default)]
    license: Text,
    #[serde(default)]
    ids: Texts,
    #[serde(default)]
    hwids: Texts,
    #[serde(default)]
    display_name: Text,
    #[serde(default)]
    play_time: Number,
    #[serde(default)]
    ts_joined: Number,
    #[serde(default)]
    ts_last_connection: Number,
}

/// A string of at most [`MAX_STRING_BYTES`]; anything else is skipped as it streams past.
#[derive(Default)]
struct Text(Option<String>);

/// The first [`MAX_IDS`] strings of a list.
#[derive(Default)]
struct Texts(Vec<String>);

#[derive(Default)]
struct Number(Option<f64>);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(Lenient::Text)
            .map(|value| match value {
                Found::Text(text) => Self(Some(text)),
                _ => Self(None),
            })
    }
}

impl<'de> Deserialize<'de> for Number {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(Lenient::Number)
            .map(|value| match value {
                Found::Number(number) => Self(Some(number)),
                _ => Self(None),
            })
    }
}

impl<'de> Deserialize<'de> for Texts {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(Lenient::Texts)
            .map(|value| match value {
                Found::Texts(texts) => Self(texts),
                _ => Self(Vec::new()),
            })
    }
}

/// What a field was expected to be.
#[derive(Clone, Copy)]
enum Lenient {
    Text,
    Number,
    Texts,
}

enum Found {
    Text(String),
    Number(f64),
    Texts(Vec<String>),
    Nothing,
}

impl<'de> Visitor<'de> for Lenient {
    type Value = Found;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Found, E> {
        Ok(match self {
            Self::Text if text.len() <= MAX_STRING_BYTES => Found::Text(text.to_owned()),
            _ => Found::Nothing,
        })
    }

    #[expect(clippy::cast_precision_loss, reason = "dates and minutes are far below 2^52")]
    fn visit_u64<E: serde::de::Error>(self, number: u64) -> Result<Found, E> {
        Ok(match self {
            Self::Number => Found::Number(number as f64),
            _ => Found::Nothing,
        })
    }

    #[expect(clippy::cast_precision_loss, reason = "dates and minutes are far below 2^52")]
    fn visit_i64<E: serde::de::Error>(self, number: i64) -> Result<Found, E> {
        Ok(match self {
            Self::Number => Found::Number(number as f64),
            _ => Found::Nothing,
        })
    }

    fn visit_f64<E: serde::de::Error>(self, number: f64) -> Result<Found, E> {
        Ok(match self {
            Self::Number => Found::Number(number),
            _ => Found::Nothing,
        })
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Found, E> {
        Ok(Found::Nothing)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Found, E> {
        Ok(Found::Nothing)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Found, A::Error> {
        let mut texts = Vec::new();
        loop {
            if matches!(self, Self::Texts) && texts.len() < MAX_IDS {
                match seq.next_element::<Text>()? {
                    Some(Text(Some(text))) => texts.push(text),
                    Some(Text(None)) => {}
                    None => break,
                }
            } else if seq.next_element::<IgnoredAny>()?.is_none() {
                break;
            }
        }
        Ok(match self {
            Self::Texts => Found::Texts(texts),
            _ => Found::Nothing,
        })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Found, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Found::Nothing)
    }
}

/// Opens the file itself, never a link in its place, and checks from the open handle that it is a
/// regular file.
fn open(path: &Path) -> Result<(File, std::fs::Metadata), PlayersFileProblem> {
    let (file, _) = observer_platform::open_resolved(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PlayersFileProblem::Missing
        } else if std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
            PlayersFileProblem::NotAFile
        } else {
            PlayersFileProblem::Unreadable
        }
    })?;
    let metadata = file.metadata().map_err(|_| PlayersFileProblem::Unreadable)?;
    if !metadata.is_file() || metadata.is_symlink() || reparse_point(&metadata) {
        return Err(PlayersFileProblem::NotAFile);
    }
    Ok((file, metadata))
}

#[cfg(windows)]
fn reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

fn record(entry: &Entry, now: Timestamp) -> Option<Record> {
    let license = entry.license.0.as_deref()?.trim().to_ascii_lowercase();
    let hex = license.strip_prefix("license:").unwrap_or(&license);
    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let license = format!("license:{hex}");
    #[expect(
        clippy::cast_possible_truncation,
        reason = "below a century of minutes, checked just before"
    )]
    let play_ms = match entry
        .play_time
        .0
        .filter(|minutes| minutes.is_finite() && *minutes > 0.0)
    {
        Some(minutes) if minutes <= MAX_MINUTES => (minutes.floor() as i64) * 60_000,
        Some(_) => return None,
        None => 0,
    };
    let date = |value: &Number| {
        value
            .0
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0 && *seconds < 2.0e11)
            .and_then(|seconds| {
                #[expect(clippy::cast_possible_truncation, reason = "within the range checked above")]
                Timestamp::from_second(seconds.floor() as i64).ok()
            })
            .map(|at| at.min(now))
    };
    let (joined, last) = match (date(&entry.ts_joined), date(&entry.ts_last_connection)) {
        (Some(joined), Some(last)) => (joined.min(last), joined.max(last)),
        (Some(one), None) | (None, Some(one)) => (one, one),
        (None, None) => (now, now),
    };
    let ids = entry
        .ids
        .0
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.eq_ignore_ascii_case(&license) && !id.to_ascii_lowercase().starts_with("ip:"));
    let shown = Identifiers::parse(ids, entry.hwids.0.iter().map(String::as_str));
    Some(Record {
        license,
        name: entry.display_name.0.as_deref().and_then(clean_name),
        play_ms,
        joined,
        last,
        others: shown
            .others
            .into_iter()
            .map(|identifier| (identifier.kind, identifier.value))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::{Value, json};

    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef01234567";

    fn now() -> Timestamp {
        "2026-10-31T12:00:00Z".parse().unwrap()
    }

    fn written(database: &Value) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("playersDB.json");
        fs::write(&path, database.to_string()).unwrap();
        (dir, path)
    }

    fn player() -> Value {
        json!({
            "license": HEX,
            "ids": [format!("license:{HEX}"), "discord:123456789012345678", "ip:192.0.2.7", "steam:x", "license2:ffffffffffffffffffffffffffffffffffffffff"],
            "hwids": [format!("4:{}", "ab".repeat(32)), "bad"],
            "displayName": "Ada\u{202E}",
            "pureName": "ada",
            "playTime": 125,
            "tsJoined": 1_700_000_000,
            "tsLastConnection": 1_760_000_000,
            "notes": { "text": "never read", "lastAdmin": null, "tsLastEdit": null },
        })
    }

    #[test]
    fn a_player_comes_in_with_their_licence_prefixed_and_minutes_in_milliseconds() {
        let (_dir, path) = written(&json!({ "version": 5, "players": [player()], "actions": [] }));

        let parsed = read(&path, now()).unwrap();

        assert_eq!(parsed.invalid, 0);
        let record = &parsed.records[0];
        assert_eq!(record.license, format!("license:{HEX}"));
        assert_eq!(record.play_ms, 125 * 60_000);
        assert_eq!(record.name.as_deref(), Some("Ada"));
        assert_eq!(record.joined, Timestamp::from_second(1_700_000_000).unwrap());
        assert_eq!(record.last, Timestamp::from_second(1_760_000_000).unwrap());
        let kinds: Vec<IdentifierKind> = record.others.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            [
                IdentifierKind::License2,
                IdentifierKind::Discord,
                IdentifierKind::Token
            ]
        );
    }

    #[test]
    fn dates_in_the_future_are_clamped_to_the_import() {
        let mut future = player();
        future["tsLastConnection"] = json!(4_000_000_000_u64);
        let (_dir, path) = written(&json!({ "version": 4, "players": [future] }));

        let parsed = read(&path, now()).unwrap();

        assert_eq!(parsed.records[0].last, now());
    }

    #[test]
    fn a_player_without_a_valid_licence_or_with_an_absurd_play_time_is_skipped() {
        let mut no_licence = player();
        no_licence["license"] = json!("abc");
        let mut absurd = player();
        absurd["playTime"] = json!(1e300);
        let (_dir, path) = written(&json!({ "version": 5, "players": [no_licence, absurd, player()] }));

        let parsed = read(&path, now()).unwrap();

        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.invalid, 2);
    }

    #[test]
    fn other_versions_are_refused() {
        for version in [3, 6] {
            let (_dir, path) = written(&json!({ "version": version, "players": [player()] }));

            assert_eq!(
                read(&path, now()),
                Err(PlayersFileProblem::UnsupportedVersion),
                "{version}"
            );
        }
    }

    #[test]
    fn a_truncated_or_foreign_file_is_refused_whole() {
        let dir = tempfile::tempdir().unwrap();
        let truncated = dir.path().join("truncated.json");
        let text = json!({ "version": 5, "players": [player(), player()] }).to_string();
        fs::write(&truncated, &text[..text.len() - 10]).unwrap();
        let foreign = dir.path().join("foreign.json");
        fs::write(&foreign, r#"{"hello":"world"}"#).unwrap();

        assert_eq!(
            read(&truncated, now()),
            Err(PlayersFileProblem::NotPlayersDatabase)
        );
        assert_eq!(read(&foreign, now()), Err(PlayersFileProblem::NotPlayersDatabase));
        assert_eq!(
            read(&dir.path().join("missing.json"), now()),
            Err(PlayersFileProblem::Missing)
        );
        assert_eq!(read(dir.path(), now()), Err(PlayersFileProblem::NotAFile));
    }

    #[test]
    fn odd_values_are_skipped_and_lists_are_capped() {
        let mut odd = player();
        odd["ids"] = Value::from(vec![Value::from(0); 10_000]);
        odd["displayName"] = json!({ "nested": ["anything"] });
        odd["hwids"] = Value::from(vec![Value::from(format!("4:{}", "cd".repeat(32))); 500]);
        let (_dir, path) = written(&json!({ "players": [odd], "version": 5, "extra": [[[1]]] }));

        let parsed = read(&path, now()).unwrap();

        assert_eq!(parsed.invalid, 0);
        assert_eq!(parsed.records[0].name, None);
        assert_eq!(parsed.records[0].others.len(), 1);
    }

    #[test]
    fn a_century_of_play_time_or_more_is_not_play_time() {
        let mut absurd = player();
        absurd["playTime"] = json!(60_000_000);
        let (_dir, path) = written(&json!({ "version": 5, "players": [absurd] }));

        assert_eq!(read(&path, now()).unwrap().invalid, 1);
    }

    #[test]
    fn a_network_or_relative_path_is_refused_before_anything_is_opened() {
        for path in [
            r"\\host\share\playersDB.json",
            r"\\.\pipe\players",
            "//host/share/playersDB.json",
            "playersDB.json",
        ] {
            assert_eq!(
                read(Path::new(path), now()),
                Err(PlayersFileProblem::NotLocal),
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_files_place_is_refused() {
        let (dir, path) = written(&json!({ "version": 5, "players": [] }));
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();

        assert_eq!(read(&link, now()), Err(PlayersFileProblem::NotAFile));
    }
}
