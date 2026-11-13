# The simulated node

The code is in `crates/drivers/mock` (package `atlas-mock`). It is a workspace crate, so the
gate formats, lints and tests it. `testing/*` is outside the workspace glob, so this folder
holds only this document. The proof is `crates/drivers/mock/tests/sim_node.rs` and
`crates/drivers/mock/tests/devices.rs`.

## What it is

- **The comm core** is Orion's own `orion-stubs` `Board` at the locked commit (`c0187f4`): the
  real server, broker, hello and keys on a Unix socket and a TCP port, in a thread of this
  process. Node logic stand-ins (records, router, files, workloads) come with it.
- **The node logic** is an `orion` client on that socket. It registers every name in
  `crates/drivers/mock/src/names.rs` as an action and declares the topics `readings`,
  `hardware` and `logs`. It answers each run through Orion's own `Registration`.
- **Mock devices** (`device.rs`): one simulated device in every `DeviceState` (running,
  recovery, boot ROM, removable disk, robot), behind a driver with discover, identify and
  grants by state. A device can be taken off the link and brought back.
- **Scripted failures** (`script.rs`): per action, the next run can be answered, refused,
  failed, stalled, or can restart the service. Per link, the next connection can be cut after
  N bytes or held still for a while. The link is a small loopback proxy in front of the
  board's network port; it is the only part that is not Orion's.
- **Slot rules and the bundle** (`slots.rs`, `bundle.rs`, `model.rs`): Lemnos's slot driver
  rules (I5) and the slot bundle format (a plain tar, `slot.json` schema 2, `rootfs.erofs`,
  `boot/`, optional `slot.sig`, Ed25519).

Runs are applied to the board state when they finish, never before. A refused, failed or
cancelled run changes nothing.

## Names and where they come from

The table is `crates/drivers/mock/src/names.rs` (`SURFACE` is the I5 method table). The
I5 surface comes from Lemnos's `I5-actions.md`. Orion needs one action per method, so each is
named `system.<contract>.<method>` with `-` for `_`.

| Brief | What it names | Node answer |
|---|---|---|
| C1 identify, reconnect | hello, sessions | Orion's own hello on both the socket and the link (`both-ways` test) |
| C2 readings, logs, hardware | topics `readings`, `hardware`, `logs` | readings with units `C`, `V`, `rpm`; description once; log lines |
| C3 actions | `system.reboot.reboot`, `locate`, `self-test`, `set-clock` | answered; `self-test` takes ticks and can stall; `set-clock` clears the offset |
| C4 files, support bundle | `support-bundle`; file push to `images/...` | bundle as text; push resumes after a cut (`a_file_dropped_mid_transfer...`) |
| C5 slots and updates | `system.slots.{list,install,write,set-default,trial,confirm,remove,pin,set-keep}` | the I5 rules, see the test `every_surface_method...` |
| C6 trial and confirm | `system.slots.trial`, `confirm`, `system.reboot.reboot` | an unconfirmed trial falls back and marks the slot failed; a confirmed one stays |
| C7 board data | `data.backup`, `data.restore`, `fs.list`, `fs.read`, `fs.write`, `fs.remove` | round trip on the data partition |
| C8 SSH, recovery shell | none | NOT simulated (see gaps) |
| C9 NT viewer | none | NOT simulated (see gaps) |

## Proof output

`cargo test -p atlas-mock` runs 27 tests: 21 on the simulated node, 4 on the mock devices,
2 unit tests. The node tests cover: every method of the I5 surface, with an assertion that
the test exercises exactly the surface; refused, failed, stalled and cancelled runs; a
streamed install with an out-of-order write refused and the install going on; cancel
discarding a partial install; a starter that disconnects having its install cancelled; a
service restart losing the running install and a fresh one starting clean; a scripted
restart step; the trial fallback and confirm paths; housekeeping with keep-last, pins and
`keep_old`; bundle refusals (symlink, broken tar, unknown member, running id, room); the
security switch (off shows `unverified`, on refuses unsigned and untrusted); readings,
hardware and link faults.

## Gaps (not simulated, and why)

- **C8 shell.** `orion-stubs` has no SSH or recovery shell service. A shell session needs
  Orion's comm-app `ssh` feature, which is outside the stubs. OUTSIDE BLOCK until the stubs or
  a comm-app harness exposes it.
- **C9 NT.** The NT4 gateway is not in `orion-stubs`. The sim publishes the values an NT
  gateway would carry, as topics, but the NT viewer's own test needs Orion's NT4 gateway
  in process.
- **C1 forwarding node.** `orion-stubs` can start a forwarding node (`start_for_node`) but
  the sim does not use it. A board behind another node is not tested here.
- **C4 one gigabyte.** The brief's 1 GB over a throttled link is for C4. A4 proves the cut and
  resume at 300 kB. The link can throttle nothing yet: it cuts and holds only.
- **Starter identity.** Orion's router does not pass the caller to the owner, so
  `NOT_PERMITTED` for "writer is not the starter" is not simulated. Every write is accepted
  from the connection that holds the install's session.
- **Session across a reboot.** The sim restarts in process: the Orion session stays up over a
  trial or reboot. A real board drops it. C6 must not depend on the session dying.

## OPEN (outside block X1 or Orion)

1. **Names.** I5 names the methods; the Orion action names here (`system.slots.set-default`,
   `system.reboot.reboot`) are ours. A change is one edit in `names.rs`.
2. **Args encoding.** Each run's args are `key=value` lines, and install/write take binary
   fixed-width fields (u64 LE, then one byte per flag). Orion's action args are opaque bytes, so
   the encoding is ours until Lemnos's client encoding is the one Orion carries.
3. **Chunk size.** Lemnos takes chunks up to 1 MiB. Orion's stub comm core runs a 64 KiB frame
   limit, so the sim refuses a chunk over 32 KiB. C5 must chunk at 32 KiB against the stub.
4. **`signed` label.** I5's enum is `trusted|untrusted|unsigned`. With the security switch off,
   the sim shows any slot not signed by a trusted key as `unverified`, as the brief for this
   PR asked. That label is not in I5; keep one or the other.
5. **Timestamps.** `installed` and `last_boot` are empty: the sim has no wall clock.
6. **Gate.** The workspace builds `orion-stubs` from git at the locked commit, a second
   declaration of the same repository and revision as the locked `orion` dependency.
