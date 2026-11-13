//! `atlas-helper`: the only part of Atlas that runs with elevated rights.
//!
//! It does one thing: write one verified image to one removable disk. It
//! trusts nothing from the app. The disk is found again in a fresh listing and
//! checked by the same rules as everywhere else (a system disk, a disk that is
//! not removable, a card that is too large, a card that changed size are all
//! refused). Every path is checked before use. No argument is a shell string:
//! each is read as one plain value.
//!
//! ```text
//! atlas-helper write --device /dev/sdb --image /abs/image.img.xz \
//!     --expected-size 31914983424 [--progress /abs/progress.jsonl] \
//!     [--cancel-file /abs/cancel] [--journal /abs/image.img.xz.journal] \
//!     [--image-sha256 <hex>] [--no-verify]
//! ```
//!
//! Progress and the result are JSON lines. They go to the `--progress` file
//! (elevation tools do not reliably pass stdout back) or to stdout without it.
//! The write stops, and keeps its journal, once `--cancel-file` exists.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_disk::{
    BlockDevice, Disk, DiskError, DiskSource, ImageFormat, ResumePolicy, SysfsDisks, WriteOptions,
    WriteProgress, WriteReport, check_target, detect_format, device_path, write_image,
};
use serde::Serialize;

const USAGE: &str = "usage: atlas-helper write --device <path> --image <path> --expected-size <bytes> [--progress <path>] [--cancel-file <path>] [--journal <path>] [--image-sha256 <hex>] [--no-verify]";

#[derive(Clone, Debug, PartialEq, Eq)]
struct WriteArgs {
    device: PathBuf,
    image: PathBuf,
    expected_size: u64,
    progress: Option<PathBuf>,
    cancel_file: Option<PathBuf>,
    journal: Option<PathBuf>,
    image_sha256: Option<String>,
    verify: bool,
}

/// One JSON line for the app to follow.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
enum Message<'a> {
    Progress { progress: &'a WriteProgress },
    Done { report: &'a WriteReport },
    Error { message: &'a str },
}

/// A path the helper will use: absolute, with no `..` and no NUL.
fn absolute(text: &str, flag: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(text);
    let plain = path.is_absolute()
        && !text.contains('\0')
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir));
    if plain {
        Ok(path)
    } else {
        Err(format!("{flag} must be an absolute path with no `..`"))
    }
}

/// A file the helper will write into: absolute, in a folder that exists, and
/// not a link (a link could send the write somewhere else).
fn output_file(text: &str, flag: &str) -> Result<PathBuf, String> {
    let path = absolute(text, flag)?;
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(format!("{flag}: the folder of {text} does not exist"));
    }
    if fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(format!("{flag}: {text} is a link; give a plain file"));
    }
    Ok(path)
}

/// The image: an absolute path to a regular file named like an image.
fn image_file(text: &str) -> Result<PathBuf, String> {
    let path = absolute(text, "--image")?;
    let canonical = fs::canonicalize(&path).map_err(|error| format!("--image {text}: {error}"))?;
    let name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !(name.ends_with(".img") || name.ends_with(".img.xz") || name.ends_with(".xz")) {
        return Err("--image must be an .img or .img.xz file".into());
    }
    if !canonical.is_file() {
        return Err(format!("--image {text} is not a regular file"));
    }
    Ok(canonical)
}

fn sha256_text(text: &str) -> Result<String, String> {
    let ok = text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if ok {
        Ok(text.to_ascii_lowercase())
    } else {
        Err("--image-sha256 must be 64 hex characters".into())
    }
}

fn parse_write(args: &[String]) -> Result<WriteArgs, String> {
    let mut device = None;
    let mut image = None;
    let mut expected_size = None;
    let mut parsed = WriteArgs {
        device: PathBuf::new(),
        image: PathBuf::new(),
        expected_size: 0,
        progress: None,
        cancel_file: None,
        journal: None,
        image_sha256: None,
        verify: true,
    };
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let mut value = || {
            iter.next()
                .map(String::as_str)
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--device" => device = Some(device_path(value()?).map_err(|error| error.to_string())?),
            "--image" => image = Some(image_file(value()?)?),
            "--expected-size" => {
                expected_size = Some(
                    value()?
                        .parse::<u64>()
                        .map_err(|_| "--expected-size must be a number of bytes".to_string())?,
                );
            }
            "--progress" => parsed.progress = Some(output_file(value()?, "--progress")?),
            "--cancel-file" => parsed.cancel_file = Some(output_file(value()?, "--cancel-file")?),
            "--journal" => parsed.journal = Some(output_file(value()?, "--journal")?),
            "--image-sha256" => parsed.image_sha256 = Some(sha256_text(value()?)?),
            "--no-verify" => parsed.verify = false,
            other => return Err(format!("unknown option {other}")),
        }
    }
    parsed.device = device.ok_or("--device is required")?;
    parsed.image = image.ok_or("--image is required")?;
    parsed.expected_size = expected_size.ok_or("--expected-size is required")?;
    if parsed.expected_size == 0 {
        return Err("--expected-size must be more than zero".into());
    }
    Ok(parsed)
}

