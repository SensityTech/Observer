//! The sockets a set of processes holds, read from the system's tables.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

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

/// A process listening on a TCP port. The system may keep whose it is to itself: another
/// account's process on Linux, a protected one on Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub pid: Option<u32>,
    pub executable: Option<PathBuf>,
}

/// The processes listening on TCP `port`, on any address of either family, each once. It reads
/// the system's tables and never binds the port. Blocks.
pub fn tcp_listeners(port: u16) -> io::Result<Vec<Listener>> {
    #[cfg(windows)]
    {
        crate::windows::sockets::tcp_listeners(port)
    }
    #[cfg(unix)]
    {
        crate::unix::sockets::tcp_listeners(port)
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn a_listening_process_is_found_by_its_port_with_its_executable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let own = std::env::current_exe().unwrap();

        let listeners = tcp_listeners(port).unwrap();

        assert_eq!(listeners.len(), 1, "{listeners:?}");
        assert_eq!(listeners[0].pid, Some(std::process::id()));
        assert_eq!(
            listeners[0]
                .executable
                .as_deref()
                .and_then(|path| path.file_name()),
            own.file_name()
        );
        drop(listener);
        assert_eq!(tcp_listeners(port).unwrap(), Vec::new());
    }
}
