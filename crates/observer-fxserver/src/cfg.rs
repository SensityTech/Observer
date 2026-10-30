//! The server's cfg as FXServer runs it: the files `exec` reaches, what they set, and what in them
//! works against Observer. The setup's report and the warnings of each launch come from this one
//! scan. The setup's reads stay inside the data folder, and no scan writes: the cfg is the owner's.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, UdpSocket};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use observer_domain::launch::{MAX_TEXT_CHARS, OneSync};
use observer_domain::server_data::{
    CfgFacts, CfgFile, CfgFileState, CfgFinding, Endpoint, EndpointProtocol, FindingKind, FindingParams,
    Game, LicenceState, Severity,
};

use crate::server_data::Resources;
use crate::{bridge, command};

/// The largest file the scan reads, the cfg or one it runs.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// How deep `exec` is followed, the cfg being 0.
const EXEC_DEPTH: usize = 5;
/// The most files one scan reads, the cfg included.
const MAX_FILES: usize = 32;
/// The most files a report names, read or not.
const MAX_LISTED: usize = 64;
const MAX_ENDPOINTS: usize = 16;
/// The endpoint commands checked, each against those before it; past them, a cfg adds more than
/// any server listens on, and the rest only count as written.
const MAX_CHECKED_ENDPOINTS: usize = 64;
/// Findings of one kind past this are left out; missing resources past it are counted instead.
const MAX_PER_KIND: usize = 20;
const MAX_HOSTNAME_CHARS: usize = 128;
/// The longest word of the cfg a finding quotes.
const MAX_WORD_CHARS: usize = 64;
/// Entries of the build's `system_resources` read.
const MAX_SYSTEM_RESOURCES: usize = 200;
const MAX_PLAYERS: RangeInclusive<u32> = 1..=2048;
/// Above this, FXServer needs OneSync.
const PLAYERS_WITHOUT_ONESYNC: u32 = 32;
/// txAdmin's port and the ones after it, which a txAdmin on the same host takes.
const TXADMIN_PORTS: RangeInclusive<u16> = 40120..=40150;
const MONITOR_NAMES: [&str; 3] = ["monitor", "txadmin", "txadminclient"];
/// The convars Observer sets on every launch, through argv or stdin.
const OBSERVER_CONVARS: [&str; 5] = [
    "onesync",
    "observer_txadmin_events",
    "sv_prometheusBasicAuthUser",
    "sv_prometheusBasicAuthPassword",
    "sv_playersToken",
];
/// Command line syntax, which FXServer does not run from a cfg.
const PLUS_COMMANDS: [&str; 4] = ["+set", "+setr", "+sets", "+exec"];
/// The line txAdmin writes above each line of the cfg it comments out.
const TXADMIN_MARK: &str = "## [txAdmin CFG validator]";

/// What the scan knows besides the files.
#[derive(Debug, Clone)]
pub struct Context {
    /// The panel's address, whose port FXServer must leave alone.
    pub panel: Option<SocketAddr>,
    /// What FXServer runs with: more than 32 players need OneSync.
    pub onesync: OneSync,
    /// Binds each endpoint to see whether it is free, which only says something while FXServer,
    /// which holds them, is stopped.
    pub probe_ports: bool,
    /// The selected build's `system_resources`, which the cfg may start too.
    pub system_resources: Option<PathBuf>,
    /// Reads the files `exec` reaches outside the data folder too, as FXServer runs them: at launch,
    /// where they may add the endpoint health is probed on. The setup's checks leave them unread:
    /// the folder they check comes from a request.
    pub follow_outside: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    pub files: Vec<CfgFile>,
    pub facts: CfgFacts,
    pub findings: Vec<CfgFinding>,
    /// The first `endpoint_add_tcp`, reached from this host: where health is probed.
    pub endpoint: Option<SocketAddr>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("not a file")]
    NotAFile,
    #[error("over 1 MiB")]
    TooLarge,
}

/// Reads a cfg as the scan reads every file: at most 1 MiB, bytes that are not UTF-8 replaced,
/// since FXServer reads bytes.
pub fn read(path: &Path) -> Result<String, ReadError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(ReadError::NotAFile);
    }
    if metadata.len() > MAX_BYTES {
        return Err(ReadError::TooLarge);
    }
    let mut bytes = Vec::new();
    // One byte past the limit tells a file that grew since its size was read.
    let read = File::open(path)?.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if u64::try_from(read).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(ReadError::TooLarge);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Scans `text`, the cfg `cfg_file` of `data`, and the files it runs. `resources` is what the walk
/// of the folder found, which the cfg starts.
pub fn scan(data: &Path, cfg_file: &str, text: &str, resources: &Resources, context: &Context) -> Scan {
    let mut scanner = Scanner {
        data,
        root: fs::canonicalize(data).ok(),
        context,
        files: Vec::new(),
        read: HashSet::new(),
        unchecked: false,
        findings: Vec::new(),
        found: HashMap::new(),
        endpoints: Vec::new(),
        endpoints_written: 0,
        health: None,
        started: Vec::new(),
        licence: None,
        max_clients: None,
        game: None,
        onesync: None,
        hostname: None,
    };
    if let Ok(resolved) = fs::canonicalize(data.join(cfg_file)) {
        scanner.read.insert(resolved);
    }
    scanner.list(cfg_file.to_owned(), CfgFileState::Read);
    scanner.file(cfg_file, text, 0);
    scanner.finish(cfg_file, resources)
}

/// FXServer listens on the address it is given; a wildcard is reached through loopback.
fn reachable(address: &str) -> Option<SocketAddr> {
    let mut address: SocketAddr = address.parse().ok()?;
    match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => address.set_ip(Ipv4Addr::LOCALHOST.into()),
        IpAddr::V6(ip) if ip.is_unspecified() => address.set_ip(Ipv6Addr::LOCALHOST.into()),
        _ => {}
    }
    Some(address)
}

/// Where a command is written.
#[derive(Debug, Clone)]
struct Place {
    file: String,
    line: u32,
}

/// An endpoint the cfg adds, valid and new.
#[derive(Debug)]
struct Written {
    protocol: EndpointProtocol,
    address: String,
    parsed: Option<SocketAddr>,
    /// What two endpoints compare by: the address as the system reads it when it is one, so that
    /// `[::]:30140` and `[0::0]:30140` are the same.
    key: String,
    place: Place,
}

