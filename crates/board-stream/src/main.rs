use std::io::{self, BufReader, Write};
use std::process::ExitCode;
use std::time::Duration;

use board_stream::{Paths, Stream, boot_id, read_request};

/// Milliseconds from an environment variable, else the default.
fn millis(name: &str, default: u64) -> Duration {
    Duration::from_millis(
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default),
    )
}

fn main() -> ExitCode {
    let mut input = BufReader::new(io::stdin().lock());
    let mut out = io::stdout().lock();
    let request = match read_request(&mut input) {
        Ok(request) => request,
        Err(refusal) => {
            let _ = write!(
                out,
                "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}\n",
                refusal.status, refusal.reason
            );
            return ExitCode::SUCCESS;
        }
    };
    // How often the files are looked at, and the longest silence.
    let poll = millis("BOARD_STREAM_POLL_MS", 200);
    let keepalive = millis("BOARD_STREAM_KEEPALIVE_MS", 10_000);
    let mut stream = Stream::new(out, Paths::from_env(), request, keepalive);
    if stream.start(&boot_id()).is_err() {
        return ExitCode::SUCCESS;
    }
    // Until a write fails: the viewer went.
    loop {
        std::thread::sleep(poll);
        if stream.tick().is_err() {
            return ExitCode::SUCCESS;
        }
    }
}
