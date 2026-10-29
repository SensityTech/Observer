//! Where the tree's processes connect, from the sockets they hold every two seconds: outbound
//! destinations with their open count and peak, and the endpoints they listen on.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use jiff::{SignedDuration, Timestamp};
use observer_domain::diagnostics::{Destination, Listener, ListenerRole, Protocol, Scope, Service};
use observer_platform::{Socket, SocketState};
use observer_store::diagnostics::SeenConnection;

/// A destination seen open is written again after this long, so that its last sighting stays
/// close in the history.
const REFRESH: SignedDuration = SignedDuration::from_secs(60);
/// The panel lists the destinations seen this recently.
pub(super) const WINDOW: SignedDuration = SignedDuration::from_hours(24);
pub(super) const MAX_LISTED: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct Key {
    pub protocol: Protocol,
    pub address: String,
    pub port: u16,
    pub process: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Known {
    first_seen: Timestamp,
    last_seen: Timestamp,
    peak: u32,
    open: u32,
    /// When it was last written; `None` until it is.
    written: Option<Timestamp>,
}

/// What one look at the sockets found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Look {
    pub destinations: BTreeMap<Key, u32>,
    pub listeners: Vec<Listener>,
}

/// Sorts the tree's sockets: a TCP listener, or an unconnected UDP socket, is a listener; an
/// established connection on a listening port is inbound and counts on it; every other
/// established TCP connection, and every connected UDP socket, goes to a destination. `names` gives each pid's
/// program, and `game` the port players reach.
pub(super) fn look(sockets: &[Socket], names: &HashMap<u32, String>, game: Option<u16>) -> Look {
    let name = |pid: u32| names.get(&pid).cloned().unwrap_or_else(|| pid.to_string());
    let protocol = |socket: &Socket| match socket.protocol {
        observer_platform::Protocol::Tcp => Protocol::Tcp,
        observer_platform::Protocol::Udp => Protocol::Udp,
    };
    let listening: Vec<&Socket> = sockets
        .iter()
        .filter(|socket| {
            socket.state == SocketState::Listen
                || (protocol(socket) == Protocol::Udp && socket.remote.is_none())
        })
        .collect();
    let tcp_ports: Vec<u16> = listening
        .iter()
        .filter(|socket| protocol(socket) == Protocol::Tcp)
        .map(|socket| socket.local.port())
        .collect();
    let mut look = Look::default();
    let mut inbound: HashMap<(u32, u16), u32> = HashMap::new();
    for socket in sockets {
        let Some(remote) = socket.remote.filter(|_| socket.state != SocketState::Listen) else {
            continue;
        };
        // A connection opening or closing is not open: a closed one lingers in FIN_WAIT for as
        // long as its peer keeps its end.
        if protocol(socket) == Protocol::Tcp && socket.state != SocketState::Established {
            continue;
        }
        if protocol(socket) == Protocol::Tcp && tcp_ports.contains(&socket.local.port()) {
            if socket.state == SocketState::Established {
                *inbound.entry((socket.pid, socket.local.port())).or_default() += 1;
            }
            continue;
        }
        let remote = unmapped(remote);
        let key = Key {
            protocol: protocol(socket),
            address: remote.ip().to_string(),
            port: remote.port(),
            process: name(socket.pid),
        };
        *look.destinations.entry(key).or_default() += 1;
    }
    for socket in listening {
        let local = unmapped(socket.local);
        let protocol = protocol(socket);
        let listener = Listener {
            protocol,
            address: local.ip().to_string(),
            port: local.port(),
            process: name(socket.pid),
            role: (game == Some(local.port())).then_some(ListenerRole::Game),
            inbound: if protocol == Protocol::Tcp {
                inbound.get(&(socket.pid, local.port())).copied().unwrap_or(0)
            } else {
                0
            },
        };
        if !look.listeners.contains(&listener) {
            look.listeners.push(listener);
        }
    }
    look.listeners
        .sort_by(|a, b| (a.port, a.protocol, &a.address).cmp(&(b.port, b.protocol, &b.address)));
    look
}

/// The destinations as the looks find them, and what to write of them.
#[derive(Debug, Default)]
pub(super) struct Destinations {
    known: HashMap<Key, Known>,
    /// What the panel was last told: the open set and the counts.
    published: Vec<(Key, u32, u32)>,
    listeners: Vec<Listener>,
}

impl Destinations {
    /// Starts from what the history holds.
    pub(super) fn restored(seen: Vec<SeenConnection>) -> Self {
        let known = seen
            .into_iter()
            .map(|seen| {
                let key = Key {
                    protocol: seen.protocol,
                    address: seen.address,
                    port: seen.port,
                    process: seen.process,
                };
                let known = Known {
                    first_seen: seen.first_seen,
                    last_seen: seen.last_seen,
                    peak: seen.peak,
                    open: 0,
                    written: Some(seen.last_seen),
                };
                (key, known)
            })
            .collect();
        Self {
            known,
            ..Self::default()
        }
    }