struct Scanner<'a> {
    data: &'a Path,
    /// The data folder as the file system resolves it, which every file read must stay inside.
    root: Option<PathBuf>,
    context: &'a Context,
    files: Vec<CfgFile>,
    read: HashSet<PathBuf>,
    /// A file FXServer may run was not read: what the chain seems to lack may be in it.
    unchecked: bool,
    findings: Vec<CfgFinding>,
    found: HashMap<FindingKind, usize>,
    endpoints: Vec<Written>,
    endpoints_written: usize,
    health: Option<SocketAddr>,
    started: Vec<(String, Place)>,
    licence: Option<(LicenceState, Place)>,
    max_clients: Option<(String, Place)>,
    game: Option<(String, Place)>,
    onesync: Option<OneSync>,
    hostname: Option<String>,
}

impl Scanner<'_> {
    fn file(&mut self, name: &str, text: &str, depth: usize) {
        let mut edited: Option<(u32, u32)> = None;
        for (index, line) in text.lines().enumerate() {
            let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            if line.trim_start().starts_with(TXADMIN_MARK) {
                edited = Some(edited.map_or((number, 1), |(first, count)| (first, count + 1)));
            }
            for command in command::split(line) {
                let place = Place {
                    file: name.to_owned(),
                    line: number,
                };
                self.command(&place, &command::words(command), depth);
            }
        }
        if let Some((first, count)) = edited {
            let params = FindingParams {
                count: Some(count),
                ..FindingParams::default()
            };
            self.report(FindingKind::EditedByTxAdmin, name, Some(first), params);
        }
    }

    fn command(&mut self, place: &Place, words: &[String], depth: usize) {
        let Some((name, args)) = words.split_first() else {
            return;
        };
        let command = name.to_ascii_lowercase();
        let first = args.first().map(String::as_str);
        match (command.as_str(), first) {
            ("start" | "ensure" | "restart", Some(resource)) => self.start(place, resource),
            ("endpoint_add_tcp", _) => self.endpoint(place, EndpointProtocol::Tcp, args),
            ("endpoint_add_udp", _) => self.endpoint(place, EndpointProtocol::Udp, args),
            ("exec", Some(target)) => self.exec(place, target, depth),
            (plus, _) if PLUS_COMMANDS.contains(&plus) => {
                let params = FindingParams {
                    name: Some(plus.to_owned()),
                    ..FindingParams::default()
                };
                self.report_at(FindingKind::PlusCommand, place, params);
            }
            _ => {}
        }
        // A convar is set by `set` and its kin, or by its bare name followed by a value.
        let set = match command.as_str() {
            "set" | "sets" | "setr" | "seta" => args
                .split_first()
                .and_then(|(convar, rest)| Some((convar.as_str(), rest.first()?.as_str()))),
            _ => first.map(|value| (name.as_str(), value)),
        };
        if let Some((convar, value)) = set {
            self.convar(place, convar, value);
        }
    }

    fn convar(&mut self, place: &Place, convar: &str, value: &str) {
        let named = |known: &str| convar.eq_ignore_ascii_case(known);
        if named("sv_licenseKey") {
            // Only what the key looks like is kept, never the key.
            self.licence = Some((licence_state(value), place.clone()));
        } else if named("sv_hostname") {
            self.hostname = Some(quote(value, MAX_HOSTNAME_CHARS));
        } else if named("sv_maxclients") {
            self.max_clients = Some((quote(value, MAX_WORD_CHARS), place.clone()));
        } else if named("gamename") {
            let value = quote(value, MAX_WORD_CHARS);
            if self
                .game
                .as_ref()
                .is_some_and(|(previous, _)| !previous.eq_ignore_ascii_case(&value))
            {
                let params = FindingParams {
                    value: Some(value.clone()),
                    ..FindingParams::default()
                };
                self.report_at(FindingKind::GameTwice, place, params);
            }
            self.game = Some((value, place.clone()));
        } else if named("txAdminServerMode") {
            self.report_at(FindingKind::SetsServerMode, place, FindingParams::default());
        } else if named("monitorMode") {
            self.report_at(FindingKind::SetsMonitorMode, place, FindingParams::default());
        } else if let Some(known) = OBSERVER_CONVARS.into_iter().find(|known| named(known)) {
            if known == "onesync" {
                self.onesync = OneSync::from_name(&value.to_ascii_lowercase());
            }
            let params = FindingParams {
                name: Some(known.to_owned()),
                ..FindingParams::default()
            };
            self.report_at(FindingKind::SetsObserverConvar, place, params);
        }
    }

    fn start(&mut self, place: &Place, resource: &str) {
        let resource = quote(resource, MAX_WORD_CHARS);
        if let Some(monitor) = MONITOR_NAMES
            .into_iter()
            .find(|name| resource.eq_ignore_ascii_case(name))
        {
            let params = FindingParams {
                name: Some(monitor.to_owned()),
                ..FindingParams::default()
            };
            self.report_at(FindingKind::StartsMonitor, place, params);
        } else {
            self.started.push((resource, place.clone()));
        }
    }

    fn endpoint(&mut self, place: &Place, protocol: EndpointProtocol, args: &[String]) {
        self.endpoints_written += 1;
        // The probe's address, as before endpoints were checked: a cfg that ran keeps its health.
        if protocol == EndpointProtocol::Tcp && self.health.is_none() {
            self.health = args.first().and_then(|address| reachable(address));
        }
        if self.endpoints_written > MAX_CHECKED_ENDPOINTS {
            return;
        }
        let (address, port) = match args {
            [address] => match endpoint_port(address) {
                Some(port) => (address, port),
                None => return self.invalid_endpoint(place, protocol, Some(address)),
            },
            _ => return self.invalid_endpoint(place, protocol, args.first()),
        };
        let parsed = address.parse::<SocketAddr>().ok();
        let key = parsed.map_or_else(|| address.to_ascii_lowercase(), |parsed| parsed.to_string());
        let params = FindingParams {
            protocol: Some(protocol),
            address: Some(quote(address, MAX_WORD_CHARS)),
            ..FindingParams::default()
        };
        if self
            .endpoints
            .iter()
            .any(|written| written.protocol == protocol && written.key == key)
        {
            self.report_at(FindingKind::EndpointDuplicate, place, params);
            return;
        }
        if TXADMIN_PORTS.contains(&port) {
            self.report_at(FindingKind::EndpointReserved, place, params.clone());
        }
        let on_panel = protocol == EndpointProtocol::Tcp
            && parsed
                .zip(self.context.panel)
                .is_some_and(|(endpoint, panel)| overlaps(endpoint, panel));
        if on_panel {
            self.report_at(FindingKind::EndpointOnPanel, place, params);
        }
        self.endpoints.push(Written {
            protocol,
            address: address.clone(),
            parsed,
            key,
            place: place.clone(),
        });
    }

    fn invalid_endpoint(&mut self, place: &Place, protocol: EndpointProtocol, address: Option<&String>) {
        let params = FindingParams {
            protocol: Some(protocol),
            address: address.map(|address| quote(address, MAX_WORD_CHARS)),
            ..FindingParams::default()
        };
        self.report_at(FindingKind::EndpointInvalid, place, params);
    }

    /// Follows `exec <target>`, where FXServer looks for it: from the data folder.
    fn exec(&mut self, place: &Place, target: &str, depth: usize) {
        let written = quote(target, MAX_TEXT_CHARS);
        let params = FindingParams {
            target: Some(written.clone()),
            ..FindingParams::default()
        };
        if target.starts_with('@') {
            self.unchecked = true;
            self.list(written, CfgFileState::Resource);
            self.report_at(FindingKind::ExecResource, place, params);
            return;
        }
        if depth >= EXEC_DEPTH {
            self.unchecked = true;
            self.list(written, CfgFileState::TooDeep);
            self.report_at(FindingKind::ExecTooDeep, place, params);
            return;
        }
        let Ok(resolved) = fs::canonicalize(self.data.join(target)) else {
            self.list(written, CfgFileState::Missing);
            self.report_at(FindingKind::ExecMissing, place, params);
            return;
        };
        // Only the resolved path tells: `..`, a full path or a link may lead out of the data folder
        // or back into it.
        let inside = self
            .root
            .as_ref()
            .and_then(|root| resolved.strip_prefix(root).ok())
            .map(slashed);
        let Some(name) = inside.or_else(|| self.context.follow_outside.then(|| written.clone())) else {
            self.unchecked = true;
            self.list(written, CfgFileState::Outside);
            self.report_at(FindingKind::ExecOutside, place, params);
            return;
        };
        if self.read.contains(&resolved) {
            self.list(name, CfgFileState::Repeated);
            self.report_at(FindingKind::ExecRepeated, place, params);
            return;
        }
        if self.read.len() >= MAX_FILES {
            self.unchecked = true;
            self.list(name, CfgFileState::TooMany);
            return;
        }
        match read(&resolved) {
            Ok(text) => {
                self.read.insert(resolved);
                self.list(name.clone(), CfgFileState::Read);
                self.file(&name, &text, depth + 1);
            }
            Err(ReadError::TooLarge) => {
                self.unchecked = true;
                self.list(name, CfgFileState::TooLarge);
            }
            Err(ReadError::Io(_) | ReadError::NotAFile) => {
                self.list(name, CfgFileState::Missing);
                self.report_at(FindingKind::ExecMissing, place, params);
            }
        }
    }

    fn list(&mut self, path: String, state: CfgFileState) {
        if self.files.len() < MAX_LISTED {
            self.files.push(CfgFile { path, state });
        }
    }

    fn report_at(&mut self, kind: FindingKind, place: &Place, params: FindingParams) {
        self.report(kind, &place.file, Some(place.line), params);
    }

    fn report(&mut self, kind: FindingKind, file: &str, line: Option<u32>, params: FindingParams) {
        let found = self.found.entry(kind).or_default();
        *found += 1;
        if *found > MAX_PER_KIND {
            return;
        }
        self.findings.push(CfgFinding {
            kind,
            severity: kind.severity(),
            file: file.to_owned(),
            line,
            params,
        });
    }

    /// What the whole chain says, once every file is read.
    fn finish(mut self, cfg_file: &str, resources: &Resources) -> Scan {
        if self.endpoints_written == 0 {
            self.report(FindingKind::NoEndpoint, cfg_file, None, FindingParams::default());
            if self.unchecked
                && let Some(finding) = self.findings.last_mut()
            {
                finding.severity = Severity::Warning;
            }
        }
        self.pair_endpoints();
        if self.context.probe_ports {
            self.probe_ports();
        }
        let licence = self.licence(cfg_file);
        let max_clients = self.max_clients();
        let game = self.game();
        self.missing_resources(cfg_file, resources);
        for folder in resources.named_observer() {
            self.report(
                FindingKind::ResourceNamedObserver,
                folder,
                None,
                FindingParams::default(),
            );
        }
        let facts = CfgFacts {
            game,
            endpoints: self
                .endpoints
                .iter()
                .take(MAX_ENDPOINTS)
                .map(|written| Endpoint {
                    protocol: written.protocol,
                    address: quote(&written.address, MAX_WORD_CHARS),
                })
                .collect(),
            max_clients,
            onesync: self.onesync,
            hostname: self.hostname.take(),
            licence,
        };
        Scan {
            files: self.files,
            facts,
            findings: self.findings,
            endpoint: self.health,
        }
    }

    /// Players connect over both: a TCP address needs its UDP twin, and the reverse. Past the
    /// endpoints checked, a twin may be among those that were not.
    fn pair_endpoints(&mut self) {
        if self.endpoints_written > MAX_CHECKED_ENDPOINTS {
            return;
        }
        let unpaired: Vec<_> = self
            .endpoints
            .iter()
            .filter(|written| {
                !self
                    .endpoints
                    .iter()
                    .any(|other| other.protocol != written.protocol && other.key == written.key)
            })
            .map(|written| (written.place.clone(), endpoint_params(written)))
            .collect();
        for (place, params) in unpaired {
            self.report_at(FindingKind::EndpointUnpaired, &place, params);
        }
    }

    fn probe_ports(&mut self) {
        let taken: Vec<_> = self
            .endpoints
            .iter()
            .filter(|written| {
                written.parsed.is_some_and(|address| match written.protocol {
                    EndpointProtocol::Tcp => TcpListener::bind(address).is_err(),
                    EndpointProtocol::Udp => UdpSocket::bind(address).is_err(),
                })
            })
            .map(|written| (written.place.clone(), endpoint_params(written)))
            .collect();
        for (place, params) in taken {
            self.report_at(FindingKind::PortInUse, &place, params);
        }
    }

    fn licence(&mut self, cfg_file: &str) -> LicenceState {
        match self.licence.take() {
            Some((state @ (LicenceState::Placeholder | LicenceState::Malformed), place)) => {
                let params = FindingParams {
                    licence: Some(state),
                    ..FindingParams::default()
                };
                self.report_at(FindingKind::LicenceUnusable, &place, params);
                state
            }
            Some((state, _)) => state,
            None if self.unchecked => LicenceState::Unknown,
            None => {
                self.report(
                    FindingKind::LicenceMissing,
                    cfg_file,
                    None,
                    FindingParams::default(),
                );
                LicenceState::Missing
            }
        }
    }

    fn max_clients(&mut self) -> Option<u32> {
        let (value, place) = self.max_clients.take()?;
        let count = value.parse::<u32>().ok();
        match count {
            Some(count) if MAX_PLAYERS.contains(&count) => {
                if count > PLAYERS_WITHOUT_ONESYNC && self.context.onesync == OneSync::Off {
                    let params = FindingParams {
                        count: Some(count),
                        ..FindingParams::default()
                    };
                    self.report_at(FindingKind::MaxClientsNeedOneSync, &place, params);
                }
            }
            _ => {
                let params = FindingParams {
                    value: Some(value),
                    ..FindingParams::default()
                };
                self.report_at(FindingKind::MaxClientsInvalid, &place, params);
            }
        }
        count
    }

    fn game(&mut self) -> Option<Game> {
        let (value, place) = self.game.take()?;
        match value.to_ascii_lowercase().as_str() {
            "gta5" => Some(Game::Gta5),
            "rdr3" => Some(Game::Rdr3),
            _ => {
                let params = FindingParams {
                    value: Some(value),
                    ..FindingParams::default()
                };
                self.report_at(FindingKind::GameInvalid, &place, params);
                None
            }
        }
    }

    /// Each name started that neither the folder nor the build holds, once, the first 20 by name
    /// and the rest counted.
    fn missing_resources(&mut self, cfg_file: &str, resources: &Resources) {
        let system = self
            .context
            .system_resources
            .as_deref()
            .map(system_resources)
            .unwrap_or_default();
        let mut seen = HashSet::new();
        let missing: Vec<_> = std::mem::take(&mut self.started)
            .into_iter()
            .filter(|(name, _)| {
                let lower = name.to_ascii_lowercase();
                !resources.holds(name)
                    && !system.contains(&lower)
                    && lower != bridge::NAME
                    && seen.insert(lower)
            })
            .collect();
        for (name, place) in missing.iter().take(MAX_PER_KIND) {
            let params = FindingParams {
                name: Some(name.clone()),
                ..FindingParams::default()
            };
            self.report_at(FindingKind::ResourceMissing, place, params);
        }
        if missing.len() > MAX_PER_KIND {
            let params = FindingParams {
                count: u32::try_from(missing.len() - MAX_PER_KIND).ok(),
                ..FindingParams::default()
            };
            self.report(FindingKind::ResourceMissingMore, cfg_file, None, params);
        }
    }
}

