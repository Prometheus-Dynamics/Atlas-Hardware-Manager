//! The one place Atlas builds a call to a Lemnos device, as Lemnos's bridge takes it.
//!
//! The bridge publishes every device's actions as `lemnos.<device>.set`, `.call` and `.power`, and
//! runs each against its device's resource: `Target::Resource("lemnos.<device>")`. A run against
//! any other target is not the bridge's (Orion sends a run only to the action its name and target
//! name). The device id comes from the board's file, so a board names its own devices: a call finds
//! the device by the contract it carries, never by a fixed name.
//!
//! A `call` takes its arguments as text: `METHOD ARG...`, where METHOD is `NAME` or `TAG:NAME`
//! (`system.slots/1:list`). The bridge splits the words on whitespace and looks the method up by
//! that form (`service/orion/src/calls.rs`, `serve`, and `find_method` in `engine/wire/src/text.rs`).
//! `describe` is the bare word, with no arguments; the bridge answers the device's description
//! as text.
//!
//! Atlas names a Lemnos method `system.<contract>.<method>` with the method spelled as the contract
//! crate spells it (`system.slots.list`, `system.reboot.reboot`, `system.slots.install_file`). There is
//! one spelling: [`actions`] holds the names, and [`method_of`] and [`name_of`] are the only mappings.
//! Every contract Atlas names is version 1.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use crate::backoff::Backoff;

use crate::Failure;
use lemnos_contract_system_flash::Flash;
use lemnos_contract_system_reboot::Reboot;
use lemnos_contract_system_slots::Slots;
use lemnos_core::Contract;
use orion::ClientError;
use orion::Orion;
use orion::actions::{ActionCall, Outcome, Target};
use orion::types::ResourceName;

/// The prefix of the resources of a node's devices (`lemnos.<device>`).
// board: Lemnos 98fbe0c2 service/orion/src/lib.rs:52 (RESOURCE_PREFIX)
pub const RESOURCE_PREFIX: &str = "lemnos.";
/// The kind a device resource is written as.
// board: Lemnos 98fbe0c2 service/orion/src/lib.rs:54 (RESOURCE_KIND)
pub const RESOURCE_KIND: &str = "lemnos.device";
/// What every file destination the bridge takes starts with: `lemnos/<device>/<name>`. The bridge
/// takes a file only under this prefix, into the inbox of the device it names; a file pushed anywhere
/// else is kept by the node and never reaches the device (Lemnos service/orion/src/files.rs:38,
/// `DESTINATION_PREFIX`; the same at a6251ecf).
// board: Lemnos 98fbe0c2 service/orion/src/files.rs:38 (DESTINATION_PREFIX)
pub const SINK_PREFIX: &str = "lemnos/";

/// The destination of a file pushed into device `device`'s inbox: `lemnos/<device>/`, with the file
/// name after it.
#[must_use]
pub fn inbox(device: &str) -> String {
    format!("{SINK_PREFIX}{device}/")
}

/// The device a device resource names: `lemnos.<device>` is `<device>`.
#[must_use]
pub fn device_of(resource: &str) -> Option<&str> {
    resource
        .strip_prefix(RESOURCE_PREFIX)
        .filter(|device| !device.is_empty())
}
/// The bare word `describe` takes: no arguments, and the device's description comes back as text.
pub const DESCRIBE: &str = "describe";
/// How long a describe may take before it is asked to stop. Reason: one round trip to lemnosd.
pub const DESCRIBE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The contract a board's slots are served by, its tag as the contract crate states it (Lemnos
/// `contracts/system-slots`, at the pin).
pub const SLOTS: &str = <dyn Slots as Contract>::TAG.as_str();
/// The contract a board's restart is served by (Lemnos `contracts/system-reboot`).
pub const REBOOT: &str = <dyn Reboot as Contract>::TAG.as_str();
/// The contract the first-flash image is written through (Lemnos `contracts/system-flash`).
pub const FLASH: &str = <dyn Flash as Contract>::TAG.as_str();

