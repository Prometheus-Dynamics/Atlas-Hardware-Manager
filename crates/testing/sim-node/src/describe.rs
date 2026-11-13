//! The text a simulated device answers `call describe` with: a test-side mirror of Lemnos's
//! encoder (`service/orion/src/description.rs` at e5c0ff0e, the I6h commit), written from it.
//!
//! The encoder is Lemnos's, and this crate does not depend on Lemnos: the key order, the words,
//! the escapes and the absent-value forms below are copied from that file, and the tests check
//! them against the vectors Lemnos's own tests use (see the tests at the end of this module and
//! `crates/board/topics/tests/description.rs`). Where Lemnos's documentation and its encoder
//! differ, the encoder is followed here and the difference is recorded in `docs/prs/C11.md`.
//!
//! Absent values are written as Lemnos writes them: a figure the device does not give is
//! `unknown`, an option that is not set (a real range, a field, a group) is `none`, and the text
//! `none` is quoted so it is never the option.

use std::fmt::Write as _;

// LOCK: a mirror of Lemnos's encoder at e5c0ff0e (I6h). Re-check it against the encoder at the lock
// move that takes I6h, since this crate does not depend on Lemnos.
/// The first line of every description: the format's version.
pub const VERSION_LINE: &str = "describe 1";

/// What an item is: a value, a control (set through its method) or an action (run).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A value the device reports.
    Value,
    /// A control: set through its method.
    Control,
    /// An action: run, with no value.
    Action,
}

/// An item's unit state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    /// The unit is the string the description gives.
    Known,
    /// No unit applies.
    Dimensionless,
    /// The device cannot say what its unit is.
    Unknown,
}

/// What an item's reading is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    /// A number.
    Number,
    /// A number with a range.
    Range,
    /// On or off.
    Switch,
    /// Text.
    Text,
    /// An array of colour components.
    ColourArray,
    /// One axis of a three-axis vector.
    ThreeAxes,
}

/// A method of a contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    /// Its id within its contract.
    pub id: u32,
    /// Its name.
    pub name: String,
    /// Its parameters: name and shape.
    pub params: Vec<(String, String)>,
    /// The shape it returns.
    pub returns: String,
    /// Whether it returns through an out buffer.
    pub out_buffer: bool,
    /// Whether it is a long job.
    pub job: bool,
}

/// A contract a device serves, with its methods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    /// Its tag, for example `sim.vane/1`.
    pub tag: String,
    /// Its kind.
    pub kind: String,
    /// Its hash.
    pub hash: u64,
    /// Its methods.
    pub methods: Vec<Method>,
}

/// An item of a device.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Whether it is a value, a control or an action.
    pub kind: ItemKind,
    /// Its name.
    pub name: String,
    /// Its unit, as text. Empty when none.
    pub unit: String,
    /// Its unit state.
    pub unit_state: UnitState,
    /// The power of ten its integer figures are scaled by.
    pub exponent: i8,
    /// The least and greatest raw value.
    pub min: Option<i64>,
    /// See `min`.
    pub max: Option<i64>,
    /// The least real value in the unit.
    pub real_min: Option<f64>,
    /// The greatest real value in the unit.
    pub real_max: Option<f64>,
    /// The real step in the unit.
    pub real_step: Option<f64>,
    /// The raw step.
    pub step: Option<i64>,
    /// The raw lower warning limit.
    pub warn_low: Option<i64>,
    /// The raw upper warning limit.
    pub warn_high: Option<i64>,
    /// The contract that serves it, by index.
    pub contract: Option<u32>,
    /// The method it is read or set through, by id.
    pub method: Option<u32>,
    /// The topic field its value is published in.
    pub field: Option<String>,
    /// What the reading is.
    pub reading: Option<Reading>,
    /// The group it is one part of.
    pub group: Option<String>,
}

