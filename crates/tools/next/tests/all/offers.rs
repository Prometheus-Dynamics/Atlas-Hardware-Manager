//! `atlas-next offers <address> --out <file>`: the bench record of what a board's node offers, run through
//! the same `execute` the binary runs, against a simulated board on this machine's loopback.

use crate::common;
use atlas_sim_node::SimNode;
use common::{env, identity, run, scratch, start_node};
use serde_json::Value;

fn direct(node: &SimNode) -> String {
    format!("orion+tcp://127.0.0.1:{}", node.network().port())
}

/// Reads the file the command wrote as one JSON document.
fn read_record(path: &std::path::Path) -> (String, Value) {
    let text = std::fs::read_to_string(path).expect("the record is written");
    let value = serde_json::from_str(&text).expect("the record is one JSON document");
    (text, value)
}

/// The record names the node's facts, every device's describe, and the status topics with their values;
/// two runs on the same board give the same bytes; the keys are sorted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pinned_board_is_recorded_once_and_the_same_twice() {
    let dir = scratch("offers");
    let node = start_node(&dir, None);
    let address = direct(&node);
    let board = identity(&node).await;
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    let first = dir.join("first.json");
    let (code, out) = run(
        &env,
        &["offers", &address, "--out", first.to_str().expect("a path")],
    )
    .await;
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("facts") && out.contains("errors"), "{out}");

    let (text, record) = read_record(&first);
    let facts = record["node"]["facts"].as_array().expect("the facts");
    let family = facts
        .iter()
        .find(|fact| fact["name"] == "family")
        .expect("the family fact is recorded");
    assert_eq!(family["text"], common::BOARD, "{family}");
    assert_eq!(record["node"]["fingerprint"], board.fingerprint.as_str());

    let resources = record["resources"].as_array().expect("the resources");
    let devices: Vec<&Value> = resources
        .iter()
        .filter(|resource| resource["kind"] == "lemnos.device")
        .collect();
    assert!(!devices.is_empty(), "{record}");
    // A device that is off refuses its describe: recorded as that error, and the rest still described.
    let mut described = 0;
    for device in &devices {
        match device["describe"].as_str() {
            Some(describe) => {
                assert!(describe.starts_with("describe 1"), "{describe}");
                described += 1;
            }
            None => assert!(device["describe"]["error"].is_string(), "{device}"),
        }
    }
    assert!(described > 0, "some devices are described: {record}");
    let off = devices
        .iter()
        .find(|device| device["name"] == "lemnos.off0")
        .expect("the off device is listed");
    assert!(off["describe"]["error"].is_string(), "{off}");

    let topics = record["topics"]["names"]
        .as_array()
        .expect("the topic names");
    assert!(
        topics
            .iter()
            .any(|name| name.as_str().is_some_and(|n| n.ends_with(".status"))),
        "{record}"
    );
    let status = record["topics"]["status"]
        .as_object()
        .expect("the status values");
    assert!(!status.is_empty(), "{record}");

    let second = dir.join("second.json");
    let (code, out) = run(
        &env,
        &[
            "offers",
            &address,
            "--out",
            second.to_str().expect("a path"),
        ],
    )
    .await;
    assert_eq!(code, 0, "{out}");
    let (again, _) = read_record(&second);
    assert_eq!(text, again, "two runs on one board give the same bytes");

    let sorted = serde_json::to_string_pretty(&sorted_copy(&record)).expect("pretty") + "\n";
    assert_eq!(
        text, sorted,
        "keys are sorted, pretty-printed, one trailing newline"
    );
}

/// Every describe refused: the command goes on, each device says why, and it still exits 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_describe_is_recorded_as_an_error_and_the_command_goes_on() {
    let dir = scratch("offers-refused");
    let node = start_node(&dir, None);
    let address = direct(&node);
    let _ = identity(&node).await;
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    node.refuse_describe(true);
    let file = dir.join("refused.json");
    let (code, out) = run(
        &env,
        &["offers", &address, "--out", file.to_str().expect("a path")],
    )
    .await;
    assert_eq!(code, 0, "a written file exits 0: {out}");
    let (_, record) = read_record(&file);
    let devices: Vec<&Value> = record["resources"]
        .as_array()
        .expect("the resources")
        .iter()
        .filter(|resource| resource["kind"] == "lemnos.device")
        .collect();
    assert!(!devices.is_empty());
    for device in devices {
        assert!(device["describe"]["error"].is_string(), "{device}");
    }
    assert!(out.contains("errors"), "{out}");
}

/// A board never pinned is refused as every command is, and no file is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unpinned_board_writes_nothing() {
    let dir = scratch("offers-unpinned");
    let node = start_node(&dir, None);
    let address = direct(&node);
    let env = env(&dir.join("state"));
    let file = dir.join("unpinned.json");
    let (code, out) = run(
        &env,
        &["offers", &address, "--out", file.to_str().expect("a path")],
    )
    .await;
    assert_eq!(code, 3, "{out}");
    assert!(!file.exists(), "nothing written for a board never pinned");
}

/// The usage: `--out` is needed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offers_without_out_is_a_usage_error() {
    let dir = scratch("offers-usage");
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["offers", "orion+tcp://127.0.0.1:9"]).await;
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--out"), "{out}");
}

/// The record's keys in sorted order, recursively, as a fresh value (for comparing the bytes).
fn sorted_copy(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                out.insert(key.clone(), sorted_copy(&map[key]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_copy).collect()),
        other => other.clone(),
    }
}
