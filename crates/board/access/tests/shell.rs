//! The terminal on the simulated node: bytes in and out, a resize, an exit status, the shell
//! kind read from the node's own state, a changed key refused, and a session lost mid-shell.

#![cfg(target_os = "linux")]

mod common;

use atlas_access::{AccessError, End, Event, OpenRequest, ShellKind, Terminal, open};
use atlas_session::Lease;
use atlas_sim_node::access::ShellServer;
use atlas_sim_node::{DEADLINE, Kind, Mode, NODE_NAME, SimNode};
use common::{
    SimCarrier, connected, fresh, handle_for, lost, scratch, sessions, start_node, stated,
};
use std::time::Duration;

/// Reads output until it contains `wanted`. A terminal that ends first fails the test.
async fn output_until(term: &mut Terminal, wanted: &str) -> String {
    let mut seen = String::new();
    loop {
        let event = tokio::time::timeout(DEADLINE, term.recv())
            .await
            .expect("output arrives")
            .expect("the terminal has not ended");
        match event {
            Event::Output(bytes) => {
                seen.push_str(&String::from_utf8_lossy(&bytes));
                if seen.contains(wanted) {
                    return seen;
                }
            }
            Event::Ended(end) => panic!("the terminal ended before {wanted:?}: {end:?}"),
        }
    }
}

/// Reads until the terminal reports its end, and returns it.
async fn end_of(term: &mut Terminal) -> End {
    loop {
        let event = tokio::time::timeout(DEADLINE, term.recv())
            .await
            .expect("the end arrives")
            .expect("the terminal reports its end before it stops");
        if let Event::Ended(end) = event {
            return end;
        }
    }
}