/// The Atlas names of the Lemnos methods, one spelling: `system.<contract>.<method>`, with the method as the
/// contract crate names it. Each is checked against [`methods`] and the contract crate's own table by the
/// test `the_action_names_are_the_contracts_methods`. The sim (`atlas-sim-node`) names the same methods,
/// and a test in this crate checks it agrees.
pub mod actions {
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:89 (list)
    pub const SLOTS_LIST: &str = "system.slots.list";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:129 (accept)
    pub const SLOTS_ACCEPT: &str = "system.slots.accept";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:138 (install_file)
    pub const SLOTS_INSTALL_FILE: &str = "system.slots.install_file";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:106 (set_default)
    pub const SLOTS_SET_DEFAULT: &str = "system.slots.set_default";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:111 (trial)
    pub const SLOTS_TRIAL: &str = "system.slots.trial";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:119 (remove)
    pub const SLOTS_REMOVE: &str = "system.slots.remove";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:122 (pin)
    pub const SLOTS_PIN: &str = "system.slots.pin";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:125 (set_keep)
    pub const SLOTS_SET_KEEP: &str = "system.slots.set_keep";
    // board: Lemnos 98fbe0c2 contracts/system-slots/src/lib.rs:155 (info)
    pub const SLOTS_INFO: &str = "system.slots.info";
    // board: Lemnos 98fbe0c2 contracts/system-reboot/src/lib.rs:12 (reboot)
    pub const REBOOT_REBOOT: &str = "system.reboot.reboot";
    // board: Lemnos 98fbe0c2 contracts/system-flash/src/lib.rs:63 (verify)
    pub const FLASH_VERIFY: &str = "system.flash.verify";
    // board: Lemnos 98fbe0c2 contracts/system-flash/src/lib.rs:38 (info)
    pub const FLASH_INFO: &str = "system.flash.info";
}

/// The method names Atlas calls, as the contract crates name them. Each is checked against the crate's
/// own method table by the test `the_method_names_atlas_calls_are_the_contracts`.
pub mod methods {
    pub const SLOTS_LIST: &str = "list";
    pub const SLOTS_ACCEPT: &str = "accept";
    pub const SLOTS_INSTALL_FILE: &str = "install_file";
    pub const SLOTS_SET_DEFAULT: &str = "set_default";
    pub const SLOTS_TRIAL: &str = "trial";
    pub const SLOTS_REMOVE: &str = "remove";
    pub const SLOTS_PIN: &str = "pin";
    pub const SLOTS_SET_KEEP: &str = "set_keep";
    pub const SLOTS_INFO: &str = "info";
    pub const REBOOT_REBOOT: &str = "reboot";
    pub const FLASH_VERIFY: &str = "verify";
    pub const FLASH_INFO: &str = "info";
}

/// A Lemnos device call, ready to run: the action's name and its target, and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// `lemnos.<device>.call`.
    pub name: String,
    /// `Target::Resource("lemnos.<device>")`.
    pub target: Target,
    /// The arguments: `TAG:METHOD ARG...`, or `describe`.
    pub args: Vec<u8>,
}

/// The call on device `resource` (a resource name, `lemnos.<device>`) for method `method` of
/// contract `contract`, with the arguments `words` already encoded (space-separated, see
/// [`crate::lemnos`] for the form). `words` is empty for a method with no arguments.
///
/// # Errors
///
/// When `resource` is not a resource name the bridge can have.
pub fn method_call(
    resource: &str,
    contract: &str,
    method: &str,
    words: &[u8],
) -> Result<Call, String> {
    let mut args = format!("{contract}:{method}").into_bytes();
    if !words.is_empty() {
        args.push(b' ');
        args.extend_from_slice(words);
    }
    call_on(resource, args)
}

