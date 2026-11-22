//! The simulated devices (feature `sim`): what the app shows with no hardware.
//!
//! [`World::install`] fills in the app's parts with simulated drivers and links only. Nothing on
//! the computer is looked at: no USB, no disks, no network. The `sim.*` calls change the
//! simulated devices the way the world changes real ones (a board is unplugged, comes back), so
//! the click tests can show every state. They exist only in a build with this feature.

use std::sync::Arc;

use atlas_mock::{MockFleet, SIM_LINK};
use atlas_types::{DeviceKey, Link, LinkId, LinkKind};
use serde::Deserialize;
use serde_json::Value;

use crate::call::{CallError, CallResult, args};
use crate::core::{Core, Parts};

/// The simulated devices of one app.
#[derive(Clone, Debug)]
pub struct World {
    fleet: MockFleet,
}

impl World {
    /// Adds the simulated drivers and links to the app's parts.
    pub fn install(parts: &mut Parts) -> Self {
        let fleet = MockFleet::one_in_every_state();
        parts.registry.register(Arc::new(fleet.driver()));
        parts.links.push(Link {
            id: LinkId::new(SIM_LINK),
            kind: LinkKind::Simulated,
            label: "Simulated devices".into(),
        });
        Self { fleet }
    }

    #[must_use]
    pub fn fleet(&self) -> &MockFleet {
        &self.fleet
    }
}

/// The `sim.*` calls.
pub async fn call(core: &Core, name: &str, arguments: Value) -> CallResult {
    let world = core
        .sim()
        .ok_or_else(|| CallError::from("the app is not running the simulated devices"))?;
    match name {
        // Takes a simulated device off its link, or puts it back.
        "sim.set_online" => {
            #[derive(Deserialize)]
            struct SetOnline {
                id: String,
                online: bool,
            }
            let SetOnline { id, online } = args(name, arguments)?;
            let key = DeviceKey::parse(&id)
                .ok_or_else(|| CallError(format!("`{id}` is not a device key")))?;
            if world.fleet().set_online(&key, online) {
                Ok(Value::Null)
            } else {
                Err(CallError(format!("no simulated device {id}")))
            }
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}
