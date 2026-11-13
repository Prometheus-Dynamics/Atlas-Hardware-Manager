//! A device as the node describes it, and what it reads now.
//!
//! The description is Lemnos V1a (`lemnos-wire`'s `DeviceDescription`, the blocks appended after
//! the job list): a label, a bus, a state (running, retrying or off) with its reason, an address,
//! a kind per contract, and per item a reading kind, a unit, a range, a step, warning limits, a
//! group (a three-axis vector), and whether it is settable with its setter. Each of these can be
//! unknown. A device is never hidden or faulted for lacking one: it is listed with the field
//! [`Reported::NotReported`].
//!
//! The bridge names them as labels on the device's resource, and an item's block as a label per
//! value field. The names are the bridge's, read here as data:
//!
//! - `lemnos.label`, `lemnos.bus`, `lemnos.state`, `lemnos.reason`, `lemnos.address`,
//!   `lemnos.kind`, `lemnos.driver`;
//! - `lemnos.tag` once per contract, and `lemnos.kind.<tag>` for that contract's kind;
//! - `lemnos.control.<item>` (the existing control labels);
//! - `lemnos.item.<field>`: an item's block, `key=value` pairs separated by `;` (the keys are in
//!   [`parse_item`]). A key that is absent is unknown.
//!
//! The status topic `<resource>.status` carries `health` and `reason` (the device's health, not
//! its state).

use std::time::Instant;

use orion::provider::ResourceState;

use crate::value::{ReadingKind, Reported, Scalar};

// LOCK: the V1a/V1b blocks are in Lemnos d6b9361d's host description (`lemnos-wire`'s `DescIdentity`,
// `DescContractKind`, `DescItemExtra`), but the bridge at d6b9361d still publishes only the driver,
// tag and control labels, and has no `describe` verb. These keys stay the carrier the simulated node
// writes until Lemnos puts the description on Orion (its paged `describe`, M8b/M8c, not merged).
// Every one of these may be unknown on a real board.
/// The device's board-file label (V1a `label`). Absent: the device is shown by its name.
pub const LABEL_KEY: &str = "lemnos.label";
/// The bus the device is on (V1a `bus`).
pub const BUS_KEY: &str = "lemnos.bus";
/// The device's state word: `running`, `retrying` or `off` (V1a `state` 0, 1, 2).
pub const STATE_KEY: &str = "lemnos.state";
/// Why the device is retrying or off (V1a `reason`).
pub const REASON_KEY: &str = "lemnos.reason";
/// The device's address on its bus, decimal or `0x` hex (V1a `address`, 7-bit I2C only today).
pub const ADDRESS_KEY: &str = "lemnos.address";
/// The device's kind: the first contract kind that is not `other` (V1a `kind`).
pub const KIND_KEY: &str = "lemnos.kind";
/// One per contract the device provides.
pub const TAG_KEY: &str = "lemnos.tag";
/// The kind of one contract: `lemnos.kind.<tag>` (V1a per-contract kind).
pub const CONTRACT_KIND_PREFIX: &str = "lemnos.kind.";
/// The driver the device runs with.
pub const DRIVER_KEY: &str = "lemnos.driver";
/// The prefix of a control's label: `lemnos.control.<item>`, valued with the shapes it takes.
pub const CONTROL_PREFIX: &str = "lemnos.control.";
/// The prefix of an item's block: `lemnos.item.<field>`.
pub const ITEM_PREFIX: &str = "lemnos.item.";
/// The suffix of a device's status topic.
pub const STATUS_SUFFIX: &str = ".status";
/// The resource prefix of the devices a node lists (`lemnos.<device>`).
pub const RESOURCE_PREFIX: &str = "lemnos.";
/// The kind a device resource is written as.
pub const RESOURCE_KIND: &str = "lemnos.device";

/// Whether a device is listed by its provider, and what the provider says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Declared, not yet ready.
    Pending,
    /// Ready.
    Ready,
    /// Present, not working.
    Failed,
    /// Its provider went away.
    Lost,
}

/// A device's state (V1a). A retrying device is a state of the device with its reason: it is not
/// a fault of the node, and it stays listed and current.
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
    fn from_word(word: &str) -> Option<Self> {
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

/// A control the device takes: `lemnos.control.<item>`, with the shapes of the values it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    /// Its item name.
    pub name: String,
    /// The shapes of the values it takes, as the description writes them. Empty for none.
    pub takes: String,
}