/// The describe call on device `resource`: the bridge's `describe`, with no other argument.
///
/// # Errors
///
/// When `resource` is not a resource name the bridge can have.
pub fn describe_call(resource: &str) -> Result<Call, String> {
    call_on(resource, DESCRIBE.as_bytes().to_vec())
}

/// The one construction of a device call: `<resource>.call`, against the resource, with `args`.
fn call_on(resource: &str, args: Vec<u8>) -> Result<Call, String> {
    let name: ResourceName = resource
        .parse()
        .map_err(|_| format!("{resource:?} is not a device resource"))?;
    Ok(Call {
        name: format!("{resource}.call"),
        target: Target::Resource(name),
        args,
    })
}

/// The contract and method an Atlas Lemnos name stands for: `system.slots.list` is
/// `("system.slots/1", "list")`, `system.slots.install_file` is `("system.slots/1",
/// "install_file")`. `None` for a name that is not of that form (a board's own action).
#[must_use]
pub fn method_of(name: &str) -> Option<(String, String)> {
    let (contract, method) = name.strip_prefix("system.")?.split_once('.')?;
    if contract.is_empty() || method.is_empty() || method.contains('.') {
        return None;
    }
    Some((format!("system.{contract}/1"), method.to_owned()))
}

/// The contract tags a describe text gives, in order: the `tag=` of each `contract` record. The
/// text is `describe 1` then one record per line (`service/orion/src/description.rs`).
#[must_use]
pub fn tags(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split(' ');
            if words.next()? != "contract" {
                return None;
            }
            words
                .find_map(|word| word.strip_prefix("tag="))
                .map(|tag| tag.trim_matches('"').to_owned())
        })
        .collect()
}

/// The Atlas name of method `method` of contract `contract`: `system.slots/1` and `install_file` are
/// `system.slots.install_file`. The inverse of [`method_of`]; `None` for a contract or method that
/// form cannot name (a contract outside `system.`, or of another version).
#[must_use]
pub fn name_of(contract: &str, method: &str) -> Option<String> {
    let name = format!("{}.{}", contract.strip_suffix("/1")?, method);
    (method_of(&name)? == (contract.to_owned(), method.to_owned())).then_some(name)
}

/// One method a device offers, as its description gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    /// The device's resource (`lemnos.<device>`).
    pub resource: String,
    /// The contract's tag (`system.reboot/1`).
    pub contract: String,
    /// The method's name in its contract (`reboot`, `install_file`).
    pub method: String,
    /// Its parameters, each a name and a shape (`keep`, `u32`), in order. `None` when the description
    /// does not give them.
    pub params: Option<Vec<(String, String)>>,
    /// Whether it is a long job. `None` when the description does not say.
    pub job: Option<bool>,
    /// The name the device gives the action that runs it (an `item kind=action` record that names this
    /// method). `None` when the device names none.
    pub label: Option<String>,
}

/// What the node offers to run: the methods of its devices, and the devices that did not answer.
#[derive(Clone, Debug, Default)]
pub struct Offers {
    /// How many devices the node's resource list says take a `call`.
    pub devices: usize,
    /// Every method those devices describe, in the order of the resource list and the descriptions.
    pub methods: Vec<Offered>,
    /// The devices whose description was refused, each with the refusal. A device that is off refuses its
    /// description; that is normal, and its methods are not offered.
    pub unanswered: Vec<(String, Failure)>,
}

