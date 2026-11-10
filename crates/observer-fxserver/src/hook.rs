//! What the Debug feature takes from a build and reads from its server: the two lines it adds to
//! the build's Lua scheduler, which load the bridge's hook into every resource, and the reports
//! the hook prints each second through the structured trace channel. Counts, names and sizes:
//! the hook never sends what an event, a state bag or a request carries.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::layout::Layout;
use crate::trace::TraceKind;

/// The hook's file in the bridge's folder. The bridge's manifest does not list it: it is no
/// script of the bridge, the scheduler's first line loads it.
pub const FILE: &str = "hook.lua";
const SOURCE: &str = include_str!("../../../resource/observer/hook.lua");
/// How the hook shows in a Lua traceback.
const CHUNK: &str = "=observer-hook";
const HEAD_OPEN: &str = "--[[obs:hook:head v1]]";
const HEAD_PREFIX: &str = "--[[obs:hook:head ";
const HEAD_CLOSE: &str = "--[[obs:hook:headend]]";
const TAIL_PREFIX: &str = "--[[obs:hook:tail ";
const TAIL_OPEN: &str = "--[[obs:hook:tail v1]]";
const TAIL_CLOSE: &str = "--[[obs:hook:tailend]]";
/// The call the hook wraps to count what a resource receives: a scheduler without it is not one
/// the hook was written for.
const KNOWN_BY: &str = "SetEventRoutine";
const BOM: &[u8] = b"\xEF\xBB\xBF";
/// The scheduler as the build shipped it, kept beside it while the two lines are in.
const ORIGINAL_SUFFIX: &str = ".original";

const WIRE_TYPE: &str = "observer_debug";
const WIRE_VERSION: u64 = 1;
/// What one report may hold of each list, and how long a name may be: a resource writes its own
/// report, and nothing says it writes it honestly.
const MAX_ROWS: usize = 512;
const MAX_LARGEST: usize = 10;
const MAX_NAME: usize = 128;

