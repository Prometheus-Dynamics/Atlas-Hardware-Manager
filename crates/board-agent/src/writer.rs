//! The device package's writer, `/usr/lib/board/update`, run as a subprocess.
//! Its state is on p1 (`board-update.env`); what the agent reads is the JSON
//! it keeps current in `/run/board/update.json`, also while it downloads and
//! copies.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// `update status` / `update.json`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WriterStatus {
    pub state: String,
    pub slot_active: String,
    pub slot_staged: String,
    pub version_active: String,
    pub version_staged: String,
    /// Per mille.
    pub progress: u64,
    pub error: String,
}

#[derive(Clone, Debug)]
pub struct Writer {
    program: PathBuf,
    run_dir: PathBuf,
}

impl Writer {
    pub fn new(program: impl Into<PathBuf>, run_dir: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            run_dir: run_dir.into(),
        }
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// What the writer last wrote, without running it (`None` before its
    /// first run in this boot, or when the file is unreadable).
    pub fn read_status(&self) -> Option<WriterStatus> {
        let text = std::fs::read_to_string(self.run_dir.join("update.json")).ok()?;
        serde_json::from_str(text.trim()).ok()
    }

    /// The PID of the stage that holds the writer's lock, if one does.
    pub fn stage_pid(&self) -> Option<u32> {
        std::fs::read_to_string(self.run_dir.join("update.pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .env("BOARD_RUN_DIR", &self.run_dir)
            .stdin(Stdio::null());
        command
    }

    /// Runs `update <args>` to the end: its stdout, or the reason it gave
    /// (its last line on stderr) when it failed.
    pub async fn run(&self, args: &[&str]) -> Result<String, String> {
        let output = self
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|error| format!("can't run {}: {error}", self.program.display()))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(
                reason(&String::from_utf8_lossy(&output.stderr)).unwrap_or_else(|| {
                    format!("`update {}` failed ({})", args.join(" "), output.status)
                }),
            )
        }
    }

    /// `update status`, which also turns a stage left over from an earlier
    /// boot or a killed process into an error.
    pub async fn status(&self) -> Result<WriterStatus, String> {
        let text = self.run(&["status"]).await?;
        serde_json::from_str(text.trim()).map_err(|error| format!("`update status`: {error}"))
    }

    /// Starts `update stage-url`. Its stderr goes to [`StderrTail`].
    pub fn spawn_stage(
        &self,
        url: &str,
        sha256: &str,
        size: u64,
    ) -> Result<(Child, StderrTail), String> {
        let mut child = self
            .command(&[
                "stage-url",
                url,
                "--sha256",
                sha256,
                "--size",
                &size.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("can't run {}: {error}", self.program.display()))?;
        let tail = StderrTail::default();
        if let Some(stderr) = child.stderr.take() {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    crate::log(&line);
                    *tail.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(line);
                }
            });
        }
        Ok((child, tail))
    }
}

/// The last line a running writer printed on stderr.
#[derive(Clone, Default)]
pub struct StderrTail(std::sync::Arc<std::sync::Mutex<Option<String>>>);

impl StderrTail {
    pub fn reason(&self) -> Option<String> {
        let line = self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()?;
        reason(&line)
    }
}

/// The writer's message without its `update: ` prefix.
fn reason(stderr: &str) -> Option<String> {
    let line = stderr.lines().rev().find(|line| !line.trim().is_empty())?;
    Some(
        line.strip_prefix("update: ")
            .unwrap_or(line)
            .trim()
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_is_the_last_line_without_the_prefix() {
        assert_eq!(
            reason("update: running the pre-stage hook\nupdate: nothing is staged (state: idle)\n"),
            Some("nothing is staged (state: idle)".into())
        );
        assert_eq!(reason("\n"), None);
    }

    #[test]
    fn status_json_parses_with_missing_fields() {
        let status: WriterStatus =
            serde_json::from_str(r#"{"state":"staging","progress":525,"extra":1}"#).unwrap();
        assert_eq!(status.state, "staging");
        assert_eq!(status.progress, 525);
        assert_eq!(status.slot_active, "");
    }
}