/// What the node offers to run, read from the two places a board says it: the node's resource list
/// (which devices there are, and that each offers `lemnos.<device>.call`) and each device's description
/// (its contracts and their methods). The descriptions are the session's kept ones ([`described`]), so a
/// device is asked once per session. Nothing here is a call to an action of the node's own.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
pub fn offers(orion: &Orion, session: u64) -> Result<Offers, orion::Error> {
    let listed = orion.resources(RESOURCE_PREFIX)?;
    let devices: Vec<String> = listed
        .iter()
        .filter(|resource| resource.kind == RESOURCE_KIND)
        .map(|resource| resource.name.clone())
        .collect();
    retain_described(session, &devices);
    let mut offers = Offers::default();
    for resource in listed {
        // Only a device whose resource lists its `call` can be called.
        let call = format!("{}.call", resource.name);
        if resource.kind != RESOURCE_KIND || !resource.actions.contains(&call) {
            continue;
        }
        offers.devices += 1;
        match described(orion, session, &resource.name)? {
            Ok(bytes) => offers.methods.extend(described_methods(
                &resource.name,
                &String::from_utf8_lossy(&bytes),
            )),
            Err(failure) => offers.unanswered.push((resource.name, failure)),
        }
    }
    Ok(offers)
}

/// The methods a describe text gives for the device `resource`: each `method` record, with the tag of
/// the `contract` record it names and the name of the `item kind=action` record that runs it. A method
/// whose contract record or name is missing is not one that can be called, and is left out.
#[must_use]
pub fn described_methods(resource: &str, text: &str) -> Vec<Offered> {
    let records: Vec<(&str, Vec<(String, String)>)> = text.lines().filter_map(record).collect();
    let value = |fields: &[(String, String)], key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    let of_word = |word: &'static str| {
        records
            .iter()
            .filter(move |(found, _)| *found == word)
            .map(|(_, fields)| fields)
    };
    let mut methods = Vec::new();
    for fields in of_word("method") {
        let (Some(index), Some(method)) = (value(fields, "contract"), value(fields, "name")) else {
            continue;
        };
        let Some(contract) = of_word("contract")
            .find(|contract| value(contract, "index").as_deref() == Some(index.as_str()))
            .and_then(|contract| value(contract, "tag"))
        else {
            continue;
        };
        let id = value(fields, "id");
        let label = of_word("item")
            .find(|item| {
                value(item, "kind").as_deref() == Some("action")
                    && value(item, "contract").as_deref() == Some(index.as_str())
                    && id.is_some()
                    && value(item, "method") == id
            })
            .and_then(|item| value(item, "name"))
            .filter(|name| !name.trim().is_empty());
        methods.push(Offered {
            resource: resource.to_owned(),
            contract,
            method,
            params: value(fields, "params").map(|params| {
                params
                    .split_whitespace()
                    .map(|param| {
                        let (name, shape) = param.split_once(':').unwrap_or((param, ""));
                        (name.to_owned(), shape.to_owned())
                    })
                    .collect()
            }),
            job: value(fields, "job").and_then(|job| match job.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }),
            label,
        });
    }
    methods
}

/// One record line of a describe text: its word and its `key=value` pairs, a quoted value unescaped
/// (the encoder's escapes: `\\ \" \n \r \t`). `None` for a line that is not a record.
fn record(line: &str) -> Option<(&str, Vec<(String, String)>)> {
    let (word, mut rest) = line.split_once(' ').unwrap_or((line, ""));
    let mut fields = Vec::new();
    while !rest.is_empty() {
        let (key, body) = rest.split_once('=')?;
        if let Some(quoted) = body.strip_prefix('"') {
            let mut text = String::new();
            let mut chars = quoted.char_indices();
            let end = loop {
                match chars.next()? {
                    (index, '"') => break index + 1,
                    (_, '\\') => text.push(match chars.next()?.1 {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    }),
                    (_, other) => text.push(other),
                }
            };
            fields.push((key.to_owned(), text));
            rest = quoted[end..].strip_prefix(' ').unwrap_or(&quoted[end..]);
        } else {
            let (bare, after) = body.split_once(' ').unwrap_or((body, ""));
            fields.push((key.to_owned(), bare.to_owned()));
            rest = after;
        }
    }
    Some((word, fields))
}

