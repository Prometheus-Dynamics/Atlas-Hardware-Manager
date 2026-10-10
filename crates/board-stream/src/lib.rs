//! board-stream: the board's push channel (docs/ota.md, "Board awareness").
//! Socket-activated per connection (board-stream.socket, TCP 5898,
//! Accept=yes): it reads one `GET /stream` request on stdin and streams
//! Server-Sent Events on stdout until the viewer goes. Read-only.

#[cfg(unix)]
pub mod hardware;
pub mod request;
pub mod stream;

pub use request::{Refusal, StreamRequest, read_request};
pub use stream::{Paths, Stream, boot_id, numbered_events};