    /// Takes in a look: returns what to write, and whether the panel must hear of it.
    pub(super) fn update(&mut self, look: Look, now: Timestamp) -> (Vec<SeenConnection>, bool) {
        let mut writes = Vec::new();
        for known in self.known.values_mut() {
            known.open = 0;
        }
        for (key, open) in look.destinations {
            let known = self.known.entry(key.clone()).or_insert(Known {
                first_seen: now,
                last_seen: now,
                peak: 0,
                open: 0,
                written: None,
            });
            known.open = open;
            known.last_seen = now;
            let higher = open > known.peak;
            known.peak = known.peak.max(open);
            let stale = known
                .written
                .is_none_or(|written| written.duration_until(now) >= REFRESH);
            if higher || stale {
                known.written = Some(now);
                writes.push(SeenConnection {
                    protocol: key.protocol,
                    address: key.address,
                    port: key.port,
                    process: key.process,
                    first_seen: known.first_seen,
                    last_seen: now,
                    peak: known.peak,
                });
            }
        }
        let mut published: Vec<(Key, u32, u32)> = self
            .known
            .iter()
            .filter(|(_, known)| known.open > 0)
            .map(|(key, known)| (key.clone(), known.open, known.peak))
            .collect();
        published.sort();
        let changed = published != self.published || look.listeners != self.listeners;
        self.published = published;
        self.listeners = look.listeners;
        (writes, changed)
    }

    /// The destinations seen in the last 24 hours, the most recent first.
    pub(super) fn listed(&self, now: Timestamp) -> Vec<Destination> {
        let mut listed: Vec<Destination> = self
            .known
            .iter()
            .filter(|(_, known)| known.last_seen >= now - WINDOW)
            .map(|(key, known)| destination(key, known))
            .collect();
        listed.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(b.open.cmp(&a.open)));
        listed.truncate(MAX_LISTED);
        listed
    }

    pub(super) fn listeners(&self) -> Vec<Listener> {
        self.listeners.clone()
    }

    /// Forgets what the history no longer keeps.
    pub(super) fn forget_before(&mut self, before: Timestamp) {
        self.known.retain(|_, known| known.last_seen >= before);
    }
}

fn destination(key: &Key, known: &Known) -> Destination {
    let ip: Option<IpAddr> = key.address.parse().ok();
    Destination {
        protocol: key.protocol,
        address: key.address.clone(),
        port: key.port,
        process: key.process.clone(),
        scope: ip.map_or(Scope::Public, scope),
        service: service(key.port),
        open: known.open,
        peak: known.peak,
        first_seen: known.first_seen,
        last_seen: known.last_seen,
    }
}

fn unmapped(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V6(v6) => v6
            .ip()
            .to_ipv4_mapped()
            .map_or(address, |ip| SocketAddr::new(IpAddr::V4(ip), v6.port())),
        SocketAddr::V4(_) => address,
    }
}

