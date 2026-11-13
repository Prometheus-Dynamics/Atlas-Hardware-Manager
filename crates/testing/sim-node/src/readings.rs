//! The devices a Lemnos node describes, as the bridge publishes them: one Orion resource per
//! device, named `lemnos.<device>` and of the kind `lemnos.device`, with a driver label, a values
//! topic (one field per value, its unit in the field) and a status topic (`state`, `health`,
//! `reason`). That is the shape of Lemnos's `service/orion/src/publish.rs`, which is the same
//! provider API at Orion c0187f4 and at 61e95a4 (checked: no difference in the provider, the
//! records or the topics crates).
//!
//! The devices here are fixtures. The Atlas topics crate knows none of their names, kinds or
//! units: it reads what they describe. Two things a real description does not carry yet are
//! left out on purpose: a human label and a grouping (Lemnos V1 adds them). A value with no unit
//! is simulated too, so a reader can see what "not reported" looks like.
//!
//! `hold_devices(true)` withdraws every device resource, the way the bridge's session is
//! published again when a fact appears or disappears; `hold_devices(false)` lists them again.
//! Their topics stay declared, as they do on a real bridge.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use orion::Orion;
use orion::provider::{Resource, ResourceState};
use orion::topics::{Field, FieldId, FieldType, Publisher, SampleTime};

/// The kind every device resource is written as. A provider's own word, not an Orion one.
pub const DEVICE_KIND: &str = "lemnos.device";
/// The label that names a device's driver.
pub const DRIVER_LABEL: &str = "lemnos.driver";
/// How many samples of each value the values topic keeps.
const HISTORY: u32 = 16;

/// One value a simulated device reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantity {
    /// A rotation in degrees.
    Angle,
    /// A speed in metres per second.
    Speed,
    /// A switch, with no unit.
    Heater,
    /// A level with no unit: a value whose unit the description does not give.
    Level,
    /// A pressure in kilopascals.
    Pressure,
    /// A temperature in degrees Celsius.
    Temperature,
}

impl Quantity {
    const fn name(self) -> &'static str {
        match self {
            Self::Angle => "angle",
            Self::Speed => "speed",
            Self::Heater => "heater",
            Self::Level => "level",
            Self::Pressure => "pressure",
            Self::Temperature => "temperature",
        }
    }

    const fn unit(self) -> &'static str {
        match self {
            Self::Angle => "deg",
            Self::Speed => "m/s",
            Self::Heater | Self::Level => "",
            Self::Pressure => "kPa",
            Self::Temperature => "°C",
        }
    }

    const fn ty(self) -> FieldType {
        match self {
            Self::Heater => FieldType::Bool,
            _ => FieldType::Float,
        }
    }

    /// The value at `tick`. Each one moves, so a reader sees values change.
    fn payload(self, tick: u64) -> Vec<u8> {
        match self {
            Self::Heater => vec![u8::from(tick.is_multiple_of(2))],
            _ => self.number(tick).to_le_bytes().to_vec(),
        }
    }

    fn number(self, tick: u64) -> f64 {
        match self {
            Self::Angle => (tick % 360) as f64,
            Self::Speed => 1.5 + (tick % 10) as f64 * 0.1,
            Self::Level => 0.5,
            Self::Pressure => 101.3 + (tick % 5) as f64 * 0.01,
            Self::Temperature => 21.0 + (tick % 20) as f64 * 0.05,
            Self::Heater => 0.0,
        }
    }
}

/// One simulated device: its id (the part after `lemnos.`), its driver, its health and what it
/// reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub id: &'static str,
    pub driver: &'static str,
    pub state: &'static str,
    pub health: &'static str,
    pub reason: &'static str,
    pub quantities: &'static [Quantity],
}

const VANE: &[Quantity] = &[
    Quantity::Angle,
    Quantity::Speed,
    Quantity::Heater,
    Quantity::Level,
];
const GAUGE: &[Quantity] = &[Quantity::Pressure, Quantity::Temperature];

/// The devices a simulated Lemnos node describes. One is healthy; one says why it is not.
pub const DEVICES: &[Spec] = &[
    Spec {
        id: "vane0",
        driver: "sim-vane",
        state: "running",
        health: "ok",
        reason: "",
        quantities: VANE,
    },
    Spec {
        id: "gauge1",
        driver: "sim-gauge",
        state: "running",
        health: "degraded",
        reason: "sensor drifting",
        quantities: GAUGE,
    },
];

/// The switch the test holds: while it is set the device resources are withdrawn.
#[derive(Clone, Debug, Default)]
pub struct Control {
    hold: Arc<AtomicBool>,
}

impl Control {
    /// Withdraws the devices (`true`) or lists them again (`false`).
    pub fn hold(&self, hold: bool) {
        self.hold.store(hold, Ordering::SeqCst);
    }

