//! The sockets a set of processes holds, read from the system's tables.

use std::io;
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Listen,
    Established,
    /// Opening, closing, or a UDP socket.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socket {
    pub pid: u32,
    pub protocol: Protocol,
    pub local: SocketAddr,
    /// Known for TCP, and on Linux for a connected UDP socket.
    pub remote: Option<SocketAddr>,
    pub state: SocketState,
}

/// The sockets the processes `pids` hold now. Blocks: on Windows it reads the whole host's tables,
/// on Linux the processes' descriptors and their network namespace's tables, through the first
/// pid's.
pub fn sockets(pids: &[u32]) -> io::Result<Vec<Socket>> {
    #[cfg(windows)]
    {
        crate::windows::sockets::sockets(pids)
    }
    #[cfg(unix)]
    {
        crate::unix::sockets::sockets(pids)
    }
}
