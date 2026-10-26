//! A slots board over SSH (or its USB console): the slots writer's status
//! (`update status`, `layout: slots`), its commands (`update boot-once |
//! set-default | remove | pin | unpin <id>`), and installing a bundle: on a
//! board plugged in here the board pulls it from this computer's image
//! server (`update stage-url`, run on its USB console: no SSH key), else it
//! is streamed into `update stage -` over SSH. Either way the writer checks
//! it against its manifest before anything is replaced. Boards list `slots-tryboot` in
//! `update_methods` (lib.sh). Orion's `slot.*` actions come first when the
//! board's agent claims them.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use atlas_driver::{
    BundleHost, CancellationToken, DriverError, Identity, LayerPush, LayerPushed, ProgressSink,
    PushMode, SlotCommand, SlotInstall, SlotSet, SlotsCapability, UpdateStep,
};

use crate::ssh::{SshUpdate, WRITER, as_atlas};
use crate::ssh_actions::writer;

/// The update method a slots board lists.
pub const SLOTS_METHOD: &str = "slots-tryboot";

/// Where boards pull bundles from (the app's image server).
static BUNDLES: RwLock<Option<Arc<dyn BundleHost>>> = RwLock::new(None);

/// Lets boards plugged in here pull slot bundles from `host` (the app's
/// image server) instead of needing an SSH key for the copy.
pub fn set_bundle_host(host: Arc<dyn BundleHost>) {
    *BUNDLES
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(host);
}

fn bundle_host() -> Option<Arc<dyn BundleHost>> {
    BUNDLES
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Why a pull didn't install the slot.
enum Pull {
    /// Nothing changed on the board: the copy over SSH may still work.
    NotStarted(String),
    Failed(DriverError),
}

/// Where `update push` leaves its answer when its stdin is the layer.
const PUSH_ANSWER: &str = "/run/board/layer-push.json";

/// The writer's flags for a push mode.
fn mode_flag(mode: PushMode) -> &'static str {
    match mode {
        PushMode::Live => "",
        PushMode::Reboot => " --reboot",
        PushMode::Keep => " --no-restart",
    }
}

/// The writer's answer to a push (its last JSON line).
fn parse_pushed(text: &str) -> Result<LayerPushed, DriverError> {
    let line = text
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .ok_or_else(|| {
            DriverError::Other("the board didn't say what it did with the layer".into())
        })?;
    serde_json::from_str(line)
        .map_err(|error| DriverError::Other(format!("unreadable push answer: {error}")))
}

pub(crate) struct BoardSlots {
    pub(crate) ssh: SshUpdate,
}

impl BoardSlots {
    /// The board pulls the bundle from this computer's image server, its
    /// writer run on the USB console: no SSH key, and the bulk data goes
    /// over the USB network rather than the console.
    async fn pull(
        &self,
        install: &SlotInstall,
        host: &dyn BundleHost,
        progress: &ProgressSink,
    ) -> Result<(), Pull> {
        let bundle = &install.bundle;
        let peer = self.ssh.host.parse().ok();
        let url = host
            .url_for(bundle, peer)
            .map_err(|error| Pull::NotStarted(error.to_string()))?;
        if url.contains('\'') {
            return Err(Pull::NotStarted(format!(
                "{url} can't be passed to the board"
            )));
        }
        progress.log(format!(
            "the board is fetching {} from this computer",
            bundle.name
        ));
        let script = as_atlas(&format!(
            "{WRITER} stage-url '{url}' --sha256 {} --size {}{}",
            bundle.sha256.to_ascii_lowercase(),
            bundle.size_bytes,
            if install.pin { " --pin" } else { "" }
        ));
        let log = progress.clone();
        let on_line: crate::console::SharedOnLine = Arc::new(move |line: &str| {
            let line = line.trim();
            if !line.is_empty() {
                log.log(line.to_string());
            }
        });
        match self.ssh.on_console(&script, Some(on_line)).await {
            None => Err(Pull::NotStarted(
                "the board's USB console didn't answer".into(),
            )),
            Some(Ok(_)) => Ok(()),
            Some(Err(DriverError::Other(reason))) => {
                let reason = reason.trim_start_matches("slots: ").to_string();
                if atlas_driver::download_failed(&reason) {
                    Err(Pull::NotStarted(format!(
                        "the board couldn't download it from this computer ({reason})"
                    )))
                } else {
                    Err(Pull::Failed(DriverError::StepFailed {
                        step: "Installing the slot".into(),
                        message: reason,
                    }))
                }
            }
            Some(Err(error)) => Err(Pull::Failed(error)),
        }
    }