/// Why the two lines did not go in or come out.
#[derive(Debug, thiserror::Error)]
pub enum Refusal {
    #[error("{} could not be read or written: {source}", path.display())]
    File {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not a scheduler Observer knows", .0.display())]
    Unknown(PathBuf),
    #[error("{} holds lines of Observer that were changed by hand", .0.display())]
    Damaged(PathBuf),
}

/// Where the build keeps the scheduler every server Lua runtime runs first.
pub fn scheduler(layout: &Layout) -> PathBuf {
    layout
        .citizen_root()
        .join("citizen")
        .join("scripting")
        .join("lua")
        .join("scheduler.lua")
}

fn head(bridge: &str) -> String {
    format!(
        "{HEAD_OPEN} local __obsHook; do local ok, api = pcall(function() local src = \
         LoadResourceFile('{bridge}', '{FILE}'); if not src then return nil end; local fn = \
         load(src, '{CHUNK}'); return fn and fn('{bridge}') end); if ok and type(api) == 'table' \
         then __obsHook = api end end {HEAD_CLOSE}"
    )
}

fn tail() -> String {
    format!(
        "{TAIL_OPEN} if __obsHook and __obsHook.init then pcall(__obsHook.init, {{ eventHandlers = \
         eventHandlers }}) end {TAIL_CLOSE}"
    )
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::find(haystack, needle)
}

/// The scheduler without Observer's two lines: byte for byte what [`with_lines`] was given.
/// `None` when a line of Observer's is there in part, which only a hand could have left.
fn without_lines(content: &[u8]) -> Option<Vec<u8>> {
    let (bom, mut body) = match content.strip_prefix(BOM) {
        Some(body) => (BOM, body),
        None => (&[][..], content),
    };
    if body.starts_with(HEAD_PREFIX.as_bytes()) {
        let line = memchr::memchr(b'\n', body).unwrap_or(body.len());
        let close = find(&body[..line], HEAD_CLOSE.as_bytes())?;
        body = &body[close + HEAD_CLOSE.len()..];
        body = body.strip_prefix(b" ").unwrap_or(body);
    }
    // The last line, whether or not the file ends with a line break.
    let (text, ending) = match body {
        [text @ .., b'\r', b'\n'] => (text, &b"\r\n"[..]),
        [text @ .., b'\n'] => (text, &b"\n"[..]),
        text => (text, &[][..]),
    };
    let start = memchr::memrchr(b'\n', text).map_or(0, |at| at + 1);
    let last = &text[start..];
    let kept = if last.starts_with(TAIL_PREFIX.as_bytes()) {
        if !last.ends_with(TAIL_CLOSE.as_bytes()) {
            return None;
        }
        if ending.is_empty() {
            // The file ended without a line break: the one before the line came with it.
            let before = &text[..start];
            before
                .strip_suffix(b"\r\n")
                .or_else(|| before.strip_suffix(b"\n"))
                .unwrap_or(before)
        } else {
            &text[..start]
        }
    } else {
        body
    };
    Some([bom, kept].concat())
}

/// The scheduler with Observer's two lines: the first before its first line, on that line, so
/// that no line of the scheduler changes its number, and the second as a last line of its own.
fn with_lines(original: &[u8], bridge: &str) -> Vec<u8> {
    let (bom, body) = match original.strip_prefix(BOM) {
        Some(body) => (BOM, body),
        None => (&[][..], original),
    };
    let eol: &[u8] = if find(body, b"\r\n").is_some() {
        b"\r\n"
    } else {
        b"\n"
    };
    let (head, tail) = (head(bridge), tail());
    let end: [&[u8]; 2] = if body.ends_with(b"\n") {
        [tail.as_bytes(), eol]
    } else {
        [eol, tail.as_bytes()]
    };
    [bom, head.as_bytes(), b" ", body, end[0], end[1]].concat()
}

fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".observer-tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

fn original_beside(scheduler: &Path) -> PathBuf {
    let mut path = scheduler.as_os_str().to_owned();
    path.push(ORIGINAL_SUFFIX);
    PathBuf::from(path)
}

/// Puts the two lines into the build's scheduler and the hook into the bridge's folder, for a
/// start that counts; takes the lines out for one that does not. The bridge is installed first.
/// A refusal leaves the scheduler as it was.
pub fn install(layout: &Layout, bridge: &str, counting: bool) -> Result<(), Refusal> {
    let path = scheduler(layout);
    let at = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Refusal::File { path, source }
    };
    let current = match fs::read(&path) {
        Ok(current) => current,
        // Nothing to take out of a build that has no scheduler.
        Err(error) if !counting && error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(at(&path)(error)),
    };
    let original = without_lines(&current).ok_or_else(|| Refusal::Damaged(path.clone()))?;
    let beside = original_beside(&path);
    if !counting {
        if original != current {
            replace(&path, &original).map_err(at(&path))?;
        }
        let _ = fs::remove_file(beside);
        return Ok(());
    }
    // The name is drawn by Observer from letters and digits; anything else would be Lua source.
    let named = bridge
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !named || find(&original, KNOWN_BY.as_bytes()).is_none() {
        return Err(Refusal::Unknown(path));
    }
    let hook = layout.system_resources().join(bridge).join(FILE);
    fs::write(&hook, SOURCE).map_err(at(&hook))?;
    fs::write(&beside, &original).map_err(at(&beside))?;
    let counted = with_lines(&original, bridge);
    if counted != current {
        replace(&path, &counted).map_err(at(&path))?;
    }
    Ok(())
}

/// Which way an event went, as the resource that reports it saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Way {
    /// Received from a resource of the server, itself included.
    Received,
    /// Received from a client.
    FromClient,
    /// Received from the engine, such as `playerJoining`.
    FromEngine,
    /// Triggered for the server's resources.
    Triggered,
    /// Sent to one client.
    ToClient,
    /// Sent to every client.
    ToAll,
}

impl Way {
    fn of(code: &str) -> Option<Self> {
        Some(match code {
            "in" => Self::Received,
            "inN" => Self::FromClient,
            "inI" => Self::FromEngine,
            "loc" => Self::Triggered,
            "toC" | "lat" => Self::ToClient,
            "toA" | "latA" => Self::ToAll,
            _ => return None,
        })
    }
}

/// The bag a state bag write went to, from the bag's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BagKind {
    Global,
    Player,
    Entity,
    LocalEntity,
    Other,
}

