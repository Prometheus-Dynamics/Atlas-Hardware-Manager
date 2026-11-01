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

/// An agent flash Atlas started, remembered until it ends, so an Atlas
/// started again (after closing or crashing mid-flash) follows it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Remembered {
    address: String,
    job: String,
    release: ReleaseRef,
    started_ms: u64,
}

fn remembered_path(config: &RpiConfig, device: &Identity) -> Option<std::path::PathBuf> {
    let dir = config.agent_jobs.as_ref()?;
    let name: String = format!("{}-{}", device.key.family, device.key.serial)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(dir.join(format!("{name}.json")))
}

fn remember(config: &RpiConfig, device: &Identity, job: &str, release: &ReleaseRef) {
    let Some(path) = remembered_path(config, device) else {
        return;
    };
    let record = Remembered {
        address: device.address.clone(),
        job: job.to_string(),
        release: release.clone(),
        started_ms: atlas_driver::util::now_ms(),
    };
    if let (Some(dir), Ok(text)) = (path.parent(), serde_json::to_string_pretty(&record)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, text);
    }
}

/// The flash remembered for this device, if it's through the same agent
/// and port.
fn recall(config: &RpiConfig, device: &Identity) -> Option<Remembered> {
    let text = std::fs::read_to_string(remembered_path(config, device)?).ok()?;
    let record: Remembered = serde_json::from_str(&text).ok()?;
    (record.address == device.address).then_some(record)
}

