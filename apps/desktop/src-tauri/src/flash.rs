//! The flash flow (F5): a first-flash image onto a board in its boot ROM, or onto a removable disk.
//!
//! What runs here is what the libraries do today, and nothing is shown as done that did not run:
//!
//! - **The image.** The person chooses a first-flash image ([`crate::board::Picker`]); it is read
//!   in one pass by `atlas-image` ([`atlas_image::flash_manifest`]), which names the board it was
//!   built for, its raw size and digest, and whether it carries a signing record. An image that
//!   names no board is refused with the library's own words.
//! - **The loader.** A board in its boot ROM is loaded by its family's [`BootRom`], as a job of
//!   `atlas-jobs` (kind [`KIND`]): it never gives up on a failure that can change on its own, the
//!   wait between tries is capped, and it can be stopped and told to try now. The one family
//!   wired is [`RockchipRom`] (`atlas-rockchip`: DDR init, the usbplug, the readiness check and
//!   the board's own report).
//! - **After the loader** the flow stops, and says so. Starting the flasher image, pushing the
//!   image and Lemnos's first-flash write are D6's flow library (`crates/core/flash-flow`), which
//!   does not exist yet; its board side (`system.flash/1`, the `indicator/1` states the steps are
//!   to be named after) is provisional in Lemnos. Nothing is written to a board by this module.
//! - **A removable disk** is written by a [`DiskWriter`]: `atlas-disk`'s verified, resumable write
//!   through its `Platform` seam ([`PlatformWriter`]), or, on a real computer, the privileged
//!   helper ([`HelperWriter`], `atlas-helper`), which checks the disk again itself.
//!
//! The page reads one topic, [`TOPIC`]. Every change is published whole.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_disk::{
    Acknowledgement, Disk, Platform, ResumePolicy, WriteOptions, WriteProgress, WriteReport,
    WriteRequest,
};
use atlas_driver::DriverError;
use atlas_image::SignatureState;
use atlas_jobs::{DeviceClaim, Event, JobId, JobKind, JobRecord, JobStatus};
use atlas_rockchip::loader::{EntryKind, Loader};
use atlas_types::{DeviceKey, DeviceState, Identity};
use atlas_usb::{OpenOptions, PortPath, UsbBus, UsbId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::call::{CallError, CallResult, args};
use crate::core::{Core, now_ms};

/// The topic the flash screens read.
pub const TOPIC: &str = "flash";
/// The job kind of a loader run.
pub const KIND: &str = "flash-loader";
/// The one step of a loader run.
const STEP: &str = "loader";
/// Log lines kept for one flash.
const LOG_LINES: usize = 200;
/// Flashes remembered under "Recent flashes here".
const RECENT: usize = 12;

// ----- the boot ROM seam ---------------------------------------------------------------------

/// What a loader run found, for the page.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Loaded {
    /// One line: what was sent and that the board answered.
    pub words: String,
    /// What the board said about itself once its loader ran, as label and value.
    pub facts: Vec<(String, String)>,
}

/// One chip family's boot ROM loader. A family joins the flash flow by having one.
#[async_trait]
pub trait BootRom: Send + Sync {
    /// The driver family whose devices this loads (the family of the device's key).
    fn family(&self) -> &str;
    /// What the family's boot ROM is called on the page.
    fn label(&self) -> &str;
    /// One line about what will be sent, shown before anything is.
    fn loader(&self) -> String;
    /// Runs the loader on the device. `say` takes a line for the flash's own log. Nothing is
    /// written to the board's storage. Safe to run again after any failure.
    async fn load(
        &self,
        device: &Identity,
        say: &(dyn Fn(String) + Send + Sync),
    ) -> Result<Loaded, DriverError>;
}

/// The Rockchip boot ROM: the maskrom download of a loader file's DDR init and usbplug, then the
/// usbplug's readiness and its report (`atlas-rockchip`).
pub struct RockchipRom {
    family: String,
    bus: Arc<dyn UsbBus>,
    file: Vec<u8>,
    loader: Loader,
    options: OpenOptions,
    reenumerate: Duration,
}

impl std::fmt::Debug for RockchipRom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RockchipRom")
            .field("family", &self.family)
            .finish_non_exhaustive()
    }
}

impl RockchipRom {
    /// A loader for the devices of `family` on `bus`, sending the loader file `file`. Fails when
    /// the file is not a loader.
    pub fn new(family: &str, bus: Arc<dyn UsbBus>, file: Vec<u8>) -> Result<Self, String> {
        let loader = Loader::parse(&file).map_err(|error| error.0)?;
        Ok(Self {
            family: family.to_owned(),
            bus,
            file,
            loader,
            options: OpenOptions::default(),
            reenumerate: Duration::from_secs(10),
        })
    }

    /// The transfer limits and how long the board may take to come back in loader mode.
    #[must_use]
    pub fn with_limits(mut self, options: OpenOptions, reenumerate: Duration) -> Self {
        self.options = options;
        self.reenumerate = reenumerate;
        self
    }

    fn stage_bytes(&self, kind: EntryKind) -> usize {
        self.loader
            .entries(kind)
            .map(|entry| entry.data(&self.file).len())
            .sum()
    }
}

