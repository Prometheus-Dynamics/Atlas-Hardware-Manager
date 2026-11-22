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
//! Atlas names a Lemnos method `system.<contract>.<method>` (`system.slots.list`,
//! `system.reboot.reboot`), where `-` in the method is `_` (`install-file` is `install_file`). Every
//! contract Atlas names is version 1.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

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
pub const RESOURCE_PREFIX: &str = "lemnos.";
/// The kind a device resource is written as.
pub const RESOURCE_KIND: &str = "lemnos.device";
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
/// `("system.slots/1", "list")`, `system.slots.install-file` is `("system.slots/1",
/// "install_file")`. `None` for a name that is not of that form (a board's own action).
#[must_use]
pub fn method_of(name: &str) -> Option<(String, String)> {
    let (contract, method) = name.strip_prefix("system.")?.split_once('.')?;
    if contract.is_empty() || method.is_empty() || method.contains('.') {
        return None;
    }
    Some((format!("system.{contract}/1"), method.replace('-', "_")))
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
/// `system.slots.install-file`. The inverse of [`method_of`]; `None` for a contract or method that
/// form cannot name (a contract outside `system.`, or of another version).
#[must_use]
pub fn name_of(contract: &str, method: &str) -> Option<String> {
    let name = format!(
        "{}.{}",
        contract.strip_suffix("/1")?,
        method.replace('_', "-")
    );
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

/// What a lookup for a contract found on the node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The device whose resource this is carries the contract.
    Found(String),
    /// The node lists devices, and none of them carries the contract.
    NoContract,
    /// Every device of the node refused its description: the first refusal's words, which say why (for
    /// example, Lemnos is stopped on the node).
    Unanswered(Failure),
    /// The node lists no Lemnos device at all. Once lemnosd is up that is a fault, not a normal state
    /// (devices are adopted after connect, so this is read only after connect).
    NoDevices,
}

/// The resources of the node's devices, by name, from the node's resource list (no call to a device).
fn device_resources(orion: &Orion) -> Result<Vec<String>, orion::Error> {
    Ok(orion
        .resources(RESOURCE_PREFIX)?
        .into_iter()
        .filter(|resource| resource.kind == RESOURCE_KIND)
        .map(|resource| resource.name)
        .collect())
}

/// Finds the device that carries `contract`, asked by its description. A device whose describe is
/// refused is not that device. Uncached: a board's connect uses it once.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
pub fn resource_for(orion: &Orion, session: u64, contract: &str) -> Result<Lookup, orion::Error> {
    search(orion, session, device_resources(orion)?, contract)
}

/// The found devices of each contract, per session: a session asks each device's description once for a
/// contract, and later calls reuse the answer. An answer is kept only while its resource is still on the
/// node's resource list, so a device that leaves or changes is asked again. A new session has a new key.
static FOUND: Mutex<BTreeMap<(u64, String), String>> = Mutex::new(BTreeMap::new());

fn found() -> MutexGuard<'static, BTreeMap<(u64, String), String>> {
    FOUND.lock().unwrap_or_else(PoisonError::into_inner)
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
    let devices = device_resources(orion)?;
    retain_described(session, &devices);
    if devices.is_empty() {
        found().retain(|(kept, _), _| *kept != session);
        return Ok(Lookup::NoDevices);
    }
    let key = (session, contract.to_owned());
    if let Some(kept) = found().get(&key).cloned()
        && devices.contains(&kept)
    {
        return Ok(Lookup::Found(kept));
    }
    found().remove(&key);
    let lookup = search(orion, session, devices, contract)?;
    if let Lookup::Found(resource) = &lookup {
        found().insert(key, resource.clone());
    }
    Ok(lookup)
}

/// The description of device `resource`, as the bridge gave it (its bytes), or the words the device
/// refused it with. This is the one describe run Atlas makes; [`described`] keeps its answer per session.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
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

/// The describe answers of each device, per session: one describe per device per session, shared by the
/// topics reader and the resolver. Keyed by (session, resource); kept while the session is the one asked
/// and the resource is listed (see [`retain_described`]).
/// One describe answer: the bridge's bytes, or the words of its refusal.
/// What a describe gave: the bridge's bytes, or the failure it gave (its words, and its kind).
pub type Answer = Result<Vec<u8>, Failure>;

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

