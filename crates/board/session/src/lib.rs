//! The connection to Orion nodes, kept for the rest of Atlas.
//!
//! A node is one Orion node on one address. Atlas keeps one [`monitor::Monitor`] per node: a
//! thread that connects, reads the node's hello document, and follows the failure rule when the
//! node is gone (never give up, a wait that doubles from a quarter second and stops at five).
//! Everything else reaches the node through a [`Handle`], which the [`Sessions`] registry hands
//! out. Readings, actions, files and the shell each call the node through a handle; none of them
//! opens its own connection.
//!
//! - [`Seed`] says where a node is: the link it sits on and its Orion address there.
//! - [`Sessions`] holds the monitors, starts one on first use, and is the manual off.
//! - [`Handle::presence`] says whether the node is connected now.
//! - [`Handle::with_client`] runs a call on the live client and answers with a [`SessionError`]
//!   that says whether the node is gone (retry later) or the call failed (it reached the node).
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use atlas_session::{RealPause, SessionError, Sessions};
//! use orion::Address;
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let sessions = Sessions::new(Arc::new(RealPause));
//! let address: Address = "orion+tcp://10.0.0.2:9200".parse()?;
//! let handle = sessions.handle(&address).ok_or("the node is turned off")?;
//!
//! // The closure runs on the node's own thread with the live client. Its error becomes
//! // `SessionError::Failed` while the session is up; a broken session is `SessionError::Gone`.
//! let name = handle
//!     .with_client(|orion| -> Result<String, orion::Error> { Ok(orion.node()?.name) })
//!     .await;
//! match name {
//!     Ok(name) => println!("node {name}"),
//!     Err(SessionError::Gone(reason)) => println!("not now, try again later: {reason}"),
//!     Err(SessionError::Failed(reason)) => println!("the node refused the call: {reason}"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The client is blocking and not `Send`, so it is only ever used inside the closure, on the
//! monitor's thread. A call that holds the session for long makes the node's other calls wait:
//! a transfer that takes long is a long operation instead ([`Handle::long`]), advanced in steps
//! between the calls.
//!
//! A long transfer, such as a file push, is not a call: [`Handle::long`] starts it on the session
//! and the session advances it in steps, between its calls (see [`long`] for the rule). The
//! transfer is a [`SteppedTransfer`], which names nothing of Orion; an adapter in the crate that
//! needs the transfer (`atlas-files`) supplies it.
//!
//! This crate depends on Orion and `atlas-types` only. It is not a driver and knows no family.

pub mod backoff;
pub mod double;
pub mod lease;
pub mod long;
pub mod monitor;
mod seed;
mod sessions;

pub use backoff::Backoff;
pub use lease::{Clock, Hold, LEASE, LEASE_RENEW, Lease, RENEW_WAIT};
pub use long::{Cancel, MAX_LONG_OPERATIONS, Outcome, STEP_WAIT, Step, StepError, SteppedTransfer};
pub use monitor::{CALL_WAIT, Handle, Monitor, PROGRAM, Pause, Presence, RealPause, SessionError};
pub use seed::Seed;
pub use sessions::Sessions;
