//! A board's SSH host keys as its own Orion node publishes them.
//!
//! The board's node publishes `identity.ssh_host_key.<type>` (`ed25519`,
//! `ecdsa`, `rsa`) as `SHA256:<base64>`, the form `ssh-keygen -l` prints.
//! Atlas compares those with the key SSH presents before it trusts a changed
//! key. A source answers only over an Orion session that is authenticated to
//! the node: a key read over an unauthenticated connection is never a proof.

use async_trait::async_trait;

/// One host key a board publishes over Orion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedHostKey {
    /// `ed25519`, `ecdsa` or `rsa`.
    pub kind: String,
    /// `SHA256:<base64>`, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
}

/// Where the keys a board publishes come from.
#[async_trait]
pub trait HostKeySource: Send + Sync {
    /// The keys the board with this serial publishes, read over an
    /// authenticated Orion session. Empty when there is no such link, or the
    /// board publishes none: the caller then keeps its own rules.
    async fn published_host_keys(&self, serial: &str) -> Vec<PublishedHostKey>;
}
