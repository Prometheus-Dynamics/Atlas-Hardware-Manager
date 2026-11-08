//! Boards with dynamic slots (Raze: docs/slots.md): any number of installed
//! OS images ("slots") on the board's /data, one the default, any one
//! bootable once on trial. What is installed and the free space, and the
//! commands that change it, whichever way the board is reached (Orion's
//! `slot.*` actions or the slots writer over SSH).

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{Artifact, DriverError, Identity, ProgressSink};

/// One installed slot, as the board's slots writer lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// `<family>-<version>-<variant>-<sha8>`.
    pub id: String,
    #[serde(default, deserialize_with = "text")]
    pub family: String,
    #[serde(default, deserialize_with = "text")]
    pub version: String,
    #[serde(default, deserialize_with = "text")]
    pub variant: String,
    #[serde(default, deserialize_with = "text")]
    pub kernel: String,
    /// The image's bytes on /data.
    #[serde(default)]
    pub size: u64,
    /// The image is opened through dm-verity (it has a root hash).
    #[serde(default)]
    pub verity: bool,
    /// The image is the size it was built at (a cut-short one won't start).
    #[serde(default = "yes")]
    pub intact: bool,
    /// Any of `running`, `default`, `previous`, `trial`, `staged`.
    #[serde(default)]
    pub flags: Vec<String>,
    /// Kept from removal.
    #[serde(default)]
    pub pinned: bool,
    /// `yes` once it booted and confirmed, `no` when its trial failed,
    /// empty when it never ran.
    #[serde(default, deserialize_with = "text")]
    pub good: String,
    #[serde(default, deserialize_with = "text")]
    pub installed: String,
    #[serde(default, deserialize_with = "text")]
    pub last_boot: String,
    /// The name the UI shows for the OS (the build recipe's display name);
    /// `None` when the slot was built without one, or by a board whose
    /// writer doesn't report it.
    #[serde(default)]
    pub display_name: Option<String>,
    /// A layered slot's layers, lowest first; `None` for a one-image slot.
    #[serde(default)]
    pub layers: Option<Vec<SlotLayer>>,
    /// How many times the slot has failed (a history: it stays after a
    /// confirm).
    #[serde(default)]
    pub failures: u32,
    /// Failed since its last confirm: `boot-once`, `set-default` and
    /// `rollback` refuse it unless forced, and housekeeping keeps no failed
    /// slot as a fallback.
    #[serde(default)]
    pub failed: bool,
    /// The last failure (its reason, the boot it happened in, when).
    #[serde(default)]
    pub last_failure: Option<SlotNote>,
    /// The last confirm (the boot, when).
    #[serde(default)]
    pub last_confirm: Option<SlotNote>,
}

/// A slot's failure or confirm, as the writer records it. The reason is
/// empty for a confirm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotNote {
    #[serde(default, deserialize_with = "text")]
    pub reason: String,
    #[serde(default, deserialize_with = "text")]
    pub boot: String,
    #[serde(default, deserialize_with = "text")]
    pub time: String,
}

/// One layer of a layered slot, as the writer lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotLayer {
    /// `base`, `platform`, `app`.
    pub name: String,
    /// Its content id (sha256).
    pub id: String,
    #[serde(default)]
    pub size: u64,
}

impl Slot {
    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

/// A slots board's installed slots and its room for more.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotSet {
    #[serde(default, rename = "slot_active", deserialize_with = "text")]
    pub running: String,
    #[serde(default, rename = "slot_default", deserialize_with = "text")]
    pub default: String,
    #[serde(default, rename = "slot_previous", deserialize_with = "text")]
    pub previous: String,
    /// Set for the boot after a boot-once until it confirms or fails.
    #[serde(default, rename = "slot_trial", deserialize_with = "text")]
    pub trial: String,
    /// Free bytes on /data, and the share the writer keeps free.
    #[serde(default)]
    pub free_bytes: u64,
    #[serde(default)]
    pub reserve_bytes: u64,
    /// How many slots housekeeping keeps when an update is confirmed (pinned
    /// ones don't count; 0: never removes any). `None` from a writer without
    /// housekeeping.
    #[serde(default)]
    pub keep: Option<u32>,
    #[serde(default)]
    pub slots: Vec<Slot>,
}

