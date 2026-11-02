//! Boards in USB boot plugged into another host that runs Atlas's USB agent
//! (`atlas-usb-agent` on a SystemCore): the SystemCore driver lists them as
//! its children (`usb-agent://<agent>/<usb port>`), and this names and
//! flashes them through the agent's API.
//!
//! A flash here is the agent's job: Atlas uploads the boot files (once) and
//! the image, starts the job and follows its events. The job goes on if
//! Atlas's link drops; Atlas keeps asking until it comes back.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    Artifact, CancellationToken, Candidate, Concurrency, DeviceKey, DeviceMode, DriverError,
    Identity, ProgressSink, ReleaseRef, UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};
use atlas_usb_agent::{
    AgentInfo, BOOT_FILES, Event, FlashRequest, JobEvents, JobStarted, SHA256_HEADER, Stored,
    UsbDevice, UsbKind, parse_board_address,
};
use atlas_usbboot::{BROADCOM_VENDOR_ID, Chip};
use tokio::io::AsyncReadExt;

use crate::RpiConfig;
use crate::driver::{BOOT_KEYS_PATH, RPI_FAMILY, recovery_attributes};
use crate::recover::{RESTARTED, forget_host_keys};

mod remembered;

use remembered::{forget, recall, remember};

/// How long the agent may stay out of reach during a flash before Atlas
/// stops following it (the flash goes on there).
const LOST_AFTER: Duration = Duration::from_secs(10 * 60);
const POLL: Duration = Duration::from_millis(500);
const UPLOAD_CHUNK: usize = 1 << 20;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default()
}

fn unreachable(agent: &str, error: impl std::fmt::Display) -> DriverError {
    DriverError::Unreachable(format!("Atlas's USB agent at {agent}: {error}"))
}

fn failed(step: UpdateStep, message: impl Into<String>) -> DriverError {
    DriverError::StepFailed {
        step: step.label().into(),
        message: message.into(),
    }
}

async fn get<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    agent: &str,
    path: &str,
) -> Result<T, DriverError> {
    http.get(format!("http://{agent}{path}"))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| unreachable(agent, error))?
        .json()
        .await
        .map_err(|error| unreachable(agent, error))
}

/// The agent's own words for a refusal (`{"error": ...}`), or the status.
async fn refusal(response: reqwest::Response) -> String {
    let status = response.status();
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| body.get("error")?.as_str().map(str::to_string))
        .unwrap_or_else(|| status.to_string())
}

/// The key serial: the board serial's last 8 hex digits, as the board
/// reports it running and in USB boot here.
fn board_serial(device: &UsbDevice) -> Option<String> {
    let serial = device.serial.as_deref()?.trim();
    let tail = serial.get(serial.len().checked_sub(8)?..)?;
    tail.chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| tail.to_ascii_lowercase())
}

/// Names a board behind an agent from what the agent lists.
pub(crate) async fn identify(
    config: &RpiConfig,
    candidate: &Candidate,
) -> Result<Identity, DriverError> {
    let (agent, usb_port) = parse_board_address(&candidate.address).ok_or_else(|| {
        DriverError::Other(format!("`{}` isn't a board address", candidate.address))
    })?;
    let devices: Vec<UsbDevice> = get(&client(), agent, "/v1/usb").await?;
    let device = devices
        .into_iter()
        .find(|device| device.port == usb_port && device.kind != UsbKind::Other)
        .ok_or_else(|| {
            DriverError::Unreachable(format!("no Raspberry Pi on USB {usb_port} behind {agent}"))
        })?;
    Ok(identity(config, candidate, agent, &device))
}

fn identity(
    config: &RpiConfig,
    candidate: &Candidate,
    agent: &str,
    device: &UsbDevice,
) -> Identity {
    let chip = u16::from_str_radix(&device.product, 16)
        .ok()
        .and_then(Chip::from_product_id);
    // In USB boot the chip names the packages; showing its eMMC, any
    // package booted that way.
    let packages: Vec<_> = config
        .catalog
        .packages()
        .iter()
        .filter(|package| {
            package.manifest.recovery.usb_boot.is_some_and(|id| {
                id.vid.0 == BROADCOM_VENDOR_ID
                    && chip.is_none_or(|chip| id.pid.0 == chip.product_id())
            })
        })
        .collect();
    let host = agent.split(':').next().unwrap_or(agent);
    let serial = board_serial(device);
    let mut attributes = recovery_attributes(chip, &packages);
    if let Some(serial) = &serial {
        attributes.insert(
            atlas_driver::attributes::BOARD_SERIAL.into(),
            serial.clone(),
        );
    }
    attributes.insert("usb_port".into(), device.port.clone());
    attributes.insert("behind".into(), host.to_string());
    let label = match packages.as_slice() {
        [package] => package.manifest.display_name().to_string(),
        _ => chip.map_or("Raspberry Pi", Chip::label).to_string(),
    };
    Identity {
        key: DeviceKey::new(
            RPI_FAMILY,
            serial.unwrap_or_else(|| format!("port-{host}-{}", device.port)),
        ),
        model: label.clone(),
        mode: DeviceMode::Recovery,
        versions: BTreeMap::from([("bootloader".to_string(), "usb boot".to_string())]),
        name: Some(format!("{label} on USB {} behind {host}", device.port)),
        link: candidate.link.clone(),
        address: candidate.address.clone(),
        attributes,
    }
}

