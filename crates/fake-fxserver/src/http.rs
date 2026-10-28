use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::Server;

/// The bounds FXServer 35245 prints for its tick histograms, as it prints them.
const BUCKETS: [&str; 15] = [
    "0.001",
    "0.002",
    "0.004",
    "0.006",
    "0.008",
    "0.01",
    "0.015",
    "0.02",
    "0.03",
    "0.05",
    "0.07000000000000001",
    "0.1",
    "0.15",
    "0.25",
    "+Inf",
];

/// Serves the endpoints from threads of their own, so they keep answering while the main loop is
/// hung, as FXServer's do.
pub fn serve(listener: TcpListener, server: Arc<Server>) {
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let server = Arc::clone(&server);
            thread::spawn(move || {
                let _ = respond(stream, &server);
            });
        }
    });
}

fn respond(mut stream: TcpStream, server: &Server) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let request = read_head(&mut stream)?;
    let mut lines = request.split("\r\n");
    let target = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("/");
    let authorization = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.trim().to_owned());
    // FXServer routes by case-insensitive prefix: `/perf/` reaches the `/perf` handler.
    let path = target.split('?').next().unwrap_or_default().to_ascii_lowercase();

    let (status, body) = if path.starts_with("/dynamic.json") {
        ("200 OK", dynamic(server))
    } else if path.starts_with("/players.json") {
        ("200 OK", "[]".to_owned())
    } else if path.starts_with("/info.json") {
        ("200 OK", info())
    } else if path.starts_with("/perf") {
        if authorized(server, authorization.as_deref()) {
            ("200 OK", perf(server))
        } else {
            ("401 Unauthorized", "Unauthorized.".to_owned())
        }
    } else {
        ("404 Not Found", "Not found.".to_owned())
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn read_head(stream: &mut TcpStream) -> io::Result<String> {
    let mut head = Vec::new();
    let mut buffer = [0; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") && head.len() < 16 * 1024 {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

fn dynamic(server: &Server) -> String {
    json!({
        "clients": 0,
        "gametype": "",
        "hostname": server.convar("sv_hostname").unwrap_or_else(|| "FXServer, but unconfigured".to_owned()),
        "iv": "0",
        "mapname": "",
        "sv_maxclients": server.convar("sv_maxclients").unwrap_or_else(|| "48".to_owned()),
    })
    .to_string()
}

/// The build is FXServer 35245's, whose formats the fake follows.
fn info() -> String {
    let platform = if cfg!(windows) { "win32" } else { "linux" };
    json!({
        "resources": ["observer"],
        "server": format!("FXServer-master SERVER v1.0.0.35245 {platform}"),
        "version": 0,
    })
    .to_string()
}

/// With either credential set, FXServer wants exactly `Basic base64(user:password)`.
fn authorized(server: &Server, authorization: Option<&str>) -> bool {
    let user = server.convar("sv_prometheusBasicAuthUser").unwrap_or_default();
    let password = server
        .convar("sv_prometheusBasicAuthPassword")
        .unwrap_or_default();
    if user.is_empty() && password.is_empty() {
        return true;
    }
    authorization
        == Some(&format!(
            "Basic {}",
            base64(format!("{user}:{password}").as_bytes())
        ))
}

/// The tick histograms. Fake ticks take no time, so every tick lands in the first bucket.
/// `svMain` counts main loop ticks and stops while it is hung; the other two run on their own.
fn perf(server: &Server) -> String {
    let elapsed = server.uptime_ms();
    let mut body = String::from(
        "# HELP tickTime Time spent on server ticks
# TYPE tickTime histogram
",
    );
    for (thread, count) in [
        ("svNetwork", elapsed / 10),
        ("svSync", elapsed / 8),
        ("svMain", server.main_ticks()),
    ] {
        let _ = writeln!(body, "tickTime_count{{name=\"{thread}\"}} {count}");
        let _ = writeln!(body, "tickTime_sum{{name=\"{thread}\"}} 0");
        for bound in BUCKETS {
            let _ = writeln!(
                body,
                "tickTime_bucket{{name=\"{thread}\",le=\"{bound}\"}} {count}"
            );
        }
    }
    body
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for group in bytes.chunks(3) {
        let padded = [group[0], *group.get(1).unwrap_or(&0), *group.get(2).unwrap_or(&0)];
        let bits = u32::from(padded[0]) << 16 | u32::from(padded[1]) << 8 | u32::from(padded[2]);
        for index in 0..4 {
            if index <= group.len() {
                out.push(char::from(ALPHABET[(bits >> (18 - 6 * index) & 0x3f) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b"observer:secret"), "b2JzZXJ2ZXI6c2VjcmV0");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
    }
}