fn usb_id(text: &str) -> Option<UsbId> {
    let (vendor, product) = text.split_once(':')?;
    Some(UsbId {
        vendor: u16::from_str_radix(vendor, 16).ok()?,
        product: u16::from_str_radix(product, 16).ok()?,
    })
}

#[async_trait]
impl BootRom for RockchipRom {
    fn family(&self) -> &str {
        &self.family
    }

    fn label(&self) -> &str {
        "Rockchip maskrom"
    }

    fn loader(&self) -> String {
        format!(
            "Atlas Rust loader, no vendor tool · DDR init {} and usbplug {}",
            bytes_words(self.stage_bytes(EntryKind::Ddr) as u64),
            bytes_words(self.stage_bytes(EntryKind::Usbplug) as u64),
        )
    }

    async fn load(
        &self,
        device: &Identity,
        say: &(dyn Fn(String) + Send + Sync),
    ) -> Result<Loaded, DriverError> {
        let port: PortPath = device.address.parse().map_err(|_| {
            DriverError::Incompatible(format!("`{}` is not a USB port", device.address))
        })?;
        let id = device
            .attributes
            .get("usb-id")
            .and_then(|text| usb_id(text))
            .ok_or_else(|| DriverError::Incompatible("the device reports no USB id".into()))?;
        // The mode is read from the bus now, not from the last scan: a try that got as far as
        // the usbplug leaves the board in loader mode, and the next one must not send it again.
        let now = self
            .bus
            .devices()
            .await
            .map_err(|error| DriverError::Unreachable(error.to_string()))?
            .into_iter()
            .find(|found| found.location == device.address)
            .ok_or_else(|| {
                DriverError::Unreachable(format!("no device on USB {}", device.address))
            })?;
        let started = tokio::time::Instant::now();
        let mut opened = if atlas_rockchip::Mode::of(&now) == atlas_rockchip::Mode::Loader {
            say("loader: the board is already in loader mode, nothing is sent".into());
            atlas_usb::open_expected(
                self.bus.as_ref(),
                &port,
                atlas_usb::Expect::AnyIdentity,
                &self.options,
                Duration::ZERO,
            )
            .await
            .into_result()
            .map_err(repeatable)?
        } else {
            say(format!(
                "loader: sending DDR init and the usbplug to USB {}",
                device.address
            ));
            atlas_rockchip::run_usbplug(
                self.bus.as_ref(),
                &port,
                id,
                &self.loader,
                &self.file,
                &self.options,
                self.reenumerate,
            )
            .await
            .into_result()
            .map_err(repeatable)?
        };
        let sent = started.elapsed();
        say("loader: the board came back in loader mode".into());
        atlas_rockchip::ready(opened.io.as_mut())
            .await
            .map_err(|error| {
                DriverError::Unreachable(format!("the usbplug is not ready: {error}"))
            })?;
        let report = atlas_rockchip::report(opened.io.as_mut())
            .await
            .map_err(|error| DriverError::Unreachable(format!("the usbplug's report: {error}")))?;
        let mut facts = Vec::new();
        if let Some(chip) = &report.chip {
            facts.push(("Chip".to_owned(), chip.clone()));
            if atlas_rockchip::chip_agrees(&self.loader, &report) == Some(false) {
                say(format!(
                    "warn: loader: the loader file is for another chip than the board's ({chip})"
                ));
            }
        }
        facts.push((
            "Storage".to_owned(),
            format!("{} seen by the loader", bytes_words(report.flash.bytes())),
        ));
        say(format!(
            "loader: the usbplug answers{}",
            report
                .chip
                .as_ref()
                .map(|chip| format!(", chip {chip}"))
                .unwrap_or_default()
        ));
        Ok(Loaded {
            words: format!(
                "sent in {:.1} s, the board answers in loader mode",
                sent.as_secs_f64()
            ),
            facts,
        })
    }
}

/// How a stopped loader step is reported to the job. A refusal (the wrong device on the port, a
/// stage the boot ROM rejects) will not change on its own and ends the run. Anything else is
/// tried again: a loader lives in the board's memory and writes nothing, and [`BootRom::load`]
/// reads the board's mode before it sends anything, so a try whose end was not seen is safe to
/// repeat. (The job engine alone would call such a stop "unknown" and stop for good, which is
/// right for a write to storage and wrong here.)
fn repeatable(stop: atlas_usb::Stop) -> DriverError {
    if stop.is_refusal() {
        DriverError::Flash(stop)
    } else {
        // The stop's own sentence, without the one-word tag it starts with.
        let text = stop.to_string();
        let words = match text.split_once(": ") {
            Some((tag, rest)) if tag.chars().all(|c| c.is_ascii_lowercase() || c == ' ') => rest,
            _ => &text,
        };
        DriverError::Other(words.to_owned())
    }
}

/// `412 MB`: decimal units, as the page writes sizes.
fn bytes_words(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let (mut value, mut unit) = (bytes as f64, 0);
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value >= 100.0 || unit == 0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit]).replace(".0 ", " ")
    }
}

// ----- the disk seam -------------------------------------------------------------------------

