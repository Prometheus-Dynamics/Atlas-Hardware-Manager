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

/// `--serve-each <port> <program>`: the resident listener (`serve`).
#[cfg(target_os = "linux")]
fn serve_each(mut args: impl Iterator<Item = String>) -> ExitCode {
    let (Some(port), Some(program)) = (args.next().and_then(|p| p.parse().ok()), args.next())
    else {
        let _ = writeln!(
            io::stderr(),
            "usage: board-stream --serve-each <port> <program>"
        );
        return ExitCode::from(2);
    };
    let served = board_stream::serve::listen(port).and_then(|listener| {
        board_stream::serve::run(listener, program.into(), board_stream::serve::RUNTIME_MAX)
    });
    if let Err(error) = served {
        let _ = writeln!(io::stderr(), "board-stream: serving {port}: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        let mut args = std::env::args().skip(1);
        if args.next().as_deref() == Some("--serve-each") {
            return serve_each(args);
        }
    }
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
    // Events and update state only: sleep until a file changes or the
    // keepalive is due. Until a write fails: the viewer went.
    loop {
        changes.wait(keepalive, poll);
        if stream.tick().is_err() {
            return ExitCode::SUCCESS;
        }
    }
}
