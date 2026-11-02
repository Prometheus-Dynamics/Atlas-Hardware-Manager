use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;

/// A shell script as the per-connection program.
fn script(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("board-serve-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("program");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A server on a loopback port, running `program` with `runtime_max`.
fn server(program: PathBuf, runtime_max: Duration) -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || run(listener, program, runtime_max));
    port
}

fn exchange(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    reply
}

#[test]
fn each_connection_gets_the_program_with_the_client_address() {
    let program = script(
        "echo",
        r#"IFS= read -r line; printf '%s from %s\n' "$line" "$REMOTE_ADDR""#,
    );
    let port = server(program, RUNTIME_MAX);
    for n in 0..3 {
        assert_eq!(
            exchange(port, &format!("GET /{n} HTTP/1.0\n")),
            format!("GET /{n} HTTP/1.0 from 127.0.0.1\n")
        );
    }
}

#[test]
fn a_program_past_its_runtime_is_killed_and_its_connection_closed() {
    let program = script("slow", "sleep 30");
    let port = server(program, Duration::from_millis(300));
    let started = Instant::now();
    assert_eq!(exchange(port, "x\n"), "");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn one_address_gets_at_most_its_share_and_the_rest_are_closed() {
    let program = script("hold", "sleep 2; echo done");
    let port = server(program, RUNTIME_MAX);
    let mut held: Vec<TcpStream> = (0..MAX_PER_SOURCE)
        .map(|_| TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap())
        .collect();
    // Let the server admit those before the one over the limit.
    std::thread::sleep(Duration::from_millis(300));
    let mut over = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    over.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut reply = String::new();
    // Closed unread: an empty reply (or a reset), long before the 2 s hold.
    let _ = over.read_to_string(&mut reply);
    assert_eq!(reply, "");
    for stream in &mut held {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "done\n");
    }
}

#[test]
fn ipv4_clients_on_the_dual_stack_socket_are_their_ipv4_address() {
    let mapped: IpAddr = "::ffff:10.77.0.1".parse().unwrap();
    assert_eq!(plain(mapped), "10.77.0.1".parse::<IpAddr>().unwrap());
    let v6: IpAddr = "fe80::1".parse().unwrap();
    assert_eq!(plain(v6), v6);
}
