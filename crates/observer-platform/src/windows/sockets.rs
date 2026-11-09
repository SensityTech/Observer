use std::ffi::c_void;
use std::io;
use std::mem::{offset_of, size_of};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, FALSE, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP_STATE_ESTAB, MIB_TCP_STATE_LISTEN,
    MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID, MIB_UDPROW_OWNER_PID, MIB_UDPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use super::processes::image;
use crate::sockets::{Listener, Protocol, Socket, SocketState};

/// A table can grow between the call that sizes it and the one that fills it.
const TRIES: usize = 4;

fn tcp(buffer: *mut c_void, size: *mut u32, family: u16) -> u32 {
    // SAFETY: `buffer` is null or `size` bytes long, as the caller sizes it.
    unsafe { GetExtendedTcpTable(buffer, size, FALSE, u32::from(family), TCP_TABLE_OWNER_PID_ALL, 0) }
}

pub(crate) fn tcp_listeners(wanted: u16) -> io::Result<Vec<Listener>> {
    let listens = |state: u32, local: u32| {
        tcp_state(state.cast_signed()) == SocketState::Listen && port(local) == wanted
    };
    let v4 = rows::<MIB_TCPROW_OWNER_PID>(
        |buffer, size| tcp(buffer, size, AF_INET),
        offset_of!(MIB_TCPTABLE_OWNER_PID, table),
    )?;
    let v6 = rows::<MIB_TCP6ROW_OWNER_PID>(
        |buffer, size| tcp(buffer, size, AF_INET6),
        offset_of!(MIB_TCP6TABLE_OWNER_PID, table),
    )?;
    let mut pids: Vec<u32> = v4
        .iter()
        .filter(|row| listens(row.dwState, row.dwLocalPort))
        .map(|row| row.dwOwningPid)
        .chain(
            v6.iter()
                .filter(|row| listens(row.dwState, row.dwLocalPort))
                .map(|row| row.dwOwningPid),
        )
        .collect();
    pids.sort_unstable();
    pids.dedup();
    Ok(pids
        .into_iter()
        .map(|pid| Listener {
            pid: Some(pid),
            executable: image(pid),
        })
        .collect())
}

pub(crate) fn sockets(pids: &[u32]) -> io::Result<Vec<Socket>> {
    let mut sockets = Vec::new();
    let ours = |pid: u32| pids.contains(&pid);
    let udp = |buffer: *mut c_void, size: *mut u32, family: u16| {
        // SAFETY: as for TCP.
        unsafe { GetExtendedUdpTable(buffer, size, FALSE, u32::from(family), UDP_TABLE_OWNER_PID, 0) }
    };

    let v4 = rows::<MIB_TCPROW_OWNER_PID>(
        |buffer, size| tcp(buffer, size, AF_INET),
        offset_of!(MIB_TCPTABLE_OWNER_PID, table),
    )?;
    for row in v4.into_iter().filter(|row| ours(row.dwOwningPid)) {
        let state = tcp_state(row.dwState.cast_signed());
        sockets.push(Socket {
            pid: row.dwOwningPid,
            protocol: Protocol::Tcp,
            local: v4_address(row.dwLocalAddr, row.dwLocalPort),
            remote: (state != SocketState::Listen).then(|| v4_address(row.dwRemoteAddr, row.dwRemotePort)),
            state,
        });
    }
    let v6 = rows::<MIB_TCP6ROW_OWNER_PID>(
        |buffer, size| tcp(buffer, size, AF_INET6),
        offset_of!(MIB_TCP6TABLE_OWNER_PID, table),
    )?;
    for row in v6.into_iter().filter(|row| ours(row.dwOwningPid)) {
        let state = tcp_state(row.dwState.cast_signed());
        sockets.push(Socket {
            pid: row.dwOwningPid,
            protocol: Protocol::Tcp,
            local: v6_address(row.ucLocalAddr, row.dwLocalPort, row.dwLocalScopeId),
            remote: (state != SocketState::Listen)
                .then(|| v6_address(row.ucRemoteAddr, row.dwRemotePort, row.dwRemoteScopeId)),
            state,
        });
    }
    let v4 = rows::<MIB_UDPROW_OWNER_PID>(
        |buffer, size| udp(buffer, size, AF_INET),
        offset_of!(MIB_UDPTABLE_OWNER_PID, table),
    )?;
    for row in v4.into_iter().filter(|row| ours(row.dwOwningPid)) {
        sockets.push(udp_socket(
            row.dwOwningPid,
            v4_address(row.dwLocalAddr, row.dwLocalPort),
        ));
    }
    let v6 = rows::<MIB_UDP6ROW_OWNER_PID>(
        |buffer, size| udp(buffer, size, AF_INET6),
        offset_of!(MIB_UDP6TABLE_OWNER_PID, table),
    )?;
    for row in v6.into_iter().filter(|row| ours(row.dwOwningPid)) {
        let local = v6_address(row.ucLocalAddr, row.dwLocalPort, row.dwLocalScopeId);
        sockets.push(udp_socket(row.dwOwningPid, local));
    }
    Ok(sockets)
}

