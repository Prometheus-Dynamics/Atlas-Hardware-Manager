# The simulated node

The code is `crates/testing/sim-node` (package `atlas-sim-node`). It is a crate under `crates/testing/`,
so the workspace glob `crates/*/*` builds, lints and tests it; this folder holds only this document. A
driver's tests use it as a dev-dependency (a driver may: it is not a driver). The mock devices that
drivers list are in `crates/drivers/mock` (`atlas-mock`), which does not depend on it.

The proof is `crates/testing/sim-node/tests/sim_node.rs` (31 tests, Linux) and the unit tests in
`src/names.rs` and `src/lib.rs`.

## The three kinds of node

One builder makes all three: `SimNode::start_with(dir, &Kind, port)`. `SimNode::start(dir)` is
`start_with(dir, &Kind::default(), None)`.

1. **A node with Lemnos** (`Kind::WithLemnos(Hello)`, the default). It publishes the hello facts
   `family`, `serial`, `state`, `model`, `version.os`, `offers` (and `name`), the `readings` and
   `hardware` topics, and the `system.slots/1` and `system.reboot/1` surface. Test:
   `a_node_publishes_the_hello_facts_...` and the slot tests.
2. **A plain Orion node** (`Kind::Plain`). No hardware facts and no `offers`; the identity is the
   hello document's key fingerprint. No Lemnos actions, so a slot call is not found. Orion's own
   services still work: a file push, its own `fs.*` actions and the `logs` topic. Test:
   `a_plain_orion_node_has_no_lemnos_facts_or_actions_...`.
3. **A node whose Lemnos stops** (case 1, then `stop_lemnos()`). The node stays up and its facts stay
   published, but every Lemnos call fails with `-4` ("not answering"), which is not the same as the
   plain node's "not found". `resume_lemnos()` brings it back. Test:
   `a_node_whose_lemnos_stops_keeps_its_facts_...`.

## What it is

- **The comm core** is Orion's `orion-stubs` `Board` at the locked commit (`c0187f4`): the real server,
  broker, hello and keys on a Unix socket and a TCP port, in this process. Its action router and file
  receiver are Orion's stand-ins, with the rules Orion itself enforces: a detached run survives its
  starter's session and can be attached to again, a plain run is not handed to another session, and a
  cancel reaches a run only from the session and call that started it.
- **The node logic** is an `orion` client on that socket (`src/node.rs`). On a node with Lemnos it publishes
  the hello facts a board publishes (`hello.family`, `serial`, `state` = running | recovery | flasher,
  `model`, `name`, `version.os`, `offers`), registers the I5 surface and the board's own actions, and
  declares the topics `readings` and `hardware`. A plain node registers only its file actions. Both
  declare `logs` and answer each run.
- **Start and stop.** `SimNode::start(dir)` starts a running node on a free port. `SimNode::start_with(dir,
  &Hello, port)` starts one in any state, on a given port. Dropping it stops the node and frees the port;
  starting it again on the same port in the same directory brings the same device back.
- **Slots over Orion** (`src/slots.rs`, `src/bundle.rs`, `src/model.rs`). The I5 rules of Lemnos's slot
  driver. A bundle goes as a file, never as an argument: `system.slots.accept <size>` checks there is room
  (ENOSPC) and that no install runs (BUSY); the sender pushes the bundle with Orion's file delivery into
  `slots/.incoming/<name>` (the file receiver holds it on the shelf); then `system.slots.install-file
  <name> <pin> <keep_old>` is a long job. Its progress is bundle bytes over verify and unpack. The file is
  removed when the job ends, however it ends. A failed install means pushing again.
  `install` + `write` (direct socket callers in Lemnos) are not on the Orion surface.
- **Arguments** are UTF-8 words separated by spaces, as Lemnos's bridge takes them
  (`lemnos_wire::text::encode_args` at Lemnos 26364ff): numbers in decimal, bools as `true`/`false` (or
  `on`/`off`, `1`/`0`), strings as one word, bytes as hex. The node mirrors those rules in
  `src/node.rs` (`words`, `flag`, `word`, `hex_bytes`). It does not depend on the Lemnos crate, which is not in
  this workspace.
