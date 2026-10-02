//! Running `atlas-helper` with elevated rights from the unprivileged app.
//!
//! The helper writes JSON lines to a progress file and stops early when a
//! cancel file appears next to it. Both files live in a fresh temp
//! directory, so nothing else can write to them.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{BlockError, HelperMessage, WriteProgress, WriteReport};

const POLL: Duration = Duration::from_millis(200);

/// What the helper should write, and where.
#[derive(Clone, Debug)]
pub struct HelperRequest {
    pub device: PathBuf,
    pub image: PathBuf,
    pub image_sha256: Option<String>,
    /// The disk size the app saw; the helper refuses the disk if it changed.
    pub expected_size: u64,
    pub verify: bool,
}

pub struct HelperClient {
    helper: PathBuf,
}

impl HelperClient {
    /// Uses `ATLAS_HELPER` if set, else `atlas-helper` next to the running
    /// executable (where the installers put it).
    pub fn locate() -> Result<Self, BlockError> {
        let name = if cfg!(windows) {
            "atlas-helper.exe"
        } else {
            "atlas-helper"
        };
        let candidate = std::env::var_os("ATLAS_HELPER")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
            })
            .filter(|path| path.is_file())
            .ok_or_else(|| BlockError::Elevation {
                message: "The disk writer (atlas-helper) is missing from this installation.".into(),
                fix: "Reinstall Atlas, or set ATLAS_HELPER to the helper's path.".into(),
            })?;
        Ok(Self { helper: candidate })
    }

    pub fn path(&self) -> &Path {
        &self.helper
    }

    /// Runs one write and blocks until it finishes. Call from a blocking
    /// thread. Setting `cancel` asks the helper to stop between chunks.
    pub fn write(
        &self,
        request: &HelperRequest,
        on_progress: &mut dyn FnMut(WriteProgress),
        cancel: &AtomicBool,
    ) -> Result<WriteReport, BlockError> {
        let (work_dir, progress_file, cancel_file) = work_files()?;

        let mut args: Vec<OsString> = vec![
            "write".into(),
            "--device".into(),
            request.device.clone().into(),
            "--image".into(),
            request.image.clone().into(),
            "--expected-size".into(),
            request.expected_size.to_string().into(),
            "--progress".into(),
            progress_file.clone().into(),
            "--cancel-file".into(),
            cancel_file.clone().into(),
        ];
        if let Some(sha) = &request.image_sha256 {
            args.push("--image-sha256".into());
            args.push(sha.into());
        }
        if !request.verify {
            args.push("--no-verify".into());
        }

        let mut child = spawn_elevated(&self.helper, &args)?;
        let result = follow(
            &mut child,
            &progress_file,
            &cancel_file,
            on_progress,
            cancel,
        );
        let _ = std::fs::remove_dir_all(&work_dir);
        match result? {
            HelperMessage::Done { report } => Ok(report),
            _ => Err(BlockError::Helper(
                "the helper did not report a write".into(),
            )),
        }
    }

    /// Lets this computer open Pi boot devices without root: installs the
    /// udev rule on Linux, binds WinUSB on Windows. One elevation prompt.
    pub fn install_usb_access(&self) -> Result<String, BlockError> {
        let (work_dir, progress_file, cancel_file) = work_files()?;
        let args: Vec<OsString> = vec![
            "install-usb-access".into(),
            "--progress".into(),
            progress_file.clone().into(),
        ];
        let mut child = spawn_elevated(&self.helper, &args)?;
        let result = follow(
            &mut child,
            &progress_file,
            &cancel_file,
            &mut |_| {},
            &AtomicBool::new(false),
        );
        let _ = std::fs::remove_dir_all(&work_dir);
        match result? {
            HelperMessage::Fixed { message } => Ok(message),
            _ => Err(BlockError::Helper(
                "the helper did not report a result".into(),
            )),
        }
    }
}

