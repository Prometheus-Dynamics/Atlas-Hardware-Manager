# Updating running devices (OTA): design draft

Status: draft, revised after Orion's review (its counterpart is Orion's
`docs/update-recovery.md`). Nothing here is built yet; atlas-driver-orion
waits for Orion's v4 types. It covers devices that run a full OS on eMMC
(Raze today) and leaves microcontrollers (STM32) for their own driver.

## Principles

1. **The device package owns the writer.** Only code shipped in the device
   package (`devices/<model>/`) writes a device's storage. The same writer
   runs no matter who asked: Atlas over USB, Atlas over the network, Orion,
   or a person at the console.
2. **Orion carries intents and progress, not bytes on disk.** Orion relays
   "update to X", streams progress, and reports the outcome. It never writes
   partitions itself and is never the only way in.
3. **Atlas works without Orion.** Every update path has a fallback that
   needs no running agent: an update over SSH (opt-in keys, package 1.0.7),
   and, always, a full reflash over USB boot.
4. **An update never bricks a board.** The new system boots once on trial
   and must confirm itself; otherwise the board falls back to the old one by
   itself. USB-boot recovery remains the last resort.
5. **One vocabulary everywhere.** The steps and outcomes are Atlas's
   existing ones (`atlas-driver` `UpdateStep`, `UpdateOutcome`), so the UI,
   jobs, staged rollout, and history work unchanged for OTA.

## Mechanism on Raze: A/B with the Pi bootloader's tryboot

The CM5 bootloader supports `autoboot.txt` with an A/B pair and a one-shot
trial boot (`reboot "0 tryboot"`): the `[tryboot]` section names the
partition to try; if that boot never confirms, the next reset boots the old
one again.

