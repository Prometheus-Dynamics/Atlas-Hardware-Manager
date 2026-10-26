//! The install methods a device takes an artifact by (docs/devices-as-data.md,
//! step 1): adapters over the capabilities its driver already has, so the
//! Software tab asks one question (what can this device take this file by,
//! and why not?) whatever the device.
//!
//! - `slot`: the slots capability (`SlotsCapability::install`), for a slot
//!   bundle or a slots build's image (the caller makes the bundle from it).
//! - `ab-image`: the update capability of a running A/B board.
//! - `fresh`: the update capability of a device in USB boot (the rpi
//!   driver's flash); a running board that can get into USB boot is offered
//!   it too, through the Software tab's hand-off.
//!
//! Each adapter says from where it got whether the device changed
//! ([`ChangeTracker`]): the board installs nothing until a copy is whole and
//! checked, and every step from Apply on changes it.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{
    ArtifactKind, CancellationToken, ChangeTracker, DeviceMode, DriverError, Identity,
    InstallFailure, InstallMethod, InstallMethodId, InstallPlan, InstallRequest, ProgressSink,
    ReleaseRef, SlotInstall, SlotsCapability, UpdateCapability, UpdateOutcome, UpdateStep,
};
use serde::Serialize;

use crate::{Atlas, CoreError};

/// One method and whether it can run now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstallChoice {
    pub method: InstallMethodId,
    pub label: String,
    /// What it would do; `None` when it can't (then `why_not` says why).
    pub plan: Option<InstallPlan>,
    pub why_not: Option<String>,
}

/// What a device can take an artifact by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstallChoices {
    pub kind: ArtifactKind,
    pub kind_label: String,
    /// In order of preference; the first with a plan is the default.
    pub choices: Vec<InstallChoice>,
}

struct SlotMethod(Arc<dyn SlotsCapability>);

#[async_trait]
impl InstallMethod for SlotMethod {
    fn id(&self) -> InstallMethodId {
        InstallMethodId::Slot
    }

    fn kinds(&self) -> &'static [ArtifactKind] {
        &[ArtifactKind::SlotBundle, ArtifactKind::SlotsImage]
    }

    fn plan(
        &self,
        _device: &Identity,
        request: &InstallRequest,
    ) -> Result<InstallPlan, DriverError> {
        let boot = request.options.boot_after;
        let mut steps = vec![
            UpdateStep::Preflight,
            UpdateStep::Transfer,
            UpdateStep::Apply,
        ];
        if boot {
            steps.push(UpdateStep::Reboot);
        }
        Ok(InstallPlan {
            method: InstallMethodId::Slot,
            steps,
            erases: false,
            restarts: boot,
            summary: if boot {
                format!(
                    "Installs {} beside the other slots and restarts into it once, on trial; settings and data stay",
                    request.version
                )
            } else {
                format!(
                    "Installs {} beside the other slots; settings and data stay",
                    request.version
                )
            },
        })
    }

    async fn run(
        &self,
        device: &Identity,
        request: &InstallRequest,
        progress: &ProgressSink,
        _cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, InstallFailure> {
        let (tracker, sink) = ChangeTracker::watch(progress);
        if request.kind != ArtifactKind::SlotBundle {
            return Err(tracker.failure(DriverError::Incompatible(
                "make the slot bundle from the image first".into(),
            )));
        }
        let install = SlotInstall {
            bundle: request.artifact.clone(),
            pin: request.options.pin,
            boot: request.options.boot_after,
        };
        self.0
            .install(device, &install, &sink)
            .await
            .map_err(|error| tracker.failure(error))?;
        Ok(UpdateOutcome::Installed {
            version: request.version.clone(),
        })
    }
}

/// A/B update (running board) or fresh flash (device in USB boot): the
/// driver's update capability, as the job runner runs it.
struct UpdateMethod {
    id: InstallMethodId,
    update: Arc<dyn UpdateCapability>,
}

