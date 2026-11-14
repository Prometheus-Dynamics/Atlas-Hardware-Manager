//! The connection to one board, behind the pin rule. Every board command goes through
//! [`pinned`]; `fingerprint`, `pin` and `repin` read the board with [`probe`] (a session that
//! trusts nothing, which only reads the key).

use std::sync::Arc;

use atlas_session::{Handle, Presence, Sessions};
use orion::Address;
use orion::node::Node;

use crate::Env;
use crate::commands::Refusal;
use crate::pins::{Pin, Pins};

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

/// The board's key and fingerprint as it answers now, with no pin. Trusts nothing.
pub async fn probe(env: &Env, address: &Address) -> Result<Node, String> {
    let sessions = Sessions::new(env.pause.clone());
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

/// The board's address as its pin says it: the pin rule in front of a board command.
pub async fn pinned(
    env: &Env,
    pins: &Pins,
    text: &str,
    address: &Address,
) -> Result<Link, Refusal> {
    let Some(pin) = pins.get(text) else {
        let node = probe(env, address)
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
    let sessions = Arc::new(Sessions::new(env.pause.clone()));
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
            let found = probe(env, address).await.ok().map(|node| node.fingerprint);
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