- **The slot format** (`src/bundle.rs`, the sim's own reader; it does not depend on `crates/core/bundle`,
  which is moving to schema 3 in another PR). `slot.json` is schema 3: `product` (lowercase `[a-z0-9-]`),
  `version`, optional `variant`, `board` (the board's id, the same value as the `family` hello fact), and
  `id`, `kernel`, `display_name`, `image.*`, `verity.*`, `boot.files`. The node refuses a slot whose `board`
  is missing or is not its own board, and refuses schema 2 (`slot.json is schema 2 ...`). A slot for another
  board is listed (`product`, `board`), but refused for set-default and trial. Tests:
  `a_slot_for_another_board_is_refused...`, `a_schema_2_slot_is_refused`,
  `a_slot_for_this_board_lists_its_product_and_board...`.
- **Signing** (`src/slots.rs`, `src/bundle.rs`). Off by default. A `slot.sig` that is present is always
  checked against every trusted key. Each slot lists `signed` (`trusted` | `untrusted` | `unsigned`) and
  `signer` (the owner of the key that signed it). `require_signed` turns the gate on: an unsigned or
  untrusted bundle is refused and leaves nothing behind. Keys come from a key file, one `<64 hex> <owner>` per
  line (`parse_keys`, `Slots::load_keys`); a line without an owner refuses the whole file, and the board
  then has no keys. A layered manifest (`layers`) is refused.
- **Scripted failures** (`src/script.rs`): per action, the next run is answered, refused, failed, stalled, or
  the service restarts at its step. Per link, the next connection can be cut after N bytes or held still.
  The link is a loopback proxy in front of the network port; it is the only part that is not Orion's.

Runs are applied to the board state when they finish, never before. A refused, failed or cancelled run
changes nothing.

## Names

The table is `crates/testing/sim-node/src/names.rs`. `SURFACE` is the I5 method table; each method is
`system.<contract>.<method>` with `-` for `_` (`install_file` is `system.slots.install-file`). The
hello topics are `hello.<fact>`.

| Brief | What it names | Node answer |
|---|---|---|
| C1 identify, reconnect | hello facts, `orion.node()`, the same port after a stop | the facts above; `start_with` on the port; `a_node_publishes...` test |
| C2 readings, logs, hardware | topics `readings`, `hardware`, `logs` | readings with units `C`, `V`, `rpm`; description once; log lines |
| C3 actions | `system.reboot.reboot`, `locate`, `self-test`, `set-clock` | answered; `self-test` takes ticks and can stall; `set-clock` clears the offset |
| C4 files, support bundle | `support-bundle`; file push to `slots/.incoming/...` | bundle as text; push resumes after a cut (`a_file_dropped_mid_transfer...`) |
| C5 slots and updates | `system.slots.{list,accept,install-file,set-default,trial,confirm,remove,pin,set-keep}` | the I5 rules; `every_surface_method...` test |
| C6 trial and confirm | `system.slots.trial`, `confirm`, `system.reboot.reboot` | an unconfirmed trial falls back and marks the slot failed; a confirmed one stays |
| C7 board data | `data.backup`, `data.restore`, `fs.list`, `fs.read`, `fs.write`, `fs.remove` | round trip on the data partition; bytes as hex |
| C8 SSH, recovery shell | none | NOT simulated (see below) |
| C9 NT viewer | none | NOT simulated (see below) |

## Proof

`cargo test -p atlas-sim-node` runs the unit tests and the 31 integration tests. They cover: every method
of the I5 surface, with an assertion that the test exercises exactly the surface; refused, failed, stalled
and cancelled runs; an install pushed as a file and installed as a long job; cancel discarding the pushed
file; a starter that disconnects having its install cancelled; a detached install that survives its starter
and is attached to again; a plain run that another session cannot attach to; a service restart losing the
running install and a fresh one starting clean; the trial fallback and confirm paths; housekeeping with
keep-last, pins and `keep_old`; bundle refusals; a layered manifest refused; signing (off by default, the
`signed`/`signer` fields, `require_signed`); the key file; hello facts and a restart on the same port;
the three kinds of node above; readings, hardware and link faults.

## What it cannot do (not simulated)

- **A forwarding node.** `orion-stubs` can start a node that forwards to boards behind it (`start_for_node`),
  but the sim does not use it: a board behind another node is not modelled. The SystemCore's forwarding is
  untested here.
- **SSH shell (C8).** `orion-stubs` has no SSH or recovery shell service; a shell session needs Orion's
  comm-app `ssh` service.
- **NT viewer (C9).** The NT4 gateway is not in `orion-stubs`; the sim publishes the values an NT gateway
  would carry, as topics, but nothing serves NT4.
- **Real Orion services.** The locked commit has `orion_comm_app::CommCore` with NT4 and SSH listeners
  (`nt4.listen`, `ssh.listen`), which would give C8 and C9 real services in process. This PR does not wire
  them in: they are a follow-up that exposes them behind options, off by default, with a run-time generated
  test key.
- **Starter identity.** Orion's router does not hand the caller's principal to the owner here, so the
  "same principal" rule for attaching to a detached run (Orion's C18) is not enforced in the sim: any session
  may attach to a detached run.
- **Throttling.** The link cuts and holds; it cannot slow a transfer down.
- **Clock.** `installed` and `last_boot` are empty: the sim has no wall clock.
- **Session across a reboot.** The sim restarts in process: the Orion session stays up over a trial or
  reboot. A real board drops it, so C6 must not depend on the session dying.

## OPEN

1. **Names.** `system.slots.*` names are ours (the I5 method names with `-`); a change is one edit in
   `names.rs`.
2. **Chunk size.** A file push runs through Orion's delivery; the sim's comm core has a 64 KiB frame limit.
3. **Gate.** The workspace builds `orion-stubs` from git at the locked commit, a second declaration of the
   same repository and revision as the locked `orion` dependency.
