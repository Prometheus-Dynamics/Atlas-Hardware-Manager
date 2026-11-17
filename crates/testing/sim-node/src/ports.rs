//! The port a simulated node listens on when its caller names none.
//!
//! A node that restarts must come back on the port it had (a board does, and the tests check it).
//! A port taken from the OS ephemeral range can be autobound by any outgoing connect on the box
//! while the node is stopped, so the restart finds it gone. This helper picks a port outside the
//! ephemeral range instead: on Linux the range is read from `/proc/sys/net/ipv4/ip_local_port_range`
//! at runtime, and the candidates are 20000 to 32767 less that range. If the file cannot be read,
//! or the OS is not Linux, the candidates are 20000 to 32767 (below the macOS and Windows defaults,
//! 49152 and up, and below Linux's 32768 and up).
//!
//! Each returned port is verified by binding it, then released for the node to bind. Ports this
//! process has already handed out are never returned again, so two nodes started in one test do
//! not pick the same port.
//!
//! Residual: another explicit bind in 20000 to 32767 (by a process or a test that chose that
//! port itself) can still take the port between the check and the node's bind. An outgoing
//! connect cannot, because it autobinds in the ephemeral range only.

use std::collections::HashSet;
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The range the helper picks from: 20000 to 32767, both included.
pub const CANDIDATES: (u16, u16) = (20000, 32767);

const PROC_RANGE: &str = "/proc/sys/net/ipv4/ip_local_port_range";

/// Ports this process has handed out, so none is handed out twice.
static HANDED_OUT: Mutex<Option<HashSet<u16>>> = Mutex::new(None);

/// Reads `ip_local_port_range` ("32768\t60999") as `(low, high)`.
pub fn parse_range(text: &str) -> Option<(u16, u16)> {
    let mut parts = text.split_whitespace();
    let low = parts.next()?.parse().ok()?;
    let high = parts.next()?.parse().ok()?;
    (parts.next().is_none() && low <= high).then_some((low, high))
}

/// The OS ephemeral range on this machine, when it can be read (Linux only).
pub fn ephemeral_range() -> Option<(u16, u16)> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    std::fs::read_to_string(PROC_RANGE)
        .ok()
        .and_then(|text| parse_range(&text))
}

/// The sub-ranges of `candidates` that do not overlap `excluded`, in order.
pub fn outside(candidates: (u16, u16), excluded: Option<(u16, u16)>) -> Vec<(u16, u16)> {
    let (low, high) = candidates;
    let Some((ex_low, ex_high)) = excluded else {
        return vec![(low, high)];
    };
    let mut pieces = Vec::new();
    if ex_low > low {
        pieces.push((low, ex_low.min(high + 1) - 1));
    }
    if ex_high < high {
        pieces.push((ex_high.max(low - 1) + 1, high));
    }
    if ex_low > high || ex_high < low {
        // No overlap at all: the whole candidate range is outside.
        return vec![(low, high)];
    }
    pieces.retain(|(a, b)| a <= b);
    pieces
}

/// Returns a port outside the ephemeral range, bound once to check it and then released. Panics
/// if no candidate port is free.
pub fn node_port() -> u16 {
    let pieces = outside(CANDIDATES, ephemeral_range());
    let total: u32 = pieces
        .iter()
        .map(|(low, high)| u32::from(high - low) + 1)
        .sum();
    let start = seed() % total;
    let mut handed = HANDED_OUT.lock().unwrap_or_else(|e| e.into_inner());
    let handed = handed.get_or_insert_with(HashSet::new);
    for step in 0..total {
        let mut index = (start + step) % total;
        let port = pieces
            .iter()
            .find_map(|(low, high)| {
                let size = u32::from(high - low) + 1;
                if index < size {
                    Some(low + index as u16)
                } else {
                    index -= size;
                    None
                }
            })
            .expect("the index is inside the pieces");
        if handed.contains(&port) {
            continue;
        }
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            handed.insert(port);
            return port;
        }
    }
    panic!("no free port outside the ephemeral range {pieces:?}");
}

fn seed() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    ((nanos ^ u64::from(std::process::id()).rotate_left(32)) as u32).wrapping_mul(2654435761)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_file_is_read_as_low_and_high() {
        assert_eq!(parse_range("32768\t60999\n"), Some((32768, 60999)));
        assert_eq!(
            parse_range("60999 32768"),
            None,
            "low above high is not a range"
        );
        assert_eq!(parse_range("junk"), None);
        assert_eq!(parse_range("1 2 3"), None);
    }

    #[test]
    fn the_candidates_avoid_the_linux_default_range() {
        assert_eq!(
            outside(CANDIDATES, Some((32768, 60999))),
            vec![(20000, 32767)]
        );
    }

    #[test]
    fn a_range_inside_the_candidates_leaves_both_sides() {
        assert_eq!(
            outside(CANDIDATES, Some((25000, 30000))),
            vec![(20000, 24999), (30001, 32767)]
        );
    }

    #[test]
    fn an_unreadable_range_means_the_whole_candidate_range() {
        assert_eq!(outside(CANDIDATES, None), vec![(20000, 32767)]);
    }

    #[test]
    fn a_range_outside_the_candidates_changes_nothing() {
        assert_eq!(
            outside(CANDIDATES, Some((1024, 2000))),
            vec![(20000, 32767)]
        );
    }

    #[test]
    fn handed_out_ports_are_outside_the_ephemeral_range_and_all_distinct() {
        let range = ephemeral_range().unwrap_or((32768, 60999));
        let mut seen = HashSet::new();
        for _ in 0..50 {
            let port = node_port();
            assert!(
                port < range.0 || port > range.1,
                "port {port} is inside the ephemeral range {range:?}"
            );
            assert!(seen.insert(port), "port {port} was handed out twice");
        }
    }

    /// The race this helper removes: an outgoing connect autobinds a local port in the ephemeral
    /// range. While such a connection is held, the helper's port is never that port's range.
    #[test]
    fn a_port_is_never_taken_from_the_range_an_outgoing_connect_autobinds_in() {
        let range = ephemeral_range().unwrap_or((32768, 60999));
        let peer = TcpListener::bind("127.0.0.1:0").expect("a peer");
        // Autobound: the kernel picks the local port, in the ephemeral range.
        let client = std::net::TcpStream::connect(peer.local_addr().unwrap()).expect("connect");
        let autobound = client.local_addr().unwrap().port();
        assert!(
            autobound >= range.0 && autobound <= range.1,
            "the autobound port {autobound} is expected in {range:?}"
        );
        for _ in 0..200 {
            let port = node_port();
            assert!(
                port < range.0 || port > range.1,
                "port {port} is inside the ephemeral range {range:?}"
            );
        }
        drop(client);
    }
}