/// Finds the disk in a fresh listing and checks it, as if it were new.
fn prepare(args: &WriteArgs, source: &dyn DiskSource) -> Result<Disk, DiskError> {
    let disks = source.disks()?;
    let disk = disks
        .into_iter()
        .find(|disk| disk.path == args.device)
        .ok_or_else(|| DiskError::NotFound(args.device.display().to_string()))?;
    check_target(&disk, Some(args.expected_size))?;
    Ok(disk)
}

/// An `.xz` name must hold xz, so a damaged download is refused rather than
/// written raw as if it were an image.
fn image_matches_name(image: &Path, format: ImageFormat) -> Result<(), DiskError> {
    let named_xz = image
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".xz"));
    if named_xz && format != ImageFormat::Xz {
        return Err(DiskError::Image(format!(
            "{} is named .xz but its contents are not xz",
            image.display()
        )));
    }
    Ok(())
}

/// Appends JSON lines to the progress file, or writes them to stdout.
struct Reporter {
    file: Option<File>,
}

impl Reporter {
    fn open(path: Option<&Path>) -> Result<Self, String> {
        let file = match path {
            Some(path) => Some(
                OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(path)
                    .map_err(|error| format!("{}: {error}", path.display()))?,
            ),
            None => None,
        };
        Ok(Self { file })
    }

