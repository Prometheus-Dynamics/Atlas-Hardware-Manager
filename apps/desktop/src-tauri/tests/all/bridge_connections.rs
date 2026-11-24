//! The UI bridge answers every request on a new connection while the browser keeps others open.
//!
//! A browser keeps its connections to the page's server open, and opens new ones as a page loads
//! (fonts, scripts, the long poll). tiny_http 0.12's connection pool could queue a new
//! connection with no thread to read it while the others stayed open, so its request was never
//! read and the page's load never finished (docs/flakes.md, flash-goto-load). This test does what
//! the browser does: it opens connections together and holds them open, then opens bursts of new
//! ones together, each kept open until every request of the burst has its answer, and checks that
//! every request gets one. The fault shows most often as a bridge is first asked (about one start
//! in two for tiny_http), so the test starts the bridge again and again.
//!
//! `ATLAS_BRIDGE_UNDER_TEST` runs the test against another bridge binary (an older build, to see
//! the fault); otherwise it runs against this crate's own.
#![cfg(feature = "sim")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

/// Connections held open for the whole test, as a browser keeps them.
const HELD: usize = 6;
/// New connections opened together in each burst.
const BURST: usize = 8;
/// Times the bridge is started, each a fresh one.
const STARTS: usize = 10;
/// Bursts made on each start.
const BURSTS: usize = 100;
/// How long one request may wait for its answer. The bridge answers a file at once; a request
/// that waits this long was never read.
const ANSWER_WAIT: Duration = Duration::from_secs(5);
const BODY: &str = "a file the bridge serves";

struct Bridge {
    child: Child,
    port: u16,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_bridge(dist: &std::path::Path, data: &std::path::Path) -> Bridge {
    let binary = std::env::var("ATLAS_BRIDGE_UNDER_TEST")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_atlas-ui-bridge").to_owned());
    let mut child = Command::new(&binary)
        .args(["--dist"])
        .arg(dist)
        .args(["--data"])
        .arg(data)
        .args(["--scene", "empty"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("the bridge {binary} starts: {error}"));
    let stdout = child.stdout.take().expect("the bridge's output");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("the bridge says where it listens");
    let port = line
        .trim()
        .rsplit(':')
        .next()
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("a port in {line:?}"));
    Bridge { child, port }
}

/// Sends one GET on `stream` and reads its whole answer, keeping the connection open.
fn get(stream: &mut TcpStream, port: u16) -> Result<String, String> {
    stream
        .set_read_timeout(Some(ANSWER_WAIT))
        .map_err(|e| e.to_string())?;
    write!(
        stream,
        "GET /file.txt HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: keep-alive\r\n\r\n"
    )
    .map_err(|e| format!("send: {e}"))?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|e| format!("no answer within {ANSWER_WAIT:?}: {e}"))?;
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .ok_or_else(|| format!("no length in {head:?}"))?;
    let mut body = vec![0u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|e| format!("the body: {e}"))?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("answered {head:?}"));
    }
    Ok(String::from_utf8_lossy(&body).to_string())
}

#[test]
fn every_request_on_a_new_connection_is_answered_while_others_stay_open() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let dist = dir.path().join("dist");
    std::fs::create_dir_all(&dist).expect("the dist directory");
    std::fs::write(dist.join("file.txt"), BODY).expect("the file");
    for start in 0..STARTS {
        let bridge = start_bridge(&dist, &dir.path().join(format!("data-{start}")));
        let port = bridge.port;

        // Opened together, as a page opens its first connections, then each asked in turn.
        let mut held: Vec<TcpStream> = (0..HELD)
            .map(|_| TcpStream::connect(("127.0.0.1", port)).expect("a held connection"))
            .collect();
        for (n, stream) in held.iter_mut().enumerate() {
            assert_eq!(
                get(stream, port).as_deref(),
                Ok(BODY),
                "start {start}: held connection {n} of {HELD}, opened together, was not answered"
            );
        }

        for burst in 0..BURSTS {
            let started = Arc::new(Barrier::new(BURST));
            let answered = Arc::new(Barrier::new(BURST));
            let workers: Vec<_> = (0..BURST)
                .map(|_| {
                    let started = Arc::clone(&started);
                    let answered = Arc::clone(&answered);
                    std::thread::spawn(move || {
                        started.wait();
                        let begun = Instant::now();
                        let result = TcpStream::connect(("127.0.0.1", port))
                            .map_err(|e| format!("connect: {e}"))
                            .and_then(|mut stream| {
                                let result = get(&mut stream, port);
                                // Kept open until the whole burst has its answers, as a page
                                // keeps its connections: a connection left without a reader
                                // stays so.
                                answered.wait();
                                drop(stream);
                                result
                            });
                        (result, begun.elapsed())
                    })
                })
                .collect();
            for worker in workers {
                let (result, took) = worker.join().expect("a burst worker");
                assert_eq!(
                    result.as_deref(),
                    Ok(BODY),
                    "start {start}, burst {burst}: a request on a new connection was not answered (after {took:?}, with {HELD} connections held open)"
                );
            }
        }
        // The held connections still answer.
        for (n, stream) in held.iter_mut().enumerate() {
            assert_eq!(
                get(stream, port).as_deref(),
                Ok(BODY),
                "start {start}: held connection {n} no longer answers"
            );
        }
    }
}
