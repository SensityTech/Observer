//! Loopback ports for the servers these tests start, each given by the system: another run of the
//! tests on the same host, from another checkout, gets other ports.

// Each test file uses some of these helpers; all are test code, though not in `#[test]` functions.
#![allow(dead_code, clippy::unwrap_used)]

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::ops::RangeInclusive;
use std::sync::LazyLock;

use tokio::sync::{Semaphore, SemaphorePermit};

/// The loopback address a test holds its port on, which leaves 127.0.0.1 to FXServer.
const CLAIMS: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
/// txAdmin's ports, among those Linux gives: the cfg scan warns of an endpoint on one of them.
const TXADMIN: RangeInclusive<u16> = 40120..=40150;

/// A port of 127.0.0.1 the system gave, this test's own until dropped.
pub struct Port {
    pub number: u16,
    /// The system may give a port again as soon as nothing listens on it, and FXServer only
    /// listens once it has started. Whoever is given this one meanwhile, in this run or another,
    /// cannot claim it and asks for another.
    _claim: TcpListener,
    _turn: Option<SemaphorePermit<'static>>,
}

impl Port {
    pub fn free() -> Self {
        loop {
            let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let number = free.local_addr().unwrap().port();
            if TXADMIN.contains(&number) {
                continue;
            }
            match TcpListener::bind((CLAIMS, number)) {
                Ok(claim) => {
                    return Self {
                        number,
                        _claim: claim,
                        _turn: None,
                    };
                }
                Err(error) if error.kind() == ErrorKind::AddrInUse => {}
                Err(error) => panic!("cannot claim port {number} on {CLAIMS}: {error}"),
            }
        }
    }
}

/// Lends ports to the labs of a test file, to so many of them at a time, which bounds how many
/// servers the file runs at once.
pub struct Ports(Semaphore);

impl Ports {
    pub const fn new(at_a_time: usize) -> Self {
        Self(Semaphore::const_new(at_a_time))
    }

    pub async fn lease(&'static self) -> Port {
        let turn = self.0.acquire().await.unwrap();
        Port {
            _turn: Some(turn),
            ..Port::free()
        }
    }
}

/// The panel's address the labs give Observer, which the cfg checks only compare with: a port held
/// for the whole run, so that no server is given it.
pub fn panel() -> SocketAddr {
    static PANEL: LazyLock<TcpListener> =
        LazyLock::new(|| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap());
    PANEL.local_addr().unwrap()
}
