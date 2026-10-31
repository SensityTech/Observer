//! `fake-archive`, which serves the fake as a build for Observer to install. The build tests and
//! the browser tests wait for the line it prints once it listens.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// The build tests' port: Cargo runs one test binary at a time.
const PORT: u16 = 41956;

/// The first line on `stdout`, unless none comes within 30 seconds.
fn first_line(stdout: ChildStdout) -> Option<String> {
    let (sender, line) = mpsc::channel();
    thread::spawn(move || {
        let mut said = String::new();
        let _ = BufReader::new(stdout).read_line(&mut said);
        let _ = sender.send(said);
    });
    line.recv_timeout(Duration::from_secs(30)).ok()
}

/// Another server on the port answers connections as well as `fake-archive` would: only its line
/// tells that it is the one listening.
#[test]
fn fake_archive_says_it_listens_only_on_a_port_of_its_own() {
    let other = TcpListener::bind(("127.0.0.1", PORT)).unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_fake-archive"))
        .arg(PORT.to_string())
        .output()
        .unwrap();
    drop(other);
    let mut serving = Command::new(env!("CARGO_BIN_EXE_fake-archive"))
        .arg(PORT.to_string())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let said = first_line(serving.stdout.take().unwrap());
    serving.kill().unwrap();
    serving.wait().unwrap();

    assert_eq!(refused.status.code(), Some(1));
    assert!(
        refused.stdout.is_empty(),
        "a port it could not take is never announced"
    );
    assert_eq!(said.as_deref(), Some("listening on 127.0.0.1:41956\n"));
}
