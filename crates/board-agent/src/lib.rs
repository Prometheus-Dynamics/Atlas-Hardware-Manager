//! board-agent: the device agent of the board device packages (Orion
//! `docs/device-agent.md`, Atlas `docs/ota.md`).
//!
//! It connects to orion-node's local IPC, claims the node actions `update`,
//! `update.cancel`, `update.rollback`, `reboot` and `locate`, runs them with
//! the package's writer (`/usr/lib/board/update`) and locate request, and
//! keeps the writer's state published as the `update.*` status keys of its
//! node. Without orion-node it waits and retries; nothing on the board
//! depends on it.
//!
//! Linux only: on other systems the crate is empty.
#![cfg(target_os = "linux")]

mod agent;
mod clock;
mod config;
mod stage;
mod writer;

use std::sync::Arc;

use orion_client::{ClientError, LocalNodeRuntime, LocalProviderService, LocalServiceRetryPolicy};
use orion_control_plane::ProviderRecord;
use orion_core::{NodeId, ProviderId};

pub use agent::{Agent, CLAIMED_ACTIONS, StageRequest, update_status_entries};
pub use config::Config;
pub use writer::{Writer, WriterStatus};

/// The client name on the node.
pub const CLIENT_NAME: &str = "board-agent";

/// `BOARD_EVENT_SOURCE` for everything the agent runs: the board's event log
/// then says Orion asked.
pub const EVENT_SOURCE: &str = "orion";

/// One line on stderr (the journal under systemd).
pub fn log(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "board-agent: {message}");
}

/// Claims the actions and serves them until the connection fails for good.
/// The service retries connecting with `config.retry` without a limit, so
/// this waits for orion-node to come up (or back).
pub async fn run(config: Config) -> Result<(), ClientError> {
    let config = Arc::new(config);
    let runtime = LocalNodeRuntime::new(&config.control_socket, &config.stream_socket);
    // Claims need no registered provider; the record only names the client.
    // The node id comes from the node itself.
    let service = LocalProviderService::new(
        runtime,
        CLIENT_NAME,
        ProviderRecord::builder(ProviderId::new(CLIENT_NAME), NodeId::new("local")).build(),
    )
    .with_retry_policy(LocalServiceRetryPolicy::fixed_delay(config.retry));
    log(&format!(
        "connecting to orion-node at {} (waiting until it accepts)",
        config.control_socket.display()
    ));
    let mut watch = service.claim_node_actions(CLAIMED_ACTIONS).await?;
    log(&format!(
        "connected to node {}; claimed {}",
        watch.node_id(),
        CLAIMED_ACTIONS.join(", ")
    ));
    let agent = Agent::new(config, watch.reporter());
    agent.publish_initial().await;
    let follower = AbortOnDrop(tokio::spawn(agent.clone().follow()));
    let mut reconnects = watch.reconnects();
    loop {
        let request = watch.next().await?;
        if watch.reconnects() != reconnects {
            reconnects = watch.reconnects();
            agent.publish(true).await;
        }
        agent.handle(request).await;
        let _ = &follower;
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