/// A flash through the agent.
pub(crate) struct RemoteFlash {
    pub(crate) config: Arc<RpiConfig>,
}

fn artifact(release: &ReleaseRef) -> Result<&Artifact, DriverError> {
    release.artifact.as_ref().ok_or_else(|| {
        DriverError::Incompatible(
            "choose an image file to write; add one on the Releases page".into(),
        )
    })
}

/// The image's name on the agent: its own when that is a plain one.
fn stored_name(artifact: &Artifact) -> String {
    if atlas_usb_agent::valid_name(&artifact.name) {
        artifact.name.clone()
    } else {
        format!(
            "image-{}",
            &artifact.sha256[..12.min(artifact.sha256.len())]
        )
    }
}

/// Streams `path` to `url`, reporting the share sent.
async fn upload(
    http: &reqwest::Client,
    url: &str,
    path: &std::path::Path,
    sha256: Option<&str>,
    on_sent: impl Fn(f32) + Send + Sync + 'static,
) -> Result<reqwest::Response, String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let total = file
        .metadata()
        .await
        .map(|meta| meta.len())
        .unwrap_or(0)
        .max(1);
    let on_sent = Arc::new(on_sent);
    let stream = futures::stream::unfold((file, 0u64), move |(mut file, sent)| {
        let on_sent = on_sent.clone();
        async move {
            let mut chunk = vec![0u8; UPLOAD_CHUNK];
            match file.read(&mut chunk).await {
                Ok(0) => None,
                Ok(read) => {
                    chunk.truncate(read);
                    let sent = sent + read as u64;
                    on_sent(sent as f32 / total as f32);
                    Some((Ok::<_, std::io::Error>(chunk), (file, sent)))
                }
                Err(error) => Some((Err(error), (file, sent))),
            }
        }
    });
    let mut request = http
        .put(url)
        .header(reqwest::header::CONTENT_LENGTH, total)
        .body(reqwest::Body::wrap_stream(stream));
    if let Some(sha256) = sha256 {
        request = request.header(SHA256_HEADER, sha256);
    }
    request.send().await.map_err(|error| error.to_string())
}

#[async_trait]
impl UpdateCapability for RemoteFlash {
    fn resumable(&self, device: &Identity) -> Option<ReleaseRef> {
        recall(&self.config, device).map(|record| record.release)
    }

    fn plan(&self, device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        let artifact = artifact(release)?;
        let (agent, usb_port) = parse_board_address(&device.address)
            .ok_or_else(|| DriverError::Incompatible("not a board behind an agent".into()))?;
        Ok(UpdatePlan {
            steps: vec![
                UpdateStep::Preflight,
                UpdateStep::Transfer,
                UpdateStep::Apply,
                UpdateStep::Confirm,
            ],
            // The agent runs one flash at a time.
            concurrency: Concurrency::Exclusive(format!("usb-agent:{agent}")),
            summary: format!(
                "Send {} to Atlas's USB agent at {agent}, which USB boots the board on its USB {usb_port}, writes the eMMC and verifies it",
                artifact.name
            ),
        })
    }

