//! The devices a Lemnos node describes, in Lemnos V1a's shape (`describe`'s blocks after the job
//! list, at Lemnos 63ca7a6b): a label, a bus, a state (running, retrying or off) with its reason,
//! an address, a kind per contract, and per item a reading kind, a unit, a range, a step, warning
//! limits, a group (a three-axis vector), and whether it is settable with its setter.
//!
//! Each device is one Orion resource named `lemnos.<device>` of the kind `lemnos.device`, with a
//! values topic (one field per item, its unit in the field) and a status topic (`health`,
//! `reason`). The Atlas topics crate knows none of these names: it reads what they describe.
//!
//! // LOCK: Lemnos V1a 63ca7a6b. The bridge at that commit publishes only the driver, tag and
//! // control labels (`service/orion/src/publish.rs`); the V1a blocks are in the host's
//! // description. Until the bridge carries them, the node writes them here as labels with the
//! // keys in the Atlas topics crate's `device` module (`lemnos.label`, `lemnos.item.<field>`,
//! // and the rest), one `key=value;...` block per item.
//!
//! Fields a real description may leave unknown are left out here on purpose: the bare device has
//! none of them, the off device has no address, the retrying device has no label.
//!
//! `hold_devices(true)` withdraws every device resource, the way the bridge's session is
//! published again when a fact appears or disappears; `hold_devices(false)` lists them again.
//! Their topics stay declared, as they do on a real bridge.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use orion::Orion;
use orion::provider::{Describing, Resource, ResourceState};
use orion::topics::{Field, FieldId, FieldType, Publisher, SampleTime};

/// The kind every device resource is written as. A provider's own word, not an Orion one.
pub const DEVICE_KIND: &str = "lemnos.device";
/// How many samples of each value the values topic keeps.
const HISTORY: u32 = 16;
/// The largest sample the values topic carries: three floats.
const MAX_PAYLOAD: u32 = 32;

/// The field type an item's value is published as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A float.
    Float,
    /// A flag.
    Bool,
    /// Text.
    Text,
    /// An array of floats.
    FloatArray,
}

/// What an item reads at a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sample {
    /// A float that moves between `base` and `base + span`, once every `period` ticks.
    Float { base: f64, span: f64, period: u64 },
    /// A flag that is on at every other tick.
    Switch,
    /// Text.
    Text(&'static str),
    /// Three floats.
    Floats([f64; 3]),
}

/// One item of a simulated device: its value field, the unit its field declares, its block (the
/// `key=value;...` text the description gives it; empty for none), and what it reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub field: &'static str,
    pub kind: Kind,
    pub unit: &'static str,
    pub block: &'static str,
    pub sample: Sample,
}

/// One simulated device: its labels (the device-level keys), its status (health and reason, or
/// none), and its items.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub id: &'static str,
    pub labels: &'static [(&'static str, &'static str)],
    pub status: Option<(&'static str, &'static str)>,
    pub items: &'static [Item],
}

const VANE: &[Item] = &[
    Item {
        field: "angle",
        kind: Kind::Float,
        unit: "deg",
        block: "reading=number;unit=deg;exponent=0;min=0;max=360",
        sample: Sample::Float {
            base: 0.0,
            span: 360.0,
            period: 360,
        },
    },
    Item {
        field: "speed",
        kind: Kind::Float,
        unit: "m/s",
        block: "reading=range;unit=m/s;exponent=-1;min=0;max=300;step=1;warn_high=250;settable=true;setter=set the wind speed",
        sample: Sample::Float {
            base: 1.5,
            span: 1.0,
            period: 10,
        },
    },
    Item {
        field: "heater",
        kind: Kind::Bool,
        unit: "",
        block: "reading=switch;settable=true;setter=switch the heater",
        sample: Sample::Switch,
    },
    Item {
        field: "level",
        kind: Kind::Float,
        unit: "",
        block: "reading=number;exponent=0",
        sample: Sample::Float {
            base: 0.5,
            span: 0.0,
            period: 1,
        },
    },
];

const GAUGE: &[Item] = &[
    Item {
        field: "pressure",
        kind: Kind::Float,
        unit: "kPa",
        block: "reading=number;unit=kPa;exponent=0;min=90;max=110;warn_high=105",
        sample: Sample::Float {
            base: 101.3,
            span: 0.05,
            period: 5,
        },
    },
    Item {
        field: "temperature",
        kind: Kind::Float,
        unit: "°C",
        block: "reading=number;unit=°C;exponent=0;warn_low=5;warn_high=45",
        sample: Sample::Float {
            base: 21.0,
            span: 1.0,
            period: 20,
        },
    },
];

