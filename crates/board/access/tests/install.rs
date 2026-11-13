//! Installing a key on the simulated node: a well-formed public key is accepted; a malformed
//! one, or a private key, is refused before anything is sent.

#![cfg(target_os = "linux")]

mod common;

use atlas_access::{AccessError, install_key};
use atlas_sim_node::access::ShellServer;
use atlas_sim_node::{Mode, NODE_NAME};
use common::{
    SimCarrier, connected, fresh, handle_for, scratch, sessions, start_node, stated, throwaway_key,
};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn a_well_formed_key_is_installed_on_the_simulated_node() {
    let dir = scratch("install");
    fresh(&dir);
    let node = start_node(&dir, Mode::Running, None);
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, node.network().port());
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());

    let key = throwaway_key("ops@desk");
    let line = key.line();
    let installed = install_key(&handle, carrier.clone(), &line, None)
        .await
        .expect("the node accepts the key");
    assert_eq!(installed.node.name, NODE_NAME);
    assert_eq!(installed.key, key);
    assert_eq!(shell.installed_keys(), vec![line]);
    assert_eq!(carrier.installs.load(Ordering::SeqCst), 1);
    assert_eq!(
        carrier.opens.load(Ordering::SeqCst),
        0,
        "no shell is opened to install a key"
    );
}

#[tokio::test]
async fn a_malformed_key_is_refused_before_anything_is_sent() {
    let dir = scratch("install-bad");
    fresh(&dir);
    let node = start_node(&dir, Mode::Running, None);
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, node.network().port());
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());

    let blob = throwaway_key("x")
        .line()
        .split(' ')
        .nth(1)
        .unwrap()
        .to_owned();
    let refused_lines = [
        String::new(),
        "ssh-rsa AAAAB3NzaC1yc2E ops".to_owned(),
        "ssh-ed25519 not*base64".to_owned(),
        "-----BEGIN OPENSSH PRIVATE KEY-----".to_owned(),
        format!("ssh-ed25519 {blob}\nssh-ed25519 {blob}"),
        format!("ssh-ed25519 {blob} ok\tsneaky"),
    ];
    for line in &refused_lines {
        let refused = install_key(&handle, carrier.clone(), line, None).await;
        assert!(
            matches!(refused, Err(AccessError::BadKey(_))),
            "{line:?} gave {refused:?}"
        );
    }
    assert_eq!(
        carrier.installs.load(Ordering::SeqCst),
        0,
        "the carrier was never called"
    );
    assert_eq!(shell.connections(), 0, "nothing was sent to the node");
    assert!(shell.installed_keys().is_empty());
}

#[tokio::test]
async fn a_key_install_on_a_node_with_another_key_than_the_one_known_is_refused() {
    let dir = scratch("install-pinned");
    fresh(&dir);
    let node = start_node(&dir, Mode::Running, None);
    let shell = ShellServer::start().expect("the shell listens");
    let sessions = sessions();
    let handle = handle_for(&sessions, node.network().port());
    connected(&handle).await;
    stated(&handle).await;
    let carrier = SimCarrier::new(shell.addr());
    let line = throwaway_key("ops@desk").line();
    let refused = install_key(&handle, carrier.clone(), &line, Some([0xBB; 32])).await;
    assert!(
        matches!(refused, Err(AccessError::KeyChanged { .. })),
        "{refused:?}"
    );
    assert_eq!(carrier.installs.load(Ordering::SeqCst), 0);
}