pub(super) fn scope(ip: IpAddr) -> Scope {
    match ip {
        IpAddr::V4(ip) if ip.is_loopback() => Scope::Loopback,
        IpAddr::V4(ip) if ip.is_private() || ip.is_link_local() || shared(ip) => Scope::Private,
        IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST => Scope::Loopback,
        IpAddr::V6(ip) if (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80 => {
            Scope::Private
        }
        IpAddr::V4(_) | IpAddr::V6(_) => Scope::Public,
    }
}

/// `100.64.0.0/10`, the carrier-grade NAT range.
fn shared(ip: Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    first == 100 && (second & 0xc0) == 64
}

pub(super) fn service(port: u16) -> Option<Service> {
    Some(match port {
        3306 => Service::Mysql,
        5432 => Service::Postgresql,
        6379 => Service::Redis,
        27017 => Service::Mongodb,
        443 => Service::Https,
        80 => Service::Http,
        53 => Service::Dns,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FXSERVER: u32 = 3312;
    const NODE: u32 = 4400;

    fn socket(
        pid: u32,
        protocol: observer_platform::Protocol,
        local: &str,
        remote: Option<&str>,
        state: SocketState,
    ) -> Socket {
        Socket {
            pid,
            protocol,
            local: local.parse().unwrap(),
            remote: remote.map(|remote| remote.parse().unwrap()),
            state,
        }
    }

    fn names() -> HashMap<u32, String> {
        HashMap::from([
            (FXSERVER, "FXServer.exe".to_owned()),
            (NODE, "FXServer.exe".to_owned()),
        ])
    }

    fn at(seconds: i64) -> Timestamp {
        "2026-10-29T10:00:00Z".parse::<Timestamp>().unwrap() + SignedDuration::from_secs(seconds)
    }

    #[test]
    fn sockets_sort_into_listeners_inbound_connections_and_destinations() {
        use observer_platform::Protocol::{Tcp, Udp};
        let sockets = [
            socket(FXSERVER, Tcp, "0.0.0.0:30120", None, SocketState::Listen),
            socket(FXSERVER, Udp, "0.0.0.0:30120", None, SocketState::Other),
            socket(
                FXSERVER,
                Tcp,
                "10.0.0.2:30120",
                Some("203.0.113.9:51000"),
                SocketState::Established,
            ),
            socket(
                FXSERVER,
                Tcp,
                "127.0.0.1:50211",
                Some("127.0.0.1:3306"),
                SocketState::Established,
            ),
            socket(
                FXSERVER,
                Tcp,
                "127.0.0.1:50212",
                Some("127.0.0.1:3306"),
                SocketState::Established,
            ),
            // Closed by FXServer, waiting for the database to close its end.
            socket(
                FXSERVER,
                Tcp,
                "127.0.0.1:50213",
                Some("127.0.0.1:3306"),
                SocketState::Other,
            ),
            socket(
                NODE,
                Tcp,
                "[::ffff:10.0.0.2]:50300",
                Some("[::ffff:162.159.135.232]:443"),
                SocketState::Established,
            ),
            socket(
                NODE,
                Udp,
                "10.0.0.2:50400",
                Some("1.1.1.1:53"),
                SocketState::Other,
            ),
        ];

        let look = look(&sockets, &names(), Some(30120));

        let destinations: Vec<(String, u16, u32)> = look
            .destinations
            .iter()
            .map(|(key, open)| (key.address.clone(), key.port, *open))
            .collect();
        assert_eq!(
            destinations,
            [
                ("127.0.0.1".to_owned(), 3306, 2),
                ("162.159.135.232".to_owned(), 443, 1),
                ("1.1.1.1".to_owned(), 53, 1),
            ]
        );
        assert_eq!(look.listeners.len(), 2);
        assert!(
            look.listeners
                .iter()
                .all(|listener| listener.role == Some(ListenerRole::Game))
        );
        let tcp = look
            .listeners
            .iter()
            .find(|listener| listener.protocol == Protocol::Tcp)
            .unwrap();
        assert_eq!(tcp.inbound, 1);
    }

    #[test]
    fn addresses_get_a_scope_and_well_known_ports_a_service() {
        let scope_of = |text: &str| scope(text.parse().unwrap());

        assert_eq!(scope_of("127.0.0.1"), Scope::Loopback);
        assert_eq!(scope_of("::1"), Scope::Loopback);
        for private in [
            "10.1.2.3",
            "172.20.0.1",
            "192.168.1.10",
            "100.64.0.1",
            "169.254.1.1",
            "fd00::1",
            "fe80::1",
        ] {
            assert_eq!(scope_of(private), Scope::Private, "{private}");
        }
        for public in ["8.8.8.8", "100.128.0.1", "2001:db8::1"] {
            assert_eq!(scope_of(public), Scope::Public, "{public}");
        }
        assert_eq!(service(3306), Some(Service::Mysql));
        assert_eq!(service(30120), None);
    }

    fn mysql(open: u32) -> Look {
        let key = Key {
            protocol: Protocol::Tcp,
            address: "127.0.0.1".to_owned(),
            port: 3306,
            process: "FXServer.exe".to_owned(),
        };
        Look {
            destinations: BTreeMap::from([(key, open)]),
            listeners: Vec::new(),
        }
    }

    #[test]
    fn a_destination_is_written_when_new_when_its_peak_grows_and_a_minute_after_its_last_write() {
        let mut destinations = Destinations::default();

        let (new, told) = destinations.update(mysql(2), at(0));
        let (same, quiet) = destinations.update(mysql(2), at(2));
        let (fewer, fewer_told) = destinations.update(mysql(1), at(4));
        let (higher, _) = destinations.update(mysql(3), at(6));
        let (refreshed, _) = destinations.update(mysql(3), at(66));

        assert_eq!(new.len(), 1);
        assert!(told);
        assert_eq!(same.len(), 0);
        assert!(!quiet, "nothing changed");
        assert_eq!(fewer.len(), 0);
        assert!(fewer_told, "the open count changed");
        assert_eq!(higher[0].peak, 3);
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].first_seen, at(0));
        assert_eq!(refreshed[0].last_seen, at(66));
    }

    #[test]
    fn a_closed_destination_stays_listed_with_nothing_open_until_it_leaves_the_window() {
        let mut destinations = Destinations::default();
        destinations.update(mysql(2), at(0));

        let (_, told) = destinations.update(Look::default(), at(2));

        assert!(told);
        let listed = destinations.listed(at(2));
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].open, listed[0].peak), (0, 2));
        assert_eq!(listed[0].service, Some(Service::Mysql));
        assert_eq!(
            destinations
                .listed(at(2) + WINDOW + SignedDuration::from_secs(1))
                .len(),
            0
        );
    }
}