fn forget(config: &RpiConfig, device: &Identity) {
    if let Some(path) = remembered_path(config, device) {
        let _ = std::fs::remove_file(path);
    }
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
mod tests {
    use super::*;

    /// An agent on loopback that knows one job, `job-1`, already finished
    /// with `done`, and answers 404 for any other.
    async fn agent_with_a_finished_job() -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut request = vec![0u8; 4096];
                let read = stream.read(&mut request).await.unwrap_or(0);
                let line = String::from_utf8_lossy(&request[..read]).to_string();
                let (status, body) = if line.starts_with("GET /v1/jobs/job-1") {
                    let events = JobEvents {
                        id: "job-1".into(),
                        events: vec![
                            Event::Log {
                                text: "writing".into(),
                                rss_kib: None,
                            },
                            Event::Done {
                                restarted: Some("ttyACM0".into()),
                                restart_error: None,
                            },
                        ],
                        next: 2,
                        finished: true,
                    };
                    ("200 OK", serde_json::to_string(&events).unwrap())
                } else {
                    ("404 Not Found", "{\"error\":\"no such job\"}".to_string())
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        address
    }

    fn board(agent: &str) -> Identity {
        Identity {
            key: DeviceKey::new(RPI_FAMILY, "e5226d57"),
            model: "Raze".into(),
            mode: DeviceMode::Recovery,
            versions: BTreeMap::new(),
            name: None,
            link: atlas_driver::LinkId("usb".into()),
            address: atlas_usb_agent::board_address(agent, "3-1.2"),
            attributes: BTreeMap::new(),
        }
    }

    fn release() -> ReleaseRef {
        ReleaseRef {
            family: atlas_driver::Family::new(RPI_FAMILY),
            version: "2026.2.0-dev.811".into(),
            artifact: Some(Artifact {
                name: "helios.img.xz".into(),
                path: "/nowhere/helios.img.xz".into(),
                sha256: "ab".repeat(32),
                size_bytes: 1,
            }),
        }
    }

    #[tokio::test]
    async fn a_flash_an_earlier_atlas_started_is_followed_not_started_again() {
        let agent = agent_with_a_finished_job().await;
        let dir = std::env::temp_dir().join(format!("atlas-agent-jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let flash = RemoteFlash {
            config: Arc::new(RpiConfig {
                agent_jobs: Some(dir.clone()),
                ..RpiConfig::default()
            }),
        };
        let device = board(&agent);
        assert_eq!(flash.resumable(&device), None);

        // Atlas started job-1, then went away; the record stayed.
        remember(&flash.config, &device, "job-1", &release());
        assert_eq!(flash.resumable(&device), Some(release()));
        // The same board on another port isn't that flash.
        let mut elsewhere = device.clone();
        elsewhere.address = atlas_usb_agent::board_address(&agent, "3-1.4");
        assert_eq!(flash.resumable(&elsewhere), None);

        // Started again: it follows job-1 to its end (the image, which is
        // gone, isn't needed) and forgets it.
        let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = lines.clone();
        let progress = ProgressSink::new(move |update| {
            if let atlas_driver::ProgressUpdate::Log { message } = update {
                seen.lock().unwrap().push(message);
            }
        });
        let outcome = flash
            .run(&device, &release(), &progress, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            outcome,
            UpdateOutcome::Verified {
                version: "2026.2.0-dev.811".into()
            }
        );
        let lines = lines.lock().unwrap().clone();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("an earlier Atlas started here")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("restarted the board")),
            "{lines:?}"
        );
        assert_eq!(flash.resumable(&device), None, "forgotten once it ended");

        // A job the agent no longer has: said so, forgotten, nothing started.
        remember(&flash.config, &device, "job-9", &release());
        let error = flash
            .run(
                &device,
                &release(),
                &ProgressSink::new(|_| {}),
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no longer has job job-9"),
            "{error}"
        );
        assert_eq!(flash.resumable(&device), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn usb(port: &str, product: &str, serial: Option<&str>) -> UsbDevice {
        UsbDevice {
            port: port.into(),
            vendor: "0a5c".into(),
            product: product.into(),
            name: None,
            serial: serial.map(str::to_string),
            kind: UsbDevice::kind_of("0a5c", product),
        }
    }

    #[test]
    fn a_board_behind_the_agent_keeps_its_serial_as_its_key() {
        let config = RpiConfig {
            boot_file_dirs: Vec::new(),
            catalog: Arc::new(atlas_devices::DeviceCatalog::default()),
            ssh_keys: Default::default(),
            known_hosts: None,
            agent_jobs: None,
        };
        let candidate = Candidate {
            link: atlas_driver::LinkId("usb".into()),
            family: atlas_driver::Family::new(RPI_FAMILY),
            address: atlas_usb_agent::board_address("172.27.0.1:5809", "3-1.2"),
        };
        let booting = identity(
            &config,
            &candidate,
            "172.27.0.1:5809",
            &usb("3-1.2", "2712", Some("e5226d57")),
        );
        let gadget = identity(
            &config,
            &candidate,
            "172.27.0.1:5809",
            &usb("3-1.2", "0104", Some("a317bcbee5226d57")),
        );
        assert_eq!(booting.key.to_string(), "rpi:e5226d57");
        assert_eq!(gadget.key, booting.key);
        assert_eq!(booting.mode, DeviceMode::Recovery);
        assert_eq!(booting.attributes["behind"], "172.27.0.1");
        assert_eq!(booting.attributes["usb_port"], "3-1.2");
        assert_eq!(booting.attributes["chip"], "BCM2712 (Pi 5 / CM5)");
        assert_eq!(
            booting.name.as_deref(),
            Some("BCM2712 (Pi 5 / CM5) on USB 3-1.2 behind 172.27.0.1")
        );
        let unnamed = identity(
            &config,
            &candidate,
            "172.27.0.1:5809",
            &usb("3-1.2", "2712", None),
        );
        assert_eq!(unnamed.key.to_string(), "rpi:port-172.27.0.1-3-1.2");
    }

    #[test]
    fn plain_image_names_are_kept() {
        let mut artifact = Artifact {
            name: "helios-2026.2.0-dev.801.img.xz".into(),
            path: "/x".into(),
            sha256: "0123456789abcdef".into(),
            size_bytes: 1,
        };
        assert_eq!(stored_name(&artifact), "helios-2026.2.0-dev.801.img.xz");
        artifact.name = "my image (1).img".into();
        assert_eq!(stored_name(&artifact), "image-0123456789ab");
    }
}