/// The names of a build's system resources, in lower case.
fn system_resources(folder: &Path) -> HashSet<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .take(MAX_SYSTEM_RESOURCES)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| Some(entry.file_name().to_str()?.to_ascii_lowercase()))
        .collect()
}

fn endpoint_params(written: &Written) -> FindingParams {
    FindingParams {
        protocol: Some(written.protocol),
        address: Some(quote(&written.address, MAX_WORD_CHARS)),
        ..FindingParams::default()
    }
}

/// The port of an endpoint txAdmin accepts, `^\[?(([0-9.]{7,15})|([a-z0-9:]{2,29}))\]?:(\d{1,5})$`
/// without regard to case, when FXServer can also bind it.
fn endpoint_port(address: &str) -> Option<u16> {
    let (host, port) = address.rsplit_once(':')?;
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host);
    let ipv4 =
        (7..=15).contains(&host.len()) && host.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.');
    let other = (2..=29).contains(&host.len())
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b':');
    let digits = (1..=5).contains(&port.len()) && port.bytes().all(|byte| byte.is_ascii_digit());
    if !(ipv4 || other) || !digits {
        return None;
    }
    port.parse().ok().filter(|&port| port != 0)
}

/// Two listeners on one port meet when their addresses are the same or either is a wildcard.
fn overlaps(endpoint: SocketAddr, panel: SocketAddr) -> bool {
    endpoint.port() == panel.port()
        && (endpoint.ip() == panel.ip() || endpoint.ip().is_unspecified() || panel.ip().is_unspecified())
}

