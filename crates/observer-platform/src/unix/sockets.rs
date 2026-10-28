use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::sockets::{Protocol, Socket, SocketState};

const LISTEN: u8 = 0x0A;
const ESTABLISHED: u8 = 0x01;

pub(crate) fn sockets(pids: &[u32]) -> io::Result<Vec<Socket>> {
    let mut owners = HashMap::new();
    for &pid in pids {
        let Ok(descriptors) = fs::read_dir(format!("/proc/{pid}/fd")) else {
            continue;
        };
        for descriptor in descriptors.filter_map(Result::ok) {
            if let Some(inode) = fs::read_link(descriptor.path())
                .ok()
                .and_then(|target| socket_inode(&target.to_string_lossy()))
            {
                owners.insert(inode, pid);
            }
        }
    }
    let Some(&namespace) = pids.first() else {
        return Ok(Vec::new());
    };
    let mut sockets = Vec::new();
    for (file, protocol) in [
        ("tcp", Protocol::Tcp),
        ("tcp6", Protocol::Tcp),
        ("udp", Protocol::Udp),
        ("udp6", Protocol::Udp),
    ] {
        let table = match fs::read_to_string(format!("/proc/{namespace}/net/{file}")) {
            Ok(table) => table,
            // A host without IPv6 has no `tcp6`.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        sockets.extend(parse_table(&table, protocol).into_iter().filter_map(|row| {
            Some(Socket {
                pid: *owners.get(&row.inode)?,
                ..row.socket
            })
        }));
    }
    Ok(sockets)
}

/// `socket:[12345]`, as a descriptor's link names a socket.
fn socket_inode(target: &str) -> Option<u64> {
    target.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

struct Row {
    inode: u64,
    /// With a pid of 0 until its owner is known.
    socket: Socket,
}

/// The rows of one of `/proc/net/{tcp,tcp6,udp,udp6}` that a process holds: the header line, rows
/// without an owner (inode 0, a `TIME_WAIT` connection) and rows cut short are left out.
fn parse_table(table: &str, protocol: Protocol) -> Vec<Row> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| parse_row(line, protocol))
        .collect()
}

fn parse_row(line: &str, protocol: Protocol) -> Option<Row> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let local = endpoint(fields.get(1)?)?;
    let remote = endpoint(fields.get(2)?)?;
    let status = u8::from_str_radix(fields.get(3)?, 16).ok()?;
    let inode: u64 = fields.get(9)?.parse().ok()?;
    if inode == 0 {
        return None;
    }
    let connected = !remote.ip().is_unspecified() || remote.port() != 0;
    let (state, remote) = match (protocol, status) {
        (Protocol::Tcp, LISTEN) => (SocketState::Listen, None),
        (Protocol::Tcp, ESTABLISHED) => (SocketState::Established, Some(remote)),
        (Protocol::Tcp, _) => (SocketState::Other, Some(remote)),
        (Protocol::Udp, _) => (SocketState::Other, connected.then_some(remote)),
    };
    Some(Row {
        inode,
        socket: Socket {
            pid: 0,
            protocol,
            local,
            remote,
            state,
        },
    })
}

/// `0100007F:1F90`: the address as the kernel's own words in hex, one for IPv4 and four for IPv6,
/// then the port in hex. An IPv4 address mapped into IPv6 is read as IPv4.
fn endpoint(text: &str) -> Option<SocketAddr> {
    let (address, port) = text.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let word = |hex: &str| u32::from_str_radix(hex, 16).ok().map(u32::to_ne_bytes);
    let ip = match address.len() {
        8 => IpAddr::V4(Ipv4Addr::from(word(address)?)),
        32 => {
            let mut bytes = [0u8; 16];
            for (index, chunk) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                chunk.copy_from_slice(&word(address.get(index * 8..index * 8 + 8)?)?);
            }
            let ip = Ipv6Addr::from(bytes);
            ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4)
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream, UdpSocket};

    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:7596 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 31337 1 0000000000000000 100 0 0 10 0
   1: 0100007F:C350 0100007F:0CEA 01 00000000:00000000 00:00000000 00000000  1000        0 31338 1 0000000000000000 20 4 30 10 -1
   2: 0100007F:C352 0100007F:0CEA 06 00000000:00000000 03:00001234 00000000     0        0 0 3 0000000000000000
   3: 0100007F:C354
";

    #[test]
    fn a_tcp_table_gives_listeners_and_connections_and_skips_what_no_one_holds() {
        let rows = parse_table(TCP, Protocol::Tcp);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].inode, 31337);
        assert_eq!(rows[0].socket.local, "127.0.0.1:30102".parse().unwrap());
        assert_eq!(rows[0].socket.state, SocketState::Listen);
        assert_eq!(rows[0].socket.remote, None);
        assert_eq!(rows[1].socket.state, SocketState::Established);
        assert_eq!(rows[1].socket.remote, Some("127.0.0.1:3306".parse().unwrap()));
    }

    #[test]
    fn ipv6_addresses_are_four_words_and_mapped_ipv4_reads_as_ipv4() {
        assert_eq!(
            endpoint("00000000000000000000000001000000:0050"),
            Some("[::1]:80".parse().unwrap())
        );
        assert_eq!(
            endpoint("0000000000000000FFFF00000100007F:01BB"),
            Some("127.0.0.1:443".parse().unwrap())
        );
        assert_eq!(
            endpoint("B80D01200000000000000000FE000000:0035"),
            Some("[2001:db8::fe]:53".parse().unwrap())
        );
        assert_eq!(endpoint("0100007F"), None);
        assert_eq!(endpoint("01007F:0050"), None);
    }

    #[test]
    fn a_udp_socket_has_a_remote_only_once_connected() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
  10: 0100007F:7596 00000000:0000 07 00000000:00000000 00:00000000 00000000  1000        0 40001 2 0000000000000000 0
  11: 0100007F:D431 0100007F:0035 01 00000000:00000000 00:00000000 00000000  1000        0 40002 2 0000000000000000 0
";

        let rows = parse_table(table, Protocol::Udp);

        assert_eq!(rows[0].socket.remote, None);
        assert_eq!(rows[1].socket.remote, Some("127.0.0.1:53".parse().unwrap()));
        assert!(rows.iter().all(|row| row.socket.state == SocketState::Other));
    }

    #[test]
    fn a_process_s_own_sockets_are_found_through_its_descriptors() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stream = TcpStream::connect(address).unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.connect(address).unwrap();
        let own = std::process::id();

        let sockets = sockets(&[own]).unwrap();

        assert!(sockets.contains(&Socket {
            pid: own,
            protocol: Protocol::Tcp,
            local: address,
            remote: None,
            state: SocketState::Listen,
        }));
        assert!(sockets.contains(&Socket {
            pid: own,
            protocol: Protocol::Tcp,
            local: stream.local_addr().unwrap(),
            remote: Some(address),
            state: SocketState::Established,
        }));
        assert!(sockets.contains(&Socket {
            pid: own,
            protocol: Protocol::Udp,
            local: udp.local_addr().unwrap(),
            remote: Some(address),
            state: SocketState::Other,
        }));
        assert_eq!(super::sockets(&[]).unwrap(), Vec::new());
    }
}