/// A device's description, as the node holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// The board's name for the device. Empty when none.
    pub label: String,
    /// The grouping its contracts give it. Empty when none.
    pub kind: String,
    /// The bus it is on. Empty when none.
    pub bus: String,
    /// The 7-bit address, when the driver names one.
    pub address: Option<u8>,
    /// Where the device is, in words. Empty when none.
    pub location: String,
    /// `0` running, `1` retrying, `2` off; `None` when unknown.
    pub state: Option<u8>,
    /// Why it is retrying or off. Empty when none.
    pub reason: String,
    /// Its contracts.
    pub contracts: Vec<Contract>,
    /// Its items, in order.
    pub items: Vec<Item>,
}

/// Text in the format's quotes: `\\`, `\"`, `\n`, `\r` and `\t` are escaped; every other character,
/// non-ASCII text included, is written as it is.
#[must_use]
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn state_name(state: Option<u8>) -> &'static str {
    match state {
        Some(0) => "running",
        Some(1) => "retrying",
        Some(2) => "off",
        _ => "unknown",
    }
}

fn reading_name(reading: Option<Reading>) -> &'static str {
    match reading {
        Some(Reading::Number) => "number",
        Some(Reading::Range) => "range",
        Some(Reading::Switch) => "switch",
        Some(Reading::Text) => "text",
        Some(Reading::ColourArray) => "colour_array",
        Some(Reading::ThreeAxes) => "three_axes",
        None => "unknown",
    }
}

/// The text of a description: its version line, then its records, with no final line break.
/// `generation` is the device's description generation (`None` before the device has one).
#[must_use]
pub fn render(device: &Device, generation: Option<u64>) -> String {
    let mut out = String::from(VERSION_LINE);
    let generation = generation.map_or_else(|| "unknown".to_owned(), |g| g.to_string());
    let address = device
        .address
        .map_or_else(|| "none".to_owned(), |a| format!("{a:#04x}"));
    let _ = write!(
        out,
        "\ndevice generation={generation} label={} kind={} bus={} address={address} location={} state={} reason={}",
        quote(&device.label),
        quote(&device.kind),
        quote(&device.bus),
        quote(&device.location),
        state_name(device.state),
        quote(&device.reason),
    );
    for (index, contract) in device.contracts.iter().enumerate() {
        let _ = write!(
            out,
            "\ncontract index={index} tag={} kind={} hash={:#018x} methods={}",
            quote(&contract.tag),
            quote(&contract.kind),
            contract.hash,
            contract.methods.len(),
        );
    }
    for (index, contract) in device.contracts.iter().enumerate() {
        for method in &contract.methods {
            let params: Vec<String> = method
                .params
                .iter()
                .map(|(name, shape)| format!("{name}:{shape}"))
                .collect();
            let _ = write!(
                out,
                "\nmethod contract={index} id={} name={} params={} returns={} out_buffer={} job={}",
                method.id,
                quote(&method.name),
                quote(&params.join(" ")),
                quote(&method.returns),
                method.out_buffer,
                method.job,
            );
        }
    }
    for item in &device.items {
        item_record(&mut out, item);
    }
    out
}

