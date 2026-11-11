//! Loopback ports for the servers these tests start, each given by the system: another run of the
//! tests on the same host, from another checkout, gets other ports.

// Each test file uses some of these helpers; all are test code, though not in `#[test]` functions.
#![allow(dead_code, clippy::unwrap_used)]

use std::io::ErrorKind;
use std::net::{Ipv4Addr, TcpListener};
use std::ops::RangeInclusive;

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
                    };
                }
                Err(error) if error.kind() == ErrorKind::AddrInUse => {}
                Err(error) => panic!("cannot claim port {number} on {CLAIMS}: {error}"),
            }
        }
    }
}
