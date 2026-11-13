//! The devices a Lemnos node describes, as the bridge publishes them at e5c0ff0e (I6h).
//!
//! Each device is one Orion resource named `lemnos.<device>` of the kind `lemnos.device`, with:
//! - a values topic (one field per item it publishes, in the item's type);
//! - a status topic (`health` and `reason`), when the device has one;
//! - a description topic (`lemnos.<device>.description`, one `generation` field): a counter that
//!   moves once per change of the device's description, as Lemnos's generation topic does.
//!
//! The description itself is not labels: `call describe` on the device's `.call` action answers
//! the text of `describe 1` (see [`crate::describe`]), rendered from the device's model here. An
//! off device refuses `describe` ("the device isn't running"), as Lemnos does; a device the node
//! withdraws is "no such device". Its readings are topics, as they are on a real bridge.
//!
//! `hold_devices(true)` withdraws every device resource, the way the bridge's session is
//! published again when a fact appears or disappears; `hold_devices(false)` lists them again.
//! Each listing is a new appearance, so the generation moves. Their topics stay declared, as they
//! do on a real bridge.
//!
//! // LOCK: Orion at 6dfb990 does not hand a network subscriber the newest sample of a topic that
//! // was written before it subscribed (the local ring does). The status and the generation are
//! // written on every tick until Orion delivers the newest sample on subscribe (Orion's
//! // last-value-on-subscribe, asked for in the coordinator's Orion shapes), so a late subscriber
//! // reads them within one tick. Marked for removal then.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use orion::Orion;
use orion::provider::{Describing, Resource, ResourceState};
use orion::topics::{Field, FieldId, FieldType, Publisher, SampleTime};

use crate::describe::{self, ItemKind, Reading, UnitState};

/// The kind every device resource is written as. A provider's own word, not an Orion one.
pub const DEVICE_KIND: &str = "lemnos.device";
/// How many samples of each value the values topic keeps.
const HISTORY: u32 = 16;
/// The largest sample the values topic carries: three floats.
const MAX_PAYLOAD: u32 = 32;
/// The name of a device's description topic (under the device's resource).
pub const DESCRIPTION_TOPIC: &str = "description";
/// The description reply's bound through the bridge: a reply over it is refused by name.
pub const REPLY_MAX: usize = 60 * 1024;

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

/// One item of a simulated device: its description, its value field and what it reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    /// The item's name.
    pub name: &'static str,
    /// The topic field its value is published in, when it is published.
    pub field: Option<&'static str>,
    /// The type of that field.
    pub kind: Kind,
    /// What it reads.
    pub sample: Sample,
    /// Whether it is a value, a control or an action.
    pub item_kind: ItemKind,
    /// Its unit (empty when none).
    pub unit: &'static str,
    /// Its unit state.
    pub unit_state: UnitState,
    /// The power of ten its integer figures are scaled by.
    pub exponent: i8,
    /// The raw least and greatest value.
    pub min: Option<i64>,
    /// See `min`.
    pub max: Option<i64>,
    /// The real least and greatest value in the unit, when the description gives them.
    pub real: Option<(f64, f64)>,
    /// The real step in the unit, when given.
    pub real_step: Option<f64>,
    /// The raw step.
    pub step: Option<i64>,
    /// The raw lower warning limit.
    pub warn_low: Option<i64>,
    /// The raw upper warning limit.
    pub warn_high: Option<i64>,
    /// The method the item is read or set through, in the device's one contract.
    pub method: Option<u32>,
    /// Its reading kind.
    pub reading: Option<Reading>,
    /// The group (a three-axis vector) it is one part of.
    pub group: Option<&'static str>,
}

/// A method of a device's one contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodSpec {
    /// Its id.
    pub id: u32,
    /// Its name.
    pub name: &'static str,
    /// Its parameters: name and shape.
    pub params: &'static [(&'static str, &'static str)],
    /// The shape it returns.
    pub returns: &'static str,
    /// Whether it is a long job.
    pub job: bool,
}

