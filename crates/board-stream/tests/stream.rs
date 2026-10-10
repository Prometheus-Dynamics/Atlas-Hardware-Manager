//! The binary as board-stream.socket runs it: a request on stdin, Server-Sent
//! Events on stdout, against a board's files in a temporary directory.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Board {
    dir: PathBuf,
}

impl Board {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("board-stream-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("run")).unwrap();
        fs::create_dir_all(dir.join("data")).unwrap();
        Self { dir }
    }

    fn event(&self, seq: u64, kind: &str) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("data/events.jsonl"))
            .unwrap();
        writeln!(
            file,
            r#"{{"t":100,"boot_id":"b1","seq":{seq},"uptime_s":5,"kind":"{kind}","source":"local","message":"{kind} {seq}","data":{{}}}}"#
        )
        .unwrap();
    }

    fn update(&self, state: &str) {
        let tmp = self.dir.join("run/update.json.tmp");
        fs::write(&tmp, format!("{{\"state\":\"{state}\"}}\n")).unwrap();
        fs::rename(tmp, self.dir.join("run/update.json")).unwrap();
    }

    /// Starts the binary with `request` and returns its output, line by line.
    fn connect(&self, request: &str) -> (Child, mpsc::Receiver<String>) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_board-stream"))
            .env("BOARD_RUN_DIR", self.dir.join("run"))
            .env("BOARD_DATA_DIR", self.dir.join("data"))
            .env("BOARD_BOOT_ID", "b1")
            .env("BOARD_STREAM_POLL_MS", "20")
            .env("BOARD_STREAM_KEEPALIVE_MS", "300")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.as_bytes())
            .unwrap();
        let (tx, rx) = mpsc::channel();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        (child, rx)
    }
}

impl Drop for Board {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Lines until one satisfies `want` (within 5 s), returning them all.
fn until(rx: &mpsc::Receiver<String>, want: impl Fn(&str) -> bool) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(5)) {
        let done = want(&line);
        seen.push(line);
        if done {
            return seen;
        }
    }
    panic!("not seen; got {seen:#?}");
}

fn has(lines: &[String], text: &str) -> bool {
    lines.iter().any(|line| line.contains(text))
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn new_events_and_update_changes_arrive_as_they_are_written() {
    let board = Board::new("live");
    board.event(1, "boot");
    board.update("idle");
    let (child, rx) = board.connect("GET /stream HTTP/1.1\r\nHost: b\r\n\r\n");
    let head = until(&rx, |line| line.starts_with("data: {\"state\":\"idle\"}"));
    assert_eq!(head[0], "HTTP/1.1 200 OK");
    assert!(has(&head, "Content-Type: text/event-stream"));
    assert!(
        has(&head, r#"data: {"boot_id":"b1","seq":1,"#),
        "hello: {head:#?}"
    );
    assert!(
        !has(&head, "event: event"),
        "nothing old without a seq to resume from"
    );

    board.event(2, "update.stage");
    let lines = until(&rx, |line| line.contains("update.stage 2"));
    assert!(has(&lines, "id: 2"));
    board.update("staging");
    until(&rx, |line| line == r#"data: {"state":"staging"}"#);
    // Quiet: a keepalive.
    until(&rx, |line| line == ": keepalive");
    stop(child);
}

#[test]
fn a_viewer_that_reconnects_gets_what_it_missed() {
    let board = Board::new("resume");
    for seq in 1..=4 {
        board.event(seq, "n");
    }
    let (child, rx) =
        board.connect("GET /stream?topics=events HTTP/1.1\r\nLast-Event-ID: 2\r\n\r\n");
    let lines = until(&rx, |line| line.contains("n 4"));
    assert!(has(&lines, "id: 3") && has(&lines, "id: 4"));
    assert!(!has(&lines, "id: 2") && !has(&lines, "id: 1"));
    stop(child);
}

#[test]
fn a_log_that_started_over_is_streamed_from_its_start() {
    let board = Board::new("reset");
    for seq in 1..=3 {
        board.event(seq, "old");
    }
    let (child, rx) = board.connect("GET /stream?topics=events HTTP/1.1\r\n\r\n");
    until(&rx, |line| line.starts_with("data: {\"boot_id\""));
    fs::remove_file(board.dir.join("data/events.jsonl")).unwrap();
    board.event(1, "fresh");
    until(&rx, |line| line.contains("fresh 1"));
    stop(child);
}

#[test]
fn a_torn_line_of_nuls_is_skipped() {
    let board = Board::new("torn");
    board.event(1, "boot");
    let (child, rx) = board.connect("GET /stream?topics=events&after_seq=0 HTTP/1.1\r\n\r\n");
    until(&rx, |line| line.contains("boot 1"));
    // A power cut mid-write, then the next event.
    let mut file = OpenOptions::new()
        .append(true)
        .open(board.dir.join("data/events.jsonl"))
        .unwrap();
    file.write_all(&[0u8; 32]).unwrap();
    file.write_all(b"\n").unwrap();
    board.event(2, "boot");
    until(&rx, |line| line.contains("boot 2"));
    stop(child);
}

#[test]
fn a_bad_request_gets_an_error_and_ends() {
    let board = Board::new("bad");
    let (mut child, rx) = board.connect("GET /nope HTTP/1.1\r\n\r\n");
    let lines = until(&rx, |line| line.starts_with("HTTP/1.1"));
    assert_eq!(lines[0], "HTTP/1.1 404 Not Found");
    assert!(child.wait().unwrap().success());
    let _: &Path = &board.dir;
}