/// Windows' UDP table holds no remote end, connected or not.
fn udp_socket(pid: u32, local: SocketAddr) -> Socket {
    Socket {
        pid,
        protocol: Protocol::Udp,
        local,
        remote: None,
        state: SocketState::Other,
    }
}

fn tcp_state(state: i32) -> SocketState {
    match state {
        // Windows 11 build 26300 lists every listener in state 0, which MIB_TCP_STATE leaves undefined.
        0 | MIB_TCP_STATE_LISTEN => SocketState::Listen,
        MIB_TCP_STATE_ESTAB => SocketState::Established,
        _ => SocketState::Other,
    }
}

/// Reads one of the tables: a count, then `Row`s from `offset`. `fetch` is the call, given a
/// buffer (null to size it) and its size.
fn rows<Row: Copy>(fetch: impl Fn(*mut c_void, *mut u32) -> u32, offset: usize) -> io::Result<Vec<Row>> {
    let mut size = 0u32;
    // `u64`s, so that the table is aligned for any row.
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..TRIES {
        let pointer = if buffer.is_empty() {
            ptr::null_mut()
        } else {
            buffer.as_mut_ptr().cast()
        };
        match fetch(pointer, &raw mut size) {
            NO_ERROR if !buffer.is_empty() => {
                let bytes = buffer.len() * size_of::<u64>();
                // SAFETY: a filled table starts with its row count.
                let listed = unsafe { buffer.as_ptr().cast::<u32>().read() } as usize;
                let count = listed.min(bytes.saturating_sub(offset) / size_of::<Row>());
                // SAFETY: the rows start at `offset`, aligned since the buffer is, and `count` of
                // them fit inside it.
                let first = unsafe { buffer.as_ptr().cast::<u8>().add(offset).cast::<Row>() };
                // SAFETY: as above, for each row up to `count`.
                return Ok((0..count)
                    .map(|index| unsafe { first.add(index).read() })
                    .collect());
            }
            NO_ERROR => return Ok(Vec::new()),
            ERROR_INSUFFICIENT_BUFFER => {
                // Room for a few more rows, should some open before the next call.
                let wanted = size as usize + 16 * size_of::<Row>();
                buffer = vec![0; wanted.div_ceil(size_of::<u64>())];
                size = u32::try_from(buffer.len() * size_of::<u64>()).map_err(io::Error::other)?;
            }
            error => return Err(io::Error::from_raw_os_error(error.cast_signed())),
        }
    }
    Err(io::Error::other(
        "the socket table kept growing while it was read",
    ))
}

/// The tables hold addresses and ports in network order, the port in the low 16 bits of a
/// `u32`, which are its first two bytes on Windows' little-endian machines.
fn port(raw: u32) -> u16 {
    let [high, low, ..] = raw.to_ne_bytes();
    u16::from_be_bytes([high, low])
}

fn v4_address(address: u32, raw_port: u32) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::from(address.to_ne_bytes()),
        port(raw_port),
    ))
}

fn v6_address(address: [u8; 16], raw_port: u32, scope: u32) -> SocketAddr {
    SocketAddr::V6(SocketAddrV6::new(
        Ipv6Addr::from(address),
        port(raw_port),
        0,
        scope,
    ))
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream, UdpSocket};

    use windows_sys::Win32::NetworkManagement::IpHelper::MIB_TCP_STATE_TIME_WAIT;

    use super::*;

    #[test]
    fn a_process_s_listener_connection_and_udp_socket_are_listed_and_no_one_else_s() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stream = TcpStream::connect(address).unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
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
        assert!(sockets.contains(&udp_socket(own, udp.local_addr().unwrap())));
        assert!(sockets.iter().all(|socket| socket.pid == own));
        assert_eq!(super::sockets(&[]).unwrap(), Vec::new());
    }

    #[test]
    fn a_listener_in_the_undefined_state_0_is_still_listening() {
        assert_eq!(tcp_state(0), SocketState::Listen);
        assert_eq!(tcp_state(MIB_TCP_STATE_LISTEN), SocketState::Listen);
        assert_eq!(tcp_state(MIB_TCP_STATE_ESTAB), SocketState::Established);
        assert_eq!(tcp_state(MIB_TCP_STATE_TIME_WAIT), SocketState::Other);
    }

    #[test]
    fn ports_are_read_from_network_order() {
        assert_eq!(port(u32::from_ne_bytes([0x75, 0x30, 0, 0])), 30000);
        assert_eq!(
            v4_address(
                u32::from_ne_bytes([127, 0, 0, 1]),
                u32::from_ne_bytes([0x75, 0x3c, 0, 0])
            ),
            "127.0.0.1:30012".parse().unwrap()
        );
    }
}
