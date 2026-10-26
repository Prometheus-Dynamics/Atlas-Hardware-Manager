//! Copying a file to a board over SSH, adding this computer's key over the
//! USB console when the board refuses it.

use super::*;

impl SshUpdate {
    /// Streams `path` to the board, reporting the share sent.
    pub(super) async fn upload(
        &self,
        path: &Path,
        size: u64,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), DriverError> {
        let script = format!(
            "mkdir -p {REMOTE_DIR} && cat > {REMOTE_IMAGE}.part && mv {REMOTE_IMAGE}.part {REMOTE_IMAGE}"
        );
        let cleanup = format!("rm -f {REMOTE_IMAGE}.part");
        self.upload_to(path, size, &script, &cleanup, progress, cancel)
            .await
    }

    /// Streams `path` into `script`'s stdin over SSH, reporting the share
    /// sent; on a cancel, runs `cleanup`.
    pub(crate) async fn upload_to(
        &self,
        path: &Path,
        size: u64,
        script: &str,
        cleanup: &str,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), DriverError> {
        match self
            .upload_once(path, size, script, cleanup, progress, cancel)
            .await
        {
            // The board is plugged in here but doesn't have this computer's
            // key: the console (no key needed) adds it, as "Add my SSH key"
            // does, and the copy runs again over the USB network.
            Err(Refused::Key(_)) if self.has_console() => {
                progress.log(
                    "the board doesn't have this computer's SSH key; adding it over the USB console"
                        .to_string(),
                );
                if let Err(why) = self.add_key_over_console().await {
                    return Err(DriverError::Other(format!(
                        "the board doesn't accept this computer's SSH key, and adding it over the USB console didn't work: {why}"
                    )));
                }
                self.upload_once(path, size, script, cleanup, progress, cancel)
                    .await
                    .map_err(|refused| match refused {
                        Refused::Key(_) => DriverError::Other(
                            "the board still doesn't accept this computer's SSH key after it was added over the USB console".into(),
                        ),
                        Refused::Other(error) => error,
                    })
            }
            Err(Refused::Key(error) | Refused::Other(error)) => Err(error),
            Ok(()) => Ok(()),
        }
    }

    /// Puts this computer's public key on the board over its USB console.
    async fn add_key_over_console(&self) -> Result<(), String> {
        let key = self
            .access
            .public_key()
            .ok_or("this computer has no SSH public key (choose one in Settings)")?;
        let script = crate::ssh_actions::add_key_script(&key).map_err(|error| error.to_string())?;
        match self.on_console(&script, None).await {
            Some(Ok(_)) => Ok(()),
            Some(Err(error)) => Err(error.to_string()),
            None => Err("the console didn't answer".into()),
        }
    }

    async fn upload_once(
        &self,
        path: &Path,
        size: u64,
        script: &str,
        cleanup: &str,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), Refused> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
        let mut child = self
            .command()
            .arg(script)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|error| DriverError::Unreachable(format!("could not run ssh: {error}")))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| DriverError::Other("ssh has no stdin".into()))?;
        let mut buffer = vec![0u8; 256 * 1024];
        let mut sent = 0u64;
        loop {
            if cancel.is_cancelled() {
                let _ = child.kill().await;
                let _ = self.run(cleanup).await;
                return Err(DriverError::Cancelled.into());
            }
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
            if read == 0 {
                break;
            }
            if let Err(error) = stdin.write_all(&buffer[..read]).await {
                // ssh exited; its stderr says why.
                drop(stdin);
                let output = child.wait_with_output().await.ok();
                let stderr = output
                    .as_ref()
                    .map(|output| String::from_utf8_lossy(&output.stderr).into_owned())
                    .unwrap_or_default();
                return Err(if stderr.trim().is_empty() {
                    DriverError::Unreachable(format!("the copy stopped: {error}")).into()
                } else {
                    Refused::from_ssh(output.and_then(|output| output.status.code()), &stderr)
                });
            }
            sent += read as u64;
            progress.step_progress(UpdateStep::Transfer, sent as f32 / size.max(1) as f32);
        }
        drop(stdin);
        let output = child
            .wait_with_output()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?;
        if !output.status.success() {
            return Err(Refused::from_ssh(
                output.status.code(),
                &String::from_utf8_lossy(&output.stderr),
            ));
        }
        Ok(())
    }
}

/// Why a copy over SSH failed: the board refused this computer's key (which
/// the USB console can fix), or anything else.
enum Refused {
    Key(DriverError),
    Other(DriverError),
}

impl Refused {
    fn from_ssh(code: Option<i32>, stderr: &str) -> Self {
        let error = ssh_error(code, stderr);
        if key_refused(stderr) {
            Self::Key(error)
        } else {
            Self::Other(error)
        }
    }
}

impl From<DriverError> for Refused {
    fn from(error: DriverError) -> Self {
        Self::Other(error)
    }
}

/// ssh got no further than the board refusing every key offered.
pub(super) fn key_refused(stderr: &str) -> bool {
    stderr.contains("Permission denied")
}