    async fn run(
        &self,
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let artifact = artifact(release)?.clone();
        let (agent, usb_port) = parse_board_address(&device.address)
            .ok_or_else(|| DriverError::Incompatible("not a board behind an agent".into()))?;
        let http = client();

        progress.step_started(UpdateStep::Preflight);
        // A flash an earlier Atlas started here: follow it, never start another.
        if let Some(earlier) = recall(&self.config, device) {
            progress.log(format!(
                "following the flash an earlier Atlas started here (agent job {}, {}), not starting another",
                earlier.job, earlier.release.version
            ));
            let probe = http
                .get(format!("http://{agent}/v1/jobs/{}?from=0", earlier.job))
                .timeout(Duration::from_secs(20))
                .send()
                .await
                .map_err(|error| unreachable(agent, error))?;
            if probe.status() == reqwest::StatusCode::NOT_FOUND {
                forget(&self.config, device);
                return Err(failed(
                    UpdateStep::Preflight,
                    format!(
                        "the agent no longer has job {} (it restarted since); check the board before flashing it again",
                        earlier.job
                    ),
                ));
            }
            progress.step_started(UpdateStep::Apply);
            return self
                .finish(
                    &http,
                    agent,
                    &earlier.job,
                    device,
                    release,
                    progress,
                    cancel,
                )
                .await;
        }
        if !artifact.path.is_file() {
            return Err(failed(
                UpdateStep::Preflight,
                format!("the image {} is missing", artifact.path.display()),
            ));
        }
        let info: AgentInfo = get(&http, agent, "/v1/agent").await?;
        if let Some(job) = info.running {
            return Err(failed(
                UpdateStep::Preflight,
                format!("the agent is still flashing (job {job}); wait for it to finish"),
            ));
        }
        progress.log(format!("Atlas's USB agent {} at {agent}", info.version));
        if !info.boot_files {
            let files = crate::boot_files::find_boot_files(&self.config.boot_file_dirs)
                .map_err(|_| failed(UpdateStep::Preflight, "USB boot files not found here"))?;
            for name in BOOT_FILES {
                let reply = upload(
                    &http,
                    &format!("http://{agent}/v1/boot/{name}"),
                    &files.dir().join(name),
                    None,
                    |_| {},
                )
                .await
                .map_err(|error| unreachable(agent, error))?;
                if !reply.status().is_success() {
                    return Err(failed(UpdateStep::Preflight, refusal(reply).await));
                }
            }
            progress.log("gave the agent the USB boot files");
        }
        if cancel.is_cancelled() {
            return Err(DriverError::Cancelled);
        }

        progress.step_started(UpdateStep::Transfer);
        let name = stored_name(&artifact);
        let sink = progress.clone();
        let reply = upload(
            &http,
            &format!("http://{agent}/v1/images/{name}"),
            &artifact.path,
            Some(&artifact.sha256),
            move |share| sink.step_progress(UpdateStep::Transfer, share),
        )
        .await
        .map_err(|error| failed(UpdateStep::Transfer, error))?;
        if !reply.status().is_success() {
            return Err(failed(UpdateStep::Transfer, refusal(reply).await));
        }
        let stored: Stored = reply
            .json()
            .await
            .map_err(|error| failed(UpdateStep::Transfer, error.to_string()))?;
        progress.log(format!(
            "the agent holds {} ({} bytes)",
            stored.name, stored.size
        ));
        if cancel.is_cancelled() {
            return Err(DriverError::Cancelled);
        }

        progress.step_started(UpdateStep::Apply);
        let mut boot_writes = BTreeMap::new();
        if let Some(keys) = self.config.ssh_keys.get() {
            boot_writes.insert(
                BOOT_KEYS_PATH.to_string(),
                crate::actions::merge_keys("", &keys),
            );
        }
        let request = FlashRequest {
            port: usb_port.to_string(),
            image: stored.name,
            sha256: Some(artifact.sha256.clone()),
            boot_writes,
        };
        let reply = http
            .post(format!("http://{agent}/v1/flash"))
            .json(&request)
            .send()
            .await
            .map_err(|error| unreachable(agent, error))?;
        if !reply.status().is_success() {
            return Err(failed(UpdateStep::Apply, refusal(reply).await));
        }
        let job: JobStarted = reply
            .json()
            .await
            .map_err(|error| unreachable(agent, error))?;
        progress.log(format!("the agent is flashing (job {})", job.id));
        remember(&self.config, device, &job.id, release);
        self.finish(&http, agent, &job.id, device, release, progress, cancel)
            .await
    }
}

impl RemoteFlash {
    /// Follows agent job `id` to its end, then tidies up after it: the
    /// remembered record (unless Atlas lost the agent, so a later Atlas
    /// still follows it) and the board's old SSH host key.
    #[allow(clippy::too_many_arguments)]
    async fn finish(
        &self,
        http: &reqwest::Client,
        agent: &str,
        id: &str,
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let done = match follow(http, agent, id, progress, cancel).await {
            Ok(done) => done,
            Err(Lost(error)) => return Err(error),
            Err(Ended(error)) => {
                forget(&self.config, device);
                return Err(error);
            }
        };
        forget(&self.config, device);
        if let Event::Done {
            restarted,
            restart_error,
        } = done
        {
            match (restarted, restart_error) {
                (Some(tty), _) => {
                    progress.log(format!("restarted the board into the new image ({tty})"));
                    progress.log(RESTARTED);
                }
                (None, Some(reason)) => progress.log(format!(
                    "could not restart the board itself ({reason}); power-cycle it without holding its boot button"
                )),
                (None, None) => {}
            }
        }
        if let (Some(path), Some(serial)) = (
            &self.config.known_hosts,
            device
                .attributes
                .get(atlas_driver::attributes::BOARD_SERIAL),
        ) && let Ok(n) = forget_host_keys(path, serial)
            && n > 0
        {
            progress.log("forgot the board's old SSH host key (the new image makes its own)");
        }
        Ok(UpdateOutcome::Verified {
            version: release.version.clone(),
        })
    }
}

