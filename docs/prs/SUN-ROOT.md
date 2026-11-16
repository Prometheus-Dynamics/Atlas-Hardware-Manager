# SUN-ROOT: a Unix socket path can never overflow sun_path, whatever the names are

**Group:** SUN (socket paths)

**Needs:** nothing (based on next d7f68463)

**Runs:** local

## Problem

A Unix socket's address is `sun_path`: 108 bytes with the NUL, so at most 107 bytes of path. Orion's test
stub binds `<dir>/orion.sock` and panics inside its own thread (`expect("socket")`, "path must be shorter than
SUN_LEN") when that path is too long. Atlas built `<TMPDIR>/<prefix>-<pid>-<name>/...` paths from caller names,
so three failures came from long names or a long TMPDIR: the gate's scratch root on CI, the test
`refresh-session`, and `tests/cycle.rs` `an_address_pinned_to_another_key_stops_at_step_3_with_exit_3`.

## Fix at the root

- `crates/core/types/src/socket.rs` (new, `atlas_types::socket`): the one socket-path helper.
  - `SUN_PATH_MAX = 107`; `check_socket_path(path)` returns the typed `SocketError::TooLong { path, bytes }`,
    whose Display names the path, its length and the limit. Nothing binds before the check.
  - `node_dir(name)` / `node_dir_in(base, name)`: a fresh directory `<base>/<fnv1a-64 of name, 16 hex>-<pid>-<counter>`,
    with the readable name written to its `name` file. The directory name has a fixed length, so a 200-character
    name gives the same short path as a 1-character one. The socket path is checked before the directory is made.
  - `node_dir(name)`: base `$XDG_RUNTIME_DIR` when it is an absolute path and leaves room, else the temp dir when that leaves room, else a typed error.
  - `NODE_SOCKET_SUFFIX_MAX = 52`: the fixed worst-case bytes past the base (`/atlas-t-` prefix, 16-digit hash, pid, counter, `/orion.sock`). The gate uses it.
- Sim and rigs (`crates/testing/sim-node`): `SimNode::start_with` / `start_secured` check `<dir>/orion.sock`
  before anything is made or bound and return the typed error as a string. `sim-node` now depends on
  `atlas-types` (an internal path dependency, no external crate). It re-exports the module as
  `atlas_sim_node::socket`.
- Product (`crates/board/session`, `crates/board/actions`, `crates/tools/next`): every `Address::Local`
  is checked where it comes in and where it is used (see the inventory below).
- Orion's stub is never handed a too-long path. Its pin (671d29d6) is not moved.

## Inventory: every place a path becomes a Unix socket path

Product (socket paths from Address::Local; there is no UnixListener in product code):

| # | file:line | what |
|---|---|---|
| P1 | crates/board/session/src/seed.rs:15 | `check_address` (the one product check, pub) |
| P2 | crates/board/session/src/seed.rs:37 | `Seed::new` refuses a too-long local socket at creation |
| P3 | crates/board/session/src/monitor.rs:706 | monitor run loop: a path error stops the monitor once (Presence::Lost names it, no retry) |
| P4 | crates/board/actions/src/jobs.rs:163 | `run_step`: saved action address checked |
| P5 | crates/board/actions/src/jobs.rs:369 | `stop_on_board`: address checked before a stop |
| P6 | crates/tools/next/src/link.rs:53 | `transport_key` |
| P7 | crates/tools/next/src/link.rs:82 | `enrol` |
| P8 | crates/tools/next/src/link.rs:110 | `probe` |

`DEFAULT_SOCKET` (`/run/orion/orion.sock`, Orion's constant) is fixed and short; no XDG path is involved.
The monitor check (P3) also covers it. No reconnect loop retries a path error: P3 breaks out of the loop.

Sim and test rigs (dir that becomes `<dir>/orion.sock`):

| # | file:line | what |
|---|---|---|
| R1 | crates/testing/sim-node/src/node.rs:353 | `SimNode::start_inner`: the check before `Board::start_with` (stub bind at :356) |
| R2 | crates/testing/sim-node/src/lib.rs | re-export `atlas_sim_node::socket` |
| R3 | crates/testing/sim-node/tests/sim_node.rs `start`, `hello-restart`, `sim-plain` | `node_dir` |
| R4 | crates/testing/sim-node/tests/identity_facts.rs `start`, `identity-plain` | `node_dir` |
| R5 | crates/testing/sim-node/tests/trial.rs `start` | `node_dir` |
| R6 | crates/board/session/tests/common/mod.rs `scratch` | `node_dir` (replaces the 50-name length table) |
| R7 | crates/board/access/tests/common/mod.rs `scratch` | `node_dir` |
| R8 | crates/board/actions/tests/actions.rs `scratch` | `node_dir` |
| R9 | crates/board/files/tests/push.rs `scratch` | `node_dir` (the source file is now `scratch(..).join("source.bin")`) |
| R10 | crates/board/topics/tests/hardware.rs `scratch` | `node_dir` |
| R11 | crates/board/nt/tests/orion_gateway.rs `Board::start` | comm core's dir via `node_dir` (nt gets an atlas-types dev-dependency) |
| R12 | crates/drivers/board/tests/simulated_node.rs `scratch` | `node_dir` |
| R13 | crates/drivers/board-slots/tests/common/mod.rs `scratch` | `node_dir` |
| R14 | crates/tools/next/tests/common/mod.rs `scratch` | `node_dir` |

Sub-directories joined under a helper directory (`dir.join("node")`, `dir.join("replacement")`) are not
checked by the helper itself. They add at most 23 bytes to the base, and `start_with` checks the final path
and returns the typed error if it does not fit. They are: tools/next `tests/common` (`start_node`, `secured`),
`tests/cycle.rs:327`, `tests/slots.rs:75`, `tests/security.rs:20`, board-slots `tests/common` (`start_leased`),
`tests/trial.rs:537,575`, `tests/slots.rs:592`, files `tests/push.rs:662,735`.

Total: 8 product sites and 14 rig or test sites, 22 in all. The queue note said about six.

## Gate

`scripts/gate.sh`: `SOCKET_SUFFIX_BYTES` is 52 (the helper's constant). The room check uses `XDG_RUNTIME_DIR` when it
is set and leaves room; only when it is unset or too short does the check fall back to TMPDIR. The self-test is kept (80 bytes refused, 52 bytes accepted). A unit test
(`the_suffix_constant_is_the_gate_figure`) keeps the Rust constant and the gate figure equal.

## Tests

- Unit (atlas-types, `socket::tests`): a 200-character name gives a path under the limit and the name file
  holds the name; two names in one process give two directories; a base that is too long gives the typed
  error naming the path and nothing is made; a path one byte too long is refused; the suffix constant is 44.
- Real rig: `sim_node.rs::a_rig_with_a_200_character_name_starts_and_answers`, which starts a node and checks
  it listens on its socket.
- Refusal: `sim_node.rs::a_socket_path_too_long_for_a_unix_socket_is_refused_at_start_naming_it`, which
  checks the error names the path and nothing is made.
- Session: `session.rs::a_long_name_still_gives_a_socket_that_fits` replaces the old length table test.
- Proof: the cycle and topics suites run with TMPDIR set to a 61-byte directory: cycle 6/6, topics 32+6+9, all
  pass. The same suites with `XDG_RUNTIME_DIR` unset (the fallback to TMPDIR): see the report.
- Cleanup check: `/run/user/1000` before and after the full changed-crate run has no `atlas-t-` entries.

## Notes

- The helper can be simplified once Orion's `SOCKET_PATH_MAX` and `Board::try_start_with` are pinned: the
  stub could return a typed error instead of panicking, and the check here becomes a second line of defence.
- Node directories are removed by their guard when they drop (see Naming and cleanup). A test that keeps a dir
  past its own scope keeps the guard with it.
- `XDG_RUNTIME_DIR` is read only by `node_dir`; the product's fixed `/run/orion/orion.sock` path needs no XDG handling.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
