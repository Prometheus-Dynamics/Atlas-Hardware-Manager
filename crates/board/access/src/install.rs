//! Installing one public key on a node: the way a person's key gets trusted on a board.

use std::sync::Arc;

use atlas_session::Handle;

use crate::carrier::Carrier;
use crate::error::AccessError;
use crate::key::PublicKey;
use crate::kind::{Identity, current, pinned};

/// A key the node accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The node the key was installed on.
    pub node: Identity,
    /// The key, as it was installed.
    pub key: PublicKey,
}

/// Installs one OpenSSH ed25519 public key on the node `handle` keeps, through `carrier`.
///
/// The line is checked in form before anything is sent: a malformed line, or a private key, is
/// refused with [`AccessError::BadKey`] and the carrier is not called. Only a public key is ever
/// sent. The node must be connected, and its key must be the one `expected_key` names if one is
/// given.
///
/// # Errors
///
/// [`AccessError::BadKey`] for a line that is not an installable public key, [`AccessError::Gone`]
/// if the node is not connected, [`AccessError::KeyChanged`] if its key is not the one the caller
/// knew, and [`AccessError::Refused`] if the carrier or the node refused.
pub async fn install_key(
    handle: &Handle,
    carrier: Arc<dyn Carrier>,
    line: &str,
    expected_key: Option<[u8; 32]>,
) -> Result<Installed, AccessError> {
    let key = PublicKey::parse(line)?;
    let node = current(handle).await?;
    let identity = Identity::of(&node);
    pinned(&identity, expected_key)?;
    let sent = key.clone();
    tokio::task::spawn_blocking(move || carrier.install_key(&sent))
        .await
        .map_err(|error| AccessError::Refused(error.to_string()))?
        .map_err(AccessError::Refused)?;
    Ok(Installed {
        node: identity,
        key,
    })
}
