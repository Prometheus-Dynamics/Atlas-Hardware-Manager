//! A minimal HTTP/1.1 client over `std::net`: one request per connection, plain http, for the
//! USB agent's API on the bench network. Reads a `Content-Length` or chunked body to its end.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const CONNECT: Duration = Duration::from_secs(10);
const IO: Duration = Duration::from_secs(60);

/// A reply: the status code and the body bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Sends one request to `agent` (`host:port`) and reads the whole reply.
///
/// # Errors
///
/// A connection, write or read failure, or a reply that is not HTTP/1.1.
pub fn request(
    agent: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<Reply, String> {
    let address = agent
        .to_socket_addrs()
        .map_err(|error| format!("{agent}: {error}"))?
        .next()
        .ok_or_else(|| format!("{agent}: no address"))?;
    let mut stream = TcpStream::connect_timeout(&address, CONNECT)
        .map_err(|error| format!("cannot reach the agent at {agent}: {error}"))?;
    stream
        .set_read_timeout(Some(IO))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(IO))
        .map_err(|error| error.to_string())?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {agent}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|error| format!("{method} {path}: {error}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|error| format!("{method} {path}: {error}"))?;
    parse_reply(&raw)
}

/// Splits a raw reply into its status and body.
///
/// # Errors
///
/// A reply without a status line, or a chunked body that is not well formed.
pub fn parse_reply(raw: &[u8]) -> Result<Reply, String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("the agent's reply has no header end")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "the reply head is not text")?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("empty reply")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("not an HTTP status line: {status_line:?}"))?;
    let chunked = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
        })
    });
    let rest = &raw[split + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else {
        rest.to_vec()
    };
    Ok(Reply { status, body })
}

fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let line_end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("a chunk has no size line")?;
        let size_text = std::str::from_utf8(&rest[..line_end]).map_err(|_| "a chunk size")?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| format!("a chunk size is not hex: {size_text:?}"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(body);
        }
        let chunk = rest.get(..size).ok_or("a chunk is cut short")?;
        body.extend_from_slice(chunk);
        rest = rest.get(size + 2..).ok_or("a chunk has no end")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_length_and_chunked_bodies_are_read() {
        let plain = parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi").unwrap();
        assert_eq!(
            plain,
            Reply {
                status: 200,
                body: b"hi".to_vec()
            }
        );
        let chunked = parse_reply(
            b"HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(
            chunked,
            Reply {
                status: 201,
                body: b"abcde".to_vec()
            }
        );
        assert!(parse_reply(b"nonsense").is_err());
    }
}