/// A contract a device provides, with its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    /// Its tag, for example `name.thing/1`.
    pub tag: String,
    /// Its kind. Not reported until the description gives one.
    pub kind: Reported<String>,
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
    /// The item's name (its value field's name).
    pub name: String,
    /// What the reading is.
    pub reading: Reported<ReadingKind>,
    /// Its unit.
    pub unit: Reported<String>,
    /// Its range.
    pub range: Reported<Range>,
    /// Its step.
    pub step: Reported<f64>,
    /// The lower warning limit.
    pub warn_low: Reported<f64>,
    /// The upper warning limit.
    pub warn_high: Reported<f64>,
    /// The three-axis vector it is one axis of.
    pub group: Reported<String>,
    /// Its axis within the vector (`x`, `y`, `z`).
    pub axis: Reported<String>,
    /// Whether the node takes a setter for it.
    pub settable: Reported<bool>,
    /// What the setter does. The call itself is not made by this crate (it is C3's).
    pub setter: Reported<String>,
    /// The newest value, or `None` before the first sample.
    pub value: Option<Scalar>,
}

/// A three-axis vector: the items that are its axes, by axis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vector {
    /// The group's name.
    pub name: String,
    /// Each axis and the item that holds it, in the order the description gives them.
    pub axes: Vec<(String, String)>,
}

/// One device the node describes.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// Its resource name (`lemnos.<device>` on a Lemnos node).
    pub name: String,
    /// Its board-file label. Not reported: show [`Device::shown_name`].
    pub label: Reported<String>,
    /// The driver it runs with.
    pub driver: Reported<String>,
    /// The bus it is on.
    pub bus: Reported<String>,
    /// Its state.
    pub state: Reported<DeviceState>,
    /// Why it is retrying or off.
    pub state_reason: Reported<String>,
    /// Its address on its bus.
    pub address: Reported<u8>,
    /// Its kind.
    pub kind: Reported<String>,
    /// The contracts it provides.
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
    /// Its three-axis vectors.
    pub vectors: Vec<Vector>,
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

/// A field of a topic as read: its name, its topic's unit and its newest value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawField {
    pub name: String,
    pub unit: String,
    pub value: Option<Scalar>,
}

/// A topic of a device as read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawTopic {
    pub name: String,
    pub live: TopicLive,
    pub fields: Vec<RawField>,
}

/// A device resource as read: what the node lists, with its topics read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawDevice {
    pub resource: String,
    pub state: ResourceState,
    pub labels: Vec<(String, String)>,
    pub topics: Vec<RawTopic>,
}

/// The figures of one item's block. Each is unknown when its key is absent or does not parse.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ItemSpec {
    pub reading: Option<ReadingKind>,
    pub unit: String,
    pub exponent: Option<i8>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub step: Option<i64>,
    pub warn_low: Option<i64>,
    pub warn_high: Option<i64>,
    pub group: String,
    pub axis: String,
    pub settable: Option<bool>,
    pub setter: String,
}

/// Parses one item's block: `key=value` pairs separated by `;`. Keys: `reading` (a kind word),
/// `unit`, `exponent` (the figures are raw × 10^exponent), `min`, `max`, `step`, `warn_low`,
/// `warn_high` (raw integers), `group`, `axis`, `settable` (`true`/`false`), `setter` (text).
/// Anything else is ignored, so a newer description still reads.
#[must_use]
pub(crate) fn parse_item(block: &str) -> ItemSpec {
    let mut spec = ItemSpec::default();
    for pair in block.split(';') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "reading" => spec.reading = ReadingKind::from_word(value),
            "unit" => spec.unit = value.to_owned(),
            "exponent" => spec.exponent = value.parse().ok(),
            "min" => spec.min = value.parse().ok(),
            "max" => spec.max = value.parse().ok(),
            "step" => spec.step = value.parse().ok(),
            "warn_low" => spec.warn_low = value.parse().ok(),
            "warn_high" => spec.warn_high = value.parse().ok(),
            "group" => spec.group = value.to_owned(),
            "axis" => spec.axis = value.to_owned(),
            "settable" => spec.settable = value.parse().ok(),
            "setter" => spec.setter = value.to_owned(),
            _ => {}
        }
    }
    spec
}

