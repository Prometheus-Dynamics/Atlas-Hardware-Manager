//! One request: `GET`/`HEAD /images/<token>/<name>`, with ranges and a
//! strong ETag, streamed from disk.

use std::convert::Infallible;
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use tokio::io::{AsyncReadExt, AsyncSeekExt, Take};
use tokio_util::io::ReaderStream;

use crate::Shared;
use crate::registry::{DownloadSlot, is_token, short};

/// Read size while streaming: large enough for gigabit, small in memory.
const CHUNK: usize = 256 * 1024;

/// Everything but RFC 3986 unreserved characters is escaped in a name.
const NAME: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) fn encode_name(name: &str) -> String {
    utf8_percent_encode(name, NAME).to_string()
}

/// A response body: nothing, a short text, or a file region.
pub enum ImageBody {
    Empty,
    Text(Option<Bytes>),
    File(Box<FileBody>),
}

pub struct FileBody {
    stream: ReaderStream<Take<tokio::fs::File>>,
    remaining: u64,
    sent: u64,
    shared: Arc<Shared>,
    label: String,
    _slot: DownloadSlot,
}

impl Body for ImageBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match self.get_mut() {
            Self::Empty => Poll::Ready(None),
            Self::Text(text) => Poll::Ready(text.take().map(|text| Ok(Frame::data(text)))),
            Self::File(file) => match Pin::new(&mut file.stream).poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    let len = chunk.len() as u64;
                    file.sent += len;
                    file.remaining = file.remaining.saturating_sub(len);
                    Poll::Ready(Some(Ok(Frame::data(chunk))))
                }
                Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            },
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Text(text) => text.is_none(),
            Self::File(file) => file.remaining == 0,
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Empty => SizeHint::with_exact(0),
            Self::Text(text) => {
                SizeHint::with_exact(text.as_ref().map_or(0, |text| text.len() as u64))
            }
            Self::File(file) => SizeHint::with_exact(file.remaining),
        }
    }
}

impl Drop for FileBody {
    fn drop(&mut self) {
        let outcome = if self.remaining == 0 {
            "sent"
        } else {
            "stopped after"
        };
        self.shared
            .log(&format!("{} {outcome} {} bytes", self.label, self.sent));
    }
}

/// The byte range a request asks for, against a file of `len` bytes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Wanted {
    Whole,
    /// Inclusive first and last byte.
    Part(u64, u64),
    Unsatisfiable,
}

/// Parses a `Range` header. Only one `bytes` range is honored; several
/// ranges or a malformed header get the whole file, as RFC 9110 allows.
pub(crate) fn parse_range(header: &str, len: u64) -> Wanted {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return Wanted::Whole;
    };
    if spec.contains(',') {
        return Wanted::Whole;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return Wanted::Whole;
    };
    let number = |text: &str| -> Option<Option<u64>> {
        let text = text.trim();
        if text.is_empty() {
            Some(None)
        } else if text.bytes().all(|byte| byte.is_ascii_digit()) {
            text.parse().ok().map(Some)
        } else {
            None
        }
    };
    let (Some(first), Some(last)) = (number(first), number(last)) else {
        return Wanted::Whole;
    };
    match (first, last) {
        (Some(first), last) => {
            if last.is_some_and(|last| last < first) {
                return Wanted::Whole;
            }
            if first >= len {
                return Wanted::Unsatisfiable;
            }
            let last = last.map_or(len - 1, |last| last.min(len - 1));
            Wanted::Part(first, last)
        }
        (None, Some(suffix)) => {
            if suffix == 0 || len == 0 {
                Wanted::Unsatisfiable
            } else {
                Wanted::Part(len.saturating_sub(suffix), len - 1)
            }
        }
        (None, None) => Wanted::Whole,
    }
}

/// `/images/<token>/<encoded name>` split into its parts.
fn parse_path(path: &str) -> Option<(&str, String)> {
    let rest = path.strip_prefix("/images/")?;
    let (token, name) = rest.split_once('/')?;
    if !is_token(token) || name.is_empty() || name.contains('/') {
        return None;
    }
    let name = percent_decode_str(name).decode_utf8().ok()?.into_owned();
    Some((token, name))
}

/// The request path with the token cut short, for the log.
fn loggable(path: &str) -> String {
    let path: String = path.chars().take(160).collect();
    match path
        .strip_prefix("/images/")
        .and_then(|rest| rest.split_once('/'))
    {
        Some((token, name)) if is_token(token) => format!("/images/{}/{name}", short(token)),
        _ => path.chars().filter(|c| !c.is_control()).collect(),
    }
}