/// Writes an image to a removable disk and reads it back.
pub trait DiskWriter: Send + Sync {
    /// Blocks until the write ends. `device` is the disk's record as the disk driver made it.
    fn write(
        &self,
        device: &Identity,
        image: &Path,
        progress: &mut dyn FnMut(WriteProgress),
        cancel: &AtomicBool,
    ) -> Result<WriteReport, String>;
}

/// The size the disk driver recorded for a disk.
fn disk_size(device: &Identity) -> Option<u64> {
    device.attributes.get("size-bytes")?.parse().ok()
}

/// `atlas-disk`'s write on a [`Platform`], in this process. For a platform that needs no extra
/// rights (the simulated one); a real disk is written by the helper ([`HelperWriter`]).
pub struct PlatformWriter {
    platform: Arc<dyn Platform>,
    policy: ResumePolicy,
}

impl std::fmt::Debug for PlatformWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformWriter").finish_non_exhaustive()
    }
}

impl PlatformWriter {
    #[must_use]
    pub fn new(platform: Arc<dyn Platform>, policy: ResumePolicy) -> Self {
        Self { platform, policy }
    }
}

impl DiskWriter for PlatformWriter {
    fn write(
        &self,
        device: &Identity,
        image: &Path,
        progress: &mut dyn FnMut(WriteProgress),
        cancel: &AtomicBool,
    ) -> Result<WriteReport, String> {
        let chosen: Disk = self
            .platform
            .disks()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|disk| disk.path.display().to_string() == device.address)
            .filter(|disk| disk_size(device).is_none_or(|size| size == disk.size_bytes))
            .ok_or_else(|| format!("{} is not there any more; choose it again", device.address))?;
        atlas_disk::write_disk(
            Arc::clone(&self.platform),
            &chosen,
            Acknowledgement::Given,
            &WriteRequest {
                image,
                options: &WriteOptions {
                    verify: true,
                    image_sha256: None,
                },
                policy: &self.policy,
                journal: None,
                allow_large: false,
            },
            progress,
            cancel,
        )
        .map_err(|error| error.to_string())
    }
}

/// The privileged helper (`atlas-helper write`), started through the system's own elevation
/// prompt. The helper trusts nothing from the app: it lists the disks again and refuses a system
/// disk, a disk that is not removable, and a disk whose size changed.
///
/// Never run by the item that wrote it (it would ask for rights and write a real disk): the
/// arguments and the reading of the helper's lines are unit-tested.
#[derive(Debug)]
pub struct HelperWriter {
    helper: PathBuf,
    work: PathBuf,
}

/// One line the helper writes to its progress file.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
enum HelperLine {
    Progress { progress: WriteProgress },
    Done { report: WriteReport },
    Error { message: String },
}

impl HelperWriter {
    /// The helper beside this program (or the one `ATLAS_HELPER` names), keeping its progress
    /// files under `work`. `None` when there is no helper to run.
    #[must_use]
    pub fn find(work: &Path) -> Option<Self> {
        let named = std::env::var_os("ATLAS_HELPER")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let beside = std::env::current_exe().ok().and_then(|exe| {
            let name = if cfg!(windows) {
                "atlas-helper.exe"
            } else {
                "atlas-helper"
            };
            Some(exe.parent()?.join(name))
        });
        let helper = named.or(beside).filter(|path| path.is_file())?;
        Some(Self {
            helper,
            work: work.to_path_buf(),
        })
    }

    fn arguments(
        device: &Identity,
        image: &Path,
        progress: &Path,
        cancel: &Path,
    ) -> Result<Vec<String>, String> {
        let size = disk_size(device).ok_or("the disk's size is not known; choose it again")?;
        let text = |path: &Path| path.display().to_string();
        Ok(vec![
            "write".into(),
            "--device".into(),
            device.address.clone(),
            "--image".into(),
            text(image),
            "--expected-size".into(),
            size.to_string(),
            "--progress".into(),
            text(progress),
            "--cancel-file".into(),
            text(cancel),
        ])
    }
}

impl DiskWriter for HelperWriter {
    fn write(
        &self,
        device: &Identity,
        image: &Path,
        progress: &mut dyn FnMut(WriteProgress),
        cancel: &AtomicBool,
    ) -> Result<WriteReport, String> {
        if !cfg!(target_os = "linux") {
            return Err("writing a disk from the app is set up on Linux only so far; use `atlas-helper write` from an elevated terminal".into());
        }
        std::fs::create_dir_all(&self.work).map_err(|error| error.to_string())?;
        let stamp = now_ms();
        let progress_file = self.work.join(format!("write-{stamp}.jsonl"));
        let cancel_file = self.work.join(format!("write-{stamp}.cancel"));
        let arguments = Self::arguments(device, image, &progress_file, &cancel_file)?;
        let mut child = std::process::Command::new("pkexec")
            .arg(&self.helper)
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| {
                format!("the helper could not be started with elevated rights: {error}")
            })?;
        let mut read = 0usize;
        let mut result = None;
        let outcome = loop {
            if cancel.load(Ordering::Relaxed) && !cancel_file.exists() {
                let _ = std::fs::write(&cancel_file, b"");
            }
            let text = std::fs::read_to_string(&progress_file).unwrap_or_default();
            // Only whole lines: the helper may be in the middle of one.
            let whole = text.rfind('\n').map_or(0, |end| end + 1);
            for line in text[read.min(whole)..whole].lines() {
                match serde_json::from_str::<HelperLine>(line) {
                    Ok(HelperLine::Progress { progress: step }) => progress(step),
                    Ok(HelperLine::Done { report }) => result = Some(Ok(report)),
                    Ok(HelperLine::Error { message }) => result = Some(Err(message)),
                    Err(_) => {}
                }
            }
            read = whole;
            match child.try_wait() {
                Ok(Some(status)) if read == whole => {
                    break result.unwrap_or_else(|| {
                        Err(format!("the helper ended without a result ({status}); nothing is known to be written"))
                    });
                }
                Ok(_) => std::thread::sleep(Duration::from_millis(200)),
                Err(error) => break Err(error.to_string()),
            }
        };
        let _ = std::fs::remove_file(&progress_file);
        let _ = std::fs::remove_file(&cancel_file);
        outcome
    }
}

