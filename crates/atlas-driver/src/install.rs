//! One install model (docs/devices-as-data.md): an artifact, what kind it
//! is (by its content), and the methods a device takes it by. Each method
//! is a small adapter over the ways Atlas already installs (a slot, an A/B
//! image, a fresh install over USB boot; later a package on a host's
//! package manager), so choosing one is the same question for every device.
//!
//! A method that fails says itself whether anything changed on the device
//! (`InstallFailure::changed_nothing`), so another method or way in is tried
//! only when it is safe; nothing infers that from an error's wording.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    Artifact, CancellationToken, DriverError, Identity, ProgressSink, ProgressUpdate,
    UpdateOutcome, UpdateStep,
};

/// What an artifact is, from its content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    /// `<id>.slot.tar`: `slot.json`, the root image, `boot/`.
    SlotBundle,
    /// A slots build's first-flash disk: a FAT with `slots/<id>/` and a data
    /// partition with the slot's root. Installs as a slot or fresh.
    SlotsImage,
    /// A whole disk on the A/B layout (boot, root A, root B, data).
    AbImage,
    /// Some other whole disk image: a fresh install only.
    DiskImage,
    /// A layer for a running layered slot.
    Layer,
    /// An opkg package (a SystemCore's package manager).
    Ipk,
    /// A Debian package.
    Deb,
    /// Firmware for a microcontroller or a bootloader.
    Firmware,
    /// None of the above.
    Unknown,
}

impl ArtifactKind {
    /// How the UI names it.
    pub fn label(self) -> &'static str {
        match self {
            Self::SlotBundle => "slot bundle",
            Self::SlotsImage => "slots image",
            Self::AbImage => "A/B image",
            Self::DiskImage => "disk image",
            Self::Layer => "layer",
            Self::Ipk => "ipk package",
            Self::Deb => "deb package",
            Self::Firmware => "firmware",
            Self::Unknown => "file",
        }
    }
}

/// The install methods Atlas has adapters for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallMethodId {
    /// Beside the board's other slots, settings kept.
    Slot,
    /// Into the A/B board's spare slot, settings kept.
    AbImage,
    /// A layer of the running slot.
    Layer,
    /// The whole storage written over USB boot: everything erased.
    Fresh,
    /// Through the device's own package manager.
    Package,
}

impl InstallMethodId {
    pub fn label(self) -> &'static str {
        match self {
            Self::Slot => "Install as a slot",
            Self::AbImage => "Update in place",
            Self::Layer => "Push a layer",
            Self::Fresh => "Fresh install",
            Self::Package => "Install the package",
        }
    }
}

/// Choices a method may use; the others ignore them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallOptions {
    /// A slot: boot it once when it is in.
    #[serde(default)]
    pub boot_after: bool,
    /// A slot: keep it from removal.
    #[serde(default)]
    pub pin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRequest {
    pub artifact: Artifact,
    pub kind: ArtifactKind,
    /// The release's version, as the UI names it (`2026.3.4`).
    pub version: String,
    #[serde(default)]
    pub options: InstallOptions,
}

/// What a method would do, before it does anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub method: InstallMethodId,
    pub steps: Vec<UpdateStep>,
    /// Everything on the device goes (the UI asks first).
    pub erases: bool,
    /// The device restarts as part of it.
    pub restarts: bool,
    pub summary: String,
}

/// A method that didn't finish, and whether the device is as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallFailure {
    pub error: DriverError,
    /// Set by the method from where it got to, never from the error's text:
    /// true only when nothing on the device changed.
    pub changed_nothing: bool,
}

impl std::fmt::Display for InstallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for InstallFailure {}

/// One way to install an artifact on a device.
#[async_trait]
pub trait InstallMethod: Send + Sync {
    fn id(&self) -> InstallMethodId;
    /// The kinds of artifact it installs.
    fn kinds(&self) -> &'static [ArtifactKind];
    /// What it would do on this device now; an error says why it can't.
    fn plan(&self, device: &Identity, request: &InstallRequest)
    -> Result<InstallPlan, DriverError>;
    async fn run(
        &self,
        device: &Identity,
        request: &InstallRequest,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, InstallFailure>;
}

/// How a method reaches the device (the manifest's `via`): Orion, the USB
/// console, SSH, a web upload, USB boot. Today each adapter keeps the ways
/// in of the capability it wraps; the engine tries them in order through
/// this once the device packages declare them (devices-as-data, step 2).
#[async_trait]
pub trait DeviceChannel: Send + Sync {
    fn id(&self) -> &'static str;
    /// Runs a command as root, where the channel can.
    async fn run(&self, script: &str) -> Result<Vec<u8>, DriverError>;
}

/// Watches a method's progress for the step that first changes the device
/// (Apply and after), so the method can say `changed_nothing` from where
/// it got to. Steps before it (Preflight, Transfer) only check and copy:
/// the board installs nothing until a copy is whole and checked.
#[derive(Clone)]
pub struct ChangeTracker {
    changed: Arc<AtomicBool>,
}

impl ChangeTracker {
    /// A sink that forwards to `progress` and notes when the device starts
    /// changing.
    pub fn watch(progress: &ProgressSink) -> (Self, ProgressSink) {
        let changed = Arc::new(AtomicBool::new(false));
        let seen = changed.clone();
        let forward = progress.clone();
        let sink = ProgressSink::new(move |update| {
            if let ProgressUpdate::StepStarted { step } = &update
                && matches!(
                    step,
                    UpdateStep::Apply | UpdateStep::Reboot | UpdateStep::Confirm
                )
            {
                seen.store(true, Ordering::SeqCst);
            }
            forward.report(update);
        });
        (Self { changed }, sink)
    }

    /// Marks the device as changed (a method that knows it is past the point).
    pub fn mark_changed(&self) {
        self.changed.store(true, Ordering::SeqCst);
    }

    /// `error` as a failure, with whether the device changed.
    pub fn failure(&self, error: DriverError) -> InstallFailure {
        InstallFailure {
            changed_nothing: !self.changed.load(Ordering::SeqCst),
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_before_apply_changed_nothing_and_after_it_did() {
        let (tracker, sink) = ChangeTracker::watch(&ProgressSink::discard());
        sink.step_started(UpdateStep::Preflight);
        sink.step_started(UpdateStep::Transfer);
        assert!(
            tracker
                .failure(DriverError::Other("x".into()))
                .changed_nothing
        );
        sink.step_started(UpdateStep::Apply);
        assert!(
            !tracker
                .failure(DriverError::Other("x".into()))
                .changed_nothing
        );
    }

    #[test]
    fn kinds_and_methods_name_themselves() {
        assert_eq!(
            serde_json::to_string(&ArtifactKind::SlotsImage).unwrap(),
            "\"slots-image\""
        );
        assert_eq!(
            serde_json::to_string(&InstallMethodId::AbImage).unwrap(),
            "\"ab-image\""
        );
        assert_eq!(InstallMethodId::Fresh.label(), "Fresh install");
    }
}