impl BagKind {
    fn of(code: &str) -> Self {
        match code {
            "global" => Self::Global,
            "player" => Self::Player,
            "entity" => Self::Entity,
            "localEntity" => Self::LocalEntity,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCount {
    pub way: Way,
    pub name: String,
    pub count: u64,
    pub bytes: u64,
    pub largest: u64,
    /// In the resource's handlers, for an event received: microseconds.
    pub handler_us: u64,
}

/// An event received from one client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderCount {
    pub name: String,
    pub player: u32,
    pub count: u64,
    pub bytes: u64,
}

/// An event a client sent that the resource never opened to clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedCount {
    pub name: String,
    /// `None` when the scheduler did not say who.
    pub player: Option<u32>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BagCount {
    pub kind: BagKind,
    pub key: String,
    pub count: u64,
    pub bytes: u64,
    pub largest: u64,
    /// How many of the writes went out to clients.
    pub replicated: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LargestWrite {
    pub bag: String,
    pub key: String,
    pub bytes: u64,
    pub replicated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCount {
    pub method: String,
    pub host: String,
    pub count: u64,
    /// Of the request bodies.
    pub bytes: u64,
}

/// What one resource counted since its report before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// FXServer's word for who printed the report, not the report's own.
    pub resource: String,
    pub events: Vec<EventCount>,
    pub senders: Vec<SenderCount>,
    pub refused: Vec<RefusedCount>,
    pub bags: Vec<BagCount>,
    pub largest: Vec<LargestWrite>,
    pub hosts: Vec<HostCount>,
    /// What the hook could not keep apart, past its limits.
    pub overflow: u64,
}

fn name(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    (!text.is_empty()).then(|| text.chars().take(MAX_NAME).collect())
}

fn rows<'a>(payload: &'a Value, key: &str, limit: usize) -> impl Iterator<Item = &'a Vec<Value>> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .take(limit)
}

fn number(row: &[Value], at: usize) -> u64 {
    row.get(at).and_then(Value::as_u64).unwrap_or(0)
}

impl Report {
    /// The hook's report in a trace, if the trace is one. Every resource sends its own, and
    /// FXServer names the resource: none can count for another.
    pub fn from_trace(trace: &TraceKind) -> Option<Self> {
        let TraceKind::Script { resource, payload } = trace else {
            return None;
        };
        if payload.get("type").and_then(Value::as_str) != Some(WIRE_TYPE)
            || payload.get("v").and_then(Value::as_u64) != Some(WIRE_VERSION)
        {
            return None;
        }
        let events = rows(payload, "ev", MAX_ROWS)
            .filter_map(|row| {
                Some(EventCount {
                    way: Way::of(row.first()?.as_str()?)?,
                    name: name(row.get(1)?)?,
                    count: number(row, 2),
                    bytes: number(row, 3),
                    largest: number(row, 4),
                    handler_us: number(row, 5),
                })
            })
            .collect();
        let player = |value: Option<&Value>| value?.as_u64().and_then(|id| u32::try_from(id).ok());
        let senders = rows(payload, "np", MAX_ROWS)
            .filter_map(|row| {
                Some(SenderCount {
                    name: name(row.first()?)?,
                    player: player(row.get(1))?,
                    count: number(row, 2),
                    bytes: number(row, 3),
                })
            })
            .collect();
        let refused = rows(payload, "un", MAX_ROWS)
            .filter_map(|row| {
                Some(RefusedCount {
                    name: name(row.first()?)?,
                    player: player(row.get(1)),
                    count: number(row, 2),
                })
            })
            .collect();
        let bags = rows(payload, "sb", MAX_ROWS)
            .filter_map(|row| {
                Some(BagCount {
                    kind: BagKind::of(row.first()?.as_str()?),
                    key: name(row.get(1)?)?,
                    count: number(row, 2),
                    bytes: number(row, 3),
                    largest: number(row, 4),
                    replicated: number(row, 5),
                })
            })
            .collect();
        let largest = rows(payload, "sbl", MAX_LARGEST)
            .filter_map(|row| {
                Some(LargestWrite {
                    bag: name(row.first()?)?,
                    key: name(row.get(1)?)?,
                    bytes: number(row, 2),
                    replicated: number(row, 3) != 0,
                })
            })
            .collect();
        let hosts = rows(payload, "http", MAX_ROWS)
            .filter_map(|row| {
                Some(HostCount {
                    method: name(row.first()?)?,
                    host: name(row.get(1)?)?.to_ascii_lowercase(),
                    count: number(row, 2),
                    bytes: number(row, 3),
                })
            })
            .collect();
        Some(Self {
            resource: resource.clone(),
            events,
            senders,
            refused,
            bags,
            largest,
            hosts,
            overflow: payload.get("ovf").and_then(Value::as_u64).unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::layout::Flavour;

    const BRIDGE: &str = "sys_abcdefgh2345";

    fn scheduler_of(body: &str) -> Vec<u8> {
        format!("local coroutine_yield = coroutine.yield\n{body}Citizen.SetEventRoutine(function() end)")
            .into_bytes()
    }

    #[test]
    fn the_two_lines_go_in_without_moving_a_line_and_come_out_byte_for_byte() {
        for original in [
            scheduler_of("local a = 1\n"),
            [scheduler_of("local a = 1\n"), b"\n".to_vec()].concat(),
            b"local y = coroutine.yield\r\nCitizen.SetEventRoutine(f)\r\nreturn y\r\n".to_vec(),
            b"local y = coroutine.yield\r\nCitizen.SetEventRoutine(f)".to_vec(),
            [BOM, &scheduler_of("-- with a mark\n"), b"\r\n"].concat(),
            b"Citizen.SetEventRoutine(routine)".to_vec(),
        ] {
            let counted = with_lines(&original, BRIDGE);

            let text = String::from_utf8_lossy(&counted);
            let lines: Vec<&str> = text.lines().collect();
            let unpatched = String::from_utf8_lossy(&original);
            assert_eq!(
                lines.len(),
                unpatched.lines().count() + 1,
                "one line more, the last"
            );
            assert!(lines[0].trim_start_matches('\u{feff}').starts_with(HEAD_OPEN));
            let first = unpatched.lines().next().unwrap().trim_start_matches('\u{feff}');
            assert!(lines[0].ends_with(first));
            assert!(lines.last().unwrap().starts_with(TAIL_OPEN));
            assert!(text.contains(&format!("LoadResourceFile('{BRIDGE}', 'hook.lua')")));
            assert_eq!(counted.starts_with(BOM), original.starts_with(BOM));
            assert_eq!(counted.ends_with(b"\n"), original.ends_with(b"\n"));
            assert_eq!(without_lines(&counted).unwrap(), original);
            assert_eq!(without_lines(&original).unwrap(), original, "nothing to take out");
            // Another start draws another name: the lines are replaced, never stacked.
            let again = with_lines(&without_lines(&counted).unwrap(), "sys_zyxwvuts7654");
            assert_eq!(without_lines(&again).unwrap(), original);
        }
    }

    #[test]
    fn a_line_of_observers_left_in_part_is_not_taken_for_the_schedulers() {
        let counted = with_lines(&scheduler_of("local a = 1\n"), BRIDGE);
        let text = String::from_utf8(counted).unwrap();

        let no_head_end = text.replacen(HEAD_CLOSE, "", 1);
        let no_tail_end = text.replacen(TAIL_CLOSE, "", 1);

        assert_eq!(without_lines(no_head_end.as_bytes()), None);
        assert_eq!(without_lines(no_tail_end.as_bytes()), None);
    }

    fn build() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), Flavour::Windows);
        fs::create_dir_all(scheduler(&layout).parent().unwrap()).unwrap();
        fs::create_dir_all(layout.system_resources().join(BRIDGE)).unwrap();
        (dir, layout)
    }

