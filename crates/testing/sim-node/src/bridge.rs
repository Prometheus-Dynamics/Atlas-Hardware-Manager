//! The devices of a simulated node, as Lemnos's bridge at a6251ecf offers them (`service/orion/src/calls.rs`,
//! `publish.rs`). This is what the default sim answers; a name or target the bridge does not offer is
//! refused by Orion itself with "not found", as on a board.
//!
//! The bridge publishes, for every device, `lemnos.<device>.set`, `.call` and `.power`, each registered
//! on `Target::Resource("lemnos.<device>")`. Orion runs a call only when its name and target both match a
//! registration, so a run against any other target never reaches the bridge: it answers "not found".
//!
//! A `call` takes its words as `TAG:METHOD ARG...` (the contract's full tag with its version, split at
//! the last `:`), or `NAME ARG...`, and the bare word `describe`. The refusals below are the bridge's
//! words.
//!
//! `set` and `power` are no-ops here: a set or a power switch is taken and not kept, because the sim has
//! no control or power state to move. Only the words are checked.

use orion::actions::Target;
use orion::types::ResourceName;

use crate::describe::ItemKind;
use crate::names::{self, Method};
use crate::readings::{self, MethodSpec, Spec};

/// The most reply an out-buffer method takes: the bridge's `ORION_REPLY_MAX` (60 KiB).
pub const OUT_LIMIT: u32 = 60 * 1024;

/// The verb of a device action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Set,
    Call,
    Power,
}