    /// Streams the bundle into `update stage -` over SSH: the writer reads
    /// it as it arrives and installs nothing until every file matches the
    /// manifest.
    async fn stream(
        &self,
        install: &SlotInstall,
        progress: &ProgressSink,
    ) -> Result<(), DriverError> {
        let bundle = &install.bundle;
        let script = as_atlas(&format!(
            "{WRITER} stage - --sha256 {}{}",
            bundle.sha256.to_ascii_lowercase(),
            if install.pin { " --pin" } else { "" }
        ));
        self.ssh
            .upload_to(
                &bundle.path,
                bundle.size_bytes,
                &script,
                "true",
                progress,
                &CancellationToken::new(),
            )
            .await
            .map_err(|error| match error {
                DriverError::Other(reason) => DriverError::StepFailed {
                    step: "Installing the slot".into(),
                    message: reason.trim_start_matches("slots: ").to_string(),
                },
                other => other,
            })
    }
}

/// Slot ids go into the board's command line: the writer's charset only.
fn checked(id: &str) -> Result<&str, DriverError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        && !id.starts_with(['.', '-']);
    if ok {
        Ok(id)
    } else {
        Err(DriverError::Incompatible(format!("`{id}` isn't a slot id")))
    }
}

/// The writer's status, or `None` when it isn't the slots writer.
pub(crate) fn parse_status(text: &str) -> Result<Option<SlotSet>, DriverError> {
    let value: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|error| DriverError::Other(format!("unreadable update status: {error}")))?;
    if value.get("layout").and_then(|v| v.as_str()) != Some("slots") {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| DriverError::Other(format!("unreadable slot list: {error}")))
}

#[async_trait]
impl SlotsCapability for BoardSlots {
    async fn slots(&self, _device: &Identity) -> Result<Option<SlotSet>, DriverError> {
        parse_status(&self.ssh.run(&format!("{WRITER} status")).await?)
    }

    async fn command(&self, _device: &Identity, command: &SlotCommand) -> Result<(), DriverError> {
        let script = format!(
            "{WRITER} {} {}",
            command.writer_command(),
            checked(command.id())?
        );
        if matches!(
            command,
            SlotCommand::BootOnce { .. } | SlotCommand::SetDefault { .. }
        ) {
            self.ssh.expect_new_host_key();
        }
        match (command, writer(&self.ssh, &script).await) {
            // The restart drops the connection.
            (SlotCommand::BootOnce { .. }, Err(DriverError::Unreachable(_))) => Ok(()),
            // The writer's refusal is the reason, not an incompatibility.
            (_, Err(DriverError::Incompatible(reason) | DriverError::Other(reason))) => {
                Err(DriverError::Other(atlas_driver::refusal(&reason)))
            }
            (_, result) => result,
        }
    }

