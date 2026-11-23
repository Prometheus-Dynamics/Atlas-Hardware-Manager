//! `atlas-next offers <address> --out <file.json>`: bench only. Records what a board's node offers, into one
//! JSON file the sim's fixture is made from: the hello facts as the session sees them, the node's resource
//! list (each device's describe as the bridge gives it), the topic names, the status topics' current values,
//! and the action names the resources list.
//!
//! Every read is a read: `with_client` and `with_bridge` on the pinned session, with the session's own call
//! wait. A read that fails is recorded in the file as `{"error": ...}` and the command goes on. The file is
//! one JSON document with its keys sorted, pretty-printed, with one trailing newline, so two runs on one board
//! give the same bytes (the node's own clock is left out for that reason).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use atlas_session::{Handle, lemnos as session_lemnos};
use orion::node::Node;
use orion::topics::{FieldId, FieldType, Status};
use orion::{Address, Orion};
use orion_msg_topics::Declare;
use orion_wire::Decode;
use serde_json::{Map, Value, json};

use crate::Env;
use crate::commands::{Refusal, load_pins};
use crate::link;
use crate::report::Report;

/// The suffix of a status topic's name: a device's health and reason (`lemnos.<device>.status`).
const STATUS_SUFFIX: &str = ".status";

/// The most a status topic is waited on for its fields' current values. A topic is read as soon as every
/// declared field has a sample; a field the node does not publish within this is recorded as `null`.
const SAMPLE_WAIT: Duration = Duration::from_secs(1);

/// The step at which a status topic's samples are pumped while waiting for them.
const SAMPLE_STEP: Duration = Duration::from_millis(20);

/// What one record holds, counted for the summary line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub facts: usize,
    pub resources: usize,
    pub described: usize,
    pub topics: usize,
    pub errors: usize,
}

/// The record of one board, as a JSON value (its keys are sorted when it is rendered).
#[derive(Debug)]
pub struct Offers {
    pub document: Value,
    pub summary: Summary,
}

/// Reads what the pinned node offers. `node` is the hello the session connected with.
pub async fn collect(handle: &Handle, node: &Node) -> Offers {
    let mut errors = 0;
    let mut described = 0;

    let listed = handle.with_client(|orion| orion.resources("")).await;
    let mut resource_count = 0;
    let mut topic_names = BTreeSet::new();
    let mut action_names = BTreeSet::new();
    let (resources, actions, topics) = match listed {
        Ok(mut found) => {
            found.sort_by(|a, b| a.name.cmp(&b.name));
            let mut resources = Vec::new();
            let mut status = Map::new();
            for resource in &found {
                let mut topics: Vec<String> = resource.topics.clone();
                topics.sort();
                let mut actions: Vec<String> = resource.actions.clone();
                actions.sort();
                topic_names.extend(topics.iter().cloned());
                action_names.extend(actions.iter().cloned());
                let mut object = json!({
                    "name": resource.name,
                    "kind": resource.kind,
                    "node": resource.node,
                    "state": format!("{:?}", resource.state),
                    "topics": topics,
                    "actions": actions,
                    "labels": resource.labels.iter().map(|(name, value)| json!({"name": name, "value": value})).collect::<Vec<_>>(),
                    "broken": resource.broken,
                });
                if resource.kind == session_lemnos::RESOURCE_KIND {
                    let name = resource.name.clone();
                    let answer = handle
                        .with_bridge(move |orion| session_lemnos::describe(orion, &name))
                        .await;
                    let describe = match answer {
                        Ok(Ok(bytes)) => {
                            described += 1;
                            describe_value(&bytes)
                        }
                        Ok(Err(failure)) => failed(failure.text().to_owned(), &mut errors),
                        Err(session) => failed(session.to_string(), &mut errors),
                    };
                    object["describe"] = describe;
                }
                resources.push(object);
            }
            for topic in topic_names
                .iter()
                .filter(|name| name.ends_with(STATUS_SUFFIX))
            {
                let name = topic.clone();
                let value = match handle
                    .with_client(move |orion| read_status(orion, &name))
                    .await
                {
                    Ok(value) => value,
                    Err(session) => failed(session.to_string(), &mut errors),
                };
                status.insert(topic.clone(), value);
            }
            resource_count = resources.len();
            let actions: Vec<String> = action_names.iter().cloned().collect();
            (json!(resources), actions, Value::Object(status))
        }
        Err(session) => (
            failed(session.to_string(), &mut errors),
            Vec::new(),
            Value::Object(Map::new()),
        ),
    };
    let topic_list: Vec<String> = topic_names.into_iter().collect();
    let facts = facts_value(node);
    let summary = Summary {
        facts: node.facts.len(),
        resources: resource_count,
        described,
        topics: topic_list.len(),
        errors,
    };
    let document = json!({
        "node": {
            "name": node.name,
            "cluster": node.cluster,
            "fingerprint": node.fingerprint,
            "public_key": hex(&node.public_key),
            "revision": node.revision,
            "facts": facts,
        },
        "image": image_value(node),
        "resources": resources,
        "actions": actions,
        "topics": {
            "names": topic_list,
            "status": topics,
        },
    });
    Offers { document, summary }
}

/// The record as text: keys sorted at every depth, pretty-printed, one trailing newline.
#[must_use]
pub fn render(document: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&sorted(document)).unwrap_or_default();
    text.push('\n');
    text
}

