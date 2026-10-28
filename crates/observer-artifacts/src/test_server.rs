//! A tiny HTTP/1.1 server on 127.0.0.1 for tests. Each connection gets one answer from the handler
//! and is closed, which is also how a reply is cut short.
//!
//! It listens only on the five ports set aside for these tests. Any free port would do on Windows,
//! but WSL hands out ephemeral ports from 32768, a range that holds ports other servers on the
//! same machine are known to use.
#![allow(clippy::unwrap_used, reason = "test support, where a failure is the test's")]

use std::fmt::Write;
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::task::JoinHandle;

const PORTS: RangeInclusive<u16> = 41950..=41954;
/// One server per port at a time, across all the tests of a run.
static SERVERS: Semaphore = Semaphore::const_new(5);

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    cut_after: Option<usize>,
}

impl Reply {
    pub fn ok(body: Vec<u8>) -> Self {
        Self::status(200).body(body)
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            cut_after: None,
        }
    }

    /// Sets the body and a matching `Content-Length`.
    #[must_use]
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.headers.retain(|(key, _)| key != "Content-Length");
        self.headers
            .push(("Content-Length".to_owned(), body.len().to_string()));
        self.body = body;
        self
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.retain(|(key, _)| key != name);
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    #[must_use]
    pub fn without_length(mut self) -> Self {
        self.headers.retain(|(key, _)| key != "Content-Length");
        self
    }

    /// Closes the connection after this many bytes of the body.
    #[must_use]
    pub fn cut_after(mut self, bytes: usize) -> Self {
        self.cut_after = Some(bytes);
        self
    }
}

pub struct TestServer {
    pub port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    task: JoinHandle<()>,
    _slot: SemaphorePermit<'static>,
}

impl TestServer {
    /// The handler also gets how many requests came before this one.
    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(&Request, usize) -> Reply + Send + Sync + 'static,
    {
        let slot = SERVERS.acquire().await.unwrap();
        let listener = listen().await;
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let Some(request) = read_request(&mut stream).await else {
                    continue;
                };
                let index = {
                    let mut seen = seen.lock().unwrap();
                    seen.push(request.clone());
                    seen.len() - 1
                };
                let reply = handler(&request, index);
                let mut head = format!("HTTP/1.1 {} Test\r\n", reply.status);
                for (name, value) in &reply.headers {
                    let _ = write!(head, "{name}: {value}\r\n");
                }
                head.push_str("Connection: close\r\n\r\n");
                let _ = stream.write_all(head.as_bytes()).await;
                if request.method != "HEAD" {
                    let end = reply.cut_after.unwrap_or(reply.body.len()).min(reply.body.len());
                    let _ = stream.write_all(&reply.body[..end]).await;
                }
                let _ = stream.shutdown().await;
            }
        });
        Self {
            port,
            requests,
            task,
            _slot: slot,
        }
    }

    pub fn url(&self, path: &str) -> Url {
        format!("http://127.0.0.1:{}{path}", self.port).parse().unwrap()
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn paths(&self) -> Vec<String> {
        self.requests().into_iter().map(|request| request.path).collect()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The first free port of the five. A server another test just dropped lets go of its port only
/// once its runtime stops, so a busy port is tried again for a while.
async fn listen() -> TcpListener {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for port in PORTS {
            if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
                return listener;
            }
        }
        assert!(Instant::now() < deadline, "ports {PORTS:?} stay taken");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 || buffer.len() > 64 * 1024 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&buffer);
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let path = request_line.next()?.to_owned();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    Some(Request {
        method,
        path,
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_test_server_listens_on_a_port_set_aside_for_these_tests() {
        let server = TestServer::start(|_, _| Reply::status(204)).await;

        assert!(PORTS.contains(&server.port), "{}", server.port);
    }
}