fn text(status: StatusCode, message: &'static str) -> Response<ImageBody> {
    let mut response = Response::new(ImageBody::Text(Some(Bytes::from_static(
        message.as_bytes(),
    ))));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

fn header_text(request: &Request<Incoming>, name: header::HeaderName) -> Option<&str> {
    request.headers().get(name)?.to_str().ok()
}

fn etag_matches(list: &str, etag: &str) -> bool {
    list.split(',')
        .map(|tag| tag.trim().trim_start_matches("W/"))
        .any(|tag| tag == "*" || tag == etag)
}

/// Serves one request and logs it.
pub(crate) async fn handle(
    shared: Arc<Shared>,
    peer: SocketAddr,
    request: Request<Incoming>,
) -> Result<Response<ImageBody>, Infallible> {
    let label = format!(
        "{} {} {}",
        request.method(),
        peer.ip(),
        loggable(request.uri().path())
    );
    let range = header_text(&request, header::RANGE)
        .map(|range| format!(" {}", range.chars().take(40).collect::<String>()))
        .unwrap_or_default();
    let response = respond(&shared, &request, &label).await;
    shared.log(&format!("{label}{range} -> {}", response.status().as_u16()));
    Ok(response)
}

async fn respond(
    shared: &Arc<Shared>,
    request: &Request<Incoming>,
    label: &str,
) -> Response<ImageBody> {
    let head = match *request.method() {
        Method::GET => false,
        Method::HEAD => true,
        _ => {
            let mut response = text(StatusCode::METHOD_NOT_ALLOWED, "GET or HEAD only\n");
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return response;
        }
    };
    let not_found = || text(StatusCode::NOT_FOUND, "not found\n");
    let Some((token, name)) = parse_path(request.uri().path()) else {
        return not_found();
    };
    let Some(found) = shared.lookup(token) else {
        return not_found();
    };
    if name != found.name {
        return not_found();
    }

    let gone = || text(StatusCode::GONE, "the image changed since it was offered\n");
    let Ok(mut file) = tokio::fs::File::open(&found.path).await else {
        return gone();
    };
    match file.metadata().await {
        Ok(meta) if meta.is_file() && meta.len() == found.size => {}
        _ => return gone(),
    }
    let len = found.size;
    let etag = format!("\"{}\"", found.sha256);

    let mut response = Response::new(ImageBody::Empty);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-transform"),
    );
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }

    if header_text(request, header::IF_NONE_MATCH).is_some_and(|list| etag_matches(list, &etag)) {
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        return response;
    }
    let honor_range = header_text(request, header::IF_RANGE).is_none_or(|tag| tag.trim() == etag);
    let wanted = match header_text(request, header::RANGE) {
        Some(range) if honor_range => parse_range(range, len),
        _ => Wanted::Whole,
    };
    let (start, count) = match wanted {
        Wanted::Whole => (0, len),
        Wanted::Part(first, last) => {
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            if let Ok(value) = HeaderValue::from_str(&format!("bytes {first}-{last}/{len}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            (first, last - first + 1)
        }
        Wanted::Unsatisfiable => {
            let mut response = text(StatusCode::RANGE_NOT_SATISFIABLE, "range not satisfiable\n");
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{len}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return response;
        }
    };
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(count));
    if head {
        return response;
    }

    let Some(slot) = DownloadSlot::acquire(&found.active, shared.max_downloads()) else {
        let mut busy = text(
            StatusCode::SERVICE_UNAVAILABLE,
            "too many downloads of this image at once\n",
        );
        busy.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("30"));
        return busy;
    };
    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return gone();
    }
    *response.body_mut() = ImageBody::File(Box::new(FileBody {
        stream: ReaderStream::with_capacity(file.take(count), CHUNK),
        remaining: count,
        sent: 0,
        shared: shared.clone(),
        label: label.to_string(),
        _slot: slot,
    }));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Wanted::Part(0, 9));
        assert_eq!(parse_range("bytes=90-", 100), Wanted::Part(90, 99));
        assert_eq!(parse_range("bytes=90-500", 100), Wanted::Part(90, 99));
        assert_eq!(parse_range("bytes=-10", 100), Wanted::Part(90, 99));
        assert_eq!(parse_range("bytes=-500", 100), Wanted::Part(0, 99));
        assert_eq!(parse_range("bytes=100-", 100), Wanted::Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 100), Wanted::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-", 0), Wanted::Unsatisfiable);
        assert_eq!(parse_range("bytes=9-0", 100), Wanted::Whole);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), Wanted::Whole);
        assert_eq!(parse_range("items=0-1", 100), Wanted::Whole);
        assert_eq!(parse_range("bytes=a-b", 100), Wanted::Whole);
        assert_eq!(parse_range("bytes=+1-2", 100), Wanted::Whole);
    }

    #[test]
    fn paths_and_names() {
        let token = "ab".repeat(32);
        let name = "photon vision+raze.img.xz";
        let path = format!("/images/{token}/{}", encode_name(name));
        assert!(!path.contains(' '));
        let (parsed, decoded) = parse_path(&path).unwrap();
        assert_eq!(parsed, token);
        assert_eq!(decoded, name);
        assert!(parse_path(&format!("/images/{token}/a/b")).is_none());
        assert!(parse_path(&format!("/images/{token}/")).is_none());
        assert!(parse_path("/images/../etc/passwd").is_none());
        assert!(parse_path(&format!("/files/{token}/x")).is_none());
        assert_eq!(
            loggable(&format!("/images/{token}/x.img")),
            "/images/abababab…/x.img"
        );
    }

    #[test]
    fn etags() {
        assert!(etag_matches("\"a\", W/\"b\"", "\"b\""));
        assert!(etag_matches("*", "\"b\""));
        assert!(!etag_matches("\"a\"", "\"b\""));
    }
}