Target eMMC layout (replaces today's two partitions):

| # | Size | Content |
|---|------|---------|
| p1 | 16 MiB FAT | `autoboot.txt` only (`tryboot_a_b=1`, `boot_partition=2`, `[tryboot] boot_partition=3`) |
| p2 | 128 MiB FAT | boot A: kernel, DTBs, overlays, `config.txt`, `cmdline.txt` (root = p5) |
| p3 | 128 MiB FAT | boot B: same, root = p6 |
| p4 | extended | |
| p5 | 2 GiB ext4 | root A |
| p6 | 2 GiB ext4 | root B |
| p7 | rest, ext4 | `/data`: settings, PhotonVision config, logs, SSH keys. Kept across updates |

Moving an existing board to this layout is a one-time full reflash over USB
boot, which Atlas already does. Images keep shipping as one `.img.xz` for
that path; the image builder also emits an update bundle (below).

## The update bundle

`<os>-<version>-<model>.pdupdate`: a tar with

- `manifest.json`: model, compatible revisions, OS name and version,
  device-package version, minimum current version (if any), and per-file
  size and sha256;
- `boot.vfat.zst` and `rootfs.ext4.zst`: the slot images;
- `manifest.sig` (optional): ed25519 over `manifest.json`. Signing follows
  the Atlas release rule: used when present, never required for a local
  file, required when the bundle arrives over an unauthenticated transport.

## The writer: `pd-device-update` (device package)

```
pd-device-update status                  # JSON on stdout, also /run/pd-device/update.json
pd-device-update stage <bundle|->        # verify + write the inactive slot; '-' reads stdin
pd-device-update apply                   # set [tryboot] to the staged slot, reboot "0 tryboot"
pd-device-update confirm                 # run by the new system once healthy: make it the default
pd-device-update rollback                # discard a staged update
```

Besides the CLI, the package ships **`pd-device-agent`**, a small daemon in
the `orion` group that connects to orion-node's local IPC socket and claims
the node-level actions `update`, `reboot`, and `locate` for Node targets (the
claim drops on disconnect, and pending actions then fail with "handler
disconnected"). It runs the same writer and `raze-leds` as the CLI and the
identity endpoint, and after every boot republishes `status` on Orion's
status lane. Without orion-node it simply isn't connected; nothing else
depends on it.

States, reported in `status` (`state`, `slot_active`, `slot_staged`,
`version_active`, `version_staged`, `progress` 0..1, `error`):

```
idle -> staging (verify, write inactive slot) -> staged -> trying (rebooted into it)
     -> confirmed (autoboot.txt now points at it)  |  rolled-back (trial not confirmed)
```

`confirm` runs from `pd-device-update-confirm.service`, after
`multi-user.target` and an OS-provided health check (`/etc/pd-device/update-health`;
for PhotonVision: the service is up and its HTTP port answers). The check
must not require Orion; it may add "orion-node READY" when Orion is
installed. The trial is bounded by the hardware watchdog
(`dtparam=watchdog=on`) with systemd `RuntimeWatchdogSec`: no confirm within
N minutes means a reset, which boots the old slot.

The identity endpoint advertises it: `"update_methods": ["image-write", "ab-tryboot"]`
and `"update": {"state": ..., "slot": ..., "staged": ...}` from `status`.

## Transports

Bundle bytes never travel over Orion. Atlas serves the bundle over HTTP
from the computer running it, and the device pulls `bundle_url` and checks
sha256, size, and (when required) the signature itself. Over SSH, Atlas
streams it to `pd-device-update stage -`.

| Path | Needs | Auth | Used when |
|------|-------|------|-----------|
| Orion (intent) + Atlas URL (bytes) | orion-node and pd-device-agent on the device | Orion's | Default for fleets |
| SSH | Package 1.0.7 keys installed | SSH key | No Orion, or Orion faulty |
| USB boot full image | Nothing on the device | Physical | Always available; migration and recovery |

There is deliberately no unauthenticated upload on the identity endpoint
(port 5899): it stays read-only apart from harmless actions such as locate.

## How Atlas drives it

One `UpdateCapability` per transport, picked by the driver in this order:
Orion, then SSH, then "needs recovery" (Atlas offers the USB-boot flash).
They map onto the existing steps:

| Atlas step | Writer |
|------------|--------|
| Preflight | `status`: compatible model/revision, enough space, not mid-update |
| Transfer | the device pulls `bundle_url` from Atlas (Orion), or `ssh … pd-device-update stage -` |
| Apply | `stage` verify + slot write, then `apply` |
| Reboot | wait for the device to drop and come back (identity endpoint, board_serial) |
| Confirm | durable state after boot: `update.state` = `confirmed` with the new version → `Verified`; `rolled-back` → `RolledBack` |

The outcome is never taken from an action result. The `update` action ends
`Succeeded` with `{phase: "rebooting", version_staged}` just before the
reboot, meaning only "staged and apply issued", because action records live
in orion-node's memory and don't survive it. Reboot and Confirm read durable
state: the node's host facts (OS/image version, `board_serial`) and the
status keys pd-device-agent republishes after boot. Atlas keeps its Orion
capability behind one interface, so moving from actions to Orion's later
durable UpdateIntent/UpdateStatus records (milestone U3) changes nothing in
these steps.

Concurrency is `Parallel`: each board updates itself, so staged rollout
(one board first) and bulk updates work as they do today.

## What Orion carries (for the v4 protocol)

Atlas does not need Orion to understand partitions; it needs:

- **Intent:** `ActionRequest { target: Node, action: "update", args: { bundle_url, sha256, size } }`.
- **Progress while it runs:** status-lane keys `action.<action_id>.state|progress|error`
  under the action's target subject (names to be confirmed with v4).
- **Durable status across reboots:** stable keys under the Node subject,
  republished by pd-device-agent after boot: `update.state`,
  `update.version_active`, `update.slot_active`, `update.error`.
- **Action result:** "staged and apply issued" only (see above).
- **Facts:** `board_serial` raw from `/proc/device-tree/serial-number`
  (DMI as a fallback), `board_model`, and `machine_id`. Atlas normalizes
  `board_serial` to its matching rule (the last 8 hex digits, lowercase) to
  merge the Orion record with the pd identity and USB records of a board.

How Atlas reaches Orion: as an enrolled peer (orion+tcp, signed), with
actions forwarded across nodes, ideally through an Orion client library
compiled into Atlas behind an optional feature, rather than a separate
orion-node on the user's computer. The HTTP control API is the fallback.

## Open questions

- Slot sizes: is 2 GiB per root enough for PhotonVision + JDK with headroom?
- Where the bundle signing key lives (device package vs OS).
- Whether HeliOS images adopt the same layout, so one writer serves both.
