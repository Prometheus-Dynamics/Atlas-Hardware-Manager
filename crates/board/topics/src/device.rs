//! A device as the node describes it, and what it reads now.
//!
//! A node lists each device as a resource (Orion's provider API): its name, a kind and labels,
//! and its topics by name. The Lemnos bridge names a device's resource `lemnos.<device>`, labels
//! its driver `lemnos.driver` and each control `lemnos.control.<item>`; its values are the
//! topics that are not its status, and its status is the topic `<resource>.status`, with the
//! fields `state`, `health` and `reason`. Those names are the bridge's, read here as data; no
//! kind of device is matched.
//!
//! What the description does not give is [`Reported::NotReported`]: a human label, a grouping,
//! a unit. The bridge does not publish a label or a grouping yet (Lemnos V1 adds them), so every
//! device has neither.

use std::time::Instant;

use orion::provider::ResourceState;

use crate::value::{ReadingKind, Reported, Scalar};

/// The label that gives a device its human name, when a description gives one. Not published by
/// the bridge yet: until it is, every device reads [`Reported::NotReported`].
pub const LABEL_KEY: &str = "label";
/// The label that names a device's driver.
pub const DRIVER_KEY: &str = "lemnos.driver";
/// The prefix of a label that names one control: `lemnos.control.<item>`, valued with the shapes
/// it takes (empty for none).
pub const CONTROL_PREFIX: &str = "lemnos.control.";
/// The suffix of a device's status topic.
pub const STATUS_SUFFIX: &str = ".status";
/// The resource prefix of the devices a node lists (`lemnos.<device>`).
pub const RESOURCE_PREFIX: &str = "lemnos.";
/// The kind a device resource is written as.
pub const RESOURCE_KIND: &str = "lemnos.device";

/// Whether a device is listed by its provider, and what state it is in.
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

/// A control the device takes, as the description names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    /// Its item name.
    pub name: String,
    /// The shapes of the values it takes, as the description writes them. Empty for none.
    pub takes: String,
}

/// One reading of a device: a field of one of its topics.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// The field's name.
    pub name: String,
    /// How it is shown.
    pub kind: ReadingKind,
    /// Its unit, as the field declares it.
    pub unit: Reported<String>,
    /// The newest value, or `None` before the first sample.
    pub value: Option<Scalar>,
}

/// One device the node describes.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// Its resource name (`lemnos.<device>` on a Lemnos node).
    pub name: String,
    /// Its human name. Not reported until a description gives one.
    pub label: Reported<String>,
    /// The driver the node runs it with.
    pub driver: Reported<String>,
    /// The group it is shown in. Not reported: a description does not give groups yet.
    pub group: Reported<String>,
    /// Whether the provider has it ready, failed or gone.
    pub availability: Reported<Availability>,
    /// The device's own state, from its status topic.
    pub state: Reported<String>,
    /// The device's health, from its status topic.
    pub health: Reported<String>,
    /// Why the health is what it is. Not reported when the device gives no reason.
    pub reason: Reported<String>,
    /// The controls it takes.
    pub controls: Vec<Control>,
    /// Its readings, in the order its topics give them.
    pub readings: Vec<Reading>,
    /// Whether the node lists it now. `false` while it is held within the grace after the node
    /// stopped listing it.
    pub listed: bool,
    /// Whether its readings are current. Set when a snapshot is taken.
    pub freshness: Freshness,
    /// Whether one of its topics has lost its publisher.
    pub(crate) gone: bool,
}

/// What a topic's publisher is doing, as the subscription sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopicLive {
    /// Nobody has declared it yet.
    Waiting,
    /// A publisher is there.
    Live,
    /// The publisher is gone; what it published is still readable.
    Gone,
}

/// A field of a topic as read: its name, type, unit and newest value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawField {
    pub name: String,
    pub kind: ReadingKind,
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

fn label<'a>(labels: &'a [(String, String)], key: &str) -> Option<&'a str> {
    labels
        .iter()
        .find(|(label, _)| label == key)
        .map(|(_, value)| value.as_str())
}