#[tokio::test]
async fn a_shell_session_on_the_simulated_node_carries_bytes_resizes_and_exits() {
    let dir = scratch("shell");
    fresh(&dir);
    let node = start_node(&dir, Mode::Running, None);
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, node.network().port());
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());

    let mut term = open(&handle, carrier.clone(), OpenRequest::new(80, 24))
        .await
        .expect("the shell opens");
    assert_eq!(term.kind(), ShellKind::Normal);
    assert_eq!(term.identity().name, NODE_NAME);

    // Bytes in: the shell echoes them and answers the line.
    term.send(b"hello\n").await.expect("bytes are sent");
    output_until(&mut term, "=> hello").await;

    // A resize reaches the shell.
    term.resize(100, 30).await.expect("the resize is sent");
    output_until(&mut term, "size 100x30").await;

    // The exit status comes back, and the terminal ends once.
    term.send(b"exit 3\n").await.expect("bytes are sent");
    assert_eq!(end_of(&mut term).await, End::Exited(3));
    assert!(term.recv().await.is_none(), "nothing follows the end");
    assert!(
        matches!(term.send(b"more\n").await, Err(AccessError::Stream(_))),
        "a terminal that ended takes no more input"
    );
    assert_eq!(shell.shells(), 1);
    assert_eq!(carrier.opens.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_shell_kind_is_read_from_what_the_node_says_about_itself() {
    for (mode, want) in [
        (Mode::Running, ShellKind::Normal),
        (Mode::Recovery, ShellKind::Recovery),
        (Mode::Flasher, ShellKind::Recovery),
    ] {
        let dir = scratch(&format!("kind-{}", mode.word()));
        fresh(&dir);
        let node = start_node(&dir, mode, None);
        let shell = ShellServer::start().expect("the shell listens");
        let sessions = sessions();
        let handle = handle_for(&sessions, node.network().port());
        connected(&handle).await;
        stated(&handle).await;
        let term = open(
            &handle,
            SimCarrier::new(shell.addr()),
            OpenRequest::new(80, 24),
        )
        .await
        .expect("the shell opens");
        assert_eq!(term.kind(), want, "a node that says {}", mode.word());
    }
}

#[tokio::test]
async fn a_node_that_says_nothing_about_its_state_gets_no_shell() {
    let dir = scratch("no-state");
    fresh(&dir);
    let node = SimNode::start_with(&dir, &Kind::Plain, None).expect("the node starts");
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, node.network().port());
    connected(&handle).await;
    let carrier = SimCarrier::new(shell.addr());
    let refused = open(&handle, carrier.clone(), OpenRequest::new(80, 24)).await;
    assert!(
        matches!(refused, Err(AccessError::NoState)),
        "no guess about the shell: {refused:?}"
    );
    assert_eq!(carrier.opens.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(shell.connections(), 0, "nothing was sent to the node");
}

#[tokio::test]
async fn a_node_with_another_key_than_the_one_known_is_refused_and_no_shell_is_asked_for() {
    let dir = scratch("pinned");
    fresh(&dir);
    let node = start_node(&dir, Mode::Running, None);
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, node.network().port());
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());
    let mut request = OpenRequest::new(80, 24);
    request.expected_key = Some([0xAA; 32]);
    let refused = open(&handle, carrier.clone(), request).await;
    assert!(
        matches!(refused, Err(AccessError::KeyChanged { .. })),
        "{refused:?}"
    );
    assert_eq!(carrier.opens.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_session_lost_mid_shell_ends_the_terminal_with_the_reason_and_nothing_reopens_it() {
    let dir = scratch("lost");
    fresh(&dir);
    let node = start_node(&dir, Mode::Running, None);
    let port = node.network().port();
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, port);
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());
    let mut term = open(&handle, carrier.clone(), OpenRequest::new(80, 24))
        .await
        .expect("the shell opens");
    term.send(b"before\n").await.expect("bytes are sent");
    output_until(&mut term, "=> before").await;

    // The node goes away mid-shell. The shell's own link is left as it is, so only the session
    // being lost can end the terminal.
    drop(node);
    lost(&handle).await;
    match end_of(&mut term).await {
        End::Lost(reason) => assert!(!reason.is_empty(), "the reason is given"),
        other => panic!("expected the session to be the cause, got {other:?}"),
    }
    assert!(term.recv().await.is_none());

    // The node is back on its port. The session reconnects; the shell does not come back by
    // itself, so no second shell was ever asked for.
    let node = start_node(&dir, Mode::Running, Some(port));
    connected(&handle).await;
    stated(&handle).await;
    assert_eq!(shell.shells(), 1);
    assert_eq!(carrier.opens.load(std::sync::atomic::Ordering::SeqCst), 1);

    // A new shell is a person's choice: asking for one opens it.
    let _again = open(&handle, carrier.clone(), OpenRequest::new(80, 24))
        .await
        .expect("a new shell opens when asked for");
    assert_eq!(carrier.opens.load(std::sync::atomic::Ordering::SeqCst), 2);
    drop(node);
}

/// A shell is a hold on its board. Its session goes silent past the lease period with the shell
/// open: the board may have dropped the shell, so it ends as a lapse, with its own words, and
/// nothing reopens it. It is not a loss: the node stays connected throughout.
#[tokio::test]
async fn a_shell_whose_session_lapses_while_it_is_open_ends_as_a_lapse_not_as_a_loss() {
    let dir = scratch("lapse");
    fresh(&dir);
    let node = start_node(&dir, Mode::Running, None);
    let shell = ShellServer::start().expect("the shell listens");
    // Renewals never come (a long renew) and the period is short: silence with the hold is a lapse.
    let sessions = sessions().with_lease(Lease {
        period: Duration::from_millis(300),
        renew: Duration::from_secs(3600),
        in_force: true,
    });
    let handle = handle_for(&sessions, node.network().port());
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());
    let mut term = open(&handle, carrier.clone(), OpenRequest::new(80, 24))
        .await
        .expect("the shell opens");
    match end_of(&mut term).await {
        End::HoldLapsed(reason) => {
            assert!(reason.contains("may have dropped this shell"), "{reason}");
        }
        other => panic!("the shell ends as a lapse, not {other:?}"),
    }
}