/// One simulated device as the node describes it: the description's fields, and its items.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    /// Its id: the device's resource is `lemnos.<id>`.
    pub id: &'static str,
    /// The board's name for it. Empty when none.
    pub label: &'static str,
    /// Its bus. Empty when none.
    pub bus: &'static str,
    /// Its 7-bit address, when the driver names one.
    pub address: Option<u8>,
    /// Where it is, in words. Empty when none.
    pub location: &'static str,
    /// Its kind. Empty when none.
    pub kind: &'static str,
    /// `0` running, `1` retrying, `2` off; `None` when unknown.
    pub state: Option<u8>,
    /// Why it is retrying or off. Empty when none.
    pub reason: &'static str,
    /// Its one contract: tag, kind and hash. `None` for a device with no contract.
    pub contract: Option<(&'static str, &'static str, u64)>,
    /// Its methods, of that contract.
    pub methods: &'static [MethodSpec],
    /// Its health and reason, on its status topic. `None` when it has no status topic.
    pub status: Option<(&'static str, &'static str)>,
    /// Its items.
    pub items: &'static [Item],
}

const PLAIN: Item = Item {
    name: "",
    field: None,
    kind: Kind::Float,
    sample: Sample::Float {
        base: 0.0,
        span: 0.0,
        period: 1,
    },
    item_kind: ItemKind::Value,
    unit: "",
    unit_state: UnitState::Unknown,
    exponent: 0,
    min: None,
    max: None,
    real: None,
    real_step: None,
    step: None,
    warn_low: None,
    warn_high: None,
    method: None,
    reading: None,
    group: None,
};

const VANE_METHODS: &[MethodSpec] = &[
    MethodSpec {
        id: 1,
        name: "angle",
        params: &[],
        returns: "f64",
        job: false,
    },
    MethodSpec {
        id: 2,
        name: "speed",
        params: &[("value", "f64")],
        returns: "()",
        job: false,
    },
    MethodSpec {
        id: 3,
        name: "heater",
        params: &[("on", "bool")],
        returns: "()",
        job: false,
    },
    MethodSpec {
        id: 4,
        name: "level",
        params: &[],
        returns: "f64",
        job: false,
    },
];

const VANE: &[Item] = &[
    Item {
        name: "angle",
        field: Some("angle"),
        kind: Kind::Float,
        unit: "deg",
        unit_state: UnitState::Known,
        min: Some(0),
        max: Some(360),
        method: Some(1),
        reading: Some(Reading::Number),
        sample: Sample::Float {
            base: 0.0,
            span: 360.0,
            period: 360,
        },
        ..PLAIN
    },
    Item {
        name: "speed",
        field: Some("speed"),
        kind: Kind::Float,
        item_kind: ItemKind::Control,
        unit: "m/s",
        unit_state: UnitState::Known,
        exponent: -1,
        min: Some(0),
        max: Some(300),
        step: Some(1),
        warn_high: Some(250),
        method: Some(2),
        reading: Some(Reading::Range),
        sample: Sample::Float {
            base: 1.5,
            span: 1.0,
            period: 10,
        },
        ..PLAIN
    },
    Item {
        name: "heater",
        field: Some("heater"),
        kind: Kind::Bool,
        item_kind: ItemKind::Control,
        unit_state: UnitState::Dimensionless,
        method: Some(3),
        reading: Some(Reading::Switch),
        sample: Sample::Switch,
        ..PLAIN
    },
    Item {
        name: "level",
        field: Some("level"),
        kind: Kind::Float,
        method: Some(4),
        reading: Some(Reading::Number),
        sample: Sample::Float {
            base: 0.5,
            span: 0.0,
            period: 1,
        },
        ..PLAIN
    },
];

const GAUGE_METHODS: &[MethodSpec] = &[
    MethodSpec {
        id: 1,
        name: "pressure",
        params: &[],
        returns: "f64",
        job: false,
    },
    MethodSpec {
        id: 2,
        name: "temperature",
        params: &[],
        returns: "f64",
        job: false,
    },
];

const GAUGE: &[Item] = &[
    Item {
        name: "pressure",
        field: Some("pressure"),
        kind: Kind::Float,
        unit: "kPa",
        unit_state: UnitState::Known,
        min: Some(90),
        max: Some(110),
        warn_high: Some(105),
        method: Some(1),
        reading: Some(Reading::Number),
        sample: Sample::Float {
            base: 101.3,
            span: 0.05,
            period: 5,
        },
        ..PLAIN
    },
    Item {
        name: "temperature",
        field: Some("temperature"),
        kind: Kind::Float,
        unit: "°C",
        unit_state: UnitState::Known,
        warn_low: Some(5),
        warn_high: Some(45),
        method: Some(2),
        reading: Some(Reading::Number),
        sample: Sample::Float {
            base: 21.0,
            span: 1.0,
            period: 20,
        },
        ..PLAIN
    },
];