impl SlotSet {
    /// The bytes a new slot may take.
    pub fn room(&self) -> u64 {
        self.free_bytes.saturating_sub(self.reserve_bytes)
    }
}

/// A change to a board's slots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum SlotCommand {
    /// Restart into the slot once, on trial: it becomes the default only
    /// when it comes up healthy (and is confirmed), else the board goes back.
    BootOnce {
        id: String,
        /// Try a slot that failed since its last confirm anyway (a person's
        /// deliberate choice; the writer refuses it otherwise).
        #[serde(default, skip_serializing_if = "is_false")]
        force: bool,
    },
    /// Make a slot that booted well the default.
    SetDefault {
        id: String,
        /// Make a slot that failed since its last confirm the default anyway.
        #[serde(default, skip_serializing_if = "is_false")]
        force: bool,
    },
    Remove {
        id: String,
    },
    Pin {
        id: String,
        pinned: bool,
    },
    /// How many slots housekeeping keeps (0: never remove any).
    Keep {
        keep: u32,
    },
}

impl SlotCommand {
    pub fn id(&self) -> &str {
        match self {
            Self::BootOnce { id, .. }
            | Self::SetDefault { id, .. }
            | Self::Remove { id }
            | Self::Pin { id, .. } => id,
            Self::Keep { .. } => "",
        }
    }

    /// Whether the command overrides a slot's failure (`--force`).
    pub fn forced(&self) -> bool {
        matches!(
            self,
            Self::BootOnce { force: true, .. } | Self::SetDefault { force: true, .. }
        )
    }

    /// The writer's command (`update <this> <id>`).
    pub fn writer_command(&self) -> &'static str {
        match self {
            Self::BootOnce { .. } => "boot-once",
            Self::SetDefault { .. } => "set-default",
            Self::Remove { .. } => "remove",
            Self::Pin { pinned: true, .. } => "pin",
            Self::Pin { pinned: false, .. } => "unpin",
            Self::Keep { .. } => "keep",
        }
    }

    /// The writer's flag for a forced command (`--force`), else nothing.
    pub fn writer_flags(&self) -> &'static str {
        if self.forced() { " --force" } else { "" }
    }

    /// The writer's arguments after its command.
    pub fn writer_argument(&self) -> String {
        match self {
            Self::Keep { keep } => keep.to_string(),
            other => other.id().to_string(),
        }
    }

    /// For the activity log.
    pub fn describe(&self) -> String {
        let id = self.id();
        match self {
            Self::BootOnce { .. } => format!("boot {id} once"),
            Self::SetDefault { .. } => format!("make {id} the default"),
            Self::Remove { .. } => format!("remove {id}"),
            Self::Pin { pinned: true, .. } => format!("pin {id}"),
            Self::Pin { pinned: false, .. } => format!("unpin {id}"),
            Self::Keep { keep: 0 } => "keep every slot".to_string(),
            Self::Keep { keep } => format!("keep the last {keep} slots"),
        }
    }
}

/// A slot bundle (`<id>.slot.tar`) to install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotInstall {
    pub bundle: Artifact,
    /// Keep it from removal.
    #[serde(default)]
    pub pin: bool,
    /// Boot it once when it is in (else install only).
    #[serde(default)]
    pub boot: bool,
    /// With `boot`: a healthy boot becomes the default. Without it the boot
    /// is a trial only: the default stays and the next restart goes back.
    #[serde(default)]
    pub make_default: bool,
    /// This update removes no old slots when it is confirmed (the board's
    /// housekeeping skips it once).
    #[serde(default)]
    pub keep_old: bool,
}

