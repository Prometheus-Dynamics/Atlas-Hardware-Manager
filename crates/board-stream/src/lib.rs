//! board-stream: the board's push channel (docs/ota.md, "Board awareness").
//! Socket-activated per connection (board-stream.socket, TCP 5898,
//! Accept=yes): it reads one `GET /stream` request on stdin and streams
//! Server-Sent Events on stdout until the viewer goes. Read-only.
//!
//! The same binary is the board's resident per-connection listener
//! (`--serve-each`, [`serve`]): board-http.service runs the identity
//! endpoint through it, so a request starts no systemd unit.

pub mod changes;
pub mod request;
#[cfg(target_os = "linux")]
pub mod serve;
pub mod stream;

pub use changes::Changes;
pub use request::{Refusal, StreamRequest, read_request};
pub use stream::{Paths, Stream, boot_id, numbered_events};