fn item_record(out: &mut String, item: &Item) {
    let kind = match item.kind {
        ItemKind::Value => "value",
        ItemKind::Control => "control",
        ItemKind::Action => "action",
    };
    let unit_state = match item.unit_state {
        UnitState::Known => "known",
        UnitState::Dimensionless => "dimensionless",
        UnitState::Unknown => "unknown",
    };
    let figure = |value: Option<i64>| value.map_or_else(|| "unknown".to_owned(), |v| v.to_string());
    let real = |value: Option<f64>| value.map_or_else(|| "none".to_owned(), |v| v.to_string());
    let text = |value: Option<&str>| value.map_or_else(|| "none".to_owned(), quote);
    let contract = item
        .contract
        .map_or_else(|| "unknown".to_owned(), |c| c.to_string());
    let method = item
        .method
        .map_or_else(|| "unknown".to_owned(), |m| m.to_string());
    let _ = write!(
        out,
        "\nitem kind={kind} name={} unit={} unit_state={unit_state} exponent={} min={} max={} real_min={} real_max={} real_step={} step={} warn_low={} warn_high={} contract={contract} method={method} field={} reading={} group={}",
        quote(&item.name),
        quote(&item.unit),
        item.exponent,
        figure(item.min),
        figure(item.max),
        real(item.real_min),
        real(item.real_max),
        real(item.real_step),
        figure(item.step),
        figure(item.warn_low),
        figure(item.warn_high),
        text(item.field.as_deref()),
        reading_name(item.reading),
        text(item.group.as_deref()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Labels and reasons that a naive format would break. Lemnos's `HARD` vector, in
    /// `service/orion/src/description.rs` (e5c0ff0e, its tests), with the quoted form the encoder's
    /// `quote` gives each one, worked out from that function.
    const HARD: &[(&str, &str)] = &[
        ("plain", r#""plain""#),
        ("two words", r#""two words""#),
        ("a=b", r#""a=b""#),
        ("say \"hi\"", r#""say \"hi\"""#),
        ("back\\slash", r#""back\\slash""#),
        ("line one\nline two", r#""line one\nline two""#),
        ("tab\there", r#""tab\there""#),
        ("carriage\rreturn", r#""carriage\rreturn""#),
        ("none", r#""none""#),
        ("ünïcødé ✓ 日本語", "\"ünïcødé ✓ 日本語\""),
        ("ends with backslash \\", r#""ends with backslash \\""#),
        ("", r#""""#),
    ];

    #[test]
    fn every_lemnos_vector_quotes_as_the_encoder_quotes_it() {
        for (text, quoted) in HARD {
            assert_eq!(quote(text), *quoted, "{text:?}");
        }
    }

    fn item(name: &str) -> Item {
        Item {
            kind: ItemKind::Value,
            name: name.to_owned(),
            unit: "m".to_owned(),
            unit_state: UnitState::Known,
            exponent: -2,
            min: Some(0),
            max: Some(100),
            real_min: Some(-1.5),
            real_max: None,
            real_step: Some(0.25),
            step: Some(1),
            warn_low: None,
            warn_high: None,
            contract: Some(0),
            method: Some(1),
            field: None,
            reading: Some(Reading::Number),
            group: None,
        }
    }

    /// Lemnos's `item_record` vectors: `real_min`, `real_max` and `real_step` are decimal or
    /// `none`, and the integer figures are `unknown` when absent.
    #[test]
    fn an_item_record_writes_the_real_range_or_none_as_lemnos_does() {
        let device = Device {
            label: String::new(),
            kind: String::new(),
            bus: String::new(),
            address: None,
            location: String::new(),
            state: None,
            reason: String::new(),
            contracts: Vec::new(),
            items: vec![item("level")],
        };
        let text = render(&device, None);
        let line = text.lines().last().expect("the item record");
        assert!(
            line.contains(" real_min=-1.5 real_max=none real_step=0.25 "),
            "{line}"
        );
        assert!(line.contains(" min=0 max=100 "), "{line}");
        assert!(
            line.contains(" warn_low=unknown warn_high=unknown "),
            "{line}"
        );
        assert!(
            line.contains(" contract=0 method=1 field=none reading=number group=none"),
            "{line}"
        );
    }

    #[test]
    fn a_device_record_writes_generation_unknown_before_there_is_one() {
        let device = Device {
            label: "none".to_owned(),
            kind: String::new(),
            bus: String::new(),
            address: Some(0x44),
            location: String::new(),
            state: Some(1),
            reason: "sensor drifting".to_owned(),
            contracts: Vec::new(),
            items: Vec::new(),
        };
        let text = render(&device, None);
        assert_eq!(
            text,
            "describe 1\ndevice generation=unknown label=\"none\" kind=\"\" bus=\"\" address=0x44 location=\"\" state=retrying reason=\"sensor drifting\""
        );
    }
}