/// How a layer pushed to a running slot takes effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PushMode {
    /// Laid over the running system now, its running units restarted (and
    /// kept for the next boot).
    #[default]
    Live,
    /// Kept, then the board restarts into it.
    Reboot,
    /// Kept for the next boot only.
    Keep,
}

/// A layer to push into the board's running (layered) slot: an archive with
/// `layer.json` and `<id>.erofs` (empty `archive` with `revert`: put back the
/// layer the last push replaced).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerPush {
    /// The layer's name in the slot (`app`).
    pub layer: String,
    pub archive: std::path::PathBuf,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub mode: PushMode,
    #[serde(default)]
    pub revert: bool,
}

/// What the board did with a pushed layer (the writer's answer).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerPushed {
    pub slot: String,
    pub layer: String,
    pub id: String,
    /// The layer it replaced (empty when nothing changed).
    #[serde(default)]
    pub previous: String,
    /// False when the slot already had this layer.
    #[serde(default)]
    pub changed: bool,
    /// Dirs it was laid over now (`/usr`).
    #[serde(default)]
    pub live: Vec<String>,
    /// Dirs whose files wait for the next boot (`/etc`).
    #[serde(default)]
    pub next_boot: Vec<String>,
    /// Units restarted.
    #[serde(default)]
    pub restarted: Vec<String>,
}

#[async_trait]
pub trait SlotsCapability: Send + Sync {
    /// Whether this source comes before the board's own driver (Orion), the
    /// driver's standing in when it can't reach the board.
    fn preferred(&self) -> bool {
        false
    }

    /// The installed slots; `None` when the board has no slots (A/B).
    async fn slots(&self, device: &Identity) -> Result<Option<SlotSet>, DriverError>;

    async fn command(&self, device: &Identity, command: &SlotCommand) -> Result<(), DriverError>;

    /// Installs a bundle as a new slot; the board checks it against its
    /// manifest before anything replaces anything.
    async fn install(
        &self,
        device: &Identity,
        install: &SlotInstall,
        progress: &ProgressSink,
    ) -> Result<(), DriverError>;

    /// Replaces one layer of the running layered slot (the app layer, in
    /// seconds, without a slot install).
    async fn push_layer(
        &self,
        _device: &Identity,
        _push: &LayerPush,
        _progress: &ProgressSink,
    ) -> Result<LayerPushed, DriverError> {
        Err(DriverError::Unsupported(
            "this way to the board can't push a layer (its SSH or USB link can)".into(),
        ))
    }
}

const fn yes() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A refusal from the board's slots writer as a reason: without the
/// writer's and the action's prefixes (`slots: `, `slot.remove: `).
pub fn refusal(text: &str) -> String {
    let mut text = text.trim();
    loop {
        let stripped = ["slots: ", "update: "]
            .iter()
            .find_map(|p| text.strip_prefix(p))
            .or_else(|| {
                text.strip_prefix("slot.")
                    .and_then(|rest| rest.split_once(": ").map(|(_, r)| r))
            });
        match stripped {
            Some(rest) => text = rest.trim(),
            None => return text.to_owned(),
        }
    }
}

fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_writer_status() {
        let set: SlotSet = serde_json::from_str(
            r#"{"state":"idle","layout":"slots","slot_active":"pv-a","slot_staged":null,"slot_default":"pv-a","slot_previous":null,"slot_trial":null,"free_bytes":9000,"reserve_bytes":1000,
            "slots":[{"id":"pv-a","family":"photonvision","version":"2026.3.0","variant":"styx","kernel":"7.2.9","size":400,"verity":true,"flags":["running","default"],"pinned":false,"good":"yes","installed":"2026-10-15","last_boot":null,"display_name":null},
                     {"id":"pv-b","family":"photonvision","version":"2026.4.0","variant":"styx","kernel":"7.2.9","size":410,"verity":true,"flags":[],"pinned":true,"good":null,"installed":null,"last_boot":null,"display_name":"PhotonVision Lab"}]}"#,
        )
        .unwrap();
        assert_eq!(set.running, "pv-a");
        assert_eq!(set.previous, "");
        assert_eq!(set.room(), 8000);
        assert_eq!(set.keep, None);
        assert_eq!(set.slots[0].good, "yes");
        assert!(set.slots[0].has("default"));
        assert_eq!(set.slots[0].display_name, None);
        assert_eq!(
            set.slots[1].display_name.as_deref(),
            Some("PhotonVision Lab")
        );
    }

    #[test]
    fn refusals_lose_their_prefixes() {
        assert_eq!(
            refusal("slot.remove: slots: pv-a is the running slot"),
            "pv-a is the running slot"
        );
        assert_eq!(refusal("slots: no slot pv-x"), "no slot pv-x");
        assert_eq!(refusal("pv-a is pinned"), "pv-a is pinned");
    }

    #[test]
    fn commands_name_the_writer_command() {
        let pin = SlotCommand::Pin {
            id: "pv-b".into(),
            pinned: false,
        };
        assert_eq!(pin.writer_command(), "unpin");
        assert_eq!(pin.describe(), "unpin pv-b");
        let json = serde_json::to_string(&SlotCommand::BootOnce {
            id: "pv-b".into(),
            force: false,
        })
        .unwrap();
        assert_eq!(json, r#"{"command":"boot-once","id":"pv-b"}"#);
        let keep: SlotCommand = serde_json::from_str(r#"{"command":"keep","keep":2}"#).unwrap();
        assert_eq!(
            (keep.writer_command(), keep.writer_argument().as_str()),
            ("keep", "2")
        );
        assert_eq!(keep.describe(), "keep the last 2 slots");
        assert_eq!(pin.writer_argument(), "pv-b");
    }

    #[test]
    fn reads_the_failure_memory_and_forced_commands() {
        let set: SlotSet = serde_json::from_str(
            r#"{"layout":"slots","slot_active":"pv-a","slot_default":"pv-a",
            "slots":[{"id":"pv-a","failures":0,"failed":false,"last_failure":null,
                      "last_confirm":{"boot":"b1","time":"2026-11-08T01:00:00Z"}},
                     {"id":"pv-b","failures":2,"failed":true,
                      "last_failure":{"reason":"slot pv-b missed its confirm deadline","boot":"b2","time":"2026-11-08T02:00:00Z"},
                      "last_confirm":null},
                     {"id":"pv-c"}]}"#,
        )
        .unwrap();
        assert!(!set.slots[0].failed);
        assert_eq!(set.slots[0].last_confirm.as_ref().unwrap().boot, "b1");
        assert!(set.slots[0].last_failure.is_none());
        assert!(set.slots[1].failed);
        assert_eq!(set.slots[1].failures, 2);
        let failure = set.slots[1].last_failure.as_ref().unwrap();
        assert_eq!(failure.reason, "slot pv-b missed its confirm deadline");
        assert_eq!(failure.boot, "b2");
        // A writer that doesn't report the fields (an older one): no failure.
        assert!(!set.slots[2].failed && set.slots[2].failures == 0);
        assert!(set.slots[2].last_confirm.is_none());

        let forced = SlotCommand::BootOnce {
            id: "pv-b".into(),
            force: true,
        };
        assert!(forced.forced());
        assert_eq!(forced.writer_flags(), " --force");
        let json = serde_json::to_string(&forced).unwrap();
        assert_eq!(json, r#"{"command":"boot-once","id":"pv-b","force":true}"#);
        let plain: SlotCommand =
            serde_json::from_str(r#"{"command":"set-default","id":"pv-b"}"#).unwrap();
        assert!(!plain.forced() && plain.writer_flags().is_empty());
    }
}
