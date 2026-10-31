//! atlas-usb-agent: Atlas's hands on another Linux host that boards in USB
//! boot are plugged into, such as a SystemCore with a Raze on its hub.
//!
//! ```text
//! atlas-usb-agent serve [--listen 0.0.0.0:5809] [--state /var/lib/atlas-usb-agent]
//! atlas-usb-agent flash --port 3-1.1 --boot-files <dir or .tar> --image <file>
//!                       [--sha256 <hex>] [--boot-file <path on p1>=<file>]... [--detached]
//! atlas-usb-agent version
//! ```
//!
//! `serve` is how it normally runs: a service (its package's systemd unit,
//! with a memory cap) serving the API in the library's docs, flashes as
//! jobs that go on when Atlas's link drops. `flash` runs one flash in the
//! foreground, its events as JSON lines on stdout; closing stdin cancels
//! it, except with `--detached` (run under `systemd-run`).

mod flash;
mod serve;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_usb_agent::{Event, PORT, valid_port};

use crate::flash::{Flash, Sink, log, rss_kib};

/// How often memory is checked, and where the agent stops itself (its
/// service's cap is the hard limit).
const MEMORY_EVERY: Duration = Duration::from_secs(5);
const MEMORY_LIMIT_KIB: u64 = 1 << 20;
const STATE: &str = "/var/lib/atlas-usb-agent";
const USAGE: &str = "usage: atlas-usb-agent serve [--listen <addr:port>] [--state <dir>] \
| flash --port <3-1.1> --boot-files <dir|tar> --image <file> [--sha256 <hex>] \
[--boot-file <path>=<file>]... [--detached] | version";

fn stdout_sink() -> Sink {
    Arc::new(|event: Event| {
        if let Ok(line) = serde_json::to_string(&event) {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    })
}

/// Checks memory every few seconds; past the limit, says so and exits.
/// `report` also logs each reading.
fn watch_memory(sink: Sink, report: bool) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(MEMORY_EVERY);
            let rss = rss_kib().unwrap_or(0);
            if rss > MEMORY_LIMIT_KIB {
                sink(Event::Error {
                    message: format!(
                        "stopped: using {} MiB, over the agent's {} MiB limit",
                        rss / 1024,
                        MEMORY_LIMIT_KIB / 1024
                    ),
                });
                std::process::exit(3);
            }
            if report {
                log(&sink, format!("memory {} MiB", rss / 1024));
            }
        }
    });
}

struct FlashArgs {
    flash: Flash,
    detached: bool,
}

fn parse_flash(args: &[String]) -> Result<FlashArgs, String> {
    let mut port = None;
    let mut boot_files = None;
    let mut image = None;
    let mut sha256 = None;
    let mut boot_writes = BTreeMap::new();
    let mut detached = false;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--port" => port = Some(value()?),
            "--boot-files" => boot_files = Some(PathBuf::from(value()?)),
            "--image" => image = Some(PathBuf::from(value()?)),
            "--sha256" => sha256 = Some(value()?),
            "--detached" => detached = true,
            "--boot-file" => {
                let pair = value()?;
                let (path, file) = pair
                    .split_once('=')
                    .ok_or_else(|| format!("--boot-file {pair}: write <path>=<file>"))?;
                let contents =
                    std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}"))?;
                boot_writes.insert(path.to_string(), contents);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let port = port.ok_or("--port is required")?;
    if !valid_port(&port) {
        return Err(format!("{port} isn't a USB port path like 3-1.1"));
    }
    Ok(FlashArgs {
        flash: Flash {
            port,
            boot_files: boot_files.ok_or("--boot-files is required")?,
            image: image.ok_or("--image is required")?,
            sha256,
            boot_writes,
        },
        detached,
    })
}

fn parse_serve(args: &[String]) -> Result<serve::Options, String> {
    let mut options = serve::Options {
        listen: SocketAddr::from(([0, 0, 0, 0], PORT)),
        state: PathBuf::from(STATE),
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--listen" => {
                options.listen = value
                    .parse()
                    .map_err(|_| format!("--listen {value}: write <address>:<port>"))?;
            }
            "--state" => options.state = PathBuf::from(value),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sink = stdout_sink();
    let result = match args.first().map(String::as_str) {
        Some("version") => {
            log(&sink, env!("ATLAS_BUILD_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("serve") => parse_serve(&args[1..]).and_then(|options| {
            watch_memory(sink.clone(), false);
            runtime()?.block_on(serve::serve(options))?;
            Ok(None)
        }),
        Some("flash") => parse_flash(&args[1..]).and_then(|FlashArgs { flash, detached }| {
            watch_memory(sink.clone(), true);
            let cancel = Arc::new(AtomicBool::new(false));
            // Closing stdin (Atlas's SSH going away) cancels, unless it
            // runs detached. What comes in is read and dropped.
            if !detached {
                let watcher = cancel.clone();
                std::thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    let mut stdin = std::io::stdin().lock();
                    while matches!(stdin.read(&mut buffer), Ok(n) if n > 0) {}
                    watcher.store(true, Ordering::Relaxed);
                });
            }
            runtime()?
                .block_on(flash::flash(flash, sink.clone(), cancel))
                .map(Some)
        }),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(done) => {
            if let Some(done) = done {
                sink(done);
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            sink(Event::Error { message });
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse() {
        let file = std::env::temp_dir().join(format!("agent-key-{}", std::process::id()));
        std::fs::write(&file, "ssh-ed25519 AAAA x\n").unwrap();
        let key_arg = format!("board/authorized_keys={}", file.display());
        let args: Vec<String> = [
            "--port",
            "3-1.1",
            "--boot-files",
            "/tmp/b",
            "--image",
            "/tmp/i.img.xz",
            "--sha256",
            "ab",
            "--detached",
            "--boot-file",
            &key_arg,
        ]
        .map(str::to_string)
        .to_vec();
        let parsed = parse_flash(&args).unwrap();
        assert_eq!(parsed.flash.port, "3-1.1");
        assert!(parsed.detached);
        assert_eq!(
            parsed.flash.boot_writes["board/authorized_keys"],
            "ssh-ed25519 AAAA x\n"
        );
        let _ = std::fs::remove_file(file);
        assert!(parse_flash(&["--port".to_string(), "x".to_string()]).is_err());
        assert!(parse_flash(&["--bogus".to_string()]).is_err());

        let serve = parse_serve(&[]).unwrap();
        assert_eq!(serve.listen.port(), PORT);
        assert_eq!(serve.state, PathBuf::from(STATE));
        let serve = parse_serve(&["--listen".into(), "127.0.0.1:9".into()]).unwrap();
        assert_eq!(serve.listen.to_string(), "127.0.0.1:9");
    }
}