impl UpdateMethod {
    fn release(device: &Identity, request: &InstallRequest) -> ReleaseRef {
        ReleaseRef {
            family: device.key.family.clone(),
            version: request.version.clone(),
            artifact: Some(request.artifact.clone()),
        }
    }
}

#[async_trait]
impl InstallMethod for UpdateMethod {
    fn id(&self) -> InstallMethodId {
        self.id
    }

    fn kinds(&self) -> &'static [ArtifactKind] {
        match self.id {
            // A file not downloaded yet is only a disk image by its name.
            InstallMethodId::AbImage => &[ArtifactKind::AbImage, ArtifactKind::DiskImage],
            _ => &[
                ArtifactKind::AbImage,
                ArtifactKind::DiskImage,
                ArtifactKind::SlotsImage,
                ArtifactKind::Firmware,
            ],
        }
    }

    fn plan(
        &self,
        device: &Identity,
        request: &InstallRequest,
    ) -> Result<InstallPlan, DriverError> {
        let plan = self.update.plan(device, &Self::release(device, request))?;
        Ok(InstallPlan {
            method: self.id,
            restarts: plan.steps.contains(&UpdateStep::Reboot),
            steps: plan.steps,
            erases: self.id == InstallMethodId::Fresh,
            summary: plan.summary,
        })
    }

    async fn run(
        &self,
        device: &Identity,
        request: &InstallRequest,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, InstallFailure> {
        let (tracker, sink) = ChangeTracker::watch(progress);
        self.update
            .run(device, &Self::release(device, request), &sink, cancel)
            .await
            .map_err(|error| tracker.failure(error))
    }
}

/// The methods a device has now, in order of preference.
fn methods(
    identity: &Identity,
    slots: Option<&Arc<dyn SlotsCapability>>,
    update: Option<&Arc<dyn UpdateCapability>>,
) -> Vec<Arc<dyn InstallMethod>> {
    let mut list: Vec<Arc<dyn InstallMethod>> = Vec::new();
    if let Some(slots) = slots {
        list.push(Arc::new(SlotMethod(slots.clone())));
    }
    match (identity.mode, update) {
        (DeviceMode::Normal, Some(update)) => list.push(Arc::new(UpdateMethod {
            id: InstallMethodId::AbImage,
            update: update.clone(),
        })),
        (DeviceMode::Recovery, Some(update)) => list.push(Arc::new(UpdateMethod {
            id: InstallMethodId::Fresh,
            update: update.clone(),
        })),
        _ => {}
    }
    list
}

/// Whether a running board can be put into USB boot for a fresh install
/// (by Atlas or by hand): it lists a method for it.
fn fresh_by_hand_off(identity: &Identity) -> bool {
    identity.mode == DeviceMode::Normal
        && identity
            .attributes
            .get("update_methods")
            .is_some_and(|methods| {
                methods
                    .split(',')
                    .map(str::trim)
                    .any(|m| matches!(m, "usb-boot-reboot" | "image-write" | "rpiboot-recovery"))
            })
}