/// A fresh private folder holding the progress and cancel files.
fn work_files() -> Result<(PathBuf, PathBuf, PathBuf), BlockError> {
    let work_dir = std::env::temp_dir().join(format!(
        "atlas-helper-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&work_dir).map_err(|error| BlockError::io(&work_dir, error))?;
    let progress_file = work_dir.join("progress.jsonl");
    let cancel_file = work_dir.join("cancel");
    std::fs::write(&progress_file, b"").map_err(|error| BlockError::io(&progress_file, error))?;
    Ok((work_dir, progress_file, cancel_file))
}

fn follow(
    child: &mut Child,
    progress_file: &Path,
    cancel_file: &Path,
    on_progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<HelperMessage, BlockError> {
    let mut offset = 0u64;
    let mut outcome: Option<Result<HelperMessage, BlockError>> = None;
    loop {
        if cancel.load(Ordering::Relaxed) && !cancel_file.exists() {
            let _ = std::fs::write(cancel_file, b"cancel");
        }
        let exited = child
            .try_wait()
            .map_err(|error| BlockError::Helper(error.to_string()))?;

        if let Ok(mut file) = std::fs::File::open(progress_file)
            && file.seek(SeekFrom::Start(offset)).is_ok()
        {
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                // Only whole lines; a partial one is read again next time.
                if !line.ends_with('\n') {
                    break;
                }
                offset += line.len() as u64;
                match serde_json::from_str::<HelperMessage>(line.trim()) {
                    Ok(HelperMessage::Progress { progress }) => on_progress(progress),
                    Ok(done @ (HelperMessage::Done { .. } | HelperMessage::Fixed { .. })) => {
                        outcome = Some(Ok(done));
                    }
                    Ok(HelperMessage::Error { message }) => {
                        outcome = Some(Err(if message.contains("cancelled") {
                            BlockError::Cancelled
                        } else {
                            BlockError::Helper(message)
                        }));
                    }
                    Err(_) => {}
                }
                line.clear();
            }
        }

        if let Some(status) = exited {
            return outcome.unwrap_or_else(|| {
                Err(if status.success() {
                    BlockError::Helper("the helper exited without a result".into())
                } else {
                    BlockError::Elevation {
                        message:
                            "The Atlas helper did not run; administrator rights were not granted."
                                .into(),
                        fix: elevation_fix().into(),
                    }
                })
            });
        }
        std::thread::sleep(POLL);
    }
}

fn elevation_fix() -> &'static str {
    if cfg!(target_os = "linux") {
        "Approve the password prompt. If none appeared, install a polkit agent (pkexec) or run Atlas from a desktop session."
    } else if cfg!(target_os = "macos") {
        "Approve the administrator password prompt to allow the disk write."
    } else {
        "Approve the User Account Control prompt to allow the disk write."
    }
}

/// Quotes one argument for a POSIX shell.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// Quotes one argument for PowerShell's single-quoted strings.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn powershell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "''"))
}

/// An AppImage runs from a FUSE mount that root cannot read, so pkexec
/// could not start the helper from there. Copy it to a fresh private folder
/// first and elevate the copy.
#[cfg(target_os = "linux")]
fn runnable_from_root(helper: &Path) -> Result<PathBuf, BlockError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let in_appimage =
        std::env::var_os("APPDIR").is_some_and(|appdir| helper.starts_with(PathBuf::from(appdir)));
    if !in_appimage {
        return Ok(helper.to_path_buf());
    }
    let dir = std::env::temp_dir().join(format!(
        "atlas-helper-bin-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|error| BlockError::io(&dir, error))?;
    let copy = dir.join("atlas-helper");
    std::fs::copy(helper, &copy).map_err(|error| BlockError::io(&copy, error))?;
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| BlockError::io(&copy, error))?;
    Ok(copy)
}

#[cfg(target_os = "linux")]
fn spawn_elevated(helper: &Path, args: &[OsString]) -> Result<Child, BlockError> {
    let helper = runnable_from_root(helper)?;
    let mut command = if nix::unistd::geteuid().is_root() {
        Command::new(&helper)
    } else {
        let mut pkexec = Command::new("pkexec");
        pkexec.arg(&helper);
        pkexec
    };
    command
        .args(args)
        .spawn()
        .map_err(|error| BlockError::Elevation {
            message: format!("Could not start the disk writer with pkexec: {error}"),
            fix: "Install polkit (pkexec) and a polkit authentication agent for your desktop."
                .into(),
        })
}

#[cfg(target_os = "macos")]
fn spawn_elevated(helper: &Path, args: &[OsString]) -> Result<Child, BlockError> {
    let mut script = shell_quote(&helper.to_string_lossy());
    for arg in args {
        script.push(' ');
        script.push_str(&shell_quote(&arg.to_string_lossy()));
    }
    let apple_script = format!(
        "do shell script \"{}\" with administrator privileges",
        script.replace('\\', "\\\\").replace('"', "\\\"")
    );
    Command::new("osascript")
        .args(["-e", &apple_script])
        .spawn()
        .map_err(|error| BlockError::Elevation {
            message: format!("Could not ask for administrator rights: {error}"),
            fix: elevation_fix().into(),
        })
}

#[cfg(target_os = "windows")]
fn spawn_elevated(helper: &Path, args: &[OsString]) -> Result<Child, BlockError> {
    let argument_list = args
        .iter()
        .map(|arg| format!("'\"{}\"'", arg.to_string_lossy().replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$p = Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode",
        powershell_quote(&helper.to_string_lossy()),
        argument_list
    );
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .spawn()
        .map_err(|error| BlockError::Elevation {
            message: format!("Could not ask for administrator rights: {error}"),
            fix: elevation_fix().into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_quotes_and_spaces() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(powershell_quote("it's"), "'it''s'");
    }
}
