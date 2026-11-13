//! A device as the node describes it, and what it reads now.
//!
//! The description is the text of `call describe` (see [`crate::description`]), read by the
//! [`crate::describe`] call and parsed whole. This module maps it onto the device model:
//!
//! - the device record gives the label, bus, state and reason, the address, the location, the
//!   kind, the generation, and the lease period when the description has one;
//! - the contract records give each contract's tag, kind and hash;
//! - each item gives a reading (number, range, switch, text, colour array, three axes), its unit
//!   and its unit state (known, dimensionless or unknown), its range (the real range when given,
//!   else the integer one scaled by the exponent), its step and warning limits, its group, and
//!   whether it is a control;
//! - the topics give the values, the health (the status topic) and the generation (the
//!   `<device>.description` topic, the same number the description carries).
//!
//! A figure the description does not give is [`Reported::NotReported`]: never a default, never a
//! table in Atlas. A device whose description is not available is still listed with its topics;
//! only the description's fields are not reported. A value that no item describes is still a
//! reading, with its description fields not reported.

use std::time::Instant;

use orion::provider::ResourceState;

use crate::description::{Description, DescriptionState, ItemRecord};
use crate::value::{ItemKind, ReadingKind, Reported, Scalar, UnitState};

/// The resource prefix of the devices a node lists (`lemnos.<device>`).
pub const RESOURCE_PREFIX: &str = "lemnos.";
/// The kind a device resource is written as.
pub const RESOURCE_KIND: &str = "lemnos.device";
/// The suffix of a device's status topic: its `health` and `reason`.
pub const STATUS_SUFFIX: &str = ".status";
/// The suffix of a device's description topic: its one field is the description generation.
pub const DESCRIPTION_SUFFIX: &str = ".description";
/// The field of the description topic that holds the generation.
pub const GENERATION_FIELD: &str = "generation";

/// Whether a device is listed by its provider, and what the provider says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Declared, not yet ready (a device that is off).
    Pending,
    /// Ready.
    Ready,
    /// Present, not working (a device that retries, or reports failed health).
    Failed,
    /// Its provider went away.
    Lost,
}

/// A device's state (`describe 1`'s `state`). A retrying device is a state of the device with its
/// reason: it is not a fault of the node, and it stays listed and current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    /// Running.
    Running,
    /// Retrying, with its reason.
    Retrying,
    /// Off, with its reason.
    Off,
}

impl DeviceState {
    /// The state a description's word names, or `None` for a word this crate does not know.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "running" => Self::Running,
            "retrying" => Self::Retrying,
            "off" => Self::Off,
            _ => return None,
        })
    }
}

/// Whether a device's readings are current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Its readings come from a live publisher.
    Live,
    /// Its readings are the last ones, from `since`: the node does not list it, its publisher has
    /// gone, or the session to the node is down.
    Stale {
        /// The last time the readings were current.
        since: Instant,
    },
}

/// A control the device takes: an item of kind control, with the parameters of its method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    /// Its item name.
    pub name: String,
    /// The parameters its method takes, as the description writes them (`name:shape`, joined by
    /// spaces). Empty when none, or when the description gives no method.
    pub takes: String,
}

/// A contract a device serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    /// Its position: the index items name it by.
    pub index: usize,
    /// Its tag, for example `name.thing/1`.
    pub tag: Reported<String>,
    /// Its kind.
    pub kind: Reported<String>,
    /// Its hash: the identity of the contract's shape.
    pub hash: Reported<u64>,
}

/// A range: the item's least and greatest value, in its unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    /// The least value.
    pub min: f64,
    /// The greatest value.
    pub max: f64,
}

