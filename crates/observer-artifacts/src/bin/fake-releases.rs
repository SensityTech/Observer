//! `fake-releases <port> <executable> [version]` answers on 127.0.0.1 as GitHub's API does for
//! Observer's latest release: `version` (this workspace's by default), whose archive for the host
//! holds `executable`, with checksums signed by the test key. An Observer built with the
//! `test-catalog` feature checks it when `OBSERVER_TEST_RELEASES` names the port.
#![allow(clippy::unwrap_used, reason = "a test tool, where a failure is the run's")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use observer_artifacts::releases::fake::FakeRelease;

/// A download goes out in this many slices, a pause after each, so that it lasts about three
/// seconds: long enough for the panel to show it under way.
const SLICES: usize = 100;
const PAUSE: Duration = Duration::from_millis(30);

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(port), Some(executable)) = (args.next().and_then(|port| port.parse::<u16>().ok()), args.next())
    else {
        fail("usage: fake-releases <port> <executable> [version]");
    };
    let version = args
        .next()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    let executable = std::fs::read(&executable)
        .unwrap_or_else(|error| fail(&format!("cannot read {executable}: {error}")));
    let release = Arc::new(FakeRelease::new(&version, &executable));
    let listener = TcpListener::bind(("127.0.0.1", port))
        .unwrap_or_else(|error| fail(&format!("cannot listen on 127.0.0.1:{port}: {error}")));
    println!("listening on 127.0.0.1:{port}, release {version}");
    for stream in listener.incoming().flatten() {
        let release = Arc::clone(&release);
        thread::spawn(move || respond(stream, &release, port));
    }
}

fn respond(mut stream: TcpStream, release: &FakeRelease, port: u16) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
        line.clear();
    }
    let path = request.split_whitespace().nth(1).unwrap_or_default();
    let base = format!("http://127.0.0.1:{port}");
    let Some(body) = release.answer(path, &base) else {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"{}\"\r\nConnection: close\r\n\r\n",
        body.len(),
        release.version
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let slice = body.len().div_ceil(SLICES).max(1);
    for chunk in body.chunks(slice) {
        if stream.write_all(chunk).is_err() {
            return;
        }
        if body.len() > 64 * 1024 {
            thread::sleep(PAUSE);
        }
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}
