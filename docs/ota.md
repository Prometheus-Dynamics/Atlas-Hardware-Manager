# Updating running devices (OTA): design draft

Status: draft, revised after Orion's review (its counterpart is Orion's
`docs/update-recovery.md`). atlas-driver-orion is built against
Orion v4 (`3cc974e`) and tested with a fake Orion; it waits for Orion's
operator client as its transport. The device-side writer and agent aren't
built yet. It covers devices that run a full OS on eMMC
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
| p5 | 512 MiB | root A: read-only EROFS (lzma) |
| p6 | 512 MiB | root B |
| p7 | rest, ext4 | `/data`: settings, PhotonVision config, logs, SSH keys. Kept across updates |

Moving an existing board to this layout is a one-time full reflash over USB
boot, which Atlas already does. Images keep shipping as one `.img.xz` for
that path; the same image also updates running boards (below).

## One image for everything

There is no separate update format. The same whole-disk image that is
flashed over USB (`.img`, `.img.xz`, `.img.zst`, `.img.gz`, A/B layout)
also updates a running board: the writer copies the image's **boot slot A
(p2)** and **root slot A (p5)** into the board's inactive slot and skips the
rest (p1's autoboot, the other slot, /data). The image's partitions must fit
the board's slots.

## The writer: `pd-device-update` (device package)

Installed as `/usr/lib/pd-device/update` (Unreleased; tested off-device by
`devices/raze/tests/update.sh` against a real A/B layout made with sfdisk).
Settings: `update.env`.

```
update status                                # JSON, also /run/pd-device/update.json
update stage <image> --sha256 <hex>          # check, then copy slot A into the inactive slot
update stage - --sha256 <hex> [--format xz]  # the same from stdin, checked at the end
update apply                                 # set [tryboot] to the staged slot, reboot "0 tryboot"
update confirm                               # on the trial boot, once healthy: keep it
update rollback                              # discard a staged update
```

- A file (copy it to `/data`; `/run` is RAM) is checked against `--sha256`
  (of the file as given) before anything is written. From stdin, the hash is
  checked when the stream ends; a mismatch leaves the slot written but never
  staged.
- `pd-image-slots` (a small C tool in the device package) reads the
  decompressed stream once: the MBR, then each EBR as it passes, copying p2
  and p5 straight into the inactive slot's partitions after checking they
  fit. Nothing seeks, so the stream can come from a pipe.
- The new root names its model (`/usr/lib/pd-device/device-package.env`,
  refused if it differs) and version (`IMAGE_VERSION`, else `VERSION_ID`, in
  its os-release).
- Both slots can use one boot image: `stage` rewrites `root=` in the written
  slot's `cmdline.txt` to that slot's root partition.

OS hooks, executables in `/etc/pd-device/update.d/`, given `PD_UPDATE_SLOT`,
`PD_UPDATE_VERSION` and `PD_UPDATE_IMAGE`:

| Hook | When | On failure |
|---|---|---|
| `pre-stage` | after the checksum, before writing | the stage stops (error) |
| `post-stage` | once staged | logged |
| `pre-reboot` | in `apply`, before the trial restart | stays staged, no restart |
| `post-boot` | on the trial boot, before the health check | logged |
| `update-health` (in `/etc/pd-device/`) | on the trial boot | restart into the old slot |

Besides the CLI, the package ships **`pd-device-agent`**, a small daemon in
the `orion` group that connects to orion-node's local IPC socket and claims
the node-level actions `update`, `reboot`, and `locate` for Node targets (the
claim drops on disconnect, and pending actions then fail with "handler
disconnected"). It runs the same writer and `raze-leds` as the CLI and the
identity endpoint, and after every boot republishes `status` on Orion's
status lane. Without orion-node it simply isn't connected; nothing else
depends on it.

States, reported in `status` (`state`, `slot_active`, `slot_staged`,
`version_active`, `version_staged`, `progress` 0..1000, `error`):

```
idle -> staging (verify, write inactive slot) -> staged -> trying (rebooted into it)
     -> confirmed (autoboot.txt now points at it)  |  rolled-back (trial not confirmed)
```

`confirm` runs from `pd-device-update-confirm.service`, after
`multi-user.target` and an OS-provided health check (`/etc/pd-device/update-health`;
for PhotonVision: the service is up and its HTTP port answers). The check
must not require Orion; it may add "orion-node READY" when Orion is
installed. A failed check restarts the board at once (`UPDATE_REBOOT_ON_FAIL`),
which boots the old slot. A trial that hangs is reset by the hardware
watchdog (systemd `RuntimeWatchdogSec=15s`), with the same result.

The identity endpoint advertises it: `"update_methods": ["image-write", "ab-tryboot"]`
(the second only when `status` has seen an A/B layout) and `"update"`, the
`status` object.

## Read-only root (EROFS)

Root slots hold a read-only, compressed EROFS filesystem. `CONFIG_EROFS_FS`
is built into the Raze kernel (`buildroot-external/linux/raze.config`, with
LZMA and ZSTD), so it mounts without an initramfs; the OS's cmdline adds
`rootfstype=erofs ro`. The writer copies slots raw, so nothing changes for
updates; it reads the new root's `os-release` and `device-package.env`
through a read-only mount, which auto-detects EROFS.

Nothing in the device package writes to `/` at runtime:

| What | Where |
|---|---|
| identity, update status, LED state, action requests, gadget DHCP leases | `/run/pd-device/` |
| SSH keys from the boot partition | `/run/pd-device/ssh/authorized_keys`, read by sshd through `/etc/ssh/sshd_config.d/50-pd-device.conf` (the OS's sshd_config must `Include /etc/ssh/sshd_config.d/*.conf` before any `AuthorizedKeysFile`) |
| update state | p1 (`pd-update.env`), shared by both slots |
| user overrides of the package's `*.env` settings | `/data/pd-device/` (read after `/etc/pd-device/`) |
| hostname | the kernel's transient hostname; `/etc` is never written |

The OS provides: `/data` mounted early, a persistent or transient
`/etc/machine-id`, SSH host keys on `/data`, and its own writable paths.

## Fresh installs without the button

A fresh install (new layout, wiped /data) needs USB boot. On a running CM5,
`/usr/lib/pd-device/usb-boot` sets a one-time boot order of RPIBOOT through
the firmware mailbox (`set_reboot_order`, tag 0x0003808b) and reboots; the
bootloader's own `BOOT_ORDER` never changes, and the next normal power-up
boots the eMMC. RPIBOOT has no timeout, so the board waits for the host
until it is flashed or power-cycled. The identity lists `usb-boot-reboot`
when the board supports it, and Atlas offers "Restart into USB boot" over
SSH (Orion later). It is never on the unauthenticated identity endpoint.

## Transports

Image bytes never travel over Orion. Atlas serves the image over HTTP
from the computer running it, and the device pulls `image_url` and checks
sha256, size, and (when required) the signature itself. Over SSH
(`atlas-driver-pd`, `ssh.rs`), Atlas streams the image to `/data/pd-update/`
and runs `update stage <file> --sha256 <hex>` (the board checks the copy),
reading progress from
`update status`, and `update apply`. It waits for a new boot id and polls
`status` until `confirmed` or `rolled-back`. It runs the system OpenSSH as
root with the key chosen in Settings. Host keys are trusted on first use and
pinned per board (`HostKeyAlias=pd-<model>-<serial>` in Atlas's own
known_hosts), because every board shares the USB gadget address. Atlas
offers it for boards whose identity lists `ab-tryboot`.

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
| Transfer | the device pulls `image_url` from Atlas (Orion), or Atlas copies it over SSH and runs `update stage <file> --sha256` |
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

### Self-test after an install

A verified flash (USB boot) or update (SSH, Orion) ends its job as soon as
the board is written or confirmed. atlas-core then remembers the board by its
board serial for 15 minutes. When a scan finds that board running, with the
PD driver and a `self-test` capability (its identity lists
`"diagnostics": ["selftest"]`), Atlas runs
`/usr/lib/pd-device/selftest --json` over SSH once:

- a board that doesn't answer over SSH yet is tried again on the next scans
  until the 15 minutes are up, then the failure to run is recorded;
- the result is kept per board serial with the inventory, shows as one line
  in the device's History (and the fleet activity), and fills the device's
  Self-test card, where "Run again" runs it by hand;
- a failed check is shown, never fatal: the update stays verified, and the
  board keeps running.

The self-test checks the LED ring, fan, camera, I2C sensors, watchdog and USB
gadget, and puts the fan and ring back as they were (devices/README.md).

## What Orion carries (for the v4 protocol)

Atlas does not need Orion to understand partitions; it needs:

- **Intent:** `ActionRequest { target: Node, action: "update", args: { image_url, sha256, size } }`.
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

- Where an image signing key lives (device package vs OS), if images get signed.
- Whether HeliOS images adopt the same layout, so one writer serves both.