const UNIT_METHODS: &[MethodSpec] = &[
    MethodSpec {
        id: 1,
        name: "accel-x",
        params: &[],
        returns: "f64",
        job: false,
    },
    MethodSpec {
        id: 2,
        name: "accel-y",
        params: &[],
        returns: "f64",
        job: false,
    },
    MethodSpec {
        id: 3,
        name: "accel-z",
        params: &[],
        returns: "f64",
        job: false,
    },
    MethodSpec {
        id: 4,
        name: "colour",
        params: &[],
        returns: "[f64]",
        job: false,
    },
    MethodSpec {
        id: 5,
        name: "position",
        params: &[("mm", "f64")],
        returns: "()",
        job: false,
    },
    MethodSpec {
        id: 6,
        name: "note",
        params: &[],
        returns: "str",
        job: false,
    },
];

const UNIT: &[Item] = &[
    Item {
        name: "accel_x",
        field: Some("accel_x"),
        kind: Kind::Float,
        unit: "m/s^2",
        unit_state: UnitState::Known,
        min: Some(-20),
        max: Some(20),
        group: Some("accel"),
        method: Some(1),
        reading: Some(Reading::ThreeAxes),
        sample: Sample::Float {
            base: 0.0,
            span: 1.0,
            period: 8,
        },
        ..PLAIN
    },
    Item {
        name: "accel_y",
        field: Some("accel_y"),
        kind: Kind::Float,
        unit: "m/s^2",
        unit_state: UnitState::Known,
        min: Some(-20),
        max: Some(20),
        group: Some("accel"),
        method: Some(2),
        reading: Some(Reading::ThreeAxes),
        sample: Sample::Float {
            base: 0.0,
            span: 1.0,
            period: 6,
        },
        ..PLAIN
    },
    Item {
        name: "accel_z",
        field: Some("accel_z"),
        kind: Kind::Float,
        unit: "m/s^2",
        unit_state: UnitState::Known,
        min: Some(-20),
        max: Some(20),
        group: Some("accel"),
        method: Some(3),
        reading: Some(Reading::ThreeAxes),
        sample: Sample::Float {
            base: 9.8,
            span: 0.2,
            period: 4,
        },
        ..PLAIN
    },
    Item {
        name: "colour",
        field: Some("colour"),
        kind: Kind::FloatArray,
        unit_state: UnitState::Dimensionless,
        method: Some(4),
        reading: Some(Reading::ColourArray),
        sample: Sample::Floats([0.2, 0.4, 0.6]),
        ..PLAIN
    },
    Item {
        name: "position",
        field: Some("position"),
        kind: Kind::Float,
        item_kind: ItemKind::Control,
        unit: "mm",
        unit_state: UnitState::Known,
        min: Some(0),
        max: Some(100),
        step: Some(5),
        // The real range wins over the integer one: 0.5 to 99.5 in millimetres, in steps of 0.5.
        real: Some((0.5, 99.5)),
        real_step: Some(0.5),
        method: Some(5),
        reading: Some(Reading::Range),
        sample: Sample::Float {
            base: 0.0,
            span: 100.0,
            period: 100,
        },
        ..PLAIN
    },
    Item {
        name: "note",
        field: Some("note"),
        kind: Kind::Text,
        unit_state: UnitState::Dimensionless,
        method: Some(6),
        reading: Some(Reading::Text),
        sample: Sample::Text("ready"),
        ..PLAIN
    },
];

const OFF_METHODS: &[MethodSpec] = &[MethodSpec {
    id: 1,
    name: "count",
    params: &[],
    returns: "f64",
    job: false,
}];

const OFF: &[Item] = &[Item {
    name: "count",
    field: Some("count"),
    kind: Kind::Float,
    unit: "count",
    unit_state: UnitState::Known,
    min: Some(0),
    max: Some(10),
    method: Some(1),
    reading: Some(Reading::Number),
    sample: Sample::Float {
        base: 0.0,
        span: 0.0,
        period: 1,
    },
    ..PLAIN
}];

const BARE: &[Item] = &[Item {
    name: "count",
    field: Some("count"),
    kind: Kind::Float,
    sample: Sample::Float {
        base: 1.0,
        span: 0.0,
        period: 1,
    },
    ..PLAIN
}];