const UNIT: &[Item] = &[
    Item {
        field: "accel_x",
        kind: Kind::Float,
        unit: "m/s^2",
        block: "reading=three axes;unit=m/s^2;exponent=0;min=-20;max=20;group=accel;axis=x",
        sample: Sample::Float {
            base: 0.0,
            span: 1.0,
            period: 8,
        },
    },
    Item {
        field: "accel_y",
        kind: Kind::Float,
        unit: "m/s^2",
        block: "reading=three axes;unit=m/s^2;exponent=0;min=-20;max=20;group=accel;axis=y",
        sample: Sample::Float {
            base: 0.0,
            span: 1.0,
            period: 6,
        },
    },
    Item {
        field: "accel_z",
        kind: Kind::Float,
        unit: "m/s^2",
        block: "reading=three axes;unit=m/s^2;exponent=0;min=-20;max=20;group=accel;axis=z",
        sample: Sample::Float {
            base: 9.8,
            span: 0.2,
            period: 4,
        },
    },
    Item {
        field: "colour",
        kind: Kind::FloatArray,
        unit: "",
        block: "reading=colour array;exponent=0",
        sample: Sample::Floats([0.2, 0.4, 0.6]),
    },
    Item {
        field: "position",
        kind: Kind::Float,
        unit: "mm",
        block: "reading=range;unit=mm;exponent=0;min=0;max=100;step=5;settable=true;setter=move to a position",
        sample: Sample::Float {
            base: 0.0,
            span: 100.0,
            period: 100,
        },
    },
    Item {
        field: "note",
        kind: Kind::Text,
        unit: "",
        block: "reading=text",
        sample: Sample::Text("ready"),
    },
];

const BARE: &[Item] = &[Item {
    field: "count",
    kind: Kind::Float,
    unit: "",
    block: "",
    sample: Sample::Float {
        base: 1.0,
        span: 0.0,
        period: 1,
    },
}];

const OFF: &[Item] = &[Item {
    field: "count",
    kind: Kind::Float,
    unit: "count",
    block: "reading=number;unit=count;exponent=0;min=0;max=10",
    sample: Sample::Float {
        base: 0.0,
        span: 0.0,
        period: 1,
    },
}];

/// The devices a simulated Lemnos node describes. One is healthy; one is retrying with a reason;
/// one is a three-axis unit with every reading kind; one is off with a reason; one has every
/// optional field unknown.
pub const DEVICES: &[Spec] = &[
    Spec {
        id: "vane0",
        labels: &[
            ("lemnos.label", "Wind vane"),
            ("lemnos.bus", "i2c"),
            ("lemnos.address", "0x44"),
            ("lemnos.state", "running"),
            ("lemnos.kind", "sensor"),
            ("lemnos.tag", "sim.vane/1"),
            ("lemnos.kind.sim.vane/1", "sensor"),
            ("lemnos.driver", "sim-vane"),
        ],
        status: Some(("ok", "")),
        items: VANE,
    },
    Spec {
        id: "gauge1",
        labels: &[
            ("lemnos.bus", "spi"),
            ("lemnos.state", "retrying"),
            ("lemnos.reason", "sensor drifting"),
            ("lemnos.kind", "sensor"),
            ("lemnos.tag", "sim.gauge/1"),
            ("lemnos.kind.sim.gauge/1", "sensor"),
            ("lemnos.driver", "sim-gauge"),
        ],
        status: Some(("degraded", "sensor drifting")),
        items: GAUGE,
    },
    Spec {
        id: "unit3",
        labels: &[
            ("lemnos.label", "Motion unit"),
            ("lemnos.bus", "i2c"),
            ("lemnos.address", "104"),
            ("lemnos.state", "running"),
            ("lemnos.kind", "motion"),
            ("lemnos.tag", "sim.motion/1"),
            ("lemnos.kind.sim.motion/1", "motion"),
            ("lemnos.driver", "sim-motion"),
        ],
        status: Some(("ok", "")),
        items: UNIT,
    },
    Spec {
        id: "off0",
        labels: &[
            ("lemnos.bus", "uart"),
            ("lemnos.state", "off"),
            ("lemnos.reason", "disabled by its board file"),
            ("lemnos.tag", "sim.counter/1"),
            ("lemnos.driver", "sim-counter"),
        ],
        status: Some(("ok", "")),
        items: OFF,
    },
    Spec {
        id: "bare0",
        labels: &[("lemnos.driver", "sim-bare")],
        status: None,
        items: BARE,
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
    _status: Option<Publisher>,
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
        let tick = self.ticks;
        for listed in &mut self.listed {
            for (index, item) in listed.spec.items.iter().enumerate() {
                let payload = payload(item.sample, tick);
                let _ = listed.values.publish(FieldId(index as u16), now, &payload);
            }
            // LOCK: Orion at 6dfb990 does not hand a network subscriber the newest sample of a
            // topic that was written before it subscribed (the local ring does; the network copy
            // gets samples from the subscription on). The bridge writes its status on change, so a
            // session made later reads nothing until the next change. Until Orion delivers the
            // newest sample on subscribe, or Lemnos republishes the status, the node writes it on
            // every tick: a late subscriber reads it within one tick.
            if let (Some(status), Some((health, reason))) =
                (&mut listed._status, listed.spec.status)
            {
                for (index, text) in ["ready", health, reason].iter().enumerate() {
                    let _ = status.publish(FieldId(index as u16), now, text.as_bytes());
                }
            }
        }
    }
}

