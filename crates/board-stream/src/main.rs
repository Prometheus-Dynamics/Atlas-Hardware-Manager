use std::io::{self, BufReader, Write};
use std::process::ExitCode;
use std::time::Duration;

use board_stream::{Changes, Paths, Stream, boot_id, read_request};

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
    let paths = Paths::from_env();
    let mut stream = Stream::new(out, paths.clone(), request, keepalive);
    if stream.start(&boot_id()).is_err() {
        return ExitCode::SUCCESS;
    }
    let changes = Changes::new(&paths);
    if stream.wants_hardware() {
        // Live readings go out in batches this often (about 60 per second);
        // the files are looked at when they change (or every `poll`).
        let window = millis("BOARD_STREAM_BATCH_MS", 16);
        let mut looked = std::time::Instant::now();
        loop {
            if stream.pump(window).is_err() {
                return ExitCode::SUCCESS;
            }
            let due = if changes.watching() {
                changes.pending()
            } else {
                looked.elapsed() >= poll
            };
            if due || looked.elapsed() >= keepalive {
                looked = std::time::Instant::now();
                if stream.tick().is_err() {
                    return ExitCode::SUCCESS;
                }
            }
        }
    }
    // Events and update state only: sleep until a file changes or the
    // keepalive is due. Until a write fails: the viewer went.
    loop {
        changes.wait(keepalive, poll);
        if stream.tick().is_err() {
            return ExitCode::SUCCESS;
        }
    }
}