/// The devices a simulated Lemnos node describes. One is healthy; one is retrying with a reason;
/// one is a three-axis unit with every reading kind; one is off with a reason; one has every
/// optional field unknown.
pub const DEVICES: &[Spec] = &[
    Spec {
        id: "vane0",
        label: "Wind vane",
        bus: "i2c",
        address: Some(0x44),
        location: "",
        kind: "sensor",
        state: Some(0),
        reason: "",
        contract: Some(("sim.vane/1", "sensor", 0x51a0)),
        methods: VANE_METHODS,
        status: Some(("ok", "")),
        items: VANE,
    },
    Spec {
        id: "gauge1",
        label: "",
        bus: "spi",
        address: None,
        location: "",
        kind: "sensor",
        state: Some(1),
        reason: "sensor drifting",
        contract: Some(("sim.gauge/1", "sensor", 0x9b07)),
        methods: GAUGE_METHODS,
        status: Some(("degraded", "sensor drifting")),
        items: GAUGE,
    },
    Spec {
        id: "unit3",
        label: "Motion unit",
        bus: "i2c",
        address: Some(104),
        location: "",
        kind: "motion",
        state: Some(0),
        reason: "",
        contract: Some(("sim.motion/1", "motion", 0x3c44)),
        methods: UNIT_METHODS,
        status: Some(("ok", "")),
        items: UNIT,
    },
    Spec {
        id: "off0",
        label: "",
        bus: "uart",
        address: None,
        location: "",
        kind: "",
        state: Some(2),
        reason: "disabled by its board file",
        contract: Some(("sim.counter/1", "other", 0x7e10)),
        methods: OFF_METHODS,
        status: Some(("ok", "")),
        items: OFF,
    },
    Spec {
        id: "bare0",
        label: "",
        bus: "",
        address: None,
        location: "",
        kind: "",
        state: None,
        reason: "",
        contract: None,
        methods: &[],
        status: None,
        items: BARE,
    },
];

/// A change to a simulated device's description: its state and reason, or its label. Each one
/// moves the device's generation once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A new state (`0` running, `1` retrying, `2` off) and reason. The resource follows: running
    /// is ready, retrying is failed, off is pending.
    State {
        /// The new state.
        state: u8,
        /// The new reason.
        reason: String,
    },
    /// A new label.
    Label(String),
}

/// The switches the test holds: while `hold` is set the device resources are withdrawn; while
/// `refuse` is set `describe` is refused on every device (a board that does not answer it, as
/// the bridge at Lemnos d6b9361d does not).
#[derive(Clone, Debug, Default)]
pub struct Control {
    hold: Arc<AtomicBool>,
    refuse: Arc<AtomicBool>,
    changes: Arc<Mutex<Vec<(String, Change)>>>,
}

impl Control {
    /// Withdraws the devices (`true`) or lists them again (`false`).
    pub fn hold(&self, hold: bool) {
        self.hold.store(hold, Ordering::SeqCst);
    }

    /// Refuses `describe` on every device (`true`), or answers it again (`false`).
    pub fn refuse_describe(&self, refuse: bool) {
        self.refuse.store(refuse, Ordering::SeqCst);
    }

    /// Queues a change of device `device`'s description. The node applies it on its next tick.
    pub fn change(&self, device: &str, change: Change) {
        if let Ok(mut queue) = self.changes.lock() {
            queue.push((device.to_owned(), change));
        }
    }

    fn held(&self) -> bool {
        self.hold.load(Ordering::SeqCst)
    }

    fn refused(&self) -> bool {
        self.refuse.load(Ordering::SeqCst)
    }

    fn take_changes(&self) -> Vec<(String, Change)> {
        self.changes
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }
}

/// One simulated device's description model, which changes with the tests.
#[derive(Debug)]
struct Model {
    spec: &'static Spec,
    device: describe::Device,
    generation: u64,
}

impl Model {
    fn new(spec: &'static Spec) -> Self {
        Self {
            spec,
            device: described(spec),
            generation: 0,
        }
    }
}

/// One listed device: its resource, and the publishers it writes through.
#[derive(Debug)]
struct Listed {
    model: usize,
    resource: Resource,
    values: Publisher,
    status: Option<Publisher>,
    description: Publisher,
}

