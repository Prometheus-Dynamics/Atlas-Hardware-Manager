//! The network, behind a small trait: the catalog asks for a URL from a byte
//! offset and reads the body in pieces. Tests give it a fake; the app uses
//! [`HttpFetcher`].

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;

/// Gets URLs. `offset` above zero asks for the rest of the file from that
/// byte on (an HTTP range request).
#[async_trait]
pub trait Fetch: Send + Sync {
    async fn get(&self, url: &str, offset: u64) -> Result<Reply, String>;
}

/// The answer to a [`Fetch::get`]: the HTTP status and the body.
pub struct Reply {
    pub status: u16,
    pub body: Box<dyn Body>,
}

impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reply")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// A response body, read one piece at a time.
#[async_trait]
pub trait Body: Send {
    /// The next piece, or `None` at the end. An error means the connection
    /// dropped.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String>;
}

/// The real network: HTTP(S) through reqwest.
#[derive(Debug)]
pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("atlas-release/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Fetch for HttpFetcher {
    async fn get(&self, url: &str, offset: u64) -> Result<Reply, String> {
        let mut request = self.client.get(url);
        if offset > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        Ok(Reply {
            status: response.status().as_u16(),
            body: Box::new(HttpBody(response)),
        })
    }
}

struct HttpBody(reqwest::Response);

#[async_trait]
impl Body for HttpBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.0
            .chunk()
            .await
            .map(|piece| piece.map(|bytes| bytes.to_vec()))
            .map_err(|error| error.to_string())
    }
}

/// Reads a whole small response (a release index). Anything but a 2xx answer,
/// or more than `limit` bytes, is an error.
pub(crate) async fn read_all(
    fetch: &dyn Fetch,
    url: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let reply = fetch.get(url, 0).await?;
    if !(200..300).contains(&reply.status) {
        return Err(format!("the server answered {}", reply.status));
    }
    let mut body = reply.body;
    let mut bytes = Vec::new();
    while let Some(piece) = body.next_chunk().await? {
        if bytes.len() + piece.len() > limit {
            return Err(format!("larger than {limit} bytes"));
        }
        bytes.extend_from_slice(&piece);
    }
    Ok(bytes)
}