fn text_of(field: &RawField) -> Reported<String> {
    match &field.value {
        Some(Scalar::Text(text)) => Reported::text(text),
        _ => Reported::NotReported,
    }
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

/// The device a resource and its topics describe. A device is listed and not gone until a
/// snapshot says otherwise.
pub(crate) fn describe(raw: RawDevice) -> Device {
    let label_text = label(&raw.labels, LABEL_KEY).map_or(Reported::NotReported, Reported::text);
    let driver = label(&raw.labels, DRIVER_KEY).map_or(Reported::NotReported, Reported::text);
    let controls = raw
        .labels
        .iter()
        .filter_map(|(key, takes)| {
            key.strip_prefix(CONTROL_PREFIX).map(|name| Control {
                name: name.to_owned(),
                takes: takes.clone(),
            })
        })
        .collect();

    let (mut state, mut health, mut reason) = (
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
    );
    let mut readings = Vec::new();
    let mut gone = false;
    for topic in raw.topics {
        gone |= topic.live == TopicLive::Gone;
        if topic.name.ends_with(STATUS_SUFFIX) {
            for field in &topic.fields {
                match field.name.as_str() {
                    "state" => state = text_of(field),
                    "health" => health = text_of(field),
                    "reason" => reason = text_of(field),
                    _ => {}
                }
            }
        } else {
            readings.extend(topic.fields.into_iter().map(|field| Reading {
                unit: Reported::text(&field.unit),
                name: field.name,
                kind: field.kind,
                value: field.value,
            }));
        }
    }

    Device {
        name: raw.resource,
        label: label_text,
        driver,
        group: Reported::NotReported,
        availability: availability(raw.state),
        state,
        health,
        reason,
        controls,
        readings,
        listed: true,
        freshness: Freshness::Live,
        gone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, unit: &str, value: Option<Scalar>) -> RawField {
        RawField {
            name: name.to_owned(),
            kind: ReadingKind::Number,
            unit: unit.to_owned(),
            value,
        }
    }

    fn device(topics: Vec<RawTopic>, labels: Vec<(String, String)>) -> RawDevice {
        RawDevice {
            resource: "lemnos.probe".into(),
            state: ResourceState::Ready,
            labels,
            topics,
        }
    }

    fn status(state: &str, health: &str, reason: &str) -> RawTopic {
        RawTopic {
            name: "lemnos.probe.status".into(),
            live: TopicLive::Live,
            fields: vec![
                RawField {
                    kind: ReadingKind::Text,
                    ..field("state", "", Some(Scalar::Text(state.into())))
                },
                RawField {
                    kind: ReadingKind::Text,
                    ..field("health", "", Some(Scalar::Text(health.into())))
                },
                RawField {
                    kind: ReadingKind::Text,
                    ..field("reason", "", Some(Scalar::Text(reason.into())))
                },
            ],
        }
    }

    #[test]
    fn a_missing_unit_reads_as_not_reported() {
        let topic = RawTopic {
            name: "lemnos.probe.values".into(),
            live: TopicLive::Live,
            fields: vec![
                field("speed", "m/s", Some(Scalar::Number(2.5))),
                field("level", "", Some(Scalar::Number(0.5))),
            ],
        };
        let described = describe(device(vec![topic], Vec::new()));
        assert_eq!(described.readings[0].unit, Reported::Value("m/s".into()));
        assert_eq!(described.readings[1].unit, Reported::NotReported);
        assert_eq!(described.readings[1].value, Some(Scalar::Number(0.5)));
    }

    #[test]
    fn a_human_label_and_a_group_are_not_reported_when_the_description_has_none() {
        let described = describe(device(
            Vec::new(),
            vec![(DRIVER_KEY.into(), "some-driver".into())],
        ));
        assert_eq!(described.label, Reported::NotReported);
        assert_eq!(described.group, Reported::NotReported);
        assert_eq!(described.driver, Reported::Value("some-driver".into()));
    }

    #[test]
    fn a_label_the_description_gives_is_its_human_name() {
        let described = describe(device(
            Vec::new(),
            vec![(LABEL_KEY.into(), "Left fan".into())],
        ));
        assert_eq!(described.label, Reported::Value("Left fan".into()));
    }

    #[test]
    fn health_and_its_reason_come_from_the_status_topic_and_an_empty_reason_is_not_reported() {
        let described = describe(device(vec![status("running", "degraded", "")], Vec::new()));
        assert_eq!(described.state, Reported::Value("running".into()));
        assert_eq!(described.health, Reported::Value("degraded".into()));
        assert_eq!(described.reason, Reported::NotReported);
        assert!(described.readings.is_empty(), "the status is not a reading");
    }

    #[test]
    fn a_gone_publisher_makes_the_device_gone_and_a_control_is_named_from_its_label() {
        let mut topic = status("gone", "ok", "");
        topic.live = TopicLive::Gone;
        let described = describe(device(
            vec![topic],
            vec![(format!("{CONTROL_PREFIX}set-speed"), "f32".into())],
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