/// The same value with every object's keys in order.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), sorted(&map[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Runs `offers` on the pinned board at `address`, and writes its record to `out`.
pub async fn run<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    out: &Path,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let pins = load_pins(state)?;
    let text = address.to_string();
    let link = link::pinned(env, &pins, &text, address, key).await?;
    let offers = collect(&link.handle, &link.node).await;
    std::fs::write(out, render(&offers.document)).map_err(|why| {
        Refusal::new(
            1,
            format!("the record cannot be written to {}: {why}", out.display()),
        )
    })?;
    let summary = offers.summary;
    r.say(format!(
        "offers: {} facts, {} resources, {} devices described, {} topics, {} errors; written to {}",
        summary.facts,
        summary.resources,
        summary.described,
        summary.topics,
        summary.errors,
        out.display()
    ));
    r.field("facts", summary.facts);
    r.field("resources", summary.resources);
    r.field("described", summary.described);
    r.field("topics", summary.topics);
    r.field("errors", summary.errors);
    r.field("file", out.display().to_string());
    Ok(())
}

/// The hello facts, every one by name and value, sorted by name (then type, then value).
fn facts_value(node: &Node) -> Vec<Value> {
    let mut facts: Vec<&orion::node::Fact> = node.facts.iter().collect();
    facts.sort_by(|a, b| (&a.name, a.ty, &a.value).cmp(&(&b.name, b.ty, &b.value)));
    facts
        .into_iter()
        .map(|fact| {
            json!({
                "name": fact.name,
                "type": fact.ty,
                "text": std::str::from_utf8(&fact.value).ok(),
                "hex": hex(&fact.value),
            })
        })
        .collect()
}

/// The image's version and revs, from the hello facts that carry them. A fact the node does not publish
/// is `null`: nothing is invented.
fn image_value(node: &Node) -> Value {
    let text = |name: &str| {
        node.facts
            .iter()
            .find(|fact| fact.name == name)
            .and_then(|fact| std::str::from_utf8(&fact.value).ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    json!({
        "version": text("version.product"),
        "lemnos_rev": text("version.lemnos"),
        "orion_rev": text("version.orion"),
        "gaia_rev": text("version.gaia"),
    })
}

/// A device's describe reply: parsed as JSON when it is JSON, else kept as its text.
fn describe_value(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes).into_owned();
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

/// A failed read, recorded in the record as its words.
fn failed(words: String, errors: &mut usize) -> Value {
    *errors += 1;
    json!({ "error": words })
}

/// A status topic's current value: its state, and each field by its declared name. The topic is pumped
/// until every declared field has a sample, or until [`SAMPLE_WAIT`] has passed. A field with no sample by
/// then is `null`. Text is the text; any other type is its bytes in hex.
fn read_status(orion: &Orion, topic: &str) -> Result<Value, orion::Error> {
    let mut subscriber = orion.subscribe(topic)?;
    let started = std::time::Instant::now();
    let state = loop {
        orion.pump(SAMPLE_STEP)?;
        let state = match subscriber.status() {
            Status::Waiting => "waiting",
            Status::Live => "live",
            Status::PublisherGone => "gone",
        };
        if state != "waiting" || started.elapsed() >= SAMPLE_WAIT {
            break state;
        }
    };
    let declared: Vec<(String, FieldType)> = subscriber.with_declaration(|bytes| {
        bytes
            .and_then(|bytes| Declare::decode(bytes).ok())
            .map(|declare| {
                declare
                    .fields
                    .iter()
                    .flatten()
                    .map(|field| (field.name.to_owned(), field.ty))
                    .collect()
            })
            .unwrap_or_default()
    });
    let mut values: Vec<Option<Value>> = vec![None; declared.len()];
    loop {
        for (index, ((_, ty), slot)) in declared.iter().zip(values.iter_mut()).enumerate() {
            if slot.is_none() {
                *slot = subscriber
                    .latest(FieldId(index as u16))
                    .map(|sample| field_value(ty, sample.payload));
            }
        }
        let complete = values.iter().all(Option::is_some);
        if complete || started.elapsed() >= SAMPLE_WAIT {
            break;
        }
        orion.pump(SAMPLE_STEP)?;
    }
    let fields: Map<String, Value> = declared
        .into_iter()
        .zip(values)
        .map(|((name, _), value)| (name, value.unwrap_or(Value::Null)))
        .collect();
    Ok(json!({ "state": state, "fields": Value::Object(fields) }))
}

fn field_value(ty: &FieldType, payload: &[u8]) -> Value {
    if *ty == FieldType::Text {
        json!(String::from_utf8_lossy(payload))
    } else {
        json!({ "hex": hex(payload) })
    }
}

/// Lower-case hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{render, sorted};

    /// Keys come out sorted at every depth, whatever order they went in, and the text is the same each time.
    #[test]
    fn keys_are_sorted_at_every_depth_and_the_text_is_the_same_each_time() {
        let forward = json!({"b": {"y": 1, "x": [{"q": 1, "p": 2}]}, "a": null});
        let mut reversed = serde_json::Map::new();
        reversed.insert("a".into(), Value::Null);
        let mut inner = serde_json::Map::new();
        inner.insert("x".into(), json!([{"p": 2, "q": 1}]));
        inner.insert("y".into(), json!(1));
        reversed.insert("b".into(), Value::Object(inner));
        let reversed = Value::Object(reversed);

        assert_eq!(render(&forward), render(&reversed));
        let text = render(&forward);
        assert!(text.ends_with("}\n"), "one trailing newline: {text:?}");
        let order: Vec<&str> = text
            .lines()
            .filter(|line| line.contains('"'))
            .map(str::trim)
            .collect();
        assert_eq!(order[0], "\"a\": null,", "{text}");
        assert_eq!(sorted(&forward)["b"]["x"][0]["p"], 2);
    }
}
