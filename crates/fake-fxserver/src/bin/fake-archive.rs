//! `fake-archive <port>` serves on 127.0.0.1 the fake FXServer built next to it as build 99001 of
//! the host's platform, for Observer to install: `server.zip` with the fake as `FXServer.exe` on
//! Windows, `fx.tar.xz` with the alpine layout and `run.sh` on Linux. A `fake-build.cfg` beside the
//! fake makes it report 99001.
//!
//! A binary of its own, so that the fake every test copies does not carry the archive writers:
//! their debug info made it two thirds larger.

use std::io::{self, BufRead, BufReader, Cursor, Write};
use std::net::{TcpListener, TcpStream};
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use lzma_rust2::{XzOptions, XzWriter};
use tar::{EntryType, Header};
use zip::CompressionMethod;
use zip::result::ZipResult;
use zip::write::{SimpleFileOptions, ZipWriter};

/// The build the archive holds.
const BUILD: u32 = 99001;
/// What Cfx.re names the archive of the host's builds.
const NAME: &str = if cfg!(windows) { "server.zip" } else { "fx.tar.xz" };
/// A download goes out in this many slices, a pause after each, so that it lasts about three
/// seconds however small the archive is: long enough for the panel to show it under way.
const SLICES: usize = 100;
const PAUSE: Duration = Duration::from_millis(30);
/// What stands for the files Observer checks but never runs.
const STAND_IN: &[u8] = b"#!/bin/sh\n";

/// Answers a `GET` or a `HEAD` of any path that ends in the archive's name with the archive, and
/// anything else with a 404. It listens once the archive is made, then says so on stdout: whoever
/// waits for that line waits for the archive too, and an answer on the port alone could come from
/// another server, such as a `fake-archive` a killed run left behind.
fn main() {
    let port = std::env::args()
        .nth(1)
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap_or_else(|| fail("usage: fake-archive <port>"));
    let archive =
        Arc::new(archive().unwrap_or_else(|error| fail(&format!("cannot make the archive: {error}"))));
    let listener = TcpListener::bind(("127.0.0.1", port))
        .unwrap_or_else(|error| fail(&format!("cannot listen on 127.0.0.1:{port}: {error}")));
    println!("listening on 127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let archive = Arc::clone(&archive);
        thread::spawn(move || {
            let _ = respond(&stream, &archive);
        });
    }
}

fn archive() -> io::Result<Vec<u8>> {
    let fake =
        std::env::current_exe()?.with_file_name(format!("fake-fxserver{}", std::env::consts::EXE_SUFFIX));
    let program = std::fs::read(fake)?;
    let cfg = format!("set fake_build {BUILD}\n");
    if cfg!(windows) {
        zip(&program, cfg.as_bytes()).map_err(io::Error::other)
    } else {
        tar_xz(&program, cfg.as_bytes())
    }
}

/// The fake as `FXServer.exe`, its cfg beside it, and the `citizen` folder.
fn zip(program: &[u8], cfg: &[u8]) -> ZipResult<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file("FXServer.exe", deflated)?;
    writer.write_all(program)?;
    writer.start_file("fake-build.cfg", deflated)?;
    writer.write_all(cfg)?;
    writer.add_directory("citizen/", deflated)?;
    Ok(writer.finish()?.into_inner())
}

/// The fake as the musl loader FXServer starts from, beside stand-ins for `run.sh` and
/// `FXServer`, its cfg, and the `citizen` folder.
fn tar_xz(program: &[u8], cfg: &[u8]) -> io::Result<Vec<u8>> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, data, mode) in [
        ("run.sh", STAND_IN, 0o755),
        ("alpine/opt/cfx-server/FXServer", STAND_IN, 0o755),
        ("alpine/opt/cfx-server/ld-musl-x86_64.so.1", program, 0o755),
        ("alpine/opt/cfx-server/fake-build.cfg", cfg, 0o644),
    ] {
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        header.set_mode(mode);
        header.set_size(data.len() as u64);
        tar.append_data(&mut header, path, data)?;
    }
    let mut folder = Header::new_gnu();
    folder.set_entry_type(EntryType::Directory);
    folder.set_mode(0o755);
    folder.set_size(0);
    tar.append_data(&mut folder, "alpine/opt/cfx-server/citizen/", io::empty())?;
    let mut xz = XzWriter::new(Vec::new(), XzOptions::with_preset(1))?;
    xz.write_all(&tar.into_inner()?)?;
    xz.finish()
}

fn respond(stream: &TcpStream, archive: &[u8]) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut lines = BufReader::new(stream).lines();
    let request = lines.next().transpose()?.unwrap_or_default();
    // The whole head is read: closing on bytes left unread would reset the connection.
    while lines.next().transpose()?.is_some_and(|line| !line.is_empty()) {}
    let mut words = request.split(' ');
    let method = words.next().unwrap_or_default();
    let path = words.next().unwrap_or_default();
    let mut out = stream;
    if !path.ends_with(&format!("/{NAME}")) {
        return write!(
            out,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    }
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        archive.len()
    )?;
    if method == "HEAD" {
        return Ok(());
    }
    for slice in archive.chunks(archive.len().div_ceil(SLICES).max(1)) {
        out.write_all(slice)?;
        thread::sleep(PAUSE);
    }
    Ok(())
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(1)
}
