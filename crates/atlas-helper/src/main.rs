//! `atlas-helper`: the only part of Atlas that runs with elevated rights.
//!
//! It does two things: write a verified image to a removable disk, and
//! install USB access for Pi boot devices (`install-usb-access`). It trusts
//! nothing from the app. It lists disks itself, refuses system, internal,
//! and resized disks, checks the image's SHA-256 when given one, and
//! reports progress as JSON lines to a file the app follows.
//!
//! ```text
//! atlas-helper list
//! atlas-helper write --device /dev/sdb --image image.img.xz --expected-size 31914983424 \
//!     --progress /tmp/x/progress.jsonl --cancel-file /tmp/x/cancel \
//!     [--image-sha256 <hex>] [--no-verify]
//! ```

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_blockdev::{HelperMessage, WriteOptions, check_target, list_disks, write_image};

struct WriteArgs {
    device: PathBuf,
    image: PathBuf,
    expected_size: u64,
    progress: Option<PathBuf>,
    cancel_file: Option<PathBuf>,
    image_sha256: Option<String>,
    verify: bool,
}

fn parse_write(args: &[String]) -> Result<WriteArgs, String> {
    let mut parsed = WriteArgs {
        device: PathBuf::new(),
        image: PathBuf::new(),
        expected_size: 0,
        progress: None,
        cancel_file: None,
        image_sha256: None,
        verify: true,
    };
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let mut value = || {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--device" => parsed.device = value()?.into(),
            "--image" => parsed.image = value()?.into(),
            "--expected-size" => {
                parsed.expected_size = value()?
                    .parse()
                    .map_err(|_| "--expected-size must be a number of bytes".to_string())?;
            }
            "--progress" => parsed.progress = Some(value()?.into()),
            "--cancel-file" => parsed.cancel_file = Some(value()?.into()),
            "--image-sha256" => parsed.image_sha256 = Some(value()?),
            "--no-verify" => parsed.verify = false,
            other => return Err(format!("unknown option {other}")),
        }
    }
    if parsed.device.as_os_str().is_empty() || parsed.image.as_os_str().is_empty() {
        return Err("--device and --image are required".into());
    }
    if parsed.expected_size == 0 {
        return Err("--expected-size is required".into());
    }
    Ok(parsed)
}

mod usb_access;

/// Sends messages to the progress file, or stdout when there is none.
pub(crate) struct Reporter {
    file: Option<std::fs::File>,
}

impl Reporter {
    pub(crate) fn open(path: Option<&std::path::Path>) -> Result<Self, String> {
        Ok(Self {
            file: match path {
                Some(path) => Some(
                    OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(path)
                        .map_err(|error| format!("{}: {error}", path.display()))?,
                ),
                None => None,
            },
        })
    }

    pub(crate) fn send(&mut self, message: &HelperMessage) {
        let line = message.to_line();
        match &mut self.file {
            Some(file) => {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
            }
            None => {
                let _ = std::io::stdout().write_all(line.as_bytes());
            }
        }
    }
}

fn run_write(args: WriteArgs) -> Result<(), String> {
    let mut reporter = Reporter::open(args.progress.as_deref())?;

    let result = (|| {
        let disks = list_disks().map_err(|error| error.to_string())?;
        let disk = disks
            .into_iter()
            .find(|disk| disk.path == args.device)
            .ok_or_else(|| format!("no disk at {}", args.device.display()))?;
        check_target(&disk, Some(args.expected_size)).map_err(|error| error.to_string())?;

        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(cancel_file) = args.cancel_file.clone() {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                while !cancel.load(Ordering::Relaxed) {
                    if cancel_file.exists() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            });
        }

        let options = WriteOptions {
            verify: args.verify,
            image_sha256: args.image_sha256.clone(),
        };
        let mut progress_reporter = |progress| {
            reporter.send(&HelperMessage::Progress { progress });
        };
        let report = write_image(
            &args.image,
            &disk,
            &options,
            &mut progress_reporter,
            &cancel,
        )
        .map_err(|error| error.to_string());
        cancel.store(true, Ordering::Relaxed);
        report
    })();

    match result {
        Ok(report) => {
            reporter.send(&HelperMessage::Done { report });
            Ok(())
        }
        Err(message) => {
            reporter.send(&HelperMessage::Error {
                message: message.clone(),
            });
            Err(message)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("list") => list_disks()
            .map_err(|error| error.to_string())
            .and_then(|disks| serde_json::to_string_pretty(&disks).map_err(|e| e.to_string()))
            .map(|json| {
                let _ = writeln!(std::io::stdout(), "{json}");
            }),
        Some("write") => parse_write(&args[1..]).and_then(run_write),
        Some("install-usb-access") => usb_access::parse(&args[1..]).and_then(usb_access::run),
        _ => Err("usage: atlas-helper list | atlas-helper install-usb-access [--progress <file>] | atlas-helper write --device <path> --image <path> --expected-size <bytes> [--progress <file>] [--cancel-file <file>] [--image-sha256 <hex>] [--no-verify]".into()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "atlas-helper: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn write_arguments_parse() {
        let parsed = parse_write(&strings(&[
            "--device",
            "/dev/sdb",
            "--image",
            "a.img",
            "--expected-size",
            "1024",
            "--no-verify",
        ]))
        .unwrap();
        assert_eq!(parsed.device, PathBuf::from("/dev/sdb"));
        assert_eq!(parsed.expected_size, 1024);
        assert!(!parsed.verify);
    }

    #[test]
    fn missing_or_unknown_arguments_are_errors() {
        assert!(parse_write(&strings(&["--device", "/dev/sdb"])).is_err());
        assert!(parse_write(&strings(&["--bogus"])).is_err());
        assert!(
            parse_write(&strings(&[
                "--device",
                "/dev/sdb",
                "--image",
                "a.img",
                "--expected-size",
                "x"
            ]))
            .is_err()
        );
    }
}
