//! txAdmin's players database, read and checked whole before anything is written: one malformed
//! file is refused rather than half imported. Only players are read; notes, bans, warnings and
//! whitelist entries are left alone.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use jiff::Timestamp;
use observer_domain::players::{IdentifierKind, Identifiers, PlayersFileProblem, clean_name};
use serde::Deserialize;
use serde_json::Value;

/// More than any server's players database: txAdmin keeps about 1 KiB per player.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
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

#[derive(Deserialize)]
struct Database {
    version: u64,
    players: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    #[serde(default)]
    license: Value,
    #[serde(default)]
    ids: Vec<Value>,
    #[serde(default)]
    hwids: Vec<Value>,
    #[serde(default)]
    display_name: Value,
    #[serde(default)]
    play_time: Value,
    #[serde(default)]
    ts_joined: Value,
    #[serde(default)]
    ts_last_connection: Value,
}

/// Reads the file at `path`, a regular file of at most 512 MiB, and checks every player. Dates
/// are clamped to `now`, so that one in the future cannot make a profile or a link immortal.
pub(super) fn read(path: &Path, now: Timestamp) -> Result<Parsed, PlayersFileProblem> {
    let (file, metadata) = open(path)?;
    if metadata.len() > MAX_BYTES {
        return Err(PlayersFileProblem::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PlayersFileProblem::Unreadable)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(PlayersFileProblem::TooLarge);
    }
    let database: Database =
        serde_json::from_slice(&bytes).map_err(|_| PlayersFileProblem::NotPlayersDatabase)?;
    if !VERSIONS.contains(&database.version) {
        return Err(PlayersFileProblem::UnsupportedVersion);
    }
    let mut parsed = Parsed {
        bytes: bytes.len() as u64,
        records: Vec::with_capacity(database.players.len()),
        invalid: 0,
    };
    for entry in &database.players {
        match record(entry, now) {
            Some(record) => parsed.records.push(record),
            None => parsed.invalid += 1,
        }
    }
    Ok(parsed)
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
    let license = entry.license.as_str()?.trim().to_ascii_lowercase();
    let hex = license.strip_prefix("license:").unwrap_or(&license);
    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let license = format!("license:{hex}");
    let minutes = entry
        .play_time
        .as_f64()
        .filter(|minutes| minutes.is_finite() && *minutes > 0.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "checked against the range of i64 just before"
    )]
    let play_ms = match minutes {
        Some(minutes) if minutes < 9.2e18 => (minutes.floor() as i64).checked_mul(60_000)?,
        Some(_) => return None,
        None => 0,
    };
    let date = |value: &Value| {
        value
            .as_f64()
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
    let strings = |values: &[Value]| -> Vec<String> {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let ids: Vec<String> = strings(&entry.ids)
        .into_iter()
        .filter(|id| {
            !id.trim().eq_ignore_ascii_case(&license) && !id.trim().to_ascii_lowercase().starts_with("ip:")
        })
        .collect();
    let hwids = strings(&entry.hwids);
    let shown = Identifiers::parse(ids.iter().map(String::as_str), hwids.iter().map(String::as_str));
    Some(Record {
        license,
        name: entry.display_name.as_str().and_then(clean_name),
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

    use serde_json::json;

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

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_files_place_is_refused() {
        let (dir, path) = written(&json!({ "version": 5, "players": [] }));
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();

        assert_eq!(read(&link, now()), Err(PlayersFileProblem::NotAFile));
    }
}