/// The payload of one sample at `tick`, in its field's type.
fn payload(sample: Sample, tick: u64) -> Vec<u8> {
    match sample {
        Sample::Float { base, span, period } => {
            let step = (tick % period) as f64 / period as f64;
            (base + span * step).to_le_bytes().to_vec()
        }
        Sample::Switch => vec![u8::from(tick.is_multiple_of(2))],
        Sample::Text(text) => text.as_bytes().to_vec(),
        Sample::Floats(values) => values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

fn field_type(kind: Kind) -> FieldType {
    match kind {
        Kind::Float => FieldType::Float,
        Kind::Bool => FieldType::Bool,
        Kind::Text => FieldType::Text,
        Kind::FloatArray => FieldType::FloatArray,
    }
}

/// Describes and lists every device, with the labels and the status each one starts with.
fn list(orion: &Orion) -> Result<Vec<Listed>, String> {
    let provider = orion.provider();
    let now = SampleTime::from_nanos(1);
    let mut listed = Vec::new();
    for spec in DEVICES {
        let mut described: Describing = provider
            .resource(&format!("lemnos.{}", spec.id), DEVICE_KIND)
            .map_err(|error| error.to_string())?;
        for (key, value) in spec.labels {
            described = described.label(key, value);
        }
        for item in spec.items {
            if !item.block.is_empty() {
                described = described.label(&format!("lemnos.item.{}", item.field), item.block);
            }
        }
        let fields: Vec<Field<'_>> = spec
            .items
            .iter()
            .map(|item| Field {
                name: item.field,
                ty: field_type(item.kind),
                unit: item.unit,
                type_name: "",
            })
            .collect();
        let values = described
            .topic("values", &fields, HISTORY, MAX_PAYLOAD)
            .map_err(|error| error.to_string())?;
        let status = match spec.status {
            Some((health, reason)) => {
                let status_fields = [
                    text_field("state"),
                    text_field("health"),
                    text_field("reason"),
                ];
                let mut status = described
                    .topic("status", &status_fields, 1, 256)
                    .map_err(|error| error.to_string())?;
                for (index, text) in ["ready", health, reason].iter().enumerate() {
                    status
                        .publish(FieldId(index as u16), now, text.as_bytes())
                        .map_err(|error| error.to_string())?;
                }
                Some(status)
            }
            None => None,
        };
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

fn text_field(name: &'static str) -> Field<'static> {
    Field {
        name,
        ty: FieldType::Text,
        unit: "",
        type_name: "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_device_has_a_driver_and_every_item_names_its_field() {
        for spec in DEVICES {
            assert!(spec.labels.iter().any(|(key, _)| *key == "lemnos.driver"));
            for item in spec.items {
                assert!(!item.field.is_empty());
            }
        }
    }

    #[test]
    fn a_switch_is_one_byte_a_number_eight_and_three_floats_twenty_four() {
        assert_eq!(payload(Sample::Switch, 2).len(), 1);
        assert_eq!(
            payload(
                Sample::Float {
                    base: 1.0,
                    span: 0.0,
                    period: 1
                },
                2
            )
            .len(),
            8
        );
        assert_eq!(payload(Sample::Floats([0.1, 0.2, 0.3]), 0).len(), 24);
    }
}