/// The describe answer of `resource` for this session: the one held, or else one asked now and kept.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
pub fn described(orion: &Orion, session: u64, resource: &str) -> Result<Answer, orion::Error> {
    if let Some(held) = held_description(session, resource) {
        return Ok(held);
    }
    let answer = describe(orion, resource)?;
    described_map().insert((session, resource.to_owned()), answer.clone());
    Ok(answer)
}

/// Drops the answer held for `resource` in `session`, so the next [`described`] asks again (a generation
/// the device published has moved).
pub fn forget_description(session: u64, resource: &str) {
    described_map().remove(&(session, resource.to_owned()));
}

/// Drops every answer of `session`: the session is lost, and a new one asks again.
pub fn forget_session(session: u64) {
    described_map().retain(|(kept, _), _| *kept != session);
}

/// Drops the answers of `session` for the resources not in `listed`: a device that left the node's
/// resource list loses its answer. The answers of other sessions are not this session's to drop (another
/// node's session, or a session that is gone, keeps its own; they are small and bounded by its devices).
pub fn retain_described(session: u64, listed: &[String]) {
    described_map().retain(|(kept, resource), _| *kept != session || listed.contains(resource));
}

/// The search for `contract` among `devices`: the device whose description carries it, or why there is
/// none. When every device refused its description, the first refusal's words are the answer.
fn search(
    orion: &Orion,
    session: u64,
    devices: Vec<String>,
    contract: &str,
) -> Result<Lookup, orion::Error> {
    if devices.is_empty() {
        return Ok(Lookup::NoDevices);
    }
    let total = devices.len();
    let mut refusals: Vec<Failure> = Vec::new();
    for resource in devices {
        match described(orion, session, &resource)? {
            Ok(bytes)
                if tags(&String::from_utf8_lossy(&bytes))
                    .iter()
                    .any(|tag| tag == contract) =>
            {
                return Ok(Lookup::Found(resource));
            }
            Ok(_) => {}
            Err(words) => refusals.push(words),
        }
    }
    if !refusals.is_empty() && refusals.len() == total {
        return Ok(Lookup::Unanswered(refusals.swap_remove(0)));
    }
    Ok(Lookup::NoContract)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each method name Atlas calls is one of its contract's methods, as the contract crate states it.
    /// A renamed method fails here, not on a board.
    #[test]
    fn the_method_names_atlas_calls_are_the_contracts() {
        fn has(info: &lemnos_core::ContractInfo, name: &str) -> bool {
            info.methods.iter().any(|method| method.name == name)
        }
        let slots = <dyn Slots as Contract>::INFO;
        for name in [
            methods::SLOTS_LIST,
            methods::SLOTS_ACCEPT,
            methods::SLOTS_INSTALL_FILE,
            methods::SLOTS_SET_DEFAULT,
            methods::SLOTS_TRIAL,
            methods::SLOTS_REMOVE,
            methods::SLOTS_PIN,
            methods::SLOTS_SET_KEEP,
            methods::SLOTS_INFO,
        ] {
            assert!(has(&slots, name), "{name} is not a system.slots/1 method");
        }
        assert!(has(&<dyn Reboot as Contract>::INFO, methods::REBOOT_REBOOT));
        let flash = <dyn Flash as Contract>::INFO;
        assert!(has(&flash, methods::FLASH_VERIFY));
        assert!(has(&flash, methods::FLASH_INFO));
    }

    #[test]
    fn the_contract_tags_are_the_crates_tags() {
        assert_eq!(SLOTS, "system.slots/1");
        assert_eq!(REBOOT, "system.reboot/1");
        assert_eq!(FLASH, "system.flash/1");
    }

    #[test]
    fn an_atlas_name_is_its_contract_and_method() {
        assert_eq!(
            method_of("system.slots.list"),
            Some(("system.slots/1".into(), "list".into()))
        );
        assert_eq!(
            method_of("system.slots.install-file"),
            Some(("system.slots/1".into(), "install_file".into()))
        );
        assert_eq!(
            method_of("system.reboot.reboot"),
            Some(("system.reboot/1".into(), "reboot".into()))
        );
        assert_eq!(method_of("locate"), None);
        assert_eq!(method_of("system.slots"), None);
        assert_eq!(method_of("system.slots.list.x"), None);
    }

    #[test]
    fn the_call_is_the_bridge_form_on_the_resource() {
        let call = method_call("lemnos.slots", SLOTS, "list", b"").unwrap();
        assert_eq!(call.name, "lemnos.slots.call");
        assert_eq!(
            call.target,
            Target::Resource("lemnos.slots".parse().unwrap())
        );
        assert_eq!(call.args, b"system.slots/1:list");
        let call = method_call("lemnos.slots", SLOTS, "pin", b"a true").unwrap();
        assert_eq!(call.args, b"system.slots/1:pin a true");
        assert_eq!(describe_call("lemnos.slots").unwrap().args, b"describe");
    }

    #[test]
    fn a_contract_and_method_have_one_atlas_name_or_none() {
        assert_eq!(
            name_of(REBOOT, "reboot").as_deref(),
            Some("system.reboot.reboot")
        );
        assert_eq!(
            name_of(SLOTS, "install_file").as_deref(),
            Some("system.slots.install-file")
        );
        assert_eq!(name_of("sim.vane/1", "read"), None);
        assert_eq!(name_of("system.slots/2", "list"), None);
    }

    #[test]
    fn the_methods_of_a_description_carry_their_contract_params_and_action_name() {
        let text = "describe 1\n\
            device generation=3 label=\"Slots \\\"A\\\"\" kind=\"slots\"\n\
            contract index=0 tag=\"system.slots/1\" kind=\"slots\" hash=0x1 methods=2\n\
            contract index=1 tag=\"system.reboot/1\" kind=\"reboot\" hash=0x2 methods=1\n\
            method contract=0 id=9 name=\"set_keep\" params=\"keep:u32\" returns=\"()\" out_buffer=false job=false\n\
            method contract=0 id=11 name=\"install_file\" params=\"name:text pin:bool keep_old:bool\" returns=\"text\" out_buffer=false job=true\n\
            method contract=1 id=1 name=\"reboot\" params=\"\" returns=\"()\" out_buffer=false job=false\n\
            method contract=7 id=1 name=\"orphan\"\n\
            item kind=action name=\"Restart now\" unit=\"\" contract=1 method=1 field=none";
        let methods = described_methods("lemnos.x", text);
        assert_eq!(
            methods.len(),
            3,
            "a method of no contract record is left out"
        );
        assert_eq!(methods[0].contract, "system.slots/1");
        assert_eq!(methods[0].method, "set_keep");
        assert_eq!(
            methods[0].params,
            Some(vec![("keep".to_owned(), "u32".to_owned())])
        );
        assert_eq!(methods[0].job, Some(false));
        assert_eq!(methods[0].label, None);
        assert_eq!(methods[1].params.as_ref().map(Vec::len), Some(3));
        assert_eq!(methods[1].job, Some(true));
        assert_eq!(methods[2].contract, "system.reboot/1");
        assert_eq!(methods[2].params, Some(Vec::new()));
        assert_eq!(methods[2].label.as_deref(), Some("Restart now"));
        assert_eq!(methods[2].resource, "lemnos.x");
        // A method record that says nothing of its params or length reports neither.
        let bare = described_methods(
            "lemnos.y",
            "describe 1\ncontract index=0 tag=\"a.b/1\"\nmethod contract=0 id=1 name=\"go\"",
        );
        assert_eq!((bare[0].params.clone(), bare[0].job), (None, None));
    }

    #[test]
    fn a_resource_name_the_bridge_cannot_have_is_refused() {
        assert!(method_call("no spaces here", SLOTS, "list", b"").is_err());
    }

    #[test]
    fn the_tags_of_a_description_are_its_contract_records() {
        let text = "describe 1\n\
            device id=\"slots\" generation=3\n\
            contract index=0 tag=system.slots/1 kind=slots hash=0x1 methods=2\n\
            method contract=0 id=1 name=list\n\
            contract index=1 tag=system.identity/1 kind=x hash=0x2 methods=0";
        assert_eq!(tags(text), ["system.slots/1", "system.identity/1"]);
        assert!(tags("describe 1\ndevice id=\"x\"").is_empty());
    }
}
