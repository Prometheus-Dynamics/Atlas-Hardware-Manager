//! Capabilities from more than one source for one device.
//!
//! A device has one owning driver: the one that found and identified it
//! (for a running Raze, the board driver over mDNS). A management agent such as
//! Orion knows the same board by its serial and can add telemetry, actions,
//! or updates, but must not become a second device. A [`CapabilitySource`]
//! does that: Atlas asks every source about each device it identifies and
//! fills in what the owning driver left empty. Without the source, the
//! device keeps exactly what its driver gives it.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{ActionsCapability, Capabilities, DeviceAction, DriverError, Identity};

/// Extra capabilities for devices other drivers own.
pub trait CapabilitySource: Send + Sync {
    /// What this source adds for `device`; empty when it doesn't know it.
    fn capabilities_for(&self, device: &Identity) -> Capabilities;
}

impl Capabilities {
    /// Fills the capabilities this device lacks from `extra`. Actions are
    /// combined: the owning driver's come first and win on equal ids.
    pub fn fill_from(&mut self, extra: Capabilities) {
        if self.update.is_none() {
            self.update = extra.update;
        }
        if self.telemetry.is_none() {
            self.telemetry = extra.telemetry;
        }
        if self.logs.is_none() {
            self.logs = extra.logs;
        }
        if self.selftest.is_none() {
            self.selftest = extra.selftest;
        }
        self.actions = match (self.actions.take(), extra.actions) {
            (Some(own), Some(more)) => Some(Arc::new(CombinedActions { own, more })),
            (own, more) => own.or(more),
        };
    }
}

struct CombinedActions {
    own: Arc<dyn ActionsCapability>,
    more: Arc<dyn ActionsCapability>,
}

#[async_trait]
impl ActionsCapability for CombinedActions {
    fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
        let mut list = self.own.actions(device);
        for action in self.more.actions(device) {
            if !list.iter().any(|existing| existing.id == action.id) {
                list.push(action);
            }
        }
        list
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        if self
            .own
            .actions(device)
            .iter()
            .any(|action| action.id == action_id)
        {
            self.own.run_action(device, action_id).await
        } else {
            self.more.run_action(device, action_id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;
    use crate::{DeviceKey, DeviceMode, LinkId};

    struct Named(&'static [&'static str], Mutex<Vec<String>>);

    #[async_trait]
    impl ActionsCapability for Named {
        fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
            self.0
                .iter()
                .map(|id| DeviceAction {
                    id: (*id).into(),
                    label: (*id).into(),
                    destructive: false,
                })
                .collect()
        }

        async fn run_action(&self, _device: &Identity, action_id: &str) -> Result<(), DriverError> {
            self.1.lock().unwrap().push(action_id.into());
            Ok(())
        }
    }

    fn device() -> Identity {
        Identity {
            key: DeviceKey::new("raze", "a1"),
            model: "Raze".into(),
            mode: DeviceMode::Normal,
            versions: BTreeMap::new(),
            name: None,
            link: LinkId("mdns".into()),
            address: "x".into(),
            attributes: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn actions_combine_and_the_owner_wins() {
        let own = Arc::new(Named(&["locate"], Mutex::default()));
        let more = Arc::new(Named(&["locate", "reboot"], Mutex::default()));
        let mut caps = Capabilities {
            actions: Some(own.clone()),
            ..Capabilities::default()
        };
        caps.fill_from(Capabilities {
            actions: Some(more.clone()),
            ..Capabilities::default()
        });
        let combined = caps.actions.unwrap();
        let ids: Vec<String> = combined
            .actions(&device())
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, vec!["locate", "reboot"]);

        combined.run_action(&device(), "locate").await.unwrap();
        combined.run_action(&device(), "reboot").await.unwrap();
        assert_eq!(*own.1.lock().unwrap(), vec!["locate"]);
        assert_eq!(*more.1.lock().unwrap(), vec!["reboot"]);
    }

    #[test]
    fn nothing_from_the_source_changes_nothing() {
        let mut caps = Capabilities::default();
        caps.fill_from(Capabilities::default());
        assert!(caps.kinds().len() == 1);
    }
}