/// One item of a device: its reading and the figures its description gives, with its value now.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The item's name. A value no item describes is named by its topic field.
    pub name: String,
    /// The topic field its value is published in.
    pub field: Reported<String>,
    /// Whether it is a value, a control or an action.
    pub item_kind: Reported<ItemKind>,
    /// What the reading is.
    pub reading: Reported<ReadingKind>,
    /// Its unit: the description's text when its unit state is known.
    pub unit: Reported<String>,
    /// Its unit state. `Unknown` when the description does not give one, or gives no unit.
    pub unit_state: UnitState,
    /// Its range: the real range when the description gives both ends, else the integer range
    /// scaled by the exponent.
    pub range: Reported<Range>,
    /// Its step: the real step when given, else the integer step scaled by the exponent.
    pub step: Reported<f64>,
    /// The lower warning limit, scaled by the exponent.
    pub warn_low: Reported<f64>,
    /// The upper warning limit, scaled by the exponent.
    pub warn_high: Reported<f64>,
    /// The group (a three-axis vector) it is one part of.
    pub group: Reported<String>,
    /// Whether the node takes a setter for it: `true` for a control, `false` for a value or an
    /// action.
    pub settable: Reported<bool>,
    /// The name of the method the setter calls, when the description gives it.
    pub setter: Reported<String>,
    /// The newest value, or `None` before the first sample.
    pub value: Option<Scalar>,
}

/// A group of items that are the parts of one vector (a three-axis reading), in the order the
/// description gives them. The description does not name the axes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vector {
    /// The group's name.
    pub name: String,
    /// The items that are its parts, in the description's order.
    pub members: Vec<String>,
}

/// One device the node describes.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// Its resource name (`lemnos.<device>` on a Lemnos node).
    pub name: String,
    /// Its board-file label. Not reported: show [`Device::shown_name`].
    pub label: Reported<String>,
    /// The bus it is on.
    pub bus: Reported<String>,
    /// Its state.
    pub state: Reported<DeviceState>,
    /// Why it is retrying or off.
    pub state_reason: Reported<String>,
    /// Its address on its bus.
    pub address: Reported<u8>,
    /// Where the driver says it is, in words.
    pub location: Reported<String>,
    /// Its kind.
    pub kind: Reported<String>,
    /// Its description generation: the number of its description topic.
    pub generation: Reported<u64>,
    /// Its lease period in seconds, when the description gives one.
    pub lease_seconds: Reported<u64>,
    /// The contracts it serves.
    pub contracts: Vec<Contract>,
    /// Whether the provider has it ready, failed or gone.
    pub availability: Reported<Availability>,
    /// The device's health, from its status topic.
    pub health: Reported<String>,
    /// Why the health is what it is.
    pub health_reason: Reported<String>,
    /// The controls it takes.
    pub controls: Vec<Control>,
    /// Its items, with their values.
    pub readings: Vec<Reading>,
    /// Its groups of parts (three-axis vectors).
    pub vectors: Vec<Vector>,
    /// What the device's description is: read, refused, or not available.
    pub description: DescriptionState,
    /// Whether the node lists it now. `false` while it is held within the grace after the node
    /// stopped listing it.
    pub listed: bool,
    /// Whether its readings are current. Set when a snapshot is taken.
    pub freshness: Freshness,
    /// Whether one of its topics has lost its publisher (a topic the publisher withdrew reads
    /// the same way).
    pub(crate) gone: bool,
}

impl Device {
    /// The name to show: the label when the description gives one, else the device's name.
    #[must_use]
    pub fn shown_name(&self) -> &str {
        self.label.get().map_or(self.name.as_str(), String::as_str)
    }

    /// The description, when it was read and whole.
    #[must_use]
    pub fn described(&self) -> Option<&Description> {
        match &self.description {
            DescriptionState::Described(description) => Some(description),
            _ => None,
        }
    }
}

/// What a topic's publisher is doing, as the subscription sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopicLive {
    /// Nobody has declared it yet.
    Waiting,
    /// A publisher is there.
    Live,
    /// The publisher is gone, or withdrew the topic; what it published is still readable.
    Gone,
}

/// A field of a topic as read: its name and its newest value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawField {
    pub name: String,
    pub value: Option<Scalar>,
}

/// A topic of a device as read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawTopic {
    pub name: String,
    pub live: TopicLive,
    pub fields: Vec<RawField>,
}

/// A device resource as read: what the node lists, its topics read, and its description.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawDevice {
    pub resource: String,
    pub state: ResourceState,
    pub topics: Vec<RawTopic>,
    pub description: DescriptionState,
}

fn text_of(value: Option<&str>) -> Reported<String> {
    value.map_or(Reported::NotReported, Reported::text)
}

fn figure<T>(value: Option<T>) -> Reported<T> {
    value.map_or(Reported::NotReported, Reported::Value)
}