    fn held(&self) -> bool {
        self.hold.load(Ordering::SeqCst)
    }
}

/// One listed device: its resource, and the publishers it writes through.
#[derive(Debug)]
struct Listed {
    spec: &'static Spec,
    resource: Resource,
    values: Publisher,
    _status: Publisher,
}

/// The simulated devices of one node. Ticked by the node's own loop.
#[derive(Debug)]
pub struct Devices {
    control: Control,
    listed: Vec<Listed>,
    /// Whether the devices are listed now (not withdrawn by the hold).
    live: bool,
    ticks: u64,
}

impl Devices {
    /// Lists every device on `orion`'s session, unless the control holds them.
    ///
    /// # Errors
    ///
    /// If the comm core refuses a resource or a topic.
    pub fn declare(orion: &Orion, control: Control) -> Result<Self, String> {
        let live = !control.held();
        let listed = if live { list(orion)? } else { Vec::new() };
        Ok(Self {
            control,
            listed,
            live,
            ticks: 0,
        })
    }

    /// Publishes the values, and withdraws or lists the devices as the control says.
    pub fn tick(&mut self, orion: &Orion, now: SampleTime) {
        self.ticks += 1;
        if self.control.held() {
            if self.live {
                for listed in self.listed.drain(..) {
                    let _ = listed.resource.withdraw();
                }
                self.live = false;
            }
            return;
        }
        if !self.live {
            if let Ok(listed) = list(orion) {
                self.listed = listed;
                self.live = true;
            }
            return;
        }
        for listed in &mut self.listed {
            for (index, quantity) in listed.spec.quantities.iter().enumerate() {
                let payload = quantity.payload(self.ticks);
                let _ = listed.values.publish(FieldId(index as u16), now, &payload);
            }
            // LOCK: Orion at c0187f4 does not hand a network subscriber the newest sample of a
            // topic that was written before it subscribed (the local ring does; the network copy
            // gets samples from the subscription on). The bridge writes its status on change, so a
            // session made later reads nothing until the next change. Until Orion delivers the
            // newest sample on subscribe, or Lemnos republishes the status, the node writes it on
            // every tick: a late subscriber reads it within one tick.
            for (index, text) in [listed.spec.state, listed.spec.health, listed.spec.reason]
                .iter()
                .enumerate()
            {
                let _ = listed
                    ._status
                    .publish(FieldId(index as u16), now, text.as_bytes());
            }
        }
    }
}

/// Describes and lists every device, with the status each one starts in.
fn list(orion: &Orion) -> Result<Vec<Listed>, String> {
    let provider = orion.provider();
    let now = SampleTime::from_nanos(1);
    let mut listed = Vec::new();
    for spec in DEVICES {
        let mut described = provider
            .resource(&format!("lemnos.{}", spec.id), DEVICE_KIND)
            .map_err(|error| error.to_string())?
            .label(DRIVER_LABEL, spec.driver);
        let fields: Vec<Field<'_>> = spec
            .quantities
            .iter()
            .map(|quantity| Field {
                name: quantity.name(),
                ty: quantity.ty(),
                unit: quantity.unit(),
                type_name: "",
            })
            .collect();
        let values = described
            .topic("values", &fields, HISTORY, 8)
            .map_err(|error| error.to_string())?;
        let mut status = described
            .topic("status", &STATUS_FIELDS, 1, 256)
            .map_err(|error| error.to_string())?;
        for (index, text) in [spec.state, spec.health, spec.reason].iter().enumerate() {
            status
                .publish(FieldId(index as u16), now, text.as_bytes())
                .map_err(|error| error.to_string())?;
        }
        let resource = described
            .publish(ResourceState::Ready)
            .map_err(|error| error.to_string())?;
        listed.push(Listed {
            spec,
            resource,
            values,
            _status: status,
        });
    }
    Ok(listed)
}

const STATUS_FIELDS: [Field<'static>; 3] = [
    Field {
        name: "state",
        ty: FieldType::Text,
        unit: "",
        type_name: "",
    },
    Field {
        name: "health",
        ty: FieldType::Text,
        unit: "",
        type_name: "",
    },
    Field {
        name: "reason",
        ty: FieldType::Text,
        unit: "",
        type_name: "",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_device_names_its_driver_and_its_values_have_names() {
        for spec in DEVICES {
            assert!(!spec.driver.is_empty());
            assert!(!spec.quantities.is_empty());
        }
    }

    #[test]
    fn the_level_has_no_unit_and_the_angle_is_in_degrees() {
        assert_eq!(Quantity::Level.unit(), "");
        assert_eq!(Quantity::Angle.unit(), "deg");
    }

    #[test]
    fn a_switch_is_one_byte_and_a_number_eight() {
        assert_eq!(Quantity::Heater.payload(2).len(), 1);
        assert_eq!(Quantity::Angle.payload(2).len(), 8);
    }
}