    fn send(&mut self, message: &Message<'_>) {
        let Ok(mut line) = serde_json::to_string(message) else {
            return;
        };
        line.push('\n');
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

fn write_disk(
    args: &WriteArgs,
    source: &Arc<dyn DiskSource>,
    reporter: &mut Reporter,
    cancel: &Arc<AtomicBool>,
) -> Result<WriteReport, DiskError> {
    let disk = prepare(args, source.as_ref())?;
    let format = detect_format(&args.image)?;
    image_matches_name(&args.image, format)?;
    let mut target = BlockDevice::open(disk, source.clone())?;
    let options = WriteOptions {
        verify: args.verify,
        image_sha256: args.image_sha256.clone(),
    };
    let mut on_progress = |progress: WriteProgress| {
        reporter.send(&Message::Progress {
            progress: &progress,
        });
    };
    write_image(
        &args.image,
        &mut target,
        &options,
        &ResumePolicy::default(),
        args.journal.as_deref(),
        &mut on_progress,
        cancel,
    )
}

fn run_write(args: WriteArgs) -> Result<(), String> {
    let mut reporter = Reporter::open(args.progress.as_deref())?;
    let source: Arc<dyn DiskSource> = Arc::new(SysfsDisks);
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
    let result = write_disk(&args, &source, &mut reporter, &cancel);
    // Stops the cancel watcher, whatever the outcome.
    cancel.store(true, Ordering::Relaxed);
    match result {
        Ok(report) => {
            reporter.send(&Message::Done { report: &report });
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            reporter.send(&Message::Error { message: &message });
            Err(message)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("write") => parse_write(&args[1..]).and_then(run_write),
        _ => Err(USAGE.to_string()),
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
    use atlas_disk::DiskError;

    /// A made-up listing. Nothing here is a device, so nothing is opened.
    struct Listing(Vec<Disk>);

    impl DiskSource for Listing {
        fn disks(&self) -> Result<Vec<Disk>, DiskError> {
            Ok(self.0.clone())
        }
    }

    fn disk(name: &str) -> Disk {
        Disk {
            path: PathBuf::from(format!("/dev/{name}")),
            name: name.into(),
            size_bytes: 32_000_000_000,
            vendor: None,
            model: None,
            serial: None,
            removable: true,
            usb: true,
            usb_port: Some("1-4".into()),
            system: false,
            mount_points: Vec::new(),
        }
    }

    fn args_for(device: &str, expected: u64) -> WriteArgs {
        WriteArgs {
            device: PathBuf::from(device),
            image: PathBuf::from("/unused.img.xz"),
            expected_size: expected,
            progress: None,
            cancel_file: None,
            journal: None,
            image_sha256: None,
            verify: true,
        }
    }

    fn strings(args: &[String]) -> Vec<String> {
        args.to_vec()
    }

    /// A real image file in the temporary directory, for argument checks.
    fn temp_image(name: &str, bytes: &[u8]) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("atlas-helper-test-{}-{name}", std::process::id()));
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_card_is_found_and_accepted() {
        let source = Listing(vec![disk("sdz")]);
        let found = prepare(&args_for("/dev/sdz", 32_000_000_000), &source).unwrap();
        assert_eq!(found.name, "sdz");
    }

    #[test]
    fn a_system_disk_is_refused() {
        let mut system = disk("nvme0n1");
        system.removable = false;
        system.usb = false;
        system.mount_points = vec!["/".into(), "/home".into()];
        let source = Listing(vec![system]);
        let result = prepare(&args_for("/dev/nvme0n1", 32_000_000_000), &source);
        assert!(
            matches!(result, Err(DiskError::Unsafe(message)) if message.contains("running system"))
        );
    }

    #[test]
    fn a_disk_holding_a_system_path_is_refused_even_when_removable() {
        let mut odd = disk("sdz");
        odd.mount_points = vec!["/var/lib/machines".into()];
        let source = Listing(vec![odd]);
        assert!(prepare(&args_for("/dev/sdz", 32_000_000_000), &source).is_err());
    }

    #[test]
    fn a_non_removable_disk_is_refused() {
        let mut fixed = disk("sdc");
        fixed.removable = false;
        let source = Listing(vec![fixed]);
        let result = prepare(&args_for("/dev/sdc", 32_000_000_000), &source);
        assert!(
            matches!(result, Err(DiskError::Unsafe(message)) if message.contains("not a removable"))
        );
    }

    #[test]
    fn a_disk_that_changed_size_or_vanished_is_refused() {
        let source = Listing(vec![disk("sdz")]);
        assert!(prepare(&args_for("/dev/sdz", 1), &source).is_err());
        assert!(matches!(
            prepare(&args_for("/dev/sdq", 1), &source),
            Err(DiskError::NotFound(_))
        ));
    }

    #[test]
    fn write_arguments_parse_and_default_to_verifying() {
        let image = temp_image("parse.img.xz", b"raw bytes");
        let parsed = parse_write(&strings(&[
            "--device".into(),
            "/dev/sdb".into(),
            "--image".into(),
            image.display().to_string(),
            "--expected-size".into(),
            "1024".into(),
        ]))
        .unwrap();
        assert_eq!(parsed.device, PathBuf::from("/dev/sdb"));
        assert_eq!(parsed.expected_size, 1024);
        assert!(parsed.verify);
        let _ = fs::remove_file(image);
    }

    #[test]
    fn a_shell_string_or_a_partition_is_never_a_device() {
        let image = temp_image("device.img", b"raw");
        let image_text = image.display().to_string();
        for device in [
            "/dev/sdb1",
            "/dev/sdb;reboot",
            "sdb",
            "/dev/../sdb",
            "/dev/sdb $(id)",
        ] {
            let result = parse_write(&strings(&[
                "--device".into(),
                device.into(),
                "--image".into(),
                image_text.clone(),
                "--expected-size".into(),
                "1".into(),
            ]));
            assert!(result.is_err(), "{device} must be refused");
        }
        let _ = fs::remove_file(image);
    }

    #[test]
    fn bad_arguments_are_refused() {
        let image = temp_image("bad.img", b"raw");
        let image_text = image.display().to_string();
        let base = |extra: &[&str]| {
            let mut all: Vec<String> = [
                "--device",
                "/dev/sdb",
                "--image",
                image_text.as_str(),
                "--expected-size",
                "1",
            ]
            .iter()
            .map(|text| text.to_string())
            .collect();
            all.extend(extra.iter().map(|text| text.to_string()));
            all
        };
        assert!(parse_write(&base(&["--bogus"])).is_err());
        assert!(parse_write(&base(&["--image-sha256", "xyz"])).is_err());
        assert!(parse_write(&base(&["--progress", "relative.jsonl"])).is_err());
        assert!(parse_write(&base(&["--progress", "/no/such/folder/p.jsonl"])).is_err());
        assert!(parse_write(&strings(&["--device".into(), "/dev/sdb".into()])).is_err());
        assert!(
            parse_write(&strings(&[
                "--device".into(),
                "/dev/sdb".into(),
                "--image".into(),
                "relative.img".into(),
                "--expected-size".into(),
                "1".into(),
            ]))
            .is_err()
        );
        let _ = fs::remove_file(image);
    }

    #[test]
    fn an_xz_name_must_hold_xz() {
        let image = Path::new("/somewhere/build.img.xz");
        assert!(image_matches_name(image, ImageFormat::Raw).is_err());
        assert!(image_matches_name(image, ImageFormat::Xz).is_ok());
        assert!(image_matches_name(Path::new("/x/plain.img"), ImageFormat::Raw).is_ok());
    }

    /// A file symlink, made the way each platform makes one.
    #[cfg(unix)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[test]
    fn a_link_is_not_an_output_file() {
        let target = temp_image("link-target", b"x");
        let link = std::env::temp_dir().join(format!("atlas-helper-link-{}", std::process::id()));
        let _ = fs::remove_file(&link);
        symlink_file(&target, &link).unwrap();
        assert!(output_file(&link.display().to_string(), "--progress").is_err());
        let _ = fs::remove_file(link);
        let _ = fs::remove_file(target);
    }
}