fn availability(state: ResourceState) -> Reported<Availability> {
    match state {
        ResourceState::Pending => Reported::Value(Availability::Pending),
        ResourceState::Ready => Reported::Value(Availability::Ready),
        ResourceState::Failed => Reported::Value(Availability::Failed),
        ResourceState::Lost => Reported::Value(Availability::Lost),
        _ => Reported::NotReported,
    }
}

/// The generation the description topic publishes now: its field, read as a whole number. `None`
/// while the device has no description topic, or no sample on it yet.
pub(crate) fn generation_of(topics: &[RawTopic]) -> Option<u64> {
    topics
        .iter()
        .filter(|topic| topic.name.ends_with(DESCRIPTION_SUFFIX))
        .flat_map(|topic| topic.fields.iter())
        .find(|field| field.name == GENERATION_FIELD)
        .and_then(|field| match field.value {
            Some(Scalar::Number(number))
                if number >= 0.0 && number.fract() == 0.0 && number <= 9.0e15 =>
            {
                Some(number as u64)
            }
            _ => None,
        })
}

/// The scaled value of a raw figure: `raw × 10^exponent`. Not reported without an exponent.
fn scaled(raw: Option<i64>, exponent: Option<i8>) -> Reported<f64> {
    match (raw, exponent) {
        (Some(raw), Some(exponent)) => {
            Reported::Value(raw as f64 * 10_f64.powi(i32::from(exponent)))
        }
        _ => Reported::NotReported,
    }
}

/// The unit state an item has. A description that predates the state is decided by its unit
/// string: unknown when the string is empty, known otherwise. A known unit with no string is
/// unknown, since there is no unit to show.
fn unit_state_of(item: &ItemRecord) -> UnitState {
    let has_unit = item.unit.as_deref().is_some_and(|unit| !unit.is_empty());
    match item.unit_state {
        Some(UnitState::Known) if has_unit => UnitState::Known,
        Some(UnitState::Known) | Some(UnitState::Unknown) => UnitState::Unknown,
        Some(UnitState::Dimensionless) => UnitState::Dimensionless,
        None if has_unit => UnitState::Known,
        None => UnitState::Unknown,
    }
}

fn range_of(item: &ItemRecord) -> Reported<Range> {
    if let (Some(min), Some(max)) = (item.real_min, item.real_max) {
        return Reported::Value(Range { min, max });
    }
    match (
        scaled(item.min, item.exponent),
        scaled(item.max, item.exponent),
    ) {
        (Reported::Value(min), Reported::Value(max)) => Reported::Value(Range { min, max }),
        _ => Reported::NotReported,
    }
}

fn reading_of(item: &ItemRecord, value: Option<Scalar>, description: &Description) -> Reading {
    let unit_state = unit_state_of(item);
    let unit = match unit_state {
        UnitState::Known => text_of(item.unit.as_deref()),
        UnitState::Dimensionless | UnitState::Unknown => Reported::NotReported,
    };
    let setter = match (item.kind, item.contract, item.method) {
        (Some(ItemKind::Control), Some(contract), Some(method)) => description
            .methods
            .iter()
            .find(|candidate| candidate.contract == contract && candidate.id == method)
            .map_or(Reported::NotReported, |candidate| {
                text_of(candidate.name.as_deref())
            }),
        _ => Reported::NotReported,
    };
    let settable = match item.kind {
        Some(ItemKind::Control) => Reported::Value(true),
        Some(ItemKind::Value | ItemKind::Action) => Reported::Value(false),
        None => Reported::NotReported,
    };
    Reading {
        name: item.name.clone().unwrap_or_default(),
        field: text_of(item.field.as_deref()),
        item_kind: figure(item.kind),
        reading: figure(item.reading),
        unit,
        unit_state,
        range: range_of(item),
        step: match item.real_step {
            Some(step) => Reported::Value(step),
            None => scaled(item.step, item.exponent),
        },
        warn_low: scaled(item.warn_low, item.exponent),
        warn_high: scaled(item.warn_high, item.exponent),
        group: text_of(item.group.as_deref()),
        settable,
        setter,
        value,
    }
}