    async fn install(
        &self,
        _device: &Identity,
        install: &SlotInstall,
        progress: &ProgressSink,
    ) -> Result<(), DriverError> {
        let bundle = &install.bundle;
        if bundle.sha256.len() != 64 || !bundle.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DriverError::Other(format!(
                "{} has no usable SHA-256, which the board needs to check its copy",
                bundle.name
            )));
        }
        progress.step_started(UpdateStep::Preflight);
        let set = self
            .slots(_device)
            .await?
            .ok_or_else(|| DriverError::Incompatible("this board has no slots".into()))?;
        if bundle.size_bytes > set.room() {
            return Err(DriverError::Incompatible(format!(
                "the board has {} MiB of room; the slot needs {} MiB (remove one first)",
                set.room() >> 20,
                bundle.size_bytes >> 20
            )));
        }
        progress.step_started(UpdateStep::Transfer);
        let pulled = match bundle_host() {
            Some(host) if self.ssh.has_console() => {
                match self.pull(install, &*host, progress).await {
                    Ok(()) => true,
                    Err(Pull::NotStarted(why)) => {
                        progress.log(format!("{why}; copying it over SSH instead"));
                        false
                    }
                    Err(Pull::Failed(error)) => return Err(error),
                }
            }
            _ => false,
        };
        if !pulled {
            self.stream(install, progress).await?;
        }
        progress.step_progress(UpdateStep::Transfer, 1.0);
        // The slot is in: from here the board has changed.
        progress.step_started(UpdateStep::Apply);
        if install.boot {
            progress.log("installed; restarting into it once".to_string());
            self.ssh.expect_new_host_key();
            match writer(&self.ssh, &format!("{WRITER} apply")).await {
                Ok(()) | Err(DriverError::Unreachable(_)) => {}
                Err(error) => return Err(error),
            }
        } else {
            progress.log("installed".to_string());
        }
        Ok(())
    }

    async fn push_layer(
        &self,
        _device: &Identity,
        push: &LayerPush,
        progress: &ProgressSink,
    ) -> Result<LayerPushed, DriverError> {
        let name = checked(&push.layer)?;
        let flag = mode_flag(push.mode);
        let refused = |error: DriverError| match error {
            DriverError::Other(reason) | DriverError::Incompatible(reason) => {
                DriverError::StepFailed {
                    step: "Pushing the layer".into(),
                    message: atlas_driver::refusal(&reason),
                }
            }
            other => other,
        };
        if push.revert {
            let answer = self
                .ssh
                .run(&as_atlas(&format!("{WRITER} push --revert {name}{flag}")))
                .await
                .map_err(refused)?;
            return parse_pushed(&answer);
        }
        progress.step_started(UpdateStep::Transfer);
        let script = as_atlas(&format!("{WRITER} push -{flag} > {PUSH_ANSWER}"));
        let sent = self
            .ssh
            .upload_to(
                &push.archive,
                push.size_bytes,
                &script,
                "true",
                progress,
                &CancellationToken::new(),
            )
            .await;
        // --reboot: the restart may drop the connection after the answer.
        match (push.mode, sent) {
            (PushMode::Reboot, Err(DriverError::Unreachable(_))) => {
                return Ok(LayerPushed {
                    layer: name.to_owned(),
                    changed: true,
                    ..LayerPushed::default()
                });
            }
            (_, sent) => sent.map_err(refused)?,
        }
        progress.step_progress(UpdateStep::Transfer, 1.0);
        parse_pushed(&self.ssh.run(&format!("cat {PUSH_ANSWER}")).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_slots_writer_has_a_slot_set() {
        assert_eq!(
            parse_status(r#"{"state":"idle","slot_active":"A"}"#).unwrap(),
            None
        );
        let set = parse_status(
            r#"{"state":"idle","layout":"slots","slot_active":"pv-a","slot_default":"pv-a","slot_previous":null,"free_bytes":10,"reserve_bytes":2,"slots":[{"id":"pv-a","flags":["running"]}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(set.running, "pv-a");
        assert_eq!(set.slots[0].id, "pv-a");
    }

    #[test]
    fn the_push_answer_is_its_last_json_line() {
        let pushed = parse_pushed(
            "slots: note\n{\"ok\":true,\"slot\":\"hx-p\",\"layer\":\"app\",\"id\":\"ab\",\"previous\":\"cd\",\"changed\":true,\"mode\":\"live\",\"live\":[\"/usr\"],\"next_boot\":[\"/etc\"],\"restarted\":[\"helios-api.service\"]}\n",
        )
        .unwrap();
        assert_eq!(pushed.slot, "hx-p");
        assert_eq!(pushed.live, ["/usr"]);
        assert_eq!(pushed.restarted, ["helios-api.service"]);
        let same =
            parse_pushed(r#"{"ok":true,"slot":"hx-p","layer":"app","id":"ab","changed":false}"#)
                .unwrap();
        assert!(!same.changed && same.previous.is_empty());
        assert!(parse_pushed("nothing").is_err());
        assert_eq!(mode_flag(PushMode::Keep), " --no-restart");
    }

    #[test]
    fn slot_ids_are_checked_before_the_shell_sees_them() {
        assert!(checked("photonvision-2026.3.0-styx-1a2b3c4d").is_ok());
        for bad in ["", "a b", "a;reboot", "-rf", "../x", "A"] {
            assert!(checked(bad).is_err(), "{bad}");
        }
    }
}
