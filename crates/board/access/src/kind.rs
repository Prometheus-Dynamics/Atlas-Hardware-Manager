//! Which shell a node gets, and who the node is, from what the node says about itself.
//!
//! The node's signed hello document is the only source. Its `state` fact says `running`, or
//! `recovery` (Orion's recovery build, the comm core that could not start as it was told to), or
//! `flasher` (a freshly loaded flasher image, which runs the recovery build). A node that says
//! nothing about its state, or says a word this crate does not know, gets no shell: nothing is
//! guessed from the node's name, its address or its facts about hardware.

use orion::node::Node;

use crate::error::AccessError;

/// The name of the hello fact that says the node's state.
const STATE_FACT: &str = "state";

/// The shell a person gets on a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    /// The normal shell of a node that runs its own software.
    Normal,
    /// The recovery shell: the node is in recovery, or a flasher image.
    Recovery,
}

/// The state a node says it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// Booted and running its own software.
    Running,
    /// In recovery.
    Recovery,
    /// A freshly loaded flasher image.
    Flasher,
}

impl ShellKind {
    /// The shell for a node in `state`.
    #[must_use]
    pub const fn for_state(state: NodeState) -> Self {
        match state {
            NodeState::Running => Self::Normal,
            NodeState::Recovery | NodeState::Flasher => Self::Recovery,
        }
    }
}

/// Reads the state a node's hello says it is in.
///
/// # Errors
///
/// [`AccessError::NoState`] if the hello has no `state` fact, [`AccessError::UnknownState`] if
/// it says a word that is not one of the three.
pub fn state_of(node: &Node) -> Result<NodeState, AccessError> {
    let fact = node
        .facts
        .iter()
        .find(|fact| fact.name == STATE_FACT)
        .ok_or(AccessError::NoState)?;
    let word = String::from_utf8_lossy(&fact.value).trim().to_owned();
    match word.as_str() {
        "running" => Ok(NodeState::Running),
        "recovery" => Ok(NodeState::Recovery),
        "flasher" => Ok(NodeState::Flasher),
        _ => Err(AccessError::UnknownState(word.chars().take(32).collect())),
    }
}

/// Who a node is, from its signed hello: the key the hello is signed with is the identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The node's name.
    pub name: String,
    /// The node's key fingerprint, for people to compare.
    pub fingerprint: String,
    /// The node's public key, as its signed hello gave it.
    pub public_key: [u8; 32],
}

impl Identity {
    /// The identity of a node the session has identified.
    #[must_use]
    pub fn of(node: &Node) -> Self {
        Self {
            name: node.name.clone(),
            fingerprint: node.fingerprint.clone(),
            public_key: node.public_key,
        }
    }
}

/// The node as it answers now: its signed hello, asked for on the live session, so the facts
/// it publishes after it connected are seen. Refused as the session says when it is not
/// connected.
pub(crate) async fn current(handle: &atlas_session::Handle) -> Result<Node, AccessError> {
    handle
        .with_client(|orion| orion.node())
        .await
        .map_err(|error| match error {
            atlas_session::SessionError::Gone(reason) => AccessError::Gone(reason),
            atlas_session::SessionError::Failed(reason) => AccessError::Refused(reason),
        })
}

/// Refuses a node whose key is not the one the caller knew this board by.
pub(crate) fn pinned(identity: &Identity, expected: Option<[u8; 32]>) -> Result<(), AccessError> {
    match expected {
        Some(key) if key != identity.public_key => Err(AccessError::KeyChanged {
            found: identity.fingerprint.clone(),
        }),
        _ => Ok(()),
    }
}