/// What a lookup for a contract found on the node. Three states of "not found" are kept apart: a
/// complete answer (every device described, none carries the contract) is final; anything else is
/// "not yet", with a sentence for people, and the caller may ask again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The device whose resource this is carries the contract.
    Found(String),
    /// Final: every device of the node was described, and none carries the contract.
    NoContract,
    /// Not final: the device that carries the contract is not answering yet (it is known from an
    /// earlier describe, and is withdrawn or refused now), or the search did not finish (a device
    /// refused its description or the resource list failed, and the contract was not found among
    /// the devices that answered). The text is the sentence for people, with the time it began.
    NotYet(String),
    /// Every device refused its description and one refusal is the node turning this client away
    /// (`denied`): its failure, which keeps its kind.
    Unanswered(Failure),
    /// The node lists no Lemnos device at all, and no device has carried the contract. Once lemnosd is up
    /// that is a fault, not a normal state (devices are adopted after connect, so this is read only after
    /// connect).
    NoDevices,
}

/// A device of the node's resource list: its name, and whether its published state is off (`Pending`,
/// the state a device disabled by its board file is published in, as the sim publishes it). Read from
/// the resource's state, never from a refusal's words.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Listed {
    name: String,
    off: bool,
}

/// The devices of the node's resource list (no call to a device).
fn device_resources(orion: &Orion) -> Result<Vec<Listed>, orion::Error> {
    Ok(orion
        .resources(RESOURCE_PREFIX)?
        .into_iter()
        .filter(|resource| resource.kind == RESOURCE_KIND)
        .map(|resource| Listed {
            off: resource.state == orion::provider::ResourceState::Pending,
            name: resource.name,
        })
        .collect())
}

/// Finds the device that carries `contract`, asked by its description. A device whose describe is
/// refused is not that device. Uncached: a board's connect uses it once.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
pub fn resource_for(orion: &Orion, session: u64, contract: &str) -> Result<Lookup, orion::Error> {
    lookup(orion, session, contract, false)
}

/// The devices that carried each contract, per session, and the first time each contract stopped being
/// answered (`NOT_YET`). A session asks each device's description once for a contract, and later calls
/// reuse the answer. A device that carried a contract is remembered for the session even when it leaves
/// the node's list, so a device that is rebound for a moment is "not answering yet", not "no such contract".
/// A new session has a new key.
static FOUND: Mutex<BTreeMap<(u64, String), String>> = Mutex::new(BTreeMap::new());
static NOT_YET: Mutex<BTreeMap<(u64, String), Instant>> = Mutex::new(BTreeMap::new());

fn found() -> MutexGuard<'static, BTreeMap<(u64, String), String>> {
    FOUND.lock().unwrap_or_else(PoisonError::into_inner)
}

fn not_yet() -> MutexGuard<'static, BTreeMap<(u64, String), Instant>> {
    NOT_YET.lock().unwrap_or_else(PoisonError::into_inner)
}

/// As [`resource_for`], for the session `session` (the Orion session the call goes on): the description
/// is asked only when no answer is kept for this session and contract, or the kept resource is gone from
/// the resource list. The resource list itself is read on every call (a read of the node, not of a
/// device).
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
pub fn resource_cached(
    orion: &Orion,
    session: u64,
    contract: &str,
) -> Result<Lookup, orion::Error> {
    lookup(orion, session, contract, true)
}

/// The one lookup. `cached` reads a kept device that is still listed without asking it again.
fn lookup(
    orion: &Orion,
    session: u64,
    contract: &str,
    cached: bool,
) -> Result<Lookup, orion::Error> {
    let listed = match device_resources(orion) {
        Ok(listed) => listed,
        Err(error) if error.is_lost() => return Err(error),
        // The node's list did not come back: the search is not complete, so it is not final.
        Err(_) => return Ok(Lookup::NotYet(incomplete(session, contract))),
    };
    let names: Vec<String> = listed.iter().map(|device| device.name.clone()).collect();
    retain_described(session, &names);
    let key = (session, contract.to_owned());
    if cached
        && let Some(kept) = found().get(&key).cloned()
        && names.contains(&kept)
    {
        return Ok(Lookup::Found(kept));
    }
    search(orion, session, listed, contract)
}

