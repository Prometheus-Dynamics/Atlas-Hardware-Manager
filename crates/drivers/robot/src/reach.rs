//! How this crate reaches a robot device: the one trait every probe goes
//! through, and its network implementation.
//!
//! The trait has three operations and no others. [`Reach::get`] is an HTTP
//! GET, the only request this crate sends. [`Reach::open`] is a TCP connect
//! that sends nothing. [`Reach::resolve`] is a name lookup. None of them can
//! write, log in or change a setting, so the crate cannot do those things even
//! by mistake. Reading a vendor's own status page and ports is the whole job:
//! this is not a board job, and it is not a way to configure anything.
//!
//! Tests pass a fake, so they need no network.

use std::net::Ipv4Addr;
use std::time::Duration;

use async_trait::async_trait;

/// How long one TCP connect may take. A robot network answers in milliseconds.
pub const CONNECT_LIMIT: Duration = Duration::from_millis(700);
/// How long one GET may take, from connect to the last body byte.
pub const HTTP_LIMIT: Duration = Duration::from_secs(2);
/// How long one name lookup may take.
pub const NAME_LIMIT: Duration = Duration::from_secs(1);
/// The most body this crate keeps from one answer. Anything past it is dropped
/// unread, so a device that streams garbage cannot grow memory.
pub const MAX_BODY: usize = 64 * 1024;

/// What a device answered to a GET.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    /// The `Server` header, when it is valid text.
    pub server: Option<String>,
    /// The body, at most [`MAX_BODY`] bytes, decoded lossily.
    pub body: String,
}

/// The one way this crate talks to a device or to the name service.
#[async_trait]
pub trait Reach: Send + Sync {
    /// `GET path` on `ip:port`. `None` when nothing HTTP answered: refused,
    /// reset, or not HTTP at all. Status codes are the caller's to judge.
    async fn get(&self, ip: Ipv4Addr, port: u16, path: &str) -> Option<Answer>;

    /// Whether something accepts a TCP connection on `ip:port`. Nothing is sent.
    async fn open(&self, ip: Ipv4Addr, port: u16) -> bool;

    /// The IPv4 addresses `name` resolves to here. Empty when none, or when the lookup fails.
    async fn resolve(&self, name: &str) -> Vec<Ipv4Addr>;
}

/// The network implementation: `reqwest` for GETs, tokio for connects and lookups.
///
/// Redirects are not followed (a redirect could send the GET to another host),
/// and proxies from the environment are ignored: robot devices are on this
/// computer's own network.
#[derive(Clone, Debug)]
pub struct HttpReach {
    client: reqwest::Client,
}

impl HttpReach {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(HTTP_LIMIT)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for HttpReach {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Reach for HttpReach {
    async fn get(&self, ip: Ipv4Addr, port: u16, path: &str) -> Option<Answer> {
        let mut response = self
            .client
            .get(format!("http://{ip}:{port}{path}"))
            .send()
            .await
            .ok()?;
        let status = response.status().as_u16();
        let server = response
            .headers()
            .get(reqwest::header::SERVER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            let room = MAX_BODY.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
            if body.len() >= MAX_BODY {
                break;
            }
        }
        Some(Answer {
            status,
            server,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    async fn open(&self, ip: Ipv4Addr, port: u16) -> bool {
        tokio::net::TcpStream::connect((ip, port)).await.is_ok()
    }

    async fn resolve(&self, name: &str) -> Vec<Ipv4Addr> {
        match tokio::net::lookup_host((name, 0)).await {
            Ok(addresses) => addresses
                .filter_map(|address| match address.ip() {
                    std::net::IpAddr::V4(ip) => Some(ip),
                    std::net::IpAddr::V6(_) => None,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// A device on the loopback that answers any GET with `body`, then hangs up.
    async fn serve(body: Vec<u8>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 1024];
                    let _ = socket.read(&mut request).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nServer: garbled/1\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn a_device_that_streams_garbage_is_cut_at_the_body_limit() {
        let port = serve(vec![0xFF; 3 * MAX_BODY]).await;
        let answer = HttpReach::new()
            .get(Ipv4Addr::LOCALHOST, port, "/")
            .await
            .expect("it answered");
        assert_eq!(answer.status, 200);
        assert_eq!(answer.server.as_deref(), Some("garbled/1"));
        // Each 0xFF byte decodes to one replacement character.
        let kept = answer.body.chars().count();
        assert!(kept > 0 && kept <= MAX_BODY, "kept {kept} characters");
    }

    #[tokio::test]
    async fn nothing_listening_is_no_answer() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let reach = HttpReach::new();
        assert_eq!(reach.get(Ipv4Addr::LOCALHOST, port, "/").await, None);
        assert!(!reach.open(Ipv4Addr::LOCALHOST, port).await);
    }

    #[tokio::test]
    async fn an_open_port_is_found_by_connecting_and_nothing_else_is_sent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = tokio::spawn(async move {
            let mut count = 0;
            while tokio::time::timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_ok()
            {
                count += 1;
            }
            count
        });
        assert!(HttpReach::new().open(Ipv4Addr::LOCALHOST, port).await);
        assert_eq!(accepted.await.unwrap(), 1);
    }
}
