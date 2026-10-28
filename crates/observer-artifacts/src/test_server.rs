//! A tiny HTTP/1.1 server on 127.0.0.1 for tests. Each connection gets one answer from the handler
//! and is closed.

use std::fmt::Write;
use std::sync::{Arc, Mutex};

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub method: String,
    pub path: String,
}

pub(crate) struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
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
        }
    }

    /// Sets the body and a matching `Content-Length`.
    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.headers.retain(|(key, _)| key != "Content-Length");
        self.headers
            .push(("Content-Length".to_owned(), body.len().to_string()));
        self.body = body;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.retain(|(key, _)| key != name);
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn without_length(mut self) -> Self {
        self.headers.retain(|(key, _)| key != "Content-Length");
        self
    }
}

pub(crate) struct TestServer {
    pub port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    task: JoinHandle<()>,
}

impl TestServer {
    /// The handler also gets how many requests came before this one.
    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(&Request, usize) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
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
                    let _ = stream.write_all(&reply.body).await;
                }
                let _ = stream.shutdown().await;
            }
        });
        Self { port, requests, task }
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
    let mut request_line = text.split("\r\n").next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let path = request_line.next()?.to_owned();
    Some(Request { method, path })
}
