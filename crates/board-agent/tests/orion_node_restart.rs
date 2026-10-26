//! board-agent when orion-node restarts under it (a board's security
//! switch restarts the node seconds after boot): the node's status lane is
//! in memory, so the agent's claimed actions and update state must come back
//! on the new node without anyone asking it for anything.

#![cfg(target_os = "linux")]

mod common;

use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restarted_node_gets_the_claims_and_state_again() {
    let mut board = Board::start("restart").await;
    board
        .wait_value(update_action::KEY_STATE, text("idle"))
        .await;
    assert!(board.value("action.claimed").is_some());

    // orion-node stops and a new one starts on the same sockets.
    board.servers.abort();
    let _ = board.servers.await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let socket = board.socket.clone();
    let stream = board.dir.join("control-stream.sock");
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&stream);
    let app = NodeApp::try_new(
        NodeConfig::for_local_node(NodeId::new("raze-1")).with_ipc_socket_path(socket.clone()),
    )
    .unwrap();
    let (_, unary) = app.start_ipc_server_graceful(&socket).await.unwrap();
    let (_, stream_server) = app.start_ipc_stream_server_graceful(&stream).await.unwrap();
    board.servers = tokio::spawn(async move {
        let _servers = (unary, stream_server);
        std::future::pending::<()>().await;
    });
    board.app = app;
    assert_eq!(
        board.value(update_action::KEY_STATE),
        None,
        "a new node knows nothing"
    );

    board
        .wait_value(update_action::KEY_STATE, text("idle"))
        .await;
    assert!(
        board.value("action.claimed").is_some(),
        "the claims are published again"
    );
    board.stop().await;
}