/// The device a resource, its topics and its description describe.
pub(crate) fn describe(raw: RawDevice) -> Device {
    let generation = figure(generation_of(&raw.topics));
    let (mut health, mut health_reason) = (Reported::NotReported, Reported::NotReported);
    let mut gone = false;
    let mut values: Vec<RawField> = Vec::new();
    for topic in &raw.topics {
        gone |= topic.live == TopicLive::Gone;
        if topic.name.ends_with(STATUS_SUFFIX) {
            for field in &topic.fields {
                let text = match &field.value {
                    Some(Scalar::Text(text)) => Reported::text(text),
                    _ => Reported::NotReported,
                };
                match field.name.as_str() {
                    "health" => health = text,
                    "reason" => health_reason = text,
                    _ => {}
                }
            }
        } else if !topic.name.ends_with(DESCRIPTION_SUFFIX) {
            values.extend(topic.fields.iter().cloned());
        }
    }

    let mut readings = Vec::new();
    let mut controls = Vec::new();
    let mut contracts = Vec::new();
    let (mut label, mut bus, mut state, mut state_reason) = (
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
    );
    let (mut address, mut location, mut kind, mut lease_seconds) = (
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
    );
    if let DescriptionState::Described(description) = &raw.description {
        let record = &description.device;
        label = text_of(record.label.as_deref());
        bus = text_of(record.bus.as_deref());
        state = figure(record.state);
        state_reason = text_of(record.reason.as_deref());
        address = figure(record.address);
        location = text_of(record.location.as_deref());
        kind = text_of(record.kind.as_deref());
        lease_seconds = figure(record.lease_seconds);
        contracts = description
            .contracts
            .iter()
            .map(|contract| Contract {
                index: contract.index,
                tag: text_of(contract.tag.as_deref()),
                kind: text_of(contract.kind.as_deref()),
                hash: figure(contract.hash),
            })
            .collect();
        let mut matched = Vec::new();
        for item in &description.items {
            let value = item.field.as_deref().and_then(|field| {
                values
                    .iter()
                    .find(|candidate| candidate.name == field)
                    .and_then(|candidate| candidate.value.clone())
            });
            if let Some(field) = &item.field {
                matched.push(field.clone());
            }
            if item.kind == Some(ItemKind::Control) {
                let takes = match (item.contract, item.method) {
                    (Some(contract), Some(method)) => description
                        .methods
                        .iter()
                        .find(|candidate| candidate.contract == contract && candidate.id == method)
                        .and_then(|candidate| candidate.params.clone())
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                controls.push(Control {
                    name: item.name.clone().unwrap_or_default(),
                    takes,
                });
            }
            readings.push(reading_of(item, value, description));
        }
        // A value no item describes is still a reading, with nothing else known of it.
        readings.extend(
            values
                .iter()
                .filter(|field| !matched.contains(&field.name))
                .map(value_only),
        );
    } else {
        // No description: the values the topics publish are still shown, with nothing described.
        readings.extend(values.iter().map(value_only));
    }

    let vectors = vectors_of(&readings);
    Device {
        name: raw.resource,
        label,
        bus,
        state,
        state_reason,
        address,
        location,
        kind,
        generation,
        lease_seconds,
        contracts,
        availability: availability(raw.state),
        health,
        health_reason,
        controls,
        readings,
        vectors,
        description: raw.description,
        listed: true,
        freshness: Freshness::Live,
        gone,
    }
}

/// A value no description gives: its name and value, and every description figure not reported.
fn value_only(field: &RawField) -> Reading {
    Reading {
        name: field.name.clone(),
        field: Reported::text(&field.name),
        item_kind: Reported::NotReported,
        reading: Reported::NotReported,
        unit: Reported::NotReported,
        unit_state: UnitState::Unknown,
        range: Reported::NotReported,
        step: Reported::NotReported,
        warn_low: Reported::NotReported,
        warn_high: Reported::NotReported,
        group: Reported::NotReported,
        settable: Reported::NotReported,
        setter: Reported::NotReported,
        value: field.value.clone(),
    }
}

/// The groups the items name: items that share a group are its parts, in the description's order.
fn vectors_of(readings: &[Reading]) -> Vec<Vector> {
    let mut vectors: Vec<Vector> = Vec::new();
    for reading in readings {
        let Reported::Value(group) = &reading.group else {
            continue;
        };
        match vectors.iter_mut().find(|vector| &vector.name == group) {
            Some(vector) => vector.members.push(reading.name.clone()),
            None => vectors.push(Vector {
                name: group.clone(),
                members: vec![reading.name.clone()],
            }),
        }
    }
    vectors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::description::parse;

    /// A device with one value topic `v` (value 2.0) and the description `text`.
    fn described(text: &str) -> Device {
        describe(RawDevice {
            resource: "lemnos.x".into(),
            state: ResourceState::Ready,
            topics: vec![RawTopic {
                name: "lemnos.x.values".into(),
                live: TopicLive::Live,
                fields: vec![RawField {
                    name: "v".into(),
                    value: Some(Scalar::Number(2.0)),
                }],
            }],
            description: DescriptionState::Described(Box::new(parse(text).expect("reads"))),
        })
    }

    #[test]
    fn a_description_that_predates_the_unit_state_decides_it_by_its_unit_string() {
        let with_unit = described("describe 1\ndevice\nitem name=\"v\" field=\"v\" unit=\"V\"");
        assert_eq!(with_unit.readings[0].unit_state, UnitState::Known);
        assert_eq!(with_unit.readings[0].unit, Reported::Value("V".into()));
        let without = described("describe 1\ndevice\nitem name=\"v\" field=\"v\" unit=\"\"");
        assert_eq!(without.readings[0].unit_state, UnitState::Unknown);
        assert_eq!(without.readings[0].unit, Reported::NotReported);
    }

    #[test]
    fn a_known_state_with_no_unit_string_is_unknown_and_a_dimensionless_one_is_not_shown_as_a_unit()
    {
        let known_empty =
            described("describe 1\ndevice\nitem name=\"v\" field=\"v\" unit=\"\" unit_state=known");
        assert_eq!(known_empty.readings[0].unit_state, UnitState::Unknown);
        let dimensionless = described(
            "describe 1\ndevice\nitem name=\"v\" field=\"v\" unit=\"\" unit_state=dimensionless",
        );
        assert_eq!(
            dimensionless.readings[0].unit_state,
            UnitState::Dimensionless
        );
        assert_eq!(dimensionless.readings[0].unit, Reported::NotReported);
    }

    #[test]
    fn the_real_range_wins_and_an_integer_range_is_scaled_only_with_an_exponent() {
        let real = described(
            "describe 1\ndevice\nitem name=\"v\" field=\"v\" exponent=-1 min=0 max=10 real_min=-0.5 real_max=2.5 real_step=0.5",
        );
        assert_eq!(
            real.readings[0].range,
            Reported::Value(Range {
                min: -0.5,
                max: 2.5
            })
        );
        assert_eq!(real.readings[0].step, Reported::Value(0.5));
        let no_exponent = described("describe 1\ndevice\nitem name=\"v\" field=\"v\" min=0 max=10");
        assert_eq!(
            no_exponent.readings[0].range,
            Reported::NotReported,
            "no exponent, no scale"
        );
    }

    #[test]
    fn a_value_no_item_describes_is_a_reading_with_nothing_else_known() {
        let device = described("describe 1\ndevice generation=4 lease_seconds=60 state=off");
        assert_eq!(device.readings.len(), 1);
        assert_eq!(device.readings[0].name, "v");
        assert_eq!(device.readings[0].unit_state, UnitState::Unknown);
        assert_eq!(device.readings[0].value, Some(Scalar::Number(2.0)));
        assert_eq!(device.lease_seconds, Reported::Value(60));
        assert_eq!(device.state, Reported::Value(DeviceState::Off));
    }

    #[test]
    fn a_control_takes_its_method_parameters_and_is_settable() {
        let device = described(
            "describe 1\ndevice\ncontract index=0 tag=\"c/1\" kind=\"other\" hash=0x1 methods=1\nmethod contract=0 id=7 name=\"set-v\" params=\"value:f64 mode:u8\" returns=\"()\" out_buffer=false job=false\nitem kind=control name=\"v\" field=\"v\" contract=0 method=7",
        );
        assert_eq!(
            device.controls,
            vec![Control {
                name: "v".into(),
                takes: "value:f64 mode:u8".into()
            }]
        );
        assert_eq!(device.readings[0].settable, Reported::Value(true));
        assert_eq!(device.readings[0].setter, Reported::Value("set-v".into()));
    }
}
