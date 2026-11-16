# PINS-2: Orion 3dde304 (C54), the image's Orion component, and the systemd account rule

**Status: DRAFT. Do not merge.** The pair in this tree is mismatched on purpose.

## Provisional pins

| Pin | Rev on this branch | Final |
|---|---|---|
| Orion (workspace `orion`, `atlas-nt`, `atlas-topics`, `atlas-sim-node`, image `orion` and `orion-recovery`) | `3dde304e396f5769d3aa25884b5a9ff5bd4b9180` (C54) | provisional |
| Lemnos (`lemnos-slot-store`, every Lemnos component) | `31e0f736a334260a7df2ea86bcfa358e9a640f08` (unchanged) | provisional |

The final pair is the Lemnos rev whose bridge pins the Orion rev that carries the pending node fix (the
fix for a link bounce that drops a new hello, INPUTS 2026-11-16). It will be announced; this PR is then
re-pinned to that pair before it can merge. Until then the tree carries Lemnos 31e0f736, whose bridge pins
Orion 671d29d, next to Orion 3dde304. The coordinator approved building the content on 3dde304 now.

Orion's own Lemnos dependency (its `shm-*` crates) moves with Orion's manifest from bded77a0 to b0a75d93.
Atlas's Lemnos pin does not move, so the lock names one Lemnos rev per package (the one-copy rule holds).

## Changes, in order

1. **Compile on Orion 3dde304** (`e4258407`). Every Orion pin moves from 671d29d; `cargo update -p` for the
   41 Orion packages changes `Cargo.lock` only. `cargo check --workspace --all-targets --locked` passes with
   no code change. The Orion-using crates' tests pass: atlas-actions, atlas-nt, atlas-topics, atlas-sim-node,
   atlas-board-slots.

2. **Run handles in the six follow and cancel sites: NOT DONE in this PR (stopped).** The sites are
   `board.rs` (follow, cancel), `jobs.rs` (the step, the cancel) and `install.rs` (the step, the cancel).
   The open question is where the handle is kept:
   - The job record's steps are fixed at submit (`crates/core/jobs`, `Jobs` has no step update), and the
     call number is held only in memory (`jobs.rs` `Tracked`, `install.rs` `Run`). Nothing is persisted
     today, so "persist the handle in the job file" needs a write-back path in the jobs crate, and a rule
     for when it is written (the start returns the call before ACTION_STARTED names the handle).
   - The Unknown state is T6b's (PR #141, not in next). Until it is, the minimal typed error lands with
     the handle work and joins T6b's Unknown state when both are in.
   The handle work is therefore not in this PR; the collision the handles fix stays open until it lands.

3. **The image's Orion component** (`57f69fd4`), `devices/components/orion` and `orion-recovery`:
   - Source ref is `3dde304e`. `scripts/check-devices.sh --write-locks` (GAIA_BIN at Gaia a6299c62, which
     reports `gaia 2.0.6-dev (a6299c62…)`) regenerated `devices/products/raze-base/gaia.lock`; it moved only
     the two Orion refs. The other product locks are unchanged.
   - Accounts, declared in this component only: `orion` (901, group orion, groups dialout) and
     `orion-comm` (902, group orion, groups dialout), with Gaia's `[[users]]` fields; the `orion` group (901)
     is declared by `[[groups]]`.
   - Units, as Orion installs them at this rev: `orion-comm.service` (User=orion-comm, RuntimeDirectory=orion),
     `orion-node.service`, `orion-login.socket` and `orion-login.service`. Nothing ships `/run/orion` or
     `/var/lib/orion`: the units make `/run/orion` at runtime, and `/var/lib/orion` is not used (see the
     state override below).
   - **The state override stays, with its reason.** `state.dir`, `keys.dir` and `ssh.host_key` point at
     `/data/orion` (orion.conf). `/var` is inside the slot root on a managed board, so a key there changes at
     a reflash or a slot change; `/data` is the same for every slot. It still works with `StateDirectory=`:
     the unit's `StateDirectory=orion` would create `/var/lib/orion` for nothing, so the drop-in clears it
     (`StateDirectory=`), and the drop-in makes `/data/orion` itself. That directory was root's, which the
     unit running as orion-comm could not write, so it is now owned by orion-comm with group orion, so
     node logic can read the keys. The node's own state is `/data/orion/node` the same way.
     `RequiresMountsFor=/data` is kept (Orion's unit has no mount ordering for it).
   - `orion-login.socket` is enabled. It starts `orion-login.service` on the first question, and it names
     `orion-comm` as its SocketUser, so the account must exist (see change 4).
   - Settings: `socket.node_uids = 901` kept; `login.comm_user = orion-comm` written out (Orion's default,
     the only user the login helper answers).
   - **Bridge address.** `lemnos-orion` gets a drop-in, `ORION_ADDRESS=unix:/run/orion/orion.sock`, the
     node's local socket. The bridge refuses a network or signed-serial address at start (Lemnos
     `service/orion/src/config.rs`), and an unset address already means this socket; the drop-in writes it out.

4. **The product check for accounts** (`4fd44bb2`), in `scripts/check-devices.sh`. For each product, every
   unit a component installs (drop-ins included) is read, and each `User=`, `Group=`, `SocketUser=` and
   `SocketGroup=` that is not `root` and not numeric must be declared by a component of the product (a
   `[[users]]` name, or a `[[groups]]` name, a user's group counting). A refusal names the unit and the
   account. The units come from `file =` beside the component, or from `source =` at the commit the lock pins,
   read from a bare clone under `ATLAS_UNIT_CACHE` (default `~/.cache/atlas/units`), which a first run clones.
   Self-test: the Raze product passes as the tree has it, and on a scratch copy with the orion-comm block
   removed it is refused, naming `orion-comm.service` and `orion-comm` (and `orion-login.socket`'s SocketUser).
   The rule's refusal today: `FAIL products/raze-base: orion-comm.service names User=orion-comm, which no
   component of the product declares as an account …`.

5. **This document.**

## What the next pin move adds (not in this PR)

- **T8, session liveness on `Reply::Alive`** (INPUTS 2026-11-16, C53): `Options.reply = Reply::Alive{ceiling}`
  with `set_reply` and `silent_for()`, so a healthy but slow reply is not `Gone("timed out")`. Lands on the
  pin after this one, with the sim's own idle window.
- **The return of the detached slot list (T6)**, on the fixed Orion rev, in one PR with a deterministic test
  (T6 was reverted by #135 after the kept-answer collision, INPUTS 2026-11-16).
- **C55 (typed too-long socket path)** is in Orion after C54 (`ee05527e`); Atlas's SUN-ROOT side uses it once
  the Lemnos bridge carries it. Not in this pin.

## Open

- The run-handle design above (item 2) needs a ruling on the jobs crate's write-back and on when the handle is
  saved.
- The pair (Lemnos rev + Orion rev) is announced later; this PR is re-pinned then and goes to ready.