    #[test]
    fn a_start_that_counts_patches_the_build_and_the_next_one_that_does_not_restores_it() {
        let (_dir, layout) = build();
        let path = scheduler(&layout);
        let original = scheduler_of("local a = 1\n");
        fs::write(&path, &original).unwrap();
        let hook = layout.system_resources().join(BRIDGE).join(FILE);
        let beside = original_beside(&path);

        install(&layout, BRIDGE, true).unwrap();

        assert_eq!(fs::read(&path).unwrap(), with_lines(&original, BRIDGE));
        assert_eq!(fs::read(&beside).unwrap(), original);
        assert_eq!(fs::read_to_string(&hook).unwrap(), SOURCE);
        install(&layout, BRIDGE, true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), with_lines(&original, BRIDGE));

        install(&layout, BRIDGE, false).unwrap();

        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!beside.exists());
        install(&layout, BRIDGE, false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn a_scheduler_observer_does_not_know_is_left_as_it_is() {
        let (_dir, layout) = build();
        let path = scheduler(&layout);

        let missing = install(&layout, BRIDGE, true).unwrap_err();
        install(&layout, BRIDGE, false).unwrap();
        fs::write(&path, "print('another runtime')\n").unwrap();
        let unknown = install(&layout, BRIDGE, true).unwrap_err();
        let odd_name = {
            fs::write(&path, scheduler_of("")).unwrap();
            install(&layout, "sys'); os.exit() --", true).unwrap_err()
        };
        let damaged = {
            let counted = with_lines(&scheduler_of(""), BRIDGE);
            let text = String::from_utf8(counted).unwrap().replacen(HEAD_CLOSE, "", 1);
            fs::write(&path, &text).unwrap();
            (install(&layout, BRIDGE, false).unwrap_err(), text)
        };

        assert!(matches!(missing, Refusal::File { .. }), "{missing}");
        assert!(matches!(unknown, Refusal::Unknown(_)), "{unknown}");
        assert!(matches!(odd_name, Refusal::Unknown(_)), "{odd_name}");
        assert!(matches!(damaged.0, Refusal::Damaged(_)), "{}", damaged.0);
        assert_eq!(fs::read_to_string(&path).unwrap(), damaged.1, "left as it was");
    }

    fn trace(resource: &str, payload: Value) -> TraceKind {
        TraceKind::Script {
            resource: resource.to_owned(),
            payload,
        }
    }

    #[test]
    fn a_report_is_read_row_by_row_under_the_name_fxserver_gives_its_resource() {
        let payload = json!({
            "type": "observer_debug", "v": 1, "seq": 4, "dur": 1000,
            "ev": [
                ["toC", "voice:talking", 186, 7626, 44, 0, 0],
                ["latA", "map:sync", 1, 4096, 4096, 0, 0],
                ["inN", "voice:talking", 24, 912, 40, 480, 60],
                ["sideways", "ignored", 1, 1, 1, 0, 0],
                ["in", "", 1, 1, 1, 0, 0],
                "not a row",
            ],
            "np": [["voice:talking", 41, 19, 722], ["voice:talking", -1, 1, 1]],
            "un": [["bank:give", 41, 3], ["bank:give", -1, 1]],
            "sb": [["player", "proximity", 28, 252, 9, 28], ["somewhere", "odd", 1, 1, 1, 0]],
            "sbl": [["entity:1042", "fuel", 2100, 1]],
            "http": [["POST", "Discord.com", 36, 14400]],
            "ovf": 2,
            "resource": "someone-else",
        });

        let report = Report::from_trace(&trace("pma-voice", payload)).unwrap();

        assert_eq!(report.resource, "pma-voice");
        assert_eq!(
            report.events,
            [
                EventCount {
                    way: Way::ToClient,
                    name: "voice:talking".to_owned(),
                    count: 186,
                    bytes: 7626,
                    largest: 44,
                    handler_us: 0,
                },
                EventCount {
                    way: Way::ToAll,
                    name: "map:sync".to_owned(),
                    count: 1,
                    bytes: 4096,
                    largest: 4096,
                    handler_us: 0,
                },
                EventCount {
                    way: Way::FromClient,
                    name: "voice:talking".to_owned(),
                    count: 24,
                    bytes: 912,
                    largest: 40,
                    handler_us: 480,
                },
            ]
        );
        assert_eq!(report.senders.len(), 1, "a sender has an id");
        assert_eq!((report.senders[0].player, report.senders[0].count), (41, 19));
        assert_eq!(report.refused[0].player, Some(41));
        assert_eq!(report.refused[1].player, None);
        assert_eq!(report.bags[0].kind, BagKind::Player);
        assert_eq!(report.bags[1].kind, BagKind::Other);
        assert!(report.largest[0].replicated);
        assert_eq!(report.hosts[0].host, "discord.com");
        assert_eq!(report.overflow, 2);
    }

    #[test]
    fn only_a_report_of_this_version_is_read_and_its_lists_are_cut_at_their_limits() {
        let many: Vec<Value> = (0..MAX_ROWS + 50)
            .map(|index| json!(["in", format!("event-{index}"), 1, 1, 1, 0, 0]))
            .collect();
        let long = "n".repeat(MAX_NAME * 2);
        let payload =
            json!({ "type": "observer_debug", "v": 1, "ev": many, "sb": [["global", long, 1, 1, 1, 0]] });

        let report = Report::from_trace(&trace("big", payload)).unwrap();

        assert_eq!(report.events.len(), MAX_ROWS);
        assert_eq!(report.bags[0].key.len(), MAX_NAME);
        let other = json!({ "type": "observer", "kind": "hb" });
        assert_eq!(Report::from_trace(&trace("big", other)), None);
        let later = json!({ "type": "observer_debug", "v": 2 });
        assert_eq!(Report::from_trace(&trace("big", later)), None);
        assert_eq!(Report::from_trace(&TraceKind::Probe), None);
    }
}