/// A raw figure in its item's unit: `raw × 10^exponent`. Unknown without an exponent.
fn scaled(raw: Option<i64>, exponent: Option<i8>) -> Reported<f64> {
    match (raw, exponent) {
        (Some(raw), Some(exponent)) => {
            Reported::Value(raw as f64 * 10_f64.powi(i32::from(exponent)))
        }
        _ => Reported::NotReported,
    }
}

fn label<'a>(labels: &'a [(String, String)], key: &str) -> Option<&'a str> {
    labels
        .iter()
        .find(|(label, _)| label == key)
        .map(|(_, value)| value.as_str())
}

fn text(labels: &[(String, String)], key: &str) -> Reported<String> {
    label(labels, key).map_or(Reported::NotReported, Reported::text)
}

fn address(labels: &[(String, String)]) -> Reported<u8> {
    let Some(text) = label(labels, ADDRESS_KEY) else {
        return Reported::NotReported;
    };
    let parsed = match text.strip_prefix("0x") {
        Some(hex) => u8::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_or(Reported::NotReported, Reported::Value)
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

fn text_of(field: &RawField) -> Reported<String> {
    match &field.value {
        Some(Scalar::Text(text)) => Reported::text(text),
        _ => Reported::NotReported,
    }
}

/// The device a resource and its topics describe. A device is listed and not gone until a
/// snapshot says otherwise.
pub(crate) fn describe(raw: RawDevice) -> Device {
    let labels = &raw.labels;
    let state = match text(labels, STATE_KEY) {
        Reported::Value(word) => {
            DeviceState::from_word(&word).map_or(Reported::NotReported, Reported::Value)
        }
        Reported::NotReported => Reported::NotReported,
    };
    let contracts = labels
        .iter()
        .filter(|(key, _)| key == TAG_KEY)
        .map(|(_, tag)| Contract {
            tag: tag.clone(),
            kind: text(labels, &format!("{CONTRACT_KIND_PREFIX}{tag}")),
        })
        .collect();
    let controls = labels
        .iter()
        .filter_map(|(key, takes)| {
            key.strip_prefix(CONTROL_PREFIX).map(|name| Control {
                name: name.to_owned(),
                takes: takes.clone(),
            })
        })
        .collect();

    let (mut health, mut health_reason) = (Reported::NotReported, Reported::NotReported);
    let mut readings = Vec::new();
    let mut gone = false;
    for topic in raw.topics {
        gone |= topic.live == TopicLive::Gone;
        if topic.name.ends_with(STATUS_SUFFIX) {
            for field in &topic.fields {
                match field.name.as_str() {
                    "health" => health = text_of(field),
                    "reason" => health_reason = text_of(field),
                    _ => {}
                }
            }
            continue;
        }
        for field in topic.fields {
            let spec = label(labels, &format!("{ITEM_PREFIX}{}", field.name))
                .map(parse_item)
                .unwrap_or_default();
            // The unit is the description's; the topic's declared unit is the same figure, and
            // stands in only when the description leaves it out.
            let unit = if spec.unit.is_empty() {
                Reported::text(&field.unit)
            } else {
                Reported::Value(spec.unit.clone())
            };
            let range = match (spec.min, spec.max) {
                (Some(_), Some(_)) => match (
                    scaled(spec.min, spec.exponent),
                    scaled(spec.max, spec.exponent),
                ) {
                    (Reported::Value(min), Reported::Value(max)) => {
                        Reported::Value(Range { min, max })
                    }
                    _ => Reported::NotReported,
                },
                _ => Reported::NotReported,
            };
            readings.push(Reading {
                reading: spec.reading.map_or(Reported::NotReported, Reported::Value),
                unit,
                range,
                step: scaled(spec.step, spec.exponent),
                warn_low: scaled(spec.warn_low, spec.exponent),
                warn_high: scaled(spec.warn_high, spec.exponent),
                group: Reported::text(&spec.group),
                axis: Reported::text(&spec.axis),
                settable: spec.settable.map_or(Reported::NotReported, Reported::Value),
                setter: Reported::text(&spec.setter),
                value: field.value,
                name: field.name,
            });
        }
    }

    let vectors = vectors_of(&readings);
    Device {
        name: raw.resource,
        label: text(labels, LABEL_KEY),
        driver: text(labels, DRIVER_KEY),
        bus: text(labels, BUS_KEY),
        state,
        state_reason: text(labels, REASON_KEY),
        address: address(labels),
        kind: text(labels, KIND_KEY),
        contracts,
        availability: availability(raw.state),
        health,
        health_reason,
        controls,
        readings,
        vectors,
        listed: true,
        freshness: Freshness::Live,
        gone,
    }
}

/// The three-axis vectors the items name: items that share a group are its axes.
fn vectors_of(readings: &[Reading]) -> Vec<Vector> {
    let mut vectors: Vec<Vector> = Vec::new();
    for reading in readings {
        let (Reported::Value(group), Reported::Value(axis)) = (&reading.group, &reading.axis)
        else {
            continue;
        };
        match vectors.iter_mut().find(|vector| &vector.name == group) {
            Some(vector) => vector.axes.push((axis.clone(), reading.name.clone())),
            None => vectors.push(Vector {
                name: group.clone(),
                axes: vec![(axis.clone(), reading.name.clone())],
            }),
        }
    }
    vectors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, unit: &str, value: Option<Scalar>) -> RawField {
        RawField {
            name: name.to_owned(),
            unit: unit.to_owned(),
            value,
        }
    }

    fn device(topics: Vec<RawTopic>, labels: &[(&str, &str)]) -> RawDevice {
        RawDevice {
            resource: "lemnos.probe".into(),
            state: ResourceState::Ready,
            labels: labels
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            topics,
        }
    }

    fn values(fields: Vec<RawField>) -> RawTopic {
        RawTopic {
            name: "lemnos.probe.values".into(),
            live: TopicLive::Live,
            fields,
        }
    }

    fn status(health: &str, reason: &str) -> RawTopic {
        RawTopic {
            name: "lemnos.probe.status".into(),
            live: TopicLive::Live,
            fields: vec![
                field("state", "", Some(Scalar::Text("ready".into()))),
                field("health", "", Some(Scalar::Text(health.into()))),
                field("reason", "", Some(Scalar::Text(reason.into()))),
            ],
        }
    }

    #[test]
    fn a_missing_unit_reads_as_not_reported() {
        let topic = values(vec![
            field("speed", "m/s", Some(Scalar::Number(2.5))),
            field("level", "", Some(Scalar::Number(0.5))),
        ]);
        let described = describe(device(
            vec![topic],
            &[
                ("lemnos.item.speed", "reading=number;unit=m/s;exponent=0"),
                ("lemnos.item.level", "reading=number;exponent=0"),
            ],
        ));
        assert_eq!(described.readings[0].unit, Reported::Value("m/s".into()));
        assert_eq!(described.readings[1].unit, Reported::NotReported);
        assert_eq!(described.readings[1].value, Some(Scalar::Number(0.5)));
    }

    #[test]
    fn the_label_falls_back_to_the_name_and_is_not_reported_without_one() {
        let bare = describe(device(Vec::new(), &[]));
        assert_eq!(bare.label, Reported::NotReported);
        assert_eq!(bare.shown_name(), "lemnos.probe");
        let named = describe(device(Vec::new(), &[(LABEL_KEY, "Left fan")]));
        assert_eq!(named.shown_name(), "Left fan");
    }

    #[test]
    fn a_state_word_is_a_state_and_an_unknown_word_is_not_reported() {
        let retrying = describe(device(
            Vec::new(),
            &[(STATE_KEY, "retrying"), (REASON_KEY, "sensor drifting")],
        ));
        assert_eq!(retrying.state, Reported::Value(DeviceState::Retrying));
        assert_eq!(
            retrying.state_reason,
            Reported::Value("sensor drifting".into())
        );
        let odd = describe(device(Vec::new(), &[(STATE_KEY, "napping")]));
        assert_eq!(odd.state, Reported::NotReported);
    }

    #[test]
    fn the_address_reads_decimal_or_hex_and_is_not_reported_otherwise() {
        assert_eq!(
            describe(device(Vec::new(), &[(ADDRESS_KEY, "0x44")])).address,
            Reported::Value(0x44)
        );
        assert_eq!(
            describe(device(Vec::new(), &[(ADDRESS_KEY, "68")])).address,
            Reported::Value(68)
        );
        assert_eq!(
            describe(device(Vec::new(), &[(ADDRESS_KEY, "x")])).address,
            Reported::NotReported
        );
        assert_eq!(
            describe(device(Vec::new(), &[])).address,
            Reported::NotReported
        );
    }

    #[test]
    fn contracts_carry_their_kinds_and_a_kind_is_not_reported_when_missing() {
        let described = describe(device(
            Vec::new(),
            &[
                (TAG_KEY, "a.thing/1"),
                (TAG_KEY, "b.other/1"),
                ("lemnos.kind.a.thing/1", "sensor"),
            ],
        ));
        assert_eq!(
            described.contracts,
            vec![
                Contract {
                    tag: "a.thing/1".into(),
                    kind: Reported::Value("sensor".into())
                },
                Contract {
                    tag: "b.other/1".into(),
                    kind: Reported::NotReported
                },
            ]
        );
    }

    #[test]
    fn an_item_block_scales_its_figures_by_its_exponent() {
        let topic = values(vec![field("speed", "m/s", Some(Scalar::Number(2.5)))]);
        let described = describe(device(
            vec![topic],
            &[(
                "lemnos.item.speed",
                "reading=range;unit=m/s;exponent=-1;min=0;max=300;step=1;warn_high=250;settable=true;setter=set the speed",
            )],
        ));
        let item = &described.readings[0];
        assert_eq!(item.reading, Reported::Value(ReadingKind::Range));
        assert_eq!(
            item.range,
            Reported::Value(Range {
                min: 0.0,
                max: 30.0
            })
        );
        assert_eq!(item.step, Reported::Value(0.1));
        assert_eq!(item.warn_high, Reported::Value(25.0));
        assert_eq!(item.warn_low, Reported::NotReported);
        assert_eq!(item.settable, Reported::Value(true));
        assert_eq!(item.setter, Reported::Value("set the speed".into()));
    }

    #[test]
    fn an_item_with_no_block_has_every_figure_unknown() {
        let described = describe(device(
            vec![values(vec![field("count", "", Some(Scalar::Number(3.0)))])],
            &[],
        ));
        let item = &described.readings[0];
        assert_eq!(item.reading, Reported::NotReported);
        assert_eq!(item.unit, Reported::NotReported);
        assert_eq!(item.range, Reported::NotReported);
        assert_eq!(item.settable, Reported::NotReported);
        assert_eq!(item.value, Some(Scalar::Number(3.0)));
    }

    #[test]
    fn items_that_share_a_group_are_one_vector_by_axis() {
        let topic = values(vec![
            field("ax", "m/s^2", Some(Scalar::Number(1.0))),
            field("ay", "m/s^2", Some(Scalar::Number(2.0))),
        ]);
        let described = describe(device(
            vec![topic],
            &[
                (
                    "lemnos.item.ax",
                    "reading=three axes;group=accel;axis=x;exponent=0",
                ),
                (
                    "lemnos.item.ay",
                    "reading=three axes;group=accel;axis=y;exponent=0",
                ),
            ],
        ));
        assert_eq!(
            described.vectors,
            vec![Vector {
                name: "accel".into(),
                axes: vec![("x".into(), "ax".into()), ("y".into(), "ay".into())],
            }]
        );
    }

    #[test]
    fn health_and_its_reason_come_from_the_status_topic_and_an_empty_reason_is_not_reported() {
        let described = describe(device(vec![status("degraded", "")], &[]));
        assert_eq!(described.health, Reported::Value("degraded".into()));
        assert_eq!(described.health_reason, Reported::NotReported);
        assert!(described.readings.is_empty(), "the status is not an item");
    }

    #[test]
    fn a_gone_publisher_makes_the_device_gone_and_a_control_is_named_from_its_label() {
        let mut topic = status("ok", "");
        topic.live = TopicLive::Gone;
        let described = describe(device(
            vec![topic],
            &[(&format!("{CONTROL_PREFIX}set-speed"), "f32")],
        ));
        assert!(described.gone);
        assert_eq!(
            described.controls,
            vec![Control {
                name: "set-speed".into(),
                takes: "f32".into()
            }]
        );
    }
}
