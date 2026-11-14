//! The connection to one board, behind the pin rule. Every board command goes through
//! [`pinned`]; `fingerprint`, `pin` and `repin` read the board with [`probe`] (a session that
//! trusts nothing, which only reads the key).

use std::sync::Arc;
use std::time::Duration;

use atlas_session::{Handle, PROGRAM, Presence, Sessions};
use orion::node::Enrollment;
use orion::node::Node;
use orion::{Address, ClientError, Identity, Options, Orion};

use crate::Env;
use crate::commands::Refusal;
use crate::pins::{Pin, Pins};

/// Orion's words for a plain session refused by a node with security on (`crates/client/connect`,
/// the transport's signed-session check, at the pinned rev).
const NEEDS_SIGNED: &str = "the node needs a signed session, and this client has no key";

/// How long a probe or an enrolment waits for the node to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(10);

/// Whether a board runs with security off, as far as a plain session can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// A plain session is refused: the node needs a signed session.
    On,
    /// A plain session opens: the node accepts anyone, and proves no key on it.
    Off,
    /// Not known: the address is not a network address, or the answer is something else.
    Unknown,
}

/// The facts a Lemnos board publishes. A node with any of them and no offers did not report what
/// it offers.
const LEMNOS_FACTS: [&str; 7] = [
    "family",
    "serial",
    "model",
    "version.slot",
    "version.product",
    "version.os",
    "version.lemnos",
];

/// A session with a board that the pin rule has let through.
pub struct Link {
    pub address: Address,
    pub handle: Handle,
    pub node: Node,
    /// Keeps the session's monitor running for the command.
    pub sessions: Arc<Sessions>,
}

/// What the board's hello says that the slot commands need.
pub struct BoardInfo {
    pub family: Option<String>,
    pub serial: Option<String>,
    pub offered: bool,
    pub reported: bool,
}

/// Whether the board at `address` runs with security off. Only a network address is asked: a plain
/// session to it either opens (security off) or is refused with Orion's words for a node that needs
/// a signed session (security on). The caller asks once per run and keeps the answer.
// Orion J9 reports the session mode in Welcomed: delete this probe in the change that moves the Orion pin to J9.
pub async fn security(address: &Address) -> Security {
    if !matches!(address, Address::Network { .. }) {
        return Security::Unknown;
    }
    let address = address.clone();
    let opened = tokio::task::spawn_blocking(move || {
        Orion::connect_with(
            &address,
            PROGRAM,
            Options {
                identity: None,
                timeout: ANSWER_WAIT,
                ..Options::default()
            },
        )
        .map(drop)
    })
    .await;
    classify(opened.ok())
}

/// What a plain session's outcome says: opened is security off; refused for a signed session is
/// security on; anything else is not known. `None` stands for a probe that could not run.
fn classify(opened: Option<Result<(), orion::Error>>) -> Security {
    match opened {
        Some(Ok(())) => Security::Off,
        Some(Err(orion::Error::Client(ClientError::Denied(why)))) if why == NEEDS_SIGNED => {
            Security::On
        }
        _ => Security::Unknown,
    }
}

/// Asks the board at `address` to enrol `secret`'s key as an operator called `name`, on a signed
/// session that expects the pinned key `pin`. A session the board turns away (a revoked key) and a
/// board that is another node come back as Orion's own errors, typed.
pub async fn enrol(
    address: &Address,
    pin: [u8; 32],
    secret: [u8; 32],
    name: &'static str,
) -> Result<Result<Enrollment, orion::Error>, String> {
    let address = match address {
        Address::Network { host, .. } => Address::Network {
            host: host.clone(),
            key: Some(pin),
        },
        other => other.clone(),
    };
    let public = Identity::from_secret(&secret).public();
    tokio::task::spawn_blocking(move || {
        let orion = Orion::connect_with(
            &address,
            PROGRAM,
            Options {
                identity: Some(Identity::from_secret(&secret)),
                timeout: ANSWER_WAIT,
                ..Options::default()
            },
        )?;
        orion.enroll(public, name, None)
    })
    .await
    .map_err(|error| error.to_string())
}