impl Atlas {
    /// Every method the device could take `request`'s artifact by, the
    /// ones that can run now first.
    pub fn install_choices(
        &self,
        key: &atlas_driver::DeviceKey,
        request: &InstallRequest,
    ) -> Result<InstallChoices, CoreError> {
        let (live, record) = self.live_device(key)?;
        let identity = &record.identity;
        let caps = &live.capabilities;
        let mut choices: Vec<InstallChoice> =
            methods(identity, caps.slots.as_ref(), caps.update.as_ref())
                .iter()
                .map(|method| {
                    let id = method.id();
                    let label = id.label().to_string();
                    if !method.kinds().contains(&request.kind) {
                        return InstallChoice {
                            method: id,
                            label,
                            plan: None,
                            why_not: Some(not_this_kind(id, request.kind)),
                        };
                    }
                    match method.plan(identity, request) {
                        Ok(plan) => InstallChoice {
                            method: id,
                            label,
                            plan: Some(plan),
                            why_not: None,
                        },
                        Err(error) => InstallChoice {
                            method: id,
                            label,
                            plan: None,
                            why_not: Some(error.to_string()),
                        },
                    }
                })
                .collect();
        if fresh_by_hand_off(identity)
            && !choices.iter().any(|c| c.method == InstallMethodId::Fresh)
        {
            let fits = matches!(
                request.kind,
                ArtifactKind::AbImage | ArtifactKind::DiskImage | ArtifactKind::SlotsImage
            );
            choices.push(InstallChoice {
                method: InstallMethodId::Fresh,
                label: InstallMethodId::Fresh.label().to_string(),
                plan: fits.then(|| InstallPlan {
                    method: InstallMethodId::Fresh,
                    steps: vec![UpdateStep::Preflight, UpdateStep::Transfer, UpdateStep::Apply, UpdateStep::Reboot],
                    erases: true,
                    restarts: true,
                    summary: format!("Restarts into USB boot and writes {} over the whole storage; everything on it is erased", request.version),
                }),
                why_not: (!fits).then(|| format!("a fresh install writes a disk image; this is a {}", request.kind.label())),
            });
        }
        // The ones that can run first, each group in the device's order.
        choices.sort_by_key(|c| c.plan.is_none());
        Ok(InstallChoices {
            kind: request.kind,
            kind_label: request.kind.label().to_string(),
            choices,
        })
    }

    /// Runs one method. A fresh install of a running board goes through
    /// the Software tab's hand-off (USB boot), not here.
    pub async fn install_by(
        &self,
        key: &atlas_driver::DeviceKey,
        method: InstallMethodId,
        request: &InstallRequest,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, InstallFailure> {
        let unchanged = |error: DriverError| InstallFailure {
            error,
            changed_nothing: true,
        };
        let (live, record) = self
            .live_device(key)
            .map_err(|error| unchanged(DriverError::Unreachable(error.to_string())))?;
        let caps = &live.capabilities;
        let chosen = methods(&record.identity, caps.slots.as_ref(), caps.update.as_ref())
            .into_iter()
            .find(|m| m.id() == method)
            .ok_or_else(|| {
                unchanged(DriverError::Unsupported(format!(
                    "{} can't install by {}",
                    record.identity.key,
                    method.label().to_lowercase()
                )))
            })?;
        if !chosen.kinds().contains(&request.kind) {
            return Err(unchanged(DriverError::Incompatible(not_this_kind(
                method,
                request.kind,
            ))));
        }
        chosen.plan(&record.identity, request).map_err(unchanged)?;
        chosen
            .run(&record.identity, request, progress, cancel)
            .await
    }
}

fn not_this_kind(method: InstallMethodId, kind: ArtifactKind) -> String {
    if method == InstallMethodId::AbImage
        && matches!(kind, ArtifactKind::SlotBundle | ArtifactKind::SlotsImage)
    {
        return "this board isn't on slots yet: it needs one fresh install of a slots image (its .img.xz), then slots install in place".into();
    }
    let takes = match method {
        InstallMethodId::Slot => "a slot bundle or a slots image",
        InstallMethodId::AbImage => "an A/B image",
        InstallMethodId::Fresh => "a disk image",
        InstallMethodId::Layer => "a layer",
        InstallMethodId::Package => "a package",
    };
    format!(
        "{} takes {takes}; this is a {}",
        method.label(),
        kind.label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_a_method_doesnt_take_says_what_it_takes() {
        assert_eq!(
            not_this_kind(InstallMethodId::Fresh, ArtifactKind::SlotBundle),
            "Fresh install takes a disk image; this is a slot bundle"
        );
        assert!(
            not_this_kind(InstallMethodId::AbImage, ArtifactKind::SlotsImage)
                .contains("needs one fresh install of a slots image")
        );
    }
}
