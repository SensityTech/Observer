use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::Server;

/// The finite bounds FXServer 35245 prints for its tick histograms, as it prints them, with their
/// value in milliseconds.
pub const BUCKETS: [(&str, u64); 14] = [
    ("0.001", 1),
    ("0.002", 2),
    ("0.004", 4),
    ("0.006", 6),
    ("0.008", 8),
    ("0.01", 10),
    ("0.015", 15),
    ("0.02", 20),
    ("0.03", 30),
    ("0.05", 50),
    ("0.07000000000000001", 70),
    ("0.1", 100),
    ("0.15", 150),
    ("0.25", 250),
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
    let headers: Vec<(&str, &str)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim(), value.trim()))
        .collect();
    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|&(_, value)| value)
    };
    let authorization = header("authorization");
    // FXServer routes by case-insensitive prefix: `/perf/` reaches the `/perf` handler.
    let path = target.split('?').next().unwrap_or_default().to_ascii_lowercase();

    let (status, body) = if path.starts_with("/dynamic.json") {
        ("200 OK", dynamic(server))
    } else if path.starts_with("/players.json") {
        ("200 OK", players(server, header("x-players-token")))
    } else if path.starts_with("/info.json") {
        ("200 OK", info(server))
    } else if path.starts_with("/perf") {
        if authorized(server, authorization) {
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

pub fn read_head(stream: &mut TcpStream) -> io::Result<String> {
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
        "clients": server.number("fake_players").unwrap_or(0),
        "gametype": "",
        "hostname": server.convar("sv_hostname").unwrap_or_else(|| "FXServer, but unconfigured".to_owned()),
        "iv": "0",
        "mapname": "",
        "sv_maxclients": server.convar("sv_maxclients").unwrap_or_else(|| "48".to_owned()),
    })
    .to_string()
}

/// Ids 1 to `fake_players`, given the token Observer set in `sv_playersToken`; otherwise the
/// public list, whose ids and pings are 0.
fn players(server: &Server, token: Option<&str>) -> String {
    let count = server.number("fake_players").unwrap_or(0);
    let honoured = token.is_some_and(|token| server.convar("sv_playersToken").as_deref() == Some(token));
    let players: Vec<serde_json::Value> = (1..=count)
        .map(|id| {
            let (id, ping) = if honoured { (id, 20 + 5 * id) } else { (0, 0) };
            json!({ "endpoint": "127.0.0.1", "id": id, "identifiers": ["license:fake"], "name": format!("player {id}"), "ping": ping })
        })
        .collect();
    serde_json::Value::from(players).to_string()
}

/// FXServer 35245's formats, with the build `fake_build` names, 35245 by default.
fn info(server: &Server) -> String {
    let platform = if cfg!(windows) { "win32" } else { "linux" };
    let build = server
        .convar("fake_build")
        .filter(|build| build.bytes().all(|digit| digit.is_ascii_digit()))
        .unwrap_or_else(|| "35245".to_owned());
    json!({
        "resources": ["observer"],
        "server": format!("FXServer-master SERVER v1.0.0.{build} {platform}"),
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

/// The tick histograms. `svMain` counts main loop ticks, each taking `fake_tick_ms`, and stops
/// while it is hung; the other two run on their own and take no time.
fn perf(server: &Server) -> String {
    let elapsed = server.uptime_ms();
    let mut body = String::from(
        "# HELP tickTime Time spent on server ticks
# TYPE tickTime histogram
",
    );
    let main = server.main_observed();
    let quick = |count: u64| (count, 0, [count; BUCKETS.len()]);
    for (thread, (count, sum_ms, within)) in [
        ("svNetwork", quick(elapsed / 10)),
        ("svSync", quick(elapsed / 8)),
        ("svMain", (main.count, main.sum_ms, main.within)),
    ] {
        let _ = writeln!(body, "tickTime_count{{name=\"{thread}\"}} {count}");
        let _ = writeln!(
            body,
            "tickTime_sum{{name=\"{thread}\"}} {}.{:03}",
            sum_ms / 1000,
            sum_ms % 1000
        );
        for ((bound, _), within) in BUCKETS.into_iter().zip(within) {
            let _ = writeln!(
                body,
                "tickTime_bucket{{name=\"{thread}\",le=\"{bound}\"}} {within}"
            );
        }
        let _ = writeln!(body, "tickTime_bucket{{name=\"{thread}\",le=\"+Inf\"}} {count}");
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