/// Why following a job stopped short of its `done`.
enum Stopped {
    /// The job ended badly (or was cancelled) on the agent.
    Ended(DriverError),
    /// The agent stayed out of reach: the flash may go on there.
    Lost(DriverError),
}
use Stopped::{Ended, Lost};

/// Whole percent of the image read, for a person.
fn percent(done: u64, total: u64) -> u64 {
    (done * 100).checked_div(total).unwrap_or(0).min(100)
}

/// Follows the job to its end, through link drops: its `done`, or why it
/// failed. Cancelling asks the agent to stop and waits for it to.
async fn follow(
    http: &reqwest::Client,
    agent: &str,
    id: &str,
    progress: &ProgressSink,
    cancel: &CancellationToken,
) -> Result<Event, Stopped> {
    let mut from = 0;
    let mut last_contact = tokio::time::Instant::now();
    let mut cancelling = false;
    let mut verifying = false;
    loop {
        if cancel.is_cancelled() && !cancelling {
            cancelling = true;
            let _ = http
                .delete(format!("http://{agent}/v1/jobs/{id}"))
                .send()
                .await;
            progress.log("asked the agent to stop the flash");
        }
        match get::<JobEvents>(http, agent, &format!("/v1/jobs/{id}?from={from}")).await {
            Ok(events) => {
                last_contact = tokio::time::Instant::now();
                from = events.next;
                for event in events.events {
                    match event {
                        Event::Log { text, .. } => progress.log(text),
                        Event::Booted {
                            serial: Some(serial),
                            ..
                        } => progress.log(format!("board serial {serial}")),
                        Event::Disk { name, size_bytes } => progress.log(format!(
                            "the eMMC showed as {name} ({} GB) on the agent's host",
                            size_bytes / 1_000_000_000
                        )),
                        Event::Write(atlas_blockdev::WriteProgress::Writing {
                            input_done,
                            input_total,
                            ..
                        }) if input_total > 0 => progress.step_progress(
                            UpdateStep::Apply,
                            input_done as f32 / input_total as f32,
                        ),
                        Event::Write(atlas_blockdev::WriteProgress::LinkDropped {
                            input_done,
                            input_total,
                            drops,
                            max_drops,
                            reason,
                            ..
                        }) => progress.log(format!(
                            "link dropped at {}% ({reason}); drop {drops} of {max_drops} allowed",
                            percent(input_done, input_total)
                        )),
                        Event::Write(atlas_blockdev::WriteProgress::Resuming {
                            input_done,
                            input_total,
                            ..
                        }) => progress.log(format!(
                            "link back, resuming at {}%",
                            percent(input_done, input_total)
                        )),
                        Event::Write(atlas_blockdev::WriteProgress::Verifying { done, total })
                            if total > 0 =>
                        {
                            if !verifying {
                                verifying = true;
                                progress.step_started(UpdateStep::Confirm);
                            }
                            progress.step_progress(UpdateStep::Confirm, done as f32 / total as f32);
                        }
                        Event::Written {
                            bytes,
                            seconds,
                            sha256,
                        } => {
                            progress.step_started(UpdateStep::Confirm);
                            progress.step_progress(UpdateStep::Confirm, 1.0);
                            progress.log(format!(
                                "wrote and verified {bytes} bytes in {seconds} s (sha256 {})",
                                &sha256[..12.min(sha256.len())]
                            ));
                        }
                        done @ Event::Done { .. } => return Ok(done),
                        Event::Error { message } => {
                            return Err(Ended(if cancelling {
                                DriverError::Cancelled
                            } else {
                                failed(UpdateStep::Apply, message)
                            }));
                        }
                        _ => {}
                    }
                }
            }
            Err(error) => {
                if last_contact.elapsed() >= LOST_AFTER {
                    return Err(Lost(failed(
                        UpdateStep::Apply,
                        format!(
                            "lost Atlas's USB agent at {agent} for {} minutes ({error}); the flash goes on there, and Atlas follows it again once it sees the board",
                            LOST_AFTER.as_secs() / 60
                        ),
                    )));
                }
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests;