fn licence_state(key: &str) -> LicenceState {
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let key = key.trim();
    // `^cfxk_\w{1,60}_\w{1,20}$`: `_` is a word character too, so any split that fits will do.
    let cfxk = key.strip_prefix("cfxk_").is_some_and(|rest| {
        rest.bytes().all(word)
            && rest.bytes().enumerate().any(|(index, byte)| {
                byte == b'_' && (1..=60).contains(&index) && (1..=20).contains(&(rest.len() - index - 1))
            })
    });
    if key.is_empty() || key.eq_ignore_ascii_case("changeme") {
        LicenceState::Placeholder
    } else if cfxk || (key.len() == 32 && key.bytes().all(word)) {
        LicenceState::Set
    } else {
        LicenceState::Malformed
    }
}

fn slashed(relative: &Path) -> String {
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// A word of the cfg as a report quotes it: printable, and short.
fn quote(word: &str, max_chars: usize) -> String {
    word.chars().filter(|c| !c.is_control()).take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use tempfile::TempDir;

    use super::*;
    use crate::server_data;

    /// Two endpoints and a key: nothing to report.
    const QUIET: &str = "endpoint_add_tcp \"0.0.0.0:30140\"\n\
                         endpoint_add_udp \"0.0.0.0:30140\"\n\
                         sv_licenseKey cfxk_ObserverLabFixture_k3y\n";

    struct Folder {
        dir: TempDir,
    }

    impl Folder {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("data")).unwrap();
            Self { dir }
        }

        fn data(&self) -> PathBuf {
            self.dir.path().join("data")
        }

        fn file(&self, name: &str, text: &str) -> &Self {
            let path = self.data().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
            self
        }

        fn resource(&self, folder: &str) -> &Self {
            self.file(
                &format!("resources/{folder}/fxmanifest.lua"),
                "fx_version 'cerulean'\n",
            )
        }

        fn scan(&self, cfg: &str) -> Scan {
            self.scan_with(cfg, &context())
        }

        fn scan_with(&self, cfg: &str, context: &Context) -> Scan {
            self.file("server.cfg", cfg);
            let resources = server_data::walk(&self.data());
            scan(&self.data(), "server.cfg", cfg, &resources, context)
        }
    }

    fn context() -> Context {
        Context {
            panel: None,
            onesync: OneSync::On,
            probe_ports: false,
            system_resources: None,
            follow_outside: false,
        }
    }

    fn kinds(scan: &Scan) -> Vec<FindingKind> {
        scan.findings.iter().map(|finding| finding.kind).collect()
    }

    /// Where each finding is: `server.cfg:4`, or `server.cfg` for what the chain lacks.
    fn places(scan: &Scan, kind: FindingKind) -> Vec<String> {
        scan.findings
            .iter()
            .filter(|finding| finding.kind == kind)
            .map(|finding| match finding.line {
                Some(line) => format!("{}:{line}", finding.file),
                None => finding.file.clone(),
            })
            .collect()
    }

    fn states(scan: &Scan) -> Vec<(&str, CfgFileState)> {
        scan.files
            .iter()
            .map(|file| (file.path.as_str(), file.state))
            .collect()
    }

    /// A folder, a cfg and a context that make the scan report `kind`, and where it should. The
    /// match has no catch-all: a new kind does not build without its case.
    struct Case {
        folder: Folder,
        cfg: String,
        context: Context,
        at: &'static str,
        /// Holds a port for the time of the scan.
        _held: Option<TcpListener>,
    }

    fn case(kind: FindingKind) -> Case {
        let folder = Folder::new();
        let mut context = context();
        let mut held = None;
        let quiet = |extra: &str| format!("{QUIET}{extra}");
        let (cfg, at) = match kind {
            FindingKind::NoEndpoint => (
                "sv_licenseKey cfxk_ObserverLabFixture_k3y\n".to_owned(),
                "server.cfg",
            ),
            FindingKind::EndpointInvalid => (quiet("endpoint_add_udp \"30141\"\n"), "server.cfg:4"),
            FindingKind::EndpointOnPanel => {
                context.panel = Some(SocketAddr::from(([127, 0, 0, 1], 30140)));
                (QUIET.to_owned(), "server.cfg:1")
            }
            FindingKind::EndpointUnpaired => (
                "endpoint_add_tcp \"0.0.0.0:30140\"\nsv_licenseKey cfxk_ObserverLabFixture_k3y\n".to_owned(),
                "server.cfg:1",
            ),
            FindingKind::EndpointDuplicate => (quiet("endpoint_add_udp \"0.0.0.0:30140\"\n"), "server.cfg:4"),
            FindingKind::EndpointReserved => (QUIET.replace("30140", "40130"), "server.cfg:1"),
            FindingKind::PortInUse => {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
                let address = listener.local_addr().unwrap();
                held = Some(listener);
                context.probe_ports = true;
                (
                    format!(
                        "endpoint_add_tcp \"{address}\"\nendpoint_add_udp \"{address}\"\n\
                         sv_licenseKey cfxk_ObserverLabFixture_k3y\n"
                    ),
                    "server.cfg:1",
                )
            }
            FindingKind::StartsMonitor => (quiet("ensure monitor\n"), "server.cfg:4"),
            FindingKind::SetsServerMode => (quiet("set txAdminServerMode true\n"), "server.cfg:4"),
            FindingKind::SetsMonitorMode => (quiet("setr monitorMode true\n"), "server.cfg:4"),
            FindingKind::SetsObserverConvar => (quiet("onesync on\n"), "server.cfg:4"),
            FindingKind::PlusCommand => (quiet("+set sv_hostname Lab\n"), "server.cfg:4"),
            FindingKind::LicenceMissing => {
                (QUIET.lines().take(2).collect::<Vec<_>>().join("\n"), "server.cfg")
            }
            FindingKind::LicenceUnusable => (
                QUIET.replace("cfxk_ObserverLabFixture_k3y", "changeme"),
                "server.cfg:3",
            ),
            FindingKind::MaxClientsInvalid => (quiet("sv_maxclients 4096\n"), "server.cfg:4"),
            FindingKind::MaxClientsNeedOneSync => {
                context.onesync = OneSync::Off;
                (quiet("sv_maxclients 64\n"), "server.cfg:4")
            }
            FindingKind::GameInvalid => (quiet("set gamename gta6\n"), "server.cfg:4"),
            FindingKind::GameTwice => (quiet("gamename rdr3\ngamename gta5\n"), "server.cfg:5"),
            FindingKind::ResourceMissing => (quiet("ensure garage\n"), "server.cfg:4"),
            FindingKind::ResourceMissingMore => {
                let mut starts = String::new();
                for index in 0..21 {
                    writeln!(starts, "ensure missing_{index}").unwrap();
                }
                (quiet(&starts), "server.cfg")
            }
            FindingKind::ResourceNamedObserver => {
                folder.resource("[local]/observer");
                (QUIET.to_owned(), "resources/[local]/observer")
            }
            FindingKind::ExecMissing => (quiet("exec missing.cfg\n"), "server.cfg:4"),
            FindingKind::ExecOutside => {
                fs::write(folder.dir.path().join("shared.cfg"), "").unwrap();
                (quiet("exec ../shared.cfg\n"), "server.cfg:4")
            }
            FindingKind::ExecResource => (quiet("exec @chat/extra.cfg\n"), "server.cfg:4"),
            FindingKind::ExecTooDeep => {
                for depth in 1..=6 {
                    folder.file(&format!("c{depth}.cfg"), &format!("exec c{}.cfg\n", depth + 1));
                }
                (quiet("exec c1.cfg\n"), "c5.cfg:1")
            }
            FindingKind::ExecRepeated => (quiet("exec server.cfg\n"), "server.cfg:4"),
            FindingKind::EditedByTxAdmin => (
                quiet(
                    "## [txAdmin CFG validator]: onesync MUST only be set in the txAdmin settings page.\n# set onesync on\n",
                ),
                "server.cfg:4",
            ),
        };
        Case {
            folder,
            cfg,
            context,
            at,
            _held: held,
        }
    }

    #[test]
    fn a_cfg_that_adds_its_endpoints_and_sets_a_key_reports_nothing() {
        let folder = Folder::new();

        let scan = folder.scan(QUIET);

        assert_eq!(scan.findings, []);
        assert_eq!(states(&scan), [("server.cfg", CfgFileState::Read)]);
        assert_eq!(scan.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30140))));
    }

    #[test]
    fn every_kind_is_reported_where_it_is_written() {
        for kind in FindingKind::ALL {
            let case = case(kind);

            let scan = case.folder.scan_with(&case.cfg, &case.context);

            let found = places(&scan, kind);
            assert!(
                found.contains(&case.at.to_owned()),
                "{kind:?} at {found:?}, not {}",
                case.at
            );
            let finding = scan.findings.iter().find(|finding| finding.kind == kind).unwrap();
            assert_eq!(finding.severity, kind.severity(), "{kind:?}");
        }
    }

    #[test]
    fn line_numbers_hold_across_semicolons_and_block_comments() {
        let folder = Folder::new();

        let scan = folder.scan(&format!(
            "{QUIET}sv_hostname Lab; ensure monitor\n\
             /* ensure monitor */ set txAdminServerMode 1\n\
             \x20 ensure /* monitor */ txadmin // ensure monitor\n\
             # ensure monitor\n\
             start \"txAdminClient\";restart monitor\n"
        ));

        assert_eq!(
            places(&scan, FindingKind::StartsMonitor),
            ["server.cfg:4", "server.cfg:6", "server.cfg:8", "server.cfg:8"]
        );
        assert_eq!(places(&scan, FindingKind::SetsServerMode), ["server.cfg:5"]);
        let names: Vec<_> = scan
            .findings
            .iter()
            .filter_map(|finding| finding.params.name.as_deref())
            .collect();
        assert_eq!(
            names,
            ["monitor", "txadmin", "txadminclient", "monitor"],
            "the name Observer knows, whatever the cfg's case"
        );
    }

    #[test]
    fn each_convar_is_set_by_its_bare_name_or_any_set_command() {
        for form in ["", "set ", "sets ", "setr ", "seta ", "SET "] {
            let folder = Folder::new();
            let cfg = [
                "endpoint_add_tcp \"0.0.0.0:30140\"",
                "endpoint_add_udp \"0.0.0.0:30140\"",
                "sv_licenseKey cfxk_ObserverLabFixture_k3y",
                "sv_hostname \"^2Observer\tlab\"",
                "sv_maxclients 48",
                "gamename rdr3",
                "onesync legacy",
                "txAdminServerMode true",
                "monitorMode true",
                "observer_txadmin_events 1",
                "sv_prometheusBasicAuthUser observer",
                "sv_prometheusBasicAuthPassword secret",
                "sv_playersToken abc",
            ]
            .map(|line| match line.split_once(' ') {
                Some((convar, value)) if !convar.starts_with("endpoint") => format!("{form}{convar} {value}"),
                _ => line.to_owned(),
            })
            .join("\n");

            let scan = folder.scan(&cfg);

            assert_eq!(
                scan.facts,
                CfgFacts {
                    game: Some(Game::Rdr3),
                    endpoints: vec![
                        Endpoint {
                            protocol: EndpointProtocol::Tcp,
                            address: "0.0.0.0:30140".to_owned(),
                        },
                        Endpoint {
                            protocol: EndpointProtocol::Udp,
                            address: "0.0.0.0:30140".to_owned(),
                        },
                    ],
                    max_clients: Some(48),
                    onesync: Some(OneSync::Legacy),
                    hostname: Some("^2Observerlab".to_owned()),
                    licence: LicenceState::Set,
                },
                "{form:?}"
            );
            let found: Vec<_> = scan
                .findings
                .iter()
                .map(|finding| (finding.kind, finding.params.name.as_deref()))
                .collect();
            let observer = |name| (FindingKind::SetsObserverConvar, Some(name));
            assert_eq!(
                found,
                [
                    observer("onesync"),
                    (FindingKind::SetsServerMode, None),
                    (FindingKind::SetsMonitorMode, None),
                    observer("observer_txadmin_events"),
                    observer("sv_prometheusBasicAuthUser"),
                    observer("sv_prometheusBasicAuthPassword"),
                    observer("sv_playersToken"),
                ],
                "{form:?}"
            );
        }
    }

    #[test]
    fn a_set_without_a_value_sets_nothing() {
        let folder = Folder::new();

        let scan = folder.scan(&format!("{QUIET}set txAdminServerMode\nsv_hostname\n"));

        assert_eq!(scan.findings, []);
        assert_eq!(scan.facts.hostname, None);
    }

    #[test]
    fn the_last_setter_wins() {
        let folder = Folder::new();
        folder.file(
            "configs/more.cfg",
            "sv_hostname Second\nsv_maxclients 64\nsv_licenseKey changeme\n",
        );

        let scan = folder.scan(&format!(
            "sv_hostname First\nsv_licenseKey \"\"\nexec configs/more.cfg\nsv_maxclients 10\n{QUIET}onesync off\nonesync on\n"
        ));

        assert_eq!(scan.facts.hostname.as_deref(), Some("Second"));
        assert_eq!(scan.facts.max_clients, Some(10));
        assert_eq!(scan.facts.licence, LicenceState::Set);
        assert_eq!(scan.facts.onesync, Some(OneSync::On));
        assert_eq!(places(&scan, FindingKind::LicenceUnusable), Vec::<String>::new());
    }

    #[test]
    fn the_licence_is_judged_by_its_shape_alone() {
        let state = |line: &str| {
            let folder = Folder::new();
            let scan = folder.scan(&format!(
                "endpoint_add_tcp \"0.0.0.0:30140\"\nendpoint_add_udp \"0.0.0.0:30140\"\n{line}\n"
            ));
            let reported = scan
                .findings
                .iter()
                .find(|finding| {
                    matches!(
                        finding.kind,
                        FindingKind::LicenceMissing | FindingKind::LicenceUnusable
                    )
                })
                .map(|finding| (finding.kind, finding.params.licence));
            (scan.facts.licence, reported)
        };
        let unusable = |licence| Some((FindingKind::LicenceUnusable, Some(licence)));

        assert_eq!(
            state(""),
            (LicenceState::Missing, Some((FindingKind::LicenceMissing, None)))
        );
        assert_eq!(
            state("sv_licenseKey cfxk_1a2b3c4d5e6f_AbC123"),
            (LicenceState::Set, None)
        );
        assert_eq!(
            state(&format!("sv_licenseKey {}", "a1".repeat(16))),
            (LicenceState::Set, None)
        );
        for placeholder in [
            "sv_licenseKey \"\"",
            "sv_licenseKey changeme",
            "set sv_licenseKey CHANGEME",
        ] {
            let expected = (LicenceState::Placeholder, unusable(LicenceState::Placeholder));
            assert_eq!(state(placeholder), expected, "{placeholder}");
        }
        let long_account = format!("sv_licenseKey cfxk_{}_x", "a".repeat(61));
        let long_secret = format!("sv_licenseKey cfxk_a_{}", "b".repeat(21));
        let short = format!("sv_licenseKey {}", "a".repeat(31));
        for malformed in [
            "sv_licenseKey cfxk_nounderscore",
            "sv_licenseKey cfxk__AbC123",
            long_account.as_str(),
            long_secret.as_str(),
            "sv_licenseKey abc-123",
            short.as_str(),
        ] {
            let expected = (LicenceState::Malformed, unusable(LicenceState::Malformed));
            assert_eq!(state(malformed), expected, "{malformed}");
        }
        assert_eq!(
            state("exec @keys/licence.cfg"),
            (LicenceState::Unknown, None),
            "a cfg that was not read may set it"
        );
    }

    #[test]
    fn the_report_never_carries_a_secret_written_in_the_cfg() {
        let folder = Folder::new();
        let secrets = [
            "cfxk_ObserverSecretKey_9x8y7z",
            "not-a-key-but-still-secret",
            "Hunter2-rcon",
            "mysql://observer:p4ssw0rd@localhost/es_extended",
        ];
        folder.file(
            "secrets.cfg",
            &format!(
                "sv_licenseKey {}\nset rcon_password \"{}\"\nset mysql_connection_string \"{}\"\n",
                secrets[0], secrets[2], secrets[3]
            ),
        );

        let scan = folder.scan(&format!(
            "endpoint_add_tcp \"0.0.0.0:30140\"\nendpoint_add_udp \"0.0.0.0:30140\"\n\
             exec secrets.cfg\nsv_licenseKey {}; ensure monitor\nrcon_password {}\n",
            secrets[1], secrets[2]
        ));

        assert_eq!(scan.facts.licence, LicenceState::Malformed);
        let report = serde_json::to_string(&(&scan.files, &scan.facts, &scan.findings)).unwrap();
        for secret in secrets {
            assert!(!report.contains(secret), "{secret} reached {report}");
        }
    }

    #[test]
    fn exec_follows_files_inside_the_data_folder_only() {
        let folder = Folder::new();
        fs::write(folder.dir.path().join("shared.cfg"), "ensure monitor\n").unwrap();
        folder.file(
            "configs/net.cfg",
            "exec configs/../server.cfg\nexec ./configs/net.cfg\n",
        );
        let absolute = folder.dir.path().join("shared.cfg");

        let scan = folder.scan(&format!(
            "{QUIET}exec configs/net.cfg\nexec missing.cfg\nexec ../shared.cfg\nexec \"{}\"\nexec @chat/extra.cfg\n",
            absolute.display().to_string().replace('\\', "\\\\")
        ));

        assert_eq!(
            states(&scan),
            [
                ("server.cfg", CfgFileState::Read),
                ("configs/net.cfg", CfgFileState::Read),
                ("server.cfg", CfgFileState::Repeated),
                ("configs/net.cfg", CfgFileState::Repeated),
                ("missing.cfg", CfgFileState::Missing),
                ("../shared.cfg", CfgFileState::Outside),
                (absolute.to_str().unwrap(), CfgFileState::Outside),
                ("@chat/extra.cfg", CfgFileState::Resource),
            ]
        );
        assert_eq!(
            places(&scan, FindingKind::ExecRepeated),
            ["configs/net.cfg:1", "configs/net.cfg:2"]
        );
        assert_eq!(places(&scan, FindingKind::StartsMonitor), Vec::<String>::new());
        assert_eq!(
            places(&scan, FindingKind::ExecOutside),
            ["server.cfg:6", "server.cfg:7"]
        );
    }

    #[test]
    fn a_target_written_in_full_or_through_its_parent_is_followed_when_it_resolves_inside() {
        let folder = Folder::new();
        folder.file(
            "configs/net.cfg",
            "endpoint_add_tcp \"0.0.0.0:30140\"\nendpoint_add_udp \"0.0.0.0:30140\"\n",
        );
        folder.file("configs/keys.cfg", "sv_licenseKey cfxk_ObserverLabFixture_k3y\n");
        let absolute = folder.data().join("configs").join("net.cfg");

        let scan = folder.scan(&format!(
            "exec \"{}\"\nexec ../data/configs/keys.cfg\n",
            absolute.display().to_string().replace('\\', "\\\\")
        ));

        assert_eq!(
            states(&scan),
            [
                ("server.cfg", CfgFileState::Read),
                ("configs/net.cfg", CfgFileState::Read),
                ("configs/keys.cfg", CfgFileState::Read),
            ]
        );
        assert_eq!(scan.findings, []);
        assert_eq!(scan.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30140))));
    }

    #[test]
    fn a_link_out_of_the_data_folder_is_not_followed() {
        let folder = Folder::new();
        fs::write(folder.dir.path().join("shared.cfg"), "ensure monitor\n").unwrap();
        let link = folder.data().join("linked.cfg");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(folder.dir.path().join("shared.cfg"), &link);
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(folder.dir.path().join("shared.cfg"), &link);
        if linked.is_err() {
            // Windows grants links to administrators and developer mode only.
            return;
        }

        let scan = folder.scan(&format!("{QUIET}exec linked.cfg\n"));

        assert_eq!(
            states(&scan),
            [
                ("server.cfg", CfgFileState::Read),
                ("linked.cfg", CfgFileState::Outside)
            ]
        );
        assert_eq!(kinds(&scan), [FindingKind::ExecOutside]);
    }

    #[test]
    fn exec_stops_five_files_deep() {
        let folder = Folder::new();
        for depth in 1..=6 {
            folder.file(&format!("c{depth}.cfg"), &format!("exec c{}.cfg\n", depth + 1));
        }

        let scan = folder.scan(&format!("{QUIET}exec c1.cfg\n"));

        let read = scan
            .files
            .iter()
            .filter(|file| file.state == CfgFileState::Read)
            .count();
        assert_eq!(read, 6, "the cfg and five below it");
        assert_eq!(
            scan.files[6],
            CfgFile {
                path: "c6.cfg".to_owned(),
                state: CfgFileState::TooDeep
            }
        );
        assert_eq!(scan.facts.licence, LicenceState::Set);
    }

    #[test]
    fn a_scan_reads_32_files_of_1_mib_at_most() {
        let folder = Folder::new();
        let mut cfg = QUIET.to_owned();
        for index in 0..40 {
            folder.file(&format!("part{index}.cfg"), "sv_hostname Part\n");
            writeln!(cfg, "exec part{index}.cfg").unwrap();
        }
        let large = Folder::new();
        large.file("large.cfg", &"# padding\n".repeat(110_000));

        let many = folder.scan(&cfg);
        let oversized = large.scan("exec large.cfg\nsv_licenseKey cfxk_ObserverLabFixture_k3y\n");

        let count = |state| many.files.iter().filter(|file| file.state == state).count();
        assert_eq!((count(CfgFileState::Read), count(CfgFileState::TooMany)), (32, 9));
        assert_eq!(
            states(&oversized),
            [
                ("server.cfg", CfgFileState::Read),
                ("large.cfg", CfgFileState::TooLarge)
            ]
        );
        assert_eq!(oversized.findings[0].kind, FindingKind::NoEndpoint);
        assert_eq!(
            oversized.findings[0].severity,
            Severity::Warning,
            "the file left unread may add them"
        );
        assert!(matches!(
            read(&large.data().join("large.cfg")),
            Err(ReadError::TooLarge)
        ));
    }

    #[test]
    fn no_endpoint_blocks_unless_a_file_left_unread_may_add_them() {
        let folder = Folder::new();

        let alone = folder.scan("sv_licenseKey cfxk_ObserverLabFixture_k3y\n");
        let with_resource =
            folder.scan("sv_licenseKey cfxk_ObserverLabFixture_k3y\nexec @net/endpoints.cfg\n");

        let severity = |scan: &Scan| {
            scan.findings
                .iter()
                .find(|finding| finding.kind == FindingKind::NoEndpoint)
                .map(|finding| finding.severity)
        };
        assert_eq!(severity(&alone), Some(Severity::Blocking));
        assert_eq!(severity(&with_resource), Some(Severity::Warning));
    }

    #[test]
    fn endpoints_pair_by_address_and_name_what_is_wrong_with_each() {
        let folder = Folder::new();
        let panel = SocketAddr::from(([0, 0, 0, 0], 40125));
        let context = Context {
            panel: Some(panel),
            ..context()
        };

        let scan = folder.scan_with(
            "endpoint_add_tcp \"0.0.0.0:30140\"\n\
             endpoint_add_udp 0.0.0.0:30140\n\
             endpoint_add_tcp \"[::]:30141\"\n\
             endpoint_add_tcp \"127.0.0.1:40125\"\n\
             endpoint_add_udp \"127.0.0.1:40125\"\n\
             endpoint_add_tcp\n\
             endpoint_add_tcp \"0.0.0.0:30140\" extra\n\
             endpoint_add_udp \"0.0.0.0:99999\"\n\
             endpoint_add_udp \"server.example.com:30140\"\n\
             sv_licenseKey cfxk_ObserverLabFixture_k3y\n",
            &context,
        );

        let found: Vec<_> = scan
            .findings
            .iter()
            .map(|finding| {
                (
                    finding.kind,
                    finding.line,
                    finding.params.protocol,
                    finding.params.address.as_deref(),
                )
            })
            .collect();
        let (tcp, udp) = (Some(EndpointProtocol::Tcp), Some(EndpointProtocol::Udp));
        assert_eq!(
            found,
            [
                (
                    FindingKind::EndpointReserved,
                    Some(4),
                    tcp,
                    Some("127.0.0.1:40125")
                ),
                (
                    FindingKind::EndpointOnPanel,
                    Some(4),
                    tcp,
                    Some("127.0.0.1:40125")
                ),
                (
                    FindingKind::EndpointReserved,
                    Some(5),
                    udp,
                    Some("127.0.0.1:40125")
                ),
                (FindingKind::EndpointInvalid, Some(6), tcp, None),
                (FindingKind::EndpointInvalid, Some(7), tcp, Some("0.0.0.0:30140")),
                (FindingKind::EndpointInvalid, Some(8), udp, Some("0.0.0.0:99999")),
                (
                    FindingKind::EndpointInvalid,
                    Some(9),
                    udp,
                    Some("server.example.com:30140")
                ),
                (FindingKind::EndpointUnpaired, Some(3), tcp, Some("[::]:30141")),
            ]
        );
        assert_eq!(scan.facts.endpoints.len(), 5);
        assert_eq!(scan.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30140))));
    }

    #[test]
    fn endpoints_past_the_64th_are_counted_but_not_checked() {
        let folder = Folder::new();
        let mut cfg = "sv_licenseKey cfxk_ObserverLabFixture_k3y\n".to_owned();
        for port in 30200..30265 {
            writeln!(cfg, "endpoint_add_tcp \"127.0.0.1:{port}\"").unwrap();
        }
        cfg.push_str("endpoint_add_tcp \"127.0.0.1:30200\"\n");

        let scan = folder.scan(&cfg);

        assert_eq!(
            scan.findings,
            [],
            "neither paired, a twin may be past them, nor compared"
        );
        assert_eq!(scan.facts.endpoints.len(), MAX_ENDPOINTS);
        assert_eq!(scan.endpoint, Some(SocketAddr::from(([127, 0, 0, 1], 30200))));
    }

    #[test]
    fn a_free_port_is_not_reported_and_ports_are_left_alone_unless_asked() {
        // The system hands out a TCP port without looking at UDP: the pair must be free for both.
        let address = std::iter::repeat_with(|| {
            let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            free.local_addr().unwrap()
        })
        .find(|address| UdpSocket::bind(address).is_ok())
        .unwrap();
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken = held.local_addr().unwrap();
        let folder = Folder::new();
        let cfg = |address: SocketAddr| {
            format!(
                "endpoint_add_tcp \"{address}\"\nendpoint_add_udp \"{address}\"\n\
                 sv_licenseKey cfxk_ObserverLabFixture_k3y\n"
            )
        };
        let probing = Context {
            probe_ports: true,
            ..context()
        };

        let free = folder.scan_with(&cfg(address), &probing);
        let running = folder.scan_with(&cfg(taken), &context());

        assert_eq!(free.findings, []);
        assert_eq!(running.findings, [], "FXServer may hold its own ports");
    }

    #[test]
    fn a_started_name_must_be_a_resource_a_category_or_a_system_resource() {
        let folder = Folder::new();
        folder.resource("chat").resource("[local]/garage");
        let build = folder.dir.path().join("build/citizen/system_resources");
        fs::create_dir_all(build.join("spawnmanager")).unwrap();
        let context = Context {
            system_resources: Some(build),
            ..context()
        };

        let scan = folder.scan_with(
            &format!(
                "{QUIET}ensure chat\nensure GARAGE\nensure [local]\nstart spawnmanager\nensure observer\n\
                 ensure bank\nrestart bank\nensure [gangs]\n"
            ),
            &context,
        );

        assert_eq!(
            places(&scan, FindingKind::ResourceMissing),
            ["server.cfg:9", "server.cfg:11"]
        );
        assert_eq!(scan.findings[0].params.name.as_deref(), Some("bank"));
    }

    #[test]
    fn findings_of_one_kind_stop_at_twenty() {
        let folder = Folder::new();
        let mut starts = String::new();
        for index in 0..25 {
            writeln!(starts, "ensure monitor\nensure gone_{index}").unwrap();
        }

        let scan = folder.scan(&format!("{QUIET}{starts}"));

        assert_eq!(places(&scan, FindingKind::StartsMonitor).len(), 20);
        assert_eq!(places(&scan, FindingKind::ResourceMissing).len(), 20);
        let more = scan
            .findings
            .iter()
            .find(|finding| finding.kind == FindingKind::ResourceMissingMore)
            .unwrap();
        assert_eq!(more.params.count, Some(5));
    }

    #[test]
    fn a_wildcard_is_reached_through_loopback() {
        assert_eq!(
            reachable("0.0.0.0:30140"),
            Some("127.0.0.1:30140".parse().unwrap())
        );
        assert_eq!(reachable("[::]:30140"), Some("[::1]:30140".parse().unwrap()));
        assert_eq!(
            reachable("192.0.2.4:30140"),
            Some("192.0.2.4:30140".parse().unwrap())
        );
        assert_eq!(reachable("not an address"), None);
    }
}
