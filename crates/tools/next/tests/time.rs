//! `atlas-next time <address> <seconds>` against a simulated board on this machine's loopback. The
//! simulated board's comm core takes no time from operators, so this shows the command's words and
//! status for such a board, and that the clock is still read. A board that takes the time is
//! checked on a board (or its emulator boot): no simulated node keeps a clock.

mod common;

use common::{env, run, scratch, start_node};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_board_that_takes_no_operator_time_is_said_so_and_is_exit_1() {
    let dir = scratch("time");
    let node = start_node(&dir, None);
    let address = format!("orion+tcp://127.0.0.1:{}", node.network().port());
    let env = env(&dir.join("state"));
    let (code, out) = run(&env, &["slots", &address, "pin"]).await;
    assert_eq!(code, 0, "{out}");

    let (code, out) = run(&env, &["time", &address, "1"]).await;
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("before: the board says its clock is not measured"),
        "{out}"
    );
    assert!(
        out.contains("this board takes no time from operators"),
        "{out}"
    );

    let (code, out) = run(&env, &["time", &address, "1", "--json"]).await;
    assert_eq!(code, 1, "{out}");
    let record: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON document");
    assert_eq!(record["offer"], "not wanted", "{record}");
    assert!(record["offset_before_ms"].is_number(), "{record}");
}