/// The simulated devices of one node. Ticked by the node's own loop.
#[derive(Debug)]
pub struct Devices {
    control: Control,
    models: Vec<Model>,
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
        let mut models: Vec<Model> = DEVICES.iter().map(Model::new).collect();
        let live = !control.held();
        let listed = if live {
            list(orion, &mut models)?
        } else {
            Vec::new()
        };
        Ok(Self {
            control,
            models,
            listed,
            live,
            ticks: 0,
        })
    }

    /// Applies the changes the tests queued, publishes the values, and withdraws or lists the
    /// devices as the control says.
    pub fn tick(&mut self, orion: &Orion, now: SampleTime) {
        self.ticks += 1;
        for (device, change) in self.control.take_changes() {
            if let Some(index) = self.models.iter().position(|model| model.spec.id == device) {
                self.apply(index, change, now);
            }
        }
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
            if let Ok(listed) = list(orion, &mut self.models) {
                self.listed = listed;
                self.live = true;
            }
            return;
        }
        let tick = self.ticks;
        for listed in &mut self.listed {
            let spec = self.models[listed.model].spec;
            // The values topic numbers its fields in the order the items that publish one give them.
            let published = spec.items.iter().filter(|item| item.field.is_some());
            for (field, item) in published.enumerate() {
                let payload = payload(item.sample, tick);
                let _ = listed.values.publish(FieldId(field as u16), now, &payload);
            }
            // LOCK: see the module note. The status and the generation are written on every tick.
            if let (Some(status), Some((health, reason))) = (&mut listed.status, spec.status) {
                for (index, text) in ["ready", health, reason].iter().enumerate() {
                    let _ = status.publish(FieldId(index as u16), now, text.as_bytes());
                }
            }
            let generation = self.models[listed.model].generation;
            let _ = listed
                .description
                .publish(FieldId(0), now, &(generation as i64).to_le_bytes());
        }
    }

    /// Applies one change: the model moves, its generation moves once, and when the device is
    /// listed its generation topic and (for a state) its resource say so.
    fn apply(&mut self, index: usize, change: Change, now: SampleTime) {
        let model = &mut self.models[index];
        let mut state = None;
        match &change {
            Change::State {
                state: word,
                reason,
            } => {
                model.device.state = Some(*word);
                model.device.reason.clone_from(reason);
                state = Some(*word);
            }
            Change::Label(label) => model.device.label.clone_from(label),
        }
        model.generation += 1;
        let generation = model.generation;
        if let Some(listed) = self.listed.iter_mut().find(|listed| listed.model == index) {
            let _ = listed
                .description
                .publish(FieldId(0), now, &(generation as i64).to_le_bytes());
            if let Some(word) = state {
                let _ = listed.resource.set_state(resource_state(word));
            }
        }
    }

    /// What `call describe` on device `device` answers, or why it is refused.
    ///
    /// # Errors
    ///
    /// The refusal's words: the board does not answer `describe`, the device is not listed, or
    /// the device is not running (an off device has no description).
    pub fn describe(&self, device: &str) -> Result<String, String> {
        if self.control.refused() {
            return Err("this board does not answer describe".to_owned());
        }
        let Some(index) = self.models.iter().position(|model| model.spec.id == device) else {
            return Err("no such device".to_owned());
        };
        if !self.live || !self.listed.iter().any(|listed| listed.model == index) {
            return Err("no such device: it is not listed".to_owned());
        }
        let model = &self.models[index];
        if model.device.state == Some(2) {
            return Err("the device isn't running".to_owned());
        }
        let text = describe::render(&model.device, Some(model.generation));
        if text.len() > REPLY_MAX {
            return Err(format!(
                "the description is {} bytes; the reply carries at most {REPLY_MAX}, and a partial one is never sent",
                text.len()
            ));
        }
        Ok(text)
    }
}

/// The resource state a device's state word shows: running is ready, retrying is failed, off is
/// pending (Lemnos's status says the same).
fn resource_state(state: u8) -> ResourceState {
    match state {
        0 => ResourceState::Ready,
        1 => ResourceState::Failed,
        _ => ResourceState::Pending,
    }
}