/// What a device action's run becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Routed {
    /// Run the simulated action `name` (one of [`names::SURFACE`]'s actions) with these words.
    Action { name: &'static str, args: Vec<u8> },
    /// `describe`: the device's description.
    Describe { device: &'static str },
    /// Answered without a board action: the reply's bytes (empty for `set` and `power`).
    Reply(Vec<u8>),
}

/// The device id and verb of a device action name `lemnos.<device>.<verb>`, for a device the node has.
#[must_use]
pub fn split(name: &str) -> Option<(&'static str, Verb)> {
    let (device, verb) = name.strip_prefix("lemnos.")?.rsplit_once('.')?;
    let verb = match verb {
        "set" => Verb::Set,
        "call" => Verb::Call,
        "power" => Verb::Power,
        _ => return None,
    };
    let spec = readings::DEVICES.iter().find(|spec| spec.id == device)?;
    Some((spec.id, verb))
}

/// Routes a device action's run. `None` when `name` is not a device action of this node; `Some(Err)`
/// is the bridge's refusal, in its words.
#[must_use]
pub fn route(name: &str, args: &[u8]) -> Option<Result<Routed, String>> {
    let (device, verb) = split(name)?;
    let spec = readings::DEVICES.iter().find(|spec| spec.id == device)?;
    let text = match std::str::from_utf8(args) {
        Ok(text) => text,
        Err(_) => return Some(Err("the arguments are not text".to_owned())),
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    Some(match verb {
        Verb::Call => call(spec, &words),
        Verb::Set => set(spec, &words),
        Verb::Power => match words.as_slice() {
            ["on"] | ["off"] => Ok(Routed::Reply(Vec::new())),
            _ => Err("power takes on or off".to_owned()),
        },
    })
}

fn call(spec: &'static Spec, words: &[&str]) -> Result<Routed, String> {
    let Some((wanted, values)) = words.split_first() else {
        return Err("call needs a method".to_owned());
    };
    if *wanted == "describe" {
        return if values.is_empty() {
            Ok(Routed::Describe { device: spec.id })
        } else {
            Err("describe takes no arguments".to_owned())
        };
    }
    let method = find_method(spec, wanted)?;
    let surface = names::SURFACE
        .iter()
        .find(|entry| Some(entry.contract) == contract_of(spec) && entry.lemnos == method.name)
        .ok_or_else(|| format!("{wanted} is not answered by the simulated node"))?;
    let params = method.params.len();
    let (args, size) = if method.out_buffer && values.len() == params + 1 {
        (&values[..params], Some(values[params]))
    } else if values.len() > params {
        return Err(if method.out_buffer {
            "too many values: the buffer's size is the last one".to_owned()
        } else {
            "too many values".to_owned()
        });
    } else {
        (values, None)
    };
    if let Some(size) = size {
        match size.parse::<u32>() {
            Ok(bytes) if bytes <= OUT_LIMIT => {}
            _ => {
                return Err(format!(
                    "{size:?} isn't a buffer size: a number of bytes, at most {OUT_LIMIT}"
                ));
            }
        }
    }
    Ok(Routed::Action {
        name: surface.action,
        args: args.join(" ").into_bytes(),
    })
}

/// A method by `NAME`, or by `TAG:NAME`: the tag is split at the last `:` and compared exactly, as the
/// bridge's `find_method` does.
fn find_method(spec: &'static Spec, wanted: &str) -> Result<&'static MethodSpec, String> {
    let (tag, name) = wanted
        .rsplit_once(':')
        .map_or((None, wanted), |(tag, name)| (Some(tag), name));
    let no_method = || format!("no method {wanted:?}");
    let contract = contract_of(spec).ok_or_else(no_method)?;
    if tag.is_some_and(|tag| tag != contract) {
        return Err(no_method());
    }
    spec.methods
        .iter()
        .find(|method| method.name == name)
        .ok_or_else(no_method)
}

fn set(spec: &'static Spec, words: &[&str]) -> Result<Routed, String> {
    let Some((item, _)) = words.split_first() else {
        return Err("set needs an item".to_owned());
    };
    let found = spec
        .items
        .iter()
        .find(|found| found.name == *item)
        .ok_or_else(|| format!("no item {item:?}"))?;
    if found.item_kind == ItemKind::Value {
        return Err(format!("{item} is read-only"));
    }
    Ok(Routed::Reply(Vec::new()))
}

fn contract_of(spec: &Spec) -> Option<&'static str> {
    spec.contract.map(|(tag, _, _)| tag)
}

/// The bridge's form of a simulated action's call, for the test helper: the device's `call` on its
/// resource, with `TAG:METHOD` and the words. `None` when the action is not a Lemnos method of this node.
#[must_use]
pub fn request(action: &str, args: &[u8]) -> Option<(String, Target, Vec<u8>)> {
    let entry: &Method = names::SURFACE.iter().find(|entry| entry.action == action)?;
    let spec = readings::DEVICES
        .iter()
        .find(|spec| contract_of(spec) == Some(entry.contract))?;
    let mut bytes = format!("{}:{}", entry.contract, entry.lemnos).into_bytes();
    if !args.is_empty() {
        bytes.push(b' ');
        bytes.extend_from_slice(args);
    }
    let resource: ResourceName = format!("lemnos.{}", spec.id).parse().ok()?;
    Some((
        format!("lemnos.{}.call", spec.id),
        Target::Resource(resource),
        bytes,
    ))
}

/// The actions a device of this node has, with what they are for: `set`, `call` and `power`, as the bridge
/// declares them on every device's resource.
pub const VERBS: [(&str, &str); 3] = [
    (
        "set",
        "Drives a control or action item. Arguments: ITEM VALUE..., as lemnos-ctl set takes them",
    ),
    (
        "call",
        "Calls any method of the device. Arguments: METHOD ARG..., the method as NAME or TAG:NAME. The output is the reply as text. `describe` with no arguments gives the device description as text",
    ),
    ("power", "Switches the device. Arguments: on or off"),
];

/// The policy every device action is registered with: root on the board, and an operator the board grants.
#[must_use]
pub fn policy() -> orion::actions::Policy {
    orion::actions::Policy::new(orion::actions::Admits::ROOT.and(orion::actions::Admits::OPERATOR))
}

/// Starts a simulated action as a test does: a Lemnos method in the bridge's form (see [`request`]), any
/// other action on the node. The run is the same either way; only the name and target differ.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone or the run could not be sent.
pub fn run(
    orion: &orion::Orion,
    action: &str,
    args: &[u8],
    deadline_ms: u64,
    detached: bool,
) -> Result<orion::actions::Run, orion::Error> {
    let (name, target, bytes) = request(action, args)
        .unwrap_or_else(|| (action.to_owned(), crate::node::target(), args.to_vec()));
    orion.run(&orion::actions::ActionCall {
        name: &name,
        target,
        args: &bytes,
        deadline_ms,
        detached,
    })
}
