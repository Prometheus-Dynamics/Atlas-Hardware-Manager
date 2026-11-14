//! The description reader against Lemnos's own vectors, and against the simulated node's encoder
//! (`atlas_sim_node::describe`, a mirror of Lemnos's encoder at e5c0ff0e).
//!
//! Where the inputs come from:
//! - `fixtures/lemnos-tracked.txt`: the `tracked` device as Lemnos's `call describe` answers it. The
//!   method and contract lines are the real run quoted in `/mnt/nvme-wd/lemnos-gates/I6g-answers.md`
//!   (Lemnos's I6h branch, before the merge), with the `describe 1` line and the `generation=1` of
//!   e5c0ff0e's device record, which Lemnos's own test asserts (`call_describe_answers_the_device_
//!   description_as_text`, `service/orion/tests/orion_bridge.rs`, e5c0ff0e).
//! - `HARD`: the labels and reasons of Lemnos's `text_round_trips_through_the_reader_whatever_it_holds`
//!   (`service/orion/src/description.rs`, e5c0ff0e), with their encoder forms.
//! - `LEVEL_ITEM`: the item of Lemnos's `an_item_record_carries_its_real_range_or_none`, with the
//!   record the encoder writes for it.

use atlas_sim_node::describe::{
    Contract, Device, Item, ItemKind as SimItemKind, Method, Reading, UnitState as SimUnitState,
    quote, render,
};
use atlas_topics::description::parse;
use atlas_topics::{ItemKind, UnitState};

const TRACKED: &str = include_str!("fixtures/lemnos-tracked.txt");
/// The same device from Lemnos's MERGED build with the lease in force (I6g ca0c7ee1, I6g2 ab31079d),
/// as the coordinator sent it on 2026-11-13: the device record carries `lease_seconds=60`, and the
/// contract has five methods.
const TRACKED_LEASED: &str = include_str!("fixtures/lemnos-tracked-leased.txt");

const HARD: &[&str] = &[
    "plain",
    "two words",
    "a=b",
    "say \"hi\"",
    "back\\slash",
    "line one\nline two",
    "tab\there",
    "carriage\rreturn",
    "none",
    "ünïcødé ✓ 日本語",
    "ends with backslash \\",
    "",
];

/// Lemnos's `item(Some(-1.5), None, Some(0.25))` at e5c0ff0e, as its encoder writes the record.
const LEVEL_ITEM: &str = "item kind=value name=\"level\" unit=\"m\" unit_state=known exponent=-2 min=0 max=100 real_min=-1.5 real_max=none real_step=0.25 step=1 warn_low=unknown warn_high=unknown contract=0 method=1 field=none reading=number group=none";

#[test]
fn lemnos_tracked_device_reads_as_its_test_asserts_it() {
    let description = parse(TRACKED).expect("Lemnos's own output reads");
    let device = &description.device;
    assert_eq!(device.label.as_deref(), Some("tracked"));
    assert_eq!(device.generation, Some(1));
    assert_eq!(device.kind.as_deref(), Some("other"));
    // Quoted empty text is text (the model reads it as not reported); the bare `none` is absent.
    assert_eq!(device.bus.as_deref(), Some(""));
    assert_eq!(device.address, None, "address=none");
    assert_eq!(device.location.as_deref(), Some(""));
    assert_eq!(device.state, Some(atlas_topics::DeviceState::Running));
    assert_eq!(device.reason.as_deref(), Some(""));
    assert_eq!(description.contracts.len(), 1);
    let contract = &description.contracts[0];
    assert_eq!(contract.tag.as_deref(), Some("tracked/1"));
    assert_eq!(contract.hash, Some(0x8a02_b467_18fd_879c));
    assert_eq!(contract.methods, Some(4));
    let hold = description
        .methods
        .iter()
        .find(|method| method.id == 4)
        .expect("the long job");
    assert_eq!(hold.name.as_deref(), Some("hold_until_cancelled"));
    assert_eq!(hold.job, Some(true));
    assert_eq!(description.methods.len(), 4);
    assert!(description.items.is_empty(), "the run quoted none");
    assert_eq!(
        (description.unknown_records, description.unknown_keys),
        (0, 0)
    );
}

#[test]
fn every_lemnos_label_round_trips_through_the_reader_and_the_encoder() {
    for label in HARD {
        let text = render(
            &Device {
                label: (*label).to_owned(),
                ..empty_device()
            },
            Some(7),
        );
        let description = parse(&text).unwrap_or_else(|refusal| panic!("{label:?}: {refusal}"));
        assert_eq!(
            description.device.label.as_deref(),
            Some(*label),
            "label {label:?}"
        );
        assert_eq!(description.device.generation, Some(7));
        let quoted_back = quote(label);
        assert!(
            text.contains(&format!("label={quoted_back} ")),
            "the quoted form is written as the encoder writes it: {quoted_back}"
        );
    }
}