/// The board's key and fingerprint as it answers now, with no pin. Trusts nothing. The session
/// signs with the PC's identity `key`.
pub async fn probe(env: &Env, address: &Address, key: &[u8; 32]) -> Result<Node, String> {
    let sessions = Sessions::with_identity(env.pause.clone(), *key);
    let handle = sessions
        .handle(address)
        .ok_or_else(|| "this address is turned off in Atlas".to_owned())?;
    let mut presence = handle.watch();
    let found = tokio::time::timeout(
        env.wait,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await;
    match found {
        Ok(Ok(presence)) => match &*presence {
            Presence::Found(node) => Ok(node.clone()),
            _ => Err("it did not answer as a node".to_owned()),
        },
        Ok(Err(_)) => Err("the session stopped".to_owned()),
        Err(_) => Err(not_answering(&handle.presence())),
    }
}

/// The board's address as its pin says it: the pin rule in front of a board command. The session
/// signs with the PC's identity `key`.
pub async fn pinned(
    env: &Env,
    pins: &Pins,
    text: &str,
    address: &Address,
    key: &[u8; 32],
) -> Result<Link, Refusal> {
    let Some(pin) = pins.get(text) else {
        let node = probe(env, address, key)
            .await
            .map_err(|why| Refusal::new(1, format!("the board at {text} did not answer: {why}")))?;
        return Err(Refusal::new(
            3,
            format!(
                "{text} has never been seen. Its key fingerprint is {}. Nothing is trusted yet: check the fingerprint, then run `atlas-next slots {text} pin`.",
                node.fingerprint
            ),
        ));
    };
    let sessions = Arc::new(Sessions::with_identity(env.pause.clone(), *key));
    let handle = sessions
        .handle_pinned(address, pin.key)
        .ok_or_else(|| Refusal::new(1, format!("the address {text} is turned off in Atlas")))?;
    let mut presence = handle.watch();
    let settled = tokio::time::timeout(
        env.wait,
        presence.wait_for(|presence| {
            matches!(presence, Presence::Found(_) | Presence::Replaced { .. })
        }),
    )
    .await;
    let settled = match settled {
        Ok(Ok(presence)) => presence.clone(),
        Ok(Err(_)) => return Err(Refusal::new(1, format!("the session for {text} stopped"))),
        Err(_) => {
            return Err(Refusal::new(
                1,
                format!(
                    "the board at {text} did not answer: {}. It is pinned as {}.",
                    not_answering(&handle.presence()),
                    pin.fingerprint
                ),
            ));
        }
    };
    match settled {
        Presence::Found(node) if node.public_key == pin.key => Ok(Link {
            address: address.clone(),
            handle,
            node,
            sessions,
        }),
        Presence::Found(node) => Err(replaced(text, pin, Some(node.fingerprint))),
        _ => {
            let found = probe(env, address, key)
                .await
                .ok()
                .map(|node| node.fingerprint);
            Err(replaced(text, pin, found))
        }
    }
}

/// The words for a board that answers with another key. `found` is its fingerprint, when it could
/// be read.
pub fn replaced(text: &str, pin: &Pin, found: Option<String>) -> Refusal {
    let found = found.unwrap_or_else(|| "not readable".to_owned());
    Refusal::new(
        3,
        format!(
            "the board at {text} answers with another key. Pinned: {}. Answering now: {found}. Nothing was sent to it. A reflash is the one legitimate cause: after a reflash run `atlas-next slots {text} repin --reflashed`.",
            pin.fingerprint
        ),
    )
}

/// What a board's hello says about its slots.
pub fn board_info(node: &Node) -> BoardInfo {
    let text = |name: &str| {
        node.facts
            .iter()
            .find(|fact| fact.name == name)
            .map(|fact| String::from_utf8_lossy(&fact.value).trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let lemnos = LEMNOS_FACTS
        .iter()
        .any(|name| node.facts.iter().any(|fact| fact.name == *name));
    let (offered, reported) = match text("offers") {
        Some(offers) => (atlas_slots::offers_slots(&offers), true),
        None => (false, !lemnos),
    };
    BoardInfo {
        family: text("family"),
        serial: text("serial"),
        offered,
        reported,
    }
}

fn not_answering(presence: &Presence) -> String {
    match presence {
        Presence::Lost(reason) => format!("it did not answer: {reason}"),
        Presence::Replaced { .. } => "another node answers at this address".to_owned(),
        Presence::Searching => "it has not answered yet".to_owned(),
        Presence::Found(_) => "it answered".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_session_that_opens_is_security_off_and_one_refused_for_a_signed_one_is_on() {
        assert_eq!(classify(Some(Ok(()))), Security::Off);
        assert_eq!(
            classify(Some(Err(orion::Error::Client(ClientError::Denied(
                NEEDS_SIGNED
            ))))),
            Security::On
        );
        assert_eq!(
            classify(Some(Err(orion::Error::Client(ClientError::Denied(
                "some other refusal"
            ))))),
            Security::Unknown
        );
        assert_eq!(classify(None), Security::Unknown);
    }
}