/// The device a spec describes, as its model starts.
fn described(spec: &Spec) -> describe::Device {
    describe::Device {
        label: spec.label.to_owned(),
        kind: spec.kind.to_owned(),
        bus: spec.bus.to_owned(),
        address: spec.address,
        location: spec.location.to_owned(),
        state: spec.state,
        reason: spec.reason.to_owned(),
        contracts: spec
            .contract
            .map(|(tag, kind, hash)| describe::Contract {
                tag: tag.to_owned(),
                kind: kind.to_owned(),
                hash,
                methods: spec
                    .methods
                    .iter()
                    .map(|method| describe::Method {
                        id: method.id,
                        name: method.name.to_owned(),
                        params: method
                            .params
                            .iter()
                            .map(|(name, shape)| ((*name).to_owned(), (*shape).to_owned()))
                            .collect(),
                        returns: method.returns.to_owned(),
                        out_buffer: false,
                        job: method.job,
                    })
                    .collect(),
            })
            .into_iter()
            .collect(),
        items: spec.items.iter().map(described_item).collect(),
    }
}

fn described_item(item: &Item) -> describe::Item {
    describe::Item {
        kind: item.item_kind,
        name: item.name.to_owned(),
        unit: item.unit.to_owned(),
        unit_state: item.unit_state,
        exponent: item.exponent,
        min: item.min,
        max: item.max,
        real_min: item.real.map(|(min, _)| min),
        real_max: item.real.map(|(_, max)| max),
        real_step: item.real_step,
        step: item.step,
        warn_low: item.warn_low,
        warn_high: item.warn_high,
        contract: item.method.map(|_| 0),
        method: item.method,
        field: item.field.map(str::to_owned),
        reading: item.reading,
        group: item.group.map(str::to_owned),
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

/// Lists every device, and each listing is an appearance: its generation moves once.
fn list(orion: &Orion, models: &mut [Model]) -> Result<Vec<Listed>, String> {
    let provider = orion.provider();
    let now = SampleTime::from_nanos(1);
    let mut listed = Vec::new();
    for (index, model) in models.iter_mut().enumerate() {
        model.generation += 1;
        let spec = model.spec;
        let builder: Describing = provider
            .resource(&format!("lemnos.{}", spec.id), DEVICE_KIND)
            .map_err(|error| error.to_string())?;
        let fields: Vec<Field<'_>> = spec
            .items
            .iter()
            .filter_map(|item| {
                item.field.map(|field| Field {
                    name: field,
                    ty: field_type(item.kind),
                    unit: item.unit,
                    type_name: "",
                })
            })
            .collect();
        let mut builder = builder;
        let values = builder
            .topic("values", &fields, HISTORY, MAX_PAYLOAD)
            .map_err(|error| error.to_string())?;
        let generation_field = [Field {
            name: "generation",
            ty: FieldType::Int,
            unit: "",
            type_name: "",
        }];
        let mut description = builder
            .topic(DESCRIPTION_TOPIC, &generation_field, 1, 8)
            .map_err(|error| error.to_string())?;
        description
            .publish(FieldId(0), now, &(model.generation as i64).to_le_bytes())
            .map_err(|error| error.to_string())?;
        let status = match spec.status {
            Some((health, reason)) => {
                let status_fields = [
                    text_field("state"),
                    text_field("health"),
                    text_field("reason"),
                ];
                let mut status = builder
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
        let resource = builder
            .publish(resource_state(spec.state.unwrap_or(0)))
            .map_err(|error| error.to_string())?;
        listed.push(Listed {
            model: index,
            resource,
            values,
            status,
            description,
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
    fn every_device_has_a_unique_id_and_every_item_method_is_one_of_its_methods() {
        let mut ids: Vec<&str> = DEVICES.iter().map(|spec| spec.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DEVICES.len());
        for spec in DEVICES {
            for item in spec.items {
                if let Some(method) = item.method {
                    assert!(
                        spec.methods.iter().any(|m| m.id == method),
                        "{} item {} names method {method}, which the device does not have",
                        spec.id,
                        item.name
                    );
                }
            }
        }
    }

    #[test]
    fn the_modelled_description_renders_and_the_off_device_has_its_reason() {
        let vane = described(&DEVICES[0]);
        let text = describe::render(&vane, Some(1));
        assert!(
            text.starts_with("describe 1\ndevice generation=1 label=\"Wind vane\""),
            "{text}"
        );
        let off = described(&DEVICES[3]);
        assert_eq!(off.reason, "disabled by its board file");
        assert_eq!(off.state, Some(2));
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
