//! The identity endpoint: HTTP/1.1, one path, GET only, one connection at a
//! time with a short deadline, so a stuck client can't hold it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use crate::PATH;

const DEADLINE: Duration = Duration::from_secs(2);
/// A request bigger than this isn't one of ours.
const MAX_REQUEST: usize = 4096;

/// The status and body for a request's head.
pub fn answer(head: &str, document: impl FnOnce() -> String) -> (&'static str, String) {
    let mut words = head.lines().next().unwrap_or("").split_ascii_whitespace();
    match (words.next(), words.next()) {
        (Some("GET"), Some(target)) if target.split('?').next() == Some(PATH) => {
            ("200 OK", document())
        }
        (Some("GET"), Some(_)) => ("404 Not Found", "{\"error\":\"not found\"}".into()),
        _ => ("405 Method Not Allowed", "{\"error\":\"GET only\"}".into()),
    }
}

fn one(stream: &mut TcpStream, document: &dyn Fn() -> String) -> std::io::Result<()> {
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;
    let mut request = Vec::new();
    let mut buffer = [0u8; 512];
    while !request.windows(4).any(|end| end == b"\r\n\r\n") && request.len() < MAX_REQUEST {
        match stream.read(&mut buffer)? {
            0 => break,
            read => request.extend_from_slice(&buffer[..read]),
        }
    }
    let head = String::from_utf8_lossy(&request);
    let (status, body) = answer(&head, document);
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Answers forever.
pub fn serve(listener: &TcpListener, document: &dyn Fn() -> String) {
    for stream in listener.incoming().flatten() {
        let mut stream = stream;
        let _ = one(&mut stream, document);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_path_and_get_only() {
        let doc = || "{}".to_string();
        assert_eq!(
            answer("GET /.well-known/pd-device HTTP/1.1\r\n", doc).0,
            "200 OK"
        );
        assert_eq!(
            answer("GET /.well-known/pd-device?x=1 HTTP/1.1\r\n", doc).0,
            "200 OK"
        );
        assert_eq!(answer("GET / HTTP/1.1\r\n", doc).0, "404 Not Found");
        assert_eq!(
            answer("POST /.well-known/pd-device HTTP/1.1\r\n", doc).0,
            "405 Method Not Allowed"
        );
        assert_eq!(answer("", doc).0, "405 Method Not Allowed");
    }

    #[test]
    fn it_answers_over_a_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || serve(&listener, &|| "{\"model\":\"x\"}".to_string()));
        let mut client = TcpStream::connect(address).unwrap();
        client
            .write_all(b"GET /.well-known/pd-device HTTP/1.1\r\nhost: x\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
        assert!(reply.ends_with("{\"model\":\"x\"}"), "{reply}");
        assert!(reply.contains("content-length: 13"), "{reply}");
    }
}