/// The plain name of a contract for people: `system.slots/1` is `slots`, `system.reboot/1` is `reboot`.
fn plain_name(contract: &str) -> &str {
    let name = contract.strip_prefix("system.").unwrap_or(contract);
    name.strip_suffix("/1").unwrap_or(name)
}

/// The moment a session first stopped getting `contract`, kept until it is found again.
fn started(session: u64, contract: &str) -> Instant {
    *not_yet()
        .entry((session, contract.to_owned()))
        .or_insert_with(Instant::now)
}

/// The sentence for a device that carries `contract` but is not answering yet.
fn device_not_yet(session: u64, contract: &str) -> String {
    format!(
        "the board's {} device isn't answering yet, for {}",
        plain_name(contract),
        crate::words::duration(started(session, contract).elapsed())
    )
}

/// The sentence for a search that did not finish.
fn incomplete(session: u64, contract: &str) -> String {
    format!(
        "couldn't finish asking the board what it offers yet, for {}",
        crate::words::duration(started(session, contract).elapsed())
    )
}

/// The description of device `resource`, as the bridge gave it (its bytes), or the words the device
/// refused it with. This is the one describe run Atlas makes; [`described`] keeps its answer per session.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone, or when the board was busy and did not take the
/// call (asked again by [`crate::Handle::with_client`]).
pub fn describe(orion: &Orion, resource: &str) -> Result<Answer, orion::Error> {
    let Ok(call) = describe_call(resource) else {
        return Ok(Err(Failure::other(format!(
            "{resource:?} is not a device resource"
        ))));
    };
    let mut run = orion.run(&ActionCall {
        name: &call.name,
        target: call.target,
        args: &call.args,
        deadline_ms: 0,
        detached: false,
    })?;
    match run.finish(DESCRIBE_WAIT) {
        Ok(finished) => Ok(match finished.outcome {
            Outcome::Succeeded => Ok(finished.output),
            _ if finished.detail.trim().is_empty() => {
                Err(Failure::other("the board did not give the description"))
            }
            _ => Err(Failure::other(finished.detail)),
        }),
        // Busy: the board had no room and did not take the call; the session asks again
        // (`Handle::with_client`), so it is never a description's answer.
        Err(error) if error.is_busy() => Err(orion::Error::Client(error)),
        // A refusal keeps its kind: a node that turns this client away (`denied`) says so to the caller.
        Err(error @ (ClientError::Refused { .. } | ClientError::Denied(_))) => {
            let text = match &error {
                ClientError::Refused { detail, .. } => detail.clone(),
                other => other.to_string(),
            };
            Ok(Err(Failure::of_client(&error, text)))
        }
        Err(ClientError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut => {
            // Not finished in time: asked to stop, so it is not left running unobserved.
            let _ = run.cancel();
            Ok(Err(Failure::other(format!(
                "the board did not answer describe within {} s",
                DESCRIBE_WAIT.as_secs()
            ))))
        }
        Err(error) => Err(orion::Error::Client(error)),
    }
}

/// What a describe gave: the bridge's bytes, or the failure it gave (its words, and its kind).
pub type Answer = Result<Vec<u8>, Failure>;

/// The describe answers of each device, per session: one describe per device per session, shared by the
/// topics reader and the resolver. Keyed by (session, resource); kept while the session is the one asked
/// and the resource is listed (see [`retain_described`]). Only a complete description is kept: a refusal
/// or an error is "not yet", and it is asked again (see [`described`]).
static DESCRIBED: Mutex<BTreeMap<(u64, String), Answer>> = Mutex::new(BTreeMap::new());

fn described_map() -> MutexGuard<'static, BTreeMap<(u64, String), Answer>> {
    DESCRIBED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The describe answer of `resource` this session already holds, without a call.
#[must_use]
pub fn held_description(session: u64, resource: &str) -> Option<Answer> {
    described_map()
        .get(&(session, resource.to_owned()))
        .cloned()
}

/// The describe answer of `resource` for this session: the one held, or else one asked now. Only a
/// complete description is kept; a refusal is returned and not kept, so the next call asks again.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
pub fn described(orion: &Orion, session: u64, resource: &str) -> Result<Answer, orion::Error> {
    if let Some(held) = held_description(session, resource) {
        return Ok(held);
    }
    let key = (session, resource.to_owned());
    // A refusal is "not yet": it is held only until its backoff has passed, then asked again.
    if let Some(retry) = refusals().get(&key)
        && Instant::now() < retry.after
    {
        return Ok(Err(retry.failure.clone()));
    }
    let answer = describe(orion, resource)?;
    match &answer {
        Ok(_) => {
            refusals().remove(&key);
            described_map().insert(key, answer.clone());
        }
        Err(failure) => {
            let mut backoff = refusals()
                .remove(&key)
                .map_or_else(Backoff::default, |retry| retry.backoff);
            let after = Instant::now() + backoff.next_delay();
            refusals().insert(
                key,
                Retry {
                    failure: failure.clone(),
                    backoff,
                    after,
                },
            );
        }
    }
    Ok(answer)
}

/// A refused describe, held for its backoff: the refusal, the backoff for the next ask, and when that ask
/// may be made.
#[derive(Clone, Debug)]
struct Retry {
    failure: Failure,
    backoff: Backoff,
    after: Instant,
}

static REFUSED: Mutex<BTreeMap<(u64, String), Retry>> = Mutex::new(BTreeMap::new());

fn refusals() -> MutexGuard<'static, BTreeMap<(u64, String), Retry>> {
    REFUSED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Drops the answer held for `resource` in `session`, so the next [`described`] asks again (a generation
/// the device published has moved).
pub fn forget_description(session: u64, resource: &str) {
    described_map().remove(&(session, resource.to_owned()));
    refusals().remove(&(session, resource.to_owned()));
}

/// Drops every answer of `session`: the session is lost, and a new one asks again.
pub fn forget_session(session: u64) {
    described_map().retain(|(kept, _), _| *kept != session);
    refusals().retain(|(kept, _), _| *kept != session);
    found().retain(|(kept, _), _| *kept != session);
    not_yet().retain(|(kept, _), _| *kept != session);
}

/// Drops the answers of `session` for the resources not in `listed`: a device that left the node's
/// resource list loses its answer. The answers of other sessions are not this session's to drop (another
/// node's session, or a session that is gone, keeps its own; they are small and bounded by its devices).
pub fn retain_described(session: u64, listed: &[String]) {
    described_map().retain(|(kept, resource), _| *kept != session || listed.contains(resource));
    refusals().retain(|(kept, resource), _| *kept != session || listed.contains(resource));
}

/// The search for `contract` among `devices`: the device whose description carries it, or the state of
/// the search (see [`Lookup`]). "No such contract" is only said from a complete answer. A device whose
/// published state is off answers nothing now: it is not a refusal to wait on (it is asked again on the
/// backoff, so a device that comes on is found).
fn search(
    orion: &Orion,
    session: u64,
    devices: Vec<Listed>,
    contract: &str,
) -> Result<Lookup, orion::Error> {
    let key = (session, contract.to_owned());
    let known = found().get(&key).cloned();
    let total = devices.len();
    let listed: Vec<String> = devices.iter().map(|device| device.name.clone()).collect();
    let mut refusals: Vec<Failure> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for device in devices {
        let resource = device.name;
        match described(orion, session, &resource)? {
            Ok(bytes)
                if tags(&String::from_utf8_lossy(&bytes))
                    .iter()
                    .any(|tag| tag == contract) =>
            {
                not_yet().remove(&key);
                found().insert(key, resource.clone());
                return Ok(Lookup::Found(resource));
            }
            Ok(_) => {}
            // Off by its board file: it offers nothing now, which is an answer.
            Err(_) if device.off => {}
            Err(words) => {
                refused.push(resource);
                refusals.push(words);
            }
        }
    }
    // A device that carried the contract and is withdrawn, or refuses while on, is not answering yet. One
    // that answers without it, or is off, no longer carries it.
    if let Some(kept) = known {
        if !listed.contains(&kept) || refused.contains(&kept) {
            return Ok(Lookup::NotYet(device_not_yet(session, contract)));
        }
        found().remove(&key);
    }
    if total == 0 {
        return Ok(Lookup::NoDevices);
    }
    if let Some(position) = refusals.iter().position(Failure::is_denied) {
        return Ok(Lookup::Unanswered(refusals.swap_remove(position)));
    }
    if refusals.is_empty() {
        not_yet().remove(&key);
        return Ok(Lookup::NoContract);
    }
    Ok(Lookup::NotYet(incomplete(session, contract)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_action_names_are_the_contracts_methods_in_one_spelling() {
        let slots: Vec<&str> = <dyn Slots as Contract>::INFO
            .methods
            .iter()
            .map(|method| method.name)
            .collect();
        let reboot: Vec<&str> = <dyn Reboot as Contract>::INFO
            .methods
            .iter()
            .map(|method| method.name)
            .collect();
        let flash: Vec<&str> = <dyn Flash as Contract>::INFO
            .methods
            .iter()
            .map(|method| method.name)
            .collect();
        let pairs = [
            (actions::SLOTS_LIST, SLOTS, methods::SLOTS_LIST, &slots),
            (actions::SLOTS_ACCEPT, SLOTS, methods::SLOTS_ACCEPT, &slots),
            (
                actions::SLOTS_INSTALL_FILE,
                SLOTS,
                methods::SLOTS_INSTALL_FILE,
                &slots,
            ),
            (
                actions::SLOTS_SET_DEFAULT,
                SLOTS,
                methods::SLOTS_SET_DEFAULT,
                &slots,
            ),
            (actions::SLOTS_TRIAL, SLOTS, methods::SLOTS_TRIAL, &slots),
            (actions::SLOTS_REMOVE, SLOTS, methods::SLOTS_REMOVE, &slots),
            (actions::SLOTS_PIN, SLOTS, methods::SLOTS_PIN, &slots),
            (
                actions::SLOTS_SET_KEEP,
                SLOTS,
                methods::SLOTS_SET_KEEP,
                &slots,
            ),
            (actions::SLOTS_INFO, SLOTS, methods::SLOTS_INFO, &slots),
            (
                actions::REBOOT_REBOOT,
                REBOOT,
                methods::REBOOT_REBOOT,
                &reboot,
            ),
            (actions::FLASH_VERIFY, FLASH, methods::FLASH_VERIFY, &flash),
            (actions::FLASH_INFO, FLASH, methods::FLASH_INFO, &flash),
        ];
        for (name, contract, method, table) in pairs {
            assert!(
                table.contains(&method),
                "{method} is a method of {contract}"
            );
            assert_eq!(name_of(contract, method).as_deref(), Some(name), "{name}");
            assert_eq!(
                method_of(name),
                Some((contract.to_owned(), method.to_owned())),
                "{name}"
            );
            assert!(
                !name.contains('-'),
                "{name} has one spelling: the contract's"
            );
        }
    }

    #[test]
    fn the_dashed_spelling_is_no_method_of_any_contract() {
        let slots: Vec<&str> = <dyn Slots as Contract>::INFO
            .methods
            .iter()
            .map(|method| method.name)
            .collect();
        assert!(
            !slots.contains(&"install-file"),
            "the contract spells it install_file"
        );
        assert_eq!(
            name_of(SLOTS, "install-file"),
            Some("system.slots.install-file".to_owned()),
            "a dashed method is not a contract's name; name_of keeps it as given"
        );
    }
}