#[test]
fn the_real_range_of_lemnos_item_reads_as_the_encoder_writes_it() {
    let text = format!(
        "describe 1\ndevice\ncontract index=0 tag=\"x/1\" kind=\"other\" hash=0x1 methods=1\nmethod contract=0 id=1 name=\"take\" params=\"\" returns=\"()\" out_buffer=false job=false\n{LEVEL_ITEM}"
    );
    let description = parse(&text).expect("reads");
    let item = &description.items[0];
    assert_eq!(item.kind, Some(ItemKind::Value));
    assert_eq!(item.name.as_deref(), Some("level"));
    assert_eq!(item.unit.as_deref(), Some("m"));
    assert_eq!(item.unit_state, Some(UnitState::Known));
    assert_eq!(item.exponent, Some(-2));
    assert_eq!((item.min, item.max), (Some(0), Some(100)));
    assert_eq!(item.real_min, Some(-1.5));
    assert_eq!(item.real_max, None, "real_max=none");
    assert_eq!(item.real_step, Some(0.25));
    assert_eq!(
        (item.warn_low, item.warn_high),
        (None, None),
        "unknown figures"
    );
    assert_eq!((item.contract, item.method), (Some(0), Some(1)));
    assert_eq!(item.field, None);
    assert_eq!(item.reading, Some(atlas_topics::ReadingKind::Number));
    assert_eq!(item.group, None);
}

#[test]
fn a_whole_description_the_encoder_writes_reads_back_as_it_was_written() {
    let device = Device {
        label: "Motion \"unit\"\\".to_owned(),
        kind: "motion".to_owned(),
        bus: "i2c 1".to_owned(),
        address: Some(0x68),
        location: "/sys/x y".to_owned(),
        state: Some(1),
        reason: "sensor drifting".to_owned(),
        contracts: vec![Contract {
            tag: "sim.motion/1".to_owned(),
            kind: "motion".to_owned(),
            hash: 0xabc,
            methods: vec![Method {
                id: 5,
                name: "position".to_owned(),
                params: vec![("mm".to_owned(), "f64".to_owned())],
                returns: "()".to_owned(),
                out_buffer: false,
                job: true,
            }],
        }],
        items: vec![Item {
            kind: SimItemKind::Control,
            name: "position".to_owned(),
            unit: "mm".to_owned(),
            unit_state: SimUnitState::Known,
            exponent: 0,
            min: Some(0),
            max: Some(100),
            real_min: Some(0.5),
            real_max: Some(99.5),
            real_step: None,
            step: Some(5),
            warn_low: None,
            warn_high: Some(90),
            contract: Some(0),
            method: Some(5),
            field: Some("position".to_owned()),
            reading: Some(Reading::Range),
            group: Some("accel".to_owned()),
        }],
    };
    let description = parse(&render(&device, Some(3))).expect("reads");
    assert_eq!(
        description.device.label.as_deref(),
        Some("Motion \"unit\"\\")
    );
    assert_eq!(description.device.address, Some(0x68));
    assert_eq!(description.device.location.as_deref(), Some("/sys/x y"));
    assert_eq!(
        description.device.reason.as_deref(),
        Some("sensor drifting")
    );
    assert_eq!(description.device.generation, Some(3));
    assert_eq!(description.contracts[0].methods, Some(1));
    assert_eq!(description.methods[0].params.as_deref(), Some("mm:f64"));
    assert_eq!(description.methods[0].job, Some(true));
    let item = &description.items[0];
    assert_eq!(item.kind, Some(ItemKind::Control));
    assert_eq!(item.real_step, None, "real_step=none");
    assert_eq!(item.warn_low, None);
    assert_eq!(item.warn_high, Some(90));
    assert_eq!(item.group.as_deref(), Some("accel"));
    assert_eq!(item.field.as_deref(), Some("position"));
}

#[test]
fn the_merged_builds_reply_with_a_lease_is_read_whole() {
    let description =
        parse(TRACKED_LEASED).expect("no line of the merged build's reply is refused");
    let device = &description.device;
    assert_eq!(device.generation, Some(1));
    assert_eq!(device.lease_seconds, Some(60));
    assert_eq!(device.label.as_deref(), Some("tracked"));
    assert_eq!(device.address, None, "address=none");
    assert_eq!(device.state, Some(atlas_topics::DeviceState::Running));
    assert_eq!(device.reason.as_deref(), Some(""));
    let contract = &description.contracts[0];
    assert_eq!(contract.hash, Some(0x8370_c24c_2e97_124e));
    // `methods=` is a count, and the method records that follow are that many.
    assert_eq!(contract.methods, Some(5));
    assert_eq!(description.methods.len(), 5);
    let jobs: Vec<_> = description
        .methods
        .iter()
        .filter(|method| method.job == Some(true))
        .filter_map(|method| method.name.as_deref())
        .collect();
    assert_eq!(jobs, ["hold_until_cancelled", "count_until_cancelled"]);
    assert_eq!(
        (description.unknown_records, description.unknown_keys),
        (0, 0),
        "every record and key of the reply is known to the reader"
    );
}

#[test]
fn lemnos_refusals_are_by_line_and_never_partial() {
    let refusal = parse(&TRACKED.replacen("describe 1", "describe 2", 1)).unwrap_err();
    assert!(refusal.reason.contains("version 2"), "{refusal}");
    let malformed = TRACKED.replacen("methods=4", "methods=\"4", 1);
    let refusal = parse(&malformed).unwrap_err();
    assert_eq!(refusal.line, 3, "{refusal}");
}

fn empty_device() -> Device {
    Device {
        label: String::new(),
        kind: String::new(),
        bus: String::new(),
        address: None,
        location: String::new(),
        state: None,
        reason: String::new(),
        contracts: Vec::new(),
        items: Vec::new(),
    }
}