// ----- state ---------------------------------------------------------------------------------

/// The chosen image, as far as it has been read.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
enum Image {
    Reading {
        path: String,
        file: String,
        bytes: u64,
    },
    Ready {
        path: String,
        file: String,
        bytes: u64,
        raw_bytes: u64,
        /// The board the image was built for, from its slots.
        board: String,
        /// The slots the image holds, read out of it in the same pass.
        slots: Vec<String>,
        sha256: String,
        /// The signing record's key, when the file carries one. Atlas does not check it.
        signed_by: Option<String>,
    },
    Refused {
        path: String,
        file: String,
        bytes: u64,
        words: String,
    },
}

impl Image {
    fn ready_path(&self) -> Option<&str> {
        match self {
            Self::Ready { path, .. } => Some(path),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct LogLine {
    at_ms: u64,
    level: &'static str,
    text: String,
}

fn log_line(text: &str) -> LogLine {
    let (level, text) = match text.split_once(": ") {
        Some(("warn", rest)) => ("warn", rest),
        Some(("err", rest)) => ("err", rest),
        _ => ("info", text),
    };
    LogLine {
        at_ms: now_ms(),
        level,
        text: text.to_owned(),
    }
}

/// One loader run, as the page shows it.
#[derive(Clone, Debug, Serialize)]
struct Run {
    job: u64,
    device: String,
    name: String,
    family: String,
    port: String,
    /// `queued`, `running`, `retrying`, `stopped`, `loaded`, `failed`, `cancelled`,
    /// `interrupted` or `unknown`.
    status: &'static str,
    attempt: u32,
    error: String,
    retry_in_ms: u64,
    retry_at_ms: u64,
    started_ms: u64,
    ended_ms: Option<u64>,
    loaded: Option<Loaded>,
    log: VecDeque<LogLine>,
    #[serde(skip)]
    identity: Identity,
}

impl Run {
    fn say(&mut self, text: &str) {
        self.log.push_back(log_line(text));
        while self.log.len() > LOG_LINES {
            self.log.pop_front();
        }
    }

    fn finished(&self) -> bool {
        matches!(
            self.status,
            "loaded" | "failed" | "cancelled" | "interrupted" | "unknown"
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Recent {
    name: String,
    at_ms: u64,
    /// `loaded`, `failed`, `cancelled`, `written` and so on: the run's last status.
    result: String,
    /// Whether it took more than one try.
    retried: bool,
}

/// One disk write, as the page shows it.
#[derive(Clone, Debug, Serialize)]
struct DiskRun {
    device: String,
    name: String,
    file: String,
    /// `writing`, `done`, `failed` or `cancelled`.
    status: &'static str,
    /// `checking`, `preparing`, `writing`, `dropped`, `reattaching` or `resuming`.
    phase: &'static str,
    input_done: u64,
    input_total: u64,
    written: u64,
    drops: u32,
    words: String,
    report: Option<WriteReport>,
    started_ms: u64,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
struct State {
    image: Option<Image>,
    run: Option<Run>,
    recent: VecDeque<Recent>,
    disk: Option<DiskRun>,
}

struct Shared {
    state: Mutex<State>,
    dirty: Notify,
    roms: Vec<Arc<dyn BootRom>>,
    writer: Option<Arc<dyn DiskWriter>>,
    recent_file: Option<PathBuf>,
    scratch: PathBuf,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The flash screens' state. Held by the core.
#[derive(Clone)]
pub struct Flashes {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Flashes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flashes").finish_non_exhaustive()
    }
}

struct LoaderKind(Arc<Shared>);

#[async_trait]
impl JobKind for LoaderKind {
    fn name(&self) -> &str {
        KIND
    }

    /// A loader lives in the board's memory: after an Atlas restart the run is started again by
    /// the person, not continued.
    fn resumable(&self) -> bool {
        false
    }

    fn needs_board_identity(&self) -> bool {
        true
    }

    async fn run_step(&self, claim: &DeviceClaim, _step: &str) -> Result<(), DriverError> {
        let shared = &self.0;
        let key = claim.devices.first().map(ToString::to_string);
        let identity = {
            let state = lock(&shared.state);
            state
                .run
                .as_ref()
                .filter(|run| Some(&run.device) == key.as_ref())
                .map(|run| run.identity.clone())
        }
        .ok_or_else(|| {
            DriverError::Incompatible("this flash is not on the page any more".into())
        })?;
        let family = identity
            .key
            .family()
            .map(|family| family.as_str().to_owned())
            .unwrap_or_default();
        let rom = shared
            .roms
            .iter()
            .find(|rom| rom.family() == family)
            .ok_or_else(|| {
                DriverError::Unsupported(format!("this build has no loader for {family}"))
            })?;
        let say = |text: String| {
            if let Some(run) = lock(&shared.state).run.as_mut() {
                run.say(&text);
            }
            shared.dirty.notify_one();
        };
        let loaded = rom.load(&identity, &say).await?;
        if let Some(run) = lock(&shared.state).run.as_mut() {
            run.loaded = Some(loaded);
        }
        shared.dirty.notify_one();
        Ok(())
    }
}

impl Flashes {
    /// The flash state with the loaders and the disk writer this build has.
    #[must_use]
    pub fn new(
        roms: Vec<Arc<dyn BootRom>>,
        writer: Option<Arc<dyn DiskWriter>>,
        data_dir: Option<&Path>,
    ) -> Self {
        let recent_file = data_dir.map(|dir| dir.join("flash-recent.json"));
        let recent = recent_file
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<VecDeque<Recent>>(&bytes).ok())
            .unwrap_or_default();
        let scratch = data_dir.map_or_else(
            || std::env::temp_dir().join(format!("atlas-flash-{}", std::process::id())),
            |dir| dir.join("flash-scratch"),
        );
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    recent,
                    ..State::default()
                }),
                dirty: Notify::new(),
                roms,
                writer,
                recent_file,
                scratch,
            }),
        }
    }

    /// The job kind the core registers, so a loader run is a job like any other.
    #[must_use]
    pub fn kind(&self) -> Arc<dyn JobKind> {
        Arc::new(LoaderKind(Arc::clone(&self.shared)))
    }

    fn view(&self) -> Value {
        let state = lock(&self.shared.state);
        json!({
            "image": state.image,
            "run": state.run,
            "recent": state.recent,
            "disk": state.disk,
            "roms": self.shared.roms.iter().map(|rom| json!({
                "family": rom.family(),
                "label": rom.label(),
                "loader": rom.loader(),
            })).collect::<Vec<_>>(),
            "can_write_disks": self.shared.writer.is_some(),
            // The longest wait between two tries (the failure rule's cap), for the page's words.
            // TODO(user): this is JobTiming::default()'s cap, not the cap of the Jobs that run the
            // loader (core.rs open_jobs: atlas_files::job_timing(), divided by Options::retry_scale
            // in the UI bridge), nor flash-flow's MAX_BACKOFF (crates/core/flash-flow/src/flow.rs,
            // whose FIRST_BACKOFF and MAX_BACKOFF are off the app's path). All three are 5 s today
            // and can drift apart. Proposed: read the running Jobs' timing. In the product it is
            // the same value; only the bridge's scaled sim differs.
            "backoff_cap_ms": u64::try_from(atlas_jobs::JobTiming::default().backoff.cap.as_millis()).unwrap_or(0),
        })
    }

    fn remember(&self, name: &str, result: &str, retried: bool) {
        let saved = {
            let mut state = lock(&self.shared.state);
            state.recent.push_front(Recent {
                name: name.to_owned(),
                at_ms: now_ms(),
                result: result.to_owned(),
                retried,
            });
            state.recent.truncate(RECENT);
            serde_json::to_vec_pretty(&state.recent).ok()
        };
        if let (Some(path), Some(bytes)) = (&self.shared.recent_file, saved) {
            let _ = std::fs::write(path, bytes);
        }
    }

    /// Takes a change of a job: the page's run follows its own job's status.
    fn job_changed(&self, job: &JobRecord) {
        let ended = {
            let mut state = lock(&self.shared.state);
            let Some(run) = state.run.as_mut().filter(|run| run.job == job.id.0) else {
                return;
            };
            let before = run.status;
            match &job.status {
                JobStatus::Queued => run.status = "queued",
                JobStatus::Running => run.status = "running",
                JobStatus::Retrying {
                    attempt,
                    error,
                    retry_in_ms,
                } => {
                    if run.attempt != *attempt || run.status != "retrying" {
                        run.say(&format!("err: loader: attempt {attempt}: {error}"));
                    }
                    run.status = "retrying";
                    run.attempt = *attempt;
                    run.error = error.clone();
                    run.retry_in_ms = *retry_in_ms;
                    run.retry_at_ms = now_ms() + retry_in_ms;
                }
                JobStatus::Paused => {
                    if before != "stopped" {
                        run.say("loader: stopped by hand; nothing is tried until Retry");
                    }
                    run.status = "stopped";
                }
                JobStatus::Done => run.status = "loaded",
                JobStatus::Failed { reason } => {
                    run.status = "failed";
                    run.error = reason.clone();
                    run.say(&format!("err: loader: {reason}"));
                }
                JobStatus::Unknown { reason, .. } => {
                    run.status = "unknown";
                    run.error = reason.clone();
                }
                JobStatus::Cancelled => run.status = "cancelled",
                JobStatus::Interrupted { reason } => {
                    run.status = "interrupted";
                    run.error = reason.clone();
                }
            }
            if run.finished() && run.ended_ms.is_none() {
                run.ended_ms = Some(now_ms());
                Some((run.name.clone(), run.status, run.attempt > 0))
            } else {
                None
            }
        };
        if let Some((name, status, retried)) = ended {
            self.remember(&name, status, retried);
        }
        self.shared.dirty.notify_one();
    }

    /// Publishes the topic now and after every change, and follows the jobs. Runs for the app's
    /// life.
    pub async fn follow(self, core: Core) {
        let mut events = core.events().subscribe();
        core.publish(TOPIC, self.view());
        loop {
            tokio::select! {
                () = self.shared.dirty.notified() => {}
                event = events.recv() => match event {
                    Ok(Event::Job { job }) if job.kind == KIND => self.job_changed(&job),
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let job = lock(&self.shared.state).run.as_ref().map(|run| JobId(run.job));
                        if let Some(job) = job.and_then(|id| core.jobs().get(id)) {
                            self.job_changed(&job);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
            }
            core.publish(TOPIC, self.view());
        }
    }
}

// ----- calls ---------------------------------------------------------------------------------

#[derive(Deserialize)]
struct OnDevice {
    id: String,
}

fn record(core: &Core, id: &str, state: DeviceState) -> Result<(Identity, String), CallError> {
    let key =
        DeviceKey::parse(id).ok_or_else(|| CallError(format!("`{id}` is not a device key")))?;
    core.with_inventory(|inventory| {
        let record = inventory
            .get(&key)
            .ok_or_else(|| CallError(format!("no device {id}")))?;
        if record.presence != atlas_inventory::Presence::Online || record.identity.state != state {
            return Err(CallError(format!(
                "{} is not there in that state any more",
                record.display_name()
            )));
        }
        let name = record
            .label
            .clone()
            .or_else(|| record.identity.name.clone())
            .or_else(|| record.identity.model.clone())
            .unwrap_or_else(|| id.to_owned());
        Ok((record.identity.clone(), name))
    })
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

async fn pick_image(core: &Core) -> CallResult {
    let flashes = core.flashes().clone();
    let picker = core.boards().picker();
    let picked = tokio::task::spawn_blocking(move || {
        picker.and_then(|picker| picker.pick("Choose a first-flash image", &["flash", "xz", "img"]))
    })
    .await
    .map_err(|error| CallError(error.to_string()))?;
    let Some(path) = picked else {
        return Ok(Value::Bool(false));
    };
    let file = file_name(&path);
    let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let shown = path.display().to_string();
    core.log()
        .breadcrumb(&format!("flash image chosen: {file}"));
    lock(&flashes.shared.state).image = Some(Image::Reading {
        path: shown.clone(),
        file: file.clone(),
        bytes,
    });
    flashes.shared.dirty.notify_one();
    let scratch = flashes.shared.scratch.clone();
    let core = core.clone();
    tokio::spawn(async move {
        let read_from = path.clone();
        let read = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
            atlas_image::flash_manifest(&read_from, &scratch, None)
                .map_err(|error| error.to_string())
        })
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
        // The file's own signature (`<file>.minisig`) against the trusted keys: with security
        // on, a file without a good one is refused here, before anything is sent to a board.
        let read = match read {
            Ok(image) => {
                let (work, checked) = (core.clone(), path.clone());
                let security = core.settings().security;
                let judged = tokio::task::spawn_blocking(move || {
                    work.trust().judge(&checked, "image to flash", security)
                })
                .await
                .unwrap_or_else(|error| Err(error.to_string()));
                crate::trust::publish(&core);
                judged.map(|_| image)
            }
            Err(words) => Err(words),
        };
        let image = match read {
            Ok(image) => Image::Ready {
                path: shown.clone(),
                file,
                bytes,
                raw_bytes: image.manifest.image_size(),
                board: image.manifest.board().to_owned(),
                slots: image.slots.clone(),
                sha256: image.manifest.image_sha256().to_owned(),
                signed_by: match &image.signature {
                    SignatureState::Unsigned => None,
                    signed @ SignatureState::Signed { .. } => signed.key_id_hex(),
                },
            },
            Err(words) => {
                core.log().failed_call("flash.read_image", &shown, &words);
                Image::Refused {
                    path: shown.clone(),
                    file,
                    bytes,
                    words,
                }
            }
        };
        let mut state = lock(&flashes.shared.state);
        // Only when this is still the image on the page: a second choice may have replaced it.
        if matches!(&state.image, Some(Image::Reading { path, .. }) if *path == shown) {
            state.image = Some(image);
        }
        drop(state);
        flashes.shared.dirty.notify_one();
    });
    Ok(Value::Bool(true))
}

async fn start(core: &Core, id: &str) -> CallResult {
    let flashes = core.flashes();
    let (identity, name) = record(core, id, DeviceState::BootRom)?;
    let family = identity
        .key
        .family()
        .map(|family| family.as_str().to_owned())
        .unwrap_or_default();
    let rom = flashes
        .shared
        .roms
        .iter()
        .find(|rom| rom.family() == family)
        .ok_or_else(|| CallError(format!("this build has no loader for {family}")))?;
    {
        let state = lock(&flashes.shared.state);
        if state.run.as_ref().is_some_and(|run| !run.finished()) {
            return Err(CallError(
                "a flash is already running; stop it first".into(),
            ));
        }
        if state.image.as_ref().and_then(Image::ready_path).is_none() {
            return Err(CallError("choose an image first".into()));
        }
    }
    let claim = DeviceClaim::writes([identity.key.clone()])
        .quiet()
        .at(identity.address.clone());
    // The run is on the page before the job starts, so the job's first step finds it.
    let mut run = Run {
        job: 0,
        device: id.to_owned(),
        name,
        family: rom.label().to_owned(),
        port: identity.address.clone(),
        status: "queued",
        attempt: 0,
        error: String::new(),
        retry_in_ms: 0,
        retry_at_ms: 0,
        started_ms: now_ms(),
        ended_ms: None,
        loaded: None,
        log: VecDeque::new(),
        identity,
    };
    run.say(&format!("usb: {} on USB {}", run.family, run.port));
    lock(&flashes.shared.state).run = Some(run);
    match core.jobs().submit(KIND, claim, vec![STEP.to_owned()]).await {
        Ok(job) => {
            if let Some(run) = lock(&flashes.shared.state).run.as_mut() {
                run.job = job.0;
            }
            // The job may have moved before its id was known here: read it once.
            if let Some(job) = core.jobs().get(job) {
                flashes.job_changed(&job);
            }
            flashes.shared.dirty.notify_one();
            Ok(json!(job.0))
        }
        Err(error) => {
            lock(&flashes.shared.state).run = None;
            flashes.shared.dirty.notify_one();
            Err(CallError(error.to_string()))
        }
    }
}

fn run_job(core: &Core) -> Result<JobId, CallError> {
    lock(&core.flashes().shared.state)
        .run
        .as_ref()
        .map(|run| JobId(run.job))
        .ok_or_else(|| CallError("no flash is on the page".into()))
}

fn write_disk(core: &Core, id: &str) -> CallResult {
    let flashes = core.flashes().clone();
    let writer = flashes.shared.writer.clone().ok_or_else(|| {
        CallError(
            "this build cannot write a disk: the helper (atlas-helper) is not beside the app"
                .into(),
        )
    })?;
    let (identity, name) = record(core, id, DeviceState::RemovableDisk)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let (image, file) = {
        let mut state = lock(&flashes.shared.state);
        if state
            .disk
            .as_ref()
            .is_some_and(|disk| disk.status == "writing")
        {
            return Err(CallError("a disk is already being written".into()));
        }
        let Some(Image::Ready {
            path, file, bytes, ..
        }) = &state.image
        else {
            return Err(CallError("choose an image first".into()));
        };
        let (image, file, bytes) = (PathBuf::from(path), file.clone(), *bytes);
        state.disk = Some(DiskRun {
            device: id.to_owned(),
            name: name.clone(),
            file: file.clone(),
            status: "writing",
            phase: "checking",
            input_done: 0,
            input_total: bytes,
            written: 0,
            drops: 0,
            words: String::new(),
            report: None,
            started_ms: now_ms(),
            cancel: Arc::clone(&cancel),
        });
        (image, file)
    };
    flashes.shared.dirty.notify_one();
    core.log().breadcrumb(&format!("writing {file} to {name}"));
    let core = core.clone();
    tokio::task::spawn_blocking(move || {
        let shared = Arc::clone(&flashes.shared);
        let mut progress = |step: WriteProgress| {
            if let Some(disk) = lock(&shared.state).disk.as_mut() {
                match step {
                    WriteProgress::CheckingImage { .. } => disk.phase = "checking",
                    WriteProgress::Preparing => disk.phase = "preparing",
                    WriteProgress::Writing {
                        input_done,
                        input_total,
                        written,
                    } => {
                        disk.phase = "writing";
                        (disk.input_done, disk.input_total, disk.written) =
                            (input_done, input_total, written);
                    }
                    WriteProgress::LinkDropped {
                        written,
                        drops,
                        reason,
                        ..
                    } => {
                        disk.phase = "dropped";
                        (disk.written, disk.drops, disk.words) = (written, drops, reason);
                    }
                    WriteProgress::Reattaching { reason, .. } => {
                        disk.phase = "reattaching";
                        disk.words = reason;
                    }
                    WriteProgress::Resuming { written, .. } => {
                        disk.phase = "resuming";
                        disk.written = written;
                        disk.words.clear();
                    }
                }
            }
            shared.dirty.notify_one();
        };
        let result = writer.write(&identity, &image, &mut progress, &cancel);
        let status = match &result {
            Ok(_) => "written",
            Err(_) if cancel.load(Ordering::Relaxed) => "cancelled",
            Err(_) => "failed",
        };
        if let Err(words) = &result
            && status == "failed"
        {
            core.log()
                .failed_call("disk.write", &format!("{file} to {name}"), words);
        }
        if let Some(disk) = lock(&flashes.shared.state).disk.as_mut() {
            disk.status = match status {
                "written" => "done",
                other => other,
            };
            match result {
                Ok(report) => disk.report = Some(report),
                Err(words) => disk.words = words,
            }
        }
        flashes.remember(&name, status, false);
        flashes.shared.dirty.notify_one();
    });
    Ok(Value::Null)
}

/// The `flash.*` and `disk.*` calls.
pub async fn call(core: &Core, name: &str, arguments: Value) -> CallResult {
    let flashes = core.flashes();
    match name {
        "flash.pick_image" => pick_image(core).await,
        "flash.clear_image" => {
            lock(&flashes.shared.state).image = None;
            flashes.shared.dirty.notify_one();
            Ok(Value::Null)
        }
        "flash.start" => {
            let OnDevice { id } = args(name, arguments)?;
            core.log().breadcrumb(&format!("flash started on {id}"));
            start(core, &id).await
        }
        // Retry now: the wait before the next try is cut short, or a stopped run goes on.
        "flash.retry" => Ok(Value::Bool(core.jobs().resume(run_job(core)?))),
        // The manual off: nothing is tried again until Retry.
        "flash.stop" => Ok(Value::Bool(core.jobs().pause(run_job(core)?))),
        "flash.cancel" => Ok(Value::Bool(core.jobs().cancel(run_job(core)?))),
        // Takes a finished run off the page.
        "flash.dismiss" => {
            let mut state = lock(&flashes.shared.state);
            if state.run.as_ref().is_some_and(Run::finished) {
                state.run = None;
            }
            drop(state);
            flashes.shared.dirty.notify_one();
            Ok(Value::Null)
        }
        "disk.write" => {
            let OnDevice { id } = args(name, arguments)?;
            write_disk(core, &id)
        }
        "disk.cancel" => {
            if let Some(disk) = lock(&flashes.shared.state).disk.as_ref() {
                disk.cancel.store(true, Ordering::Relaxed);
            }
            Ok(Value::Null)
        }
        "disk.dismiss" => {
            let mut state = lock(&flashes.shared.state);
            if state
                .disk
                .as_ref()
                .is_some_and(|disk| disk.status != "writing")
            {
                state.disk = None;
            }
            drop(state);
            flashes.shared.dirty.notify_one();
            Ok(Value::Null)
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn disk_identity() -> Identity {
        Identity {
            key: DeviceKey::from_static("removable-disk", "CARD-1"),
            model: Some("SD card".into()),
            state: DeviceState::RemovableDisk,
            lemnos: atlas_types::LemnosState::Absent,
            versions: BTreeMap::new(),
            name: None,
            link: atlas_types::LinkId::new("disks"),
            address: "/dev/sim-card".into(),
            attributes: BTreeMap::from([("size-bytes".to_string(), "64000000000".to_string())]),
        }
    }

    #[test]
    fn the_helper_is_asked_for_one_disk_one_image_and_the_size_that_was_chosen() {
        let arguments = HelperWriter::arguments(
            &disk_identity(),
            Path::new("/images/a.img.xz"),
            Path::new("/work/p.jsonl"),
            Path::new("/work/p.cancel"),
        )
        .unwrap();
        assert_eq!(
            arguments,
            [
                "write",
                "--device",
                "/dev/sim-card",
                "--image",
                "/images/a.img.xz",
                "--expected-size",
                "64000000000",
                "--progress",
                "/work/p.jsonl",
                "--cancel-file",
                "/work/p.cancel",
            ]
        );
        let mut unsized_disk = disk_identity();
        unsized_disk.attributes.clear();
        assert!(
            HelperWriter::arguments(
                &unsized_disk,
                Path::new("/a"),
                Path::new("/p"),
                Path::new("/c")
            )
            .is_err()
        );
    }

    #[test]
    fn the_helpers_lines_are_read_as_progress_a_result_or_its_error() {
        let progress = r#"{"type":"progress","progress":{"phase":"writing","input_done":5,"input_total":10,"written":4096}}"#;
        assert!(matches!(
            serde_json::from_str::<HelperLine>(progress).unwrap(),
            HelperLine::Progress {
                progress: WriteProgress::Writing { written: 4096, .. }
            }
        ));
        let done = r#"{"type":"done","report":{"bytes_written":10,"sha256":"ab","verified":true,"seconds":1}}"#;
        assert!(matches!(
            serde_json::from_str::<HelperLine>(done).unwrap(),
            HelperLine::Done { report } if report.verified && report.bytes_written == 10
        ));
        let error = r#"{"type":"error","message":"sda holds the running system"}"#;
        assert!(matches!(
            serde_json::from_str::<HelperLine>(error).unwrap(),
            HelperLine::Error { message } if message.contains("running system")
        ));
    }

    #[test]
    fn sizes_and_log_levels_are_written_the_way_the_page_shows_them() {
        assert_eq!(bytes_words(999), "999 B");
        assert_eq!(bytes_words(1_500_000), "1.5 MB");
        assert_eq!(bytes_words(64_000_000_000), "64 GB");
        assert_eq!(log_line("warn: loader: odd").level, "warn");
        assert_eq!(log_line("warn: loader: odd").text, "loader: odd");
        assert_eq!(log_line("usb: 1-3").level, "info");
        assert_eq!(
            usb_id("2207:350b"),
            Some(UsbId {
                vendor: 0x2207,
                product: 0x350b
            })
        );
        assert_eq!(usb_id("nope"), None);
    }
}
