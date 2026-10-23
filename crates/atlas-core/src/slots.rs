//! A slots board's installed OS images (dynamic slots): list them, boot one
//! once, make one the default, remove or pin one, install a bundle, push a
//! layer into the running one. Each change is logged.

use atlas_driver::{
    DeviceKey, LayerPush, LayerPushed, ProgressSink, SlotCommand, SlotInstall, SlotSet,
};

use crate::{ActivityEntry, ActivityKind, ActivityLevel, Atlas, CoreError};

impl Atlas {
    /// The board's slots; `None` when it has none (an A/B board, or one
    /// that hasn't said yet).
    pub async fn slots(&self, key: &DeviceKey) -> Result<Option<SlotSet>, CoreError> {
        let (live, record) = self.live_device(key)?;
        let Some(slots) = live.capabilities.slots else {
            return Ok(None);
        };
        Ok(slots.slots(&record.identity).await?)
    }

    pub async fn slot_command(
        &self,
        key: &DeviceKey,
        command: &SlotCommand,
    ) -> Result<(), CoreError> {
        let (live, record) = self.live_device(key)?;
        let slots = live.capabilities.slots.ok_or(CoreError::Unsupported {
            device: key.clone(),
            what: "slots",
        })?;
        let result = slots.command(&record.identity, command).await;
        self.log_slot_change(&record, &command.describe(), &result);
        Ok(result?)
    }

    /// Installs a slot bundle, reporting the transfer through `progress`.
    pub async fn install_slot(
        &self,
        key: &DeviceKey,
        install: &SlotInstall,
        progress: &ProgressSink,
    ) -> Result<(), CoreError> {
        let (live, record) = self.live_device(key)?;
        let slots = live.capabilities.slots.ok_or(CoreError::Unsupported {
            device: key.clone(),
            what: "slots",
        })?;
        let result = slots.install(&record.identity, install, progress).await;
        let what = if install.boot {
            format!("install {} and boot it once", install.bundle.name)
        } else {
            format!("install {}", install.bundle.name)
        };
        self.log_slot_change(&record, &what, &result);
        Ok(result?)
    }

    /// Replaces one layer of the board's running layered slot (an app
    /// layer, in seconds); the board's answer says what took effect now.
    pub async fn push_layer(
        &self,
        key: &DeviceKey,
        push: &LayerPush,
        progress: &ProgressSink,
    ) -> Result<LayerPushed, CoreError> {
        let (live, record) = self.live_device(key)?;
        let slots = live.capabilities.slots.ok_or(CoreError::Unsupported {
            device: key.clone(),
            what: "slots",
        })?;
        let result = slots.push_layer(&record.identity, push, progress).await;
        let what = if push.revert {
            format!("put back the previous {} layer", push.layer)
        } else {
            format!("push a new {} layer", push.layer)
        };
        self.log_slot_change(
            &record,
            &what,
            &result.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        Ok(result?)
    }

    fn log_slot_change(
        &self,
        record: &crate::DeviceRecord,
        what: &str,
        result: &Result<(), atlas_driver::DriverError>,
    ) {
        let name = record.display_name();
        self.inner.record_activity(vec![match result {
            Ok(()) => ActivityEntry::about(
                record,
                ActivityKind::ActionRun,
                ActivityLevel::Info,
                format!("Slots: {what} on {name}"),
            ),
            Err(error) => ActivityEntry::about(
                record,
                ActivityKind::ActionRun,
                ActivityLevel::Error,
                format!("Slots: {what} on {name} failed: {error}"),
            ),
        }]);
        self.inner.persist();
    }
}
