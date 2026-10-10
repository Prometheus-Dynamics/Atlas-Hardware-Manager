# Updating running devices (OTA): design draft

Status: draft, revised after Orion's review (its counterparts are Orion's
`docs/update-recovery.md` and `docs/device-agent.md`). atlas-driver-orion is
built against Orion `f3efb26` (control protocol 4, request/response
actions) and tested with a fake
Orion. The device-side writer (`/usr/lib/board/update`) is tested
off-device, and `board-agent` (crates/board-agent) against a real
orion-node with a fake writer; neither has run on a board with Orion yet.
It covers devices that run a full OS on eMMC (Raze today) and leaves
microcontrollers (STM32) for their own driver.

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

## The writer: `board-update` (device package)

Installed as `/usr/lib/board/update` (Unreleased; tested off-device by
`devices/raze/tests/update.sh` against a real A/B layout made with sfdisk).
Settings: `update.env`.

```
update status                                # JSON, also /run/board/update.json
update stage <image> --sha256 <hex>          # check, then copy slot A into the inactive slot
update stage - --sha256 <hex> [--format xz]  # the same from stdin, checked at the end
update stage-url <url> --sha256 <hex> --size <bytes>
                                             # download to /data/board/update/, check, stage
update apply                                 # set [tryboot] to the staged slot, reboot "0 tryboot"
update confirm                               # on the trial boot, once healthy: keep it
update cancel                                # stop a running stage, or forget a staged update
update rollback [--no-reboot]                # boot the previous confirmed slot again
```

Exit status 3 means "refused, nothing changed": the wrong state, another
command still holding the lock after `UPDATE_LOCK_WAIT` seconds (60), or a
boot partition that is read-only (the refusal says where it is mounted so).
Other failures after staging began are recorded as `error`. A write to p1
that fails (state or `autoboot.txt`) ends the command with status 1 and the
error in `update.json`; `autoboot.txt` is put back if the state couldn't
follow it.

- One command changes things at a time. A command that changes state waits
  for the lock; `status` doesn't take it: it answers from `update.json`
  (written by every command, with the boot it describes in
  `update.json.boot`) and reads p1 only in a new boot or while a stage
  runs. `check-link` waits for the services before it takes the lock, and
  `board-update-link.service` runs after `board-update-confirm.service`.
- The drift check (`status --refresh-drift`) mounts the boot slot and p1
  read-only only while it holds the lock shared, and unmounts them before it
  lets go. A read-only mount of p1 makes any other mount of it read-only
  (they share the superblock), so while an update command runs the drift
  check skips its round.

- `stage-url` downloads with curl or wget, whichever the image has
  (`UPDATE_DOWNLOADER` picks one), into `/data/board/update/<sha256>.img`.
  An interrupted download resumes from its `.part` file; a server that
  can't resume starts over once. It checks the size (`--size`) and the
  SHA-256 before staging, removes a download that fails either, and removes
  the file once staged. An image already staged (same SHA-256) is done.
  Progress: the download is 0-500 per mille, the stage 500-1000.
- `update.json` stays current while a stage downloads or copies (every
  `UPDATE_POLL` seconds), so a reader needs no lock.
- `cancel`: a running stage records its PID in `/run/board/update.pid`.
  `cancel` freezes that process tree, ends it, takes the lock, and records
  `cancelled`; it does the same for a staged update (the inactive slot keeps
  what was written, unused). It prints `cancelled`, or `idle` when there was
  nothing to cancel, and refuses once the update is past staging.
- `rollback` makes `PREVIOUS_SLOT` (the slot the last confirmed update came
  from) the default again, records `rolled-back`, and restarts plainly. It
  is refused while staging, on a trial boot, or without a previous slot (a
  stage overwrites the previous slot, so it is forgotten then). The slot it
  leaves becomes the previous one, so a second rollback goes forward again.
  `--no-reboot` only switches (board-agent reports first, then restarts).

- A file (copy it to `/data`; `/run` is RAM) is checked against `--sha256`
  (of the file as given) before anything is written. From stdin, the hash is
  checked when the stream ends; a mismatch leaves the slot written but never
  staged.
- `board-image-slots` (a small C tool in the device package) reads the
  decompressed stream once: the MBR, then each EBR as it passes, copying p2
  and p5 straight into the inactive slot's partitions after checking they
  fit. Nothing seeks, so the stream can come from a pipe.
- The new root names its model (`/usr/lib/board/board-package.env`,
  refused if it differs) and version (`IMAGE_VERSION`, else `VERSION_ID`, in
  its os-release).
- Both slots can use one boot image: `stage` rewrites `root=` in the written
  slot's `cmdline.txt` to that slot's root partition.

OS hooks, executables in `/etc/board/update.d/`, given `BOARD_UPDATE_SLOT`,
`BOARD_UPDATE_VERSION` and `BOARD_UPDATE_IMAGE`:

| Hook | When | On failure |
|---|---|---|
| `pre-stage` | after the checksum, before writing | the stage stops (error) |
| `post-stage` | once staged | logged |
| `pre-reboot` | in `apply`, before the trial restart | stays staged, no restart |
| `post-boot` | on the trial boot, before the health check | logged |
| `update-health` (in `/etc/board/`) | on the trial boot | restart into the old slot |

Besides the CLI, the package ships **`board-agent`** (crates/board-agent,
`/usr/bin/board-agent`, `board-agent.service`), the device agent of Orion's
`docs/device-agent.md`. It connects to orion-node's local IPC
(`/run/orion/control.sock`, `/run/orion/control-stream.sock`; root is let in
by the drop-in `orion-node.service.d/50-board-agent.conf`) and claims the
node actions `update`, `update.cancel`, `update.rollback`, `reboot` and
`locate`:

| Action | What board-agent does |
|---|---|
| `update {image_url, sha256, size}` | starts `update stage-url` in the background and succeeds with `phase = "staging"` once the writer holds its lock (a writer refusal rejects it). The same SHA-256 again succeeds again; another image is rejected until cancelled. When the stage ends well it runs `update apply`. |
| `update.cancel` | `update cancel`: `phase = "cancelled"`, or `"idle"` with nothing to cancel |
| `update.rollback` | rejected while staging; `update rollback --no-reboot` (rejected without a previous slot), reports `phase = "rebooting"`, then `systemctl reboot` |
| `reboot` | reports `phase = "rebooting"`, then `systemctl reboot` after `delay_ms` |
| `locate` | drops the package's request (`/run/board/requests/locate`, which `board-locate.path` turns into the LED ring's locate pattern); `enabled = false` stops `board-locate.service` |
| `clock.set {unix \| time}` | sets the clock to `unix` (`Int`/`UInt` seconds) or `time` (`YYYY-MM-DDThh:mm:ssZ`, UTC) with `date -u -s @<s>`; times before 2024 or from 2100 are rejected. Logs a `clock.set` event (source orion, `old`/`new`) and succeeds with `old` and `new` |

It lists what it claims in the status key `action.claimed` (comma-separated),
so Atlas knows what each node offers. Atlas prefers Orion for the actions
both it and the board's own transport offer (`locate`, `reboot`,
`set-clock`, `update.cancel`, `update.rollback`), and uses SSH or the
identity endpoint only when Orion can't be reached (never after an action
was sent). Power off, USB boot and the self-test stay SSH only.

It publishes the writer's state as the `update.*` keys of its node
(`state`, `version_active`, `version_staged`, `slot_active`, `slot_staged`,
`progress`, `error`, `started_by`, `version_previous`, `phase`, plus `boot_id` from
`/proc/sys/kernel/random/boot_id`) and the newest board event
(`update.event_seq`, `update.event_kind`, `update.event_t`: under `update.`
because Orion lets an agent publish only `action.*` and its claimed actions'
keys) after connecting, within a second of a change to `update.json` or the
event log, and every 5 s (so the keys are back soon after orion-node
restarts). The writer prints an empty value as `null`, which reads as empty. Without
orion-node it waits and retries; nothing else depends on it. Stopping it
stops a stage it started (the stage is in its cgroup); `status` then records
the interrupted stage as an error. Settings: `BOARD_AGENT_*` in
`/etc/board/agent.env` or `/data/board/agent.env`. Gaia builds it
(`devices/raze/gaia/board-agent.toml`) as a static aarch64 musl binary from
the OS build's `atlas` source; on an OS without Orion it stays idle.

States, reported in `status` (`state`, `slot_active`, `slot_staged`,
`version_active`, `version_staged`, `progress` 0..1000, `error`), the same
values as Orion's `update_action::STATE_*`:

```
idle -> staging (download, verify, write inactive slot) -> staged
     -> rebooting (apply's restart accepted) -> trying (the trial boot)
     -> confirmed (autoboot.txt now points at it)  |  rolled-back (trial not confirmed)
staging | staged -> cancelled (update cancel)
staging -> error (download, checksum or write failed; `error`)
confirmed -> rebooting -> rolled-back (update rollback)
```

`apply` and `rollback` record the boot they were made in: `rebooting` read
in a later boot is reported as `trying` until `confirm` runs (and `confirm`
treats it like `trying`), and a rollback reads `rebooting` until the board
has restarted.

`confirm` runs from `board-update-confirm.service`, after
`multi-user.target` and an OS-provided health check (`/etc/board/update-health`;
for PhotonVision: the service is up and its HTTP port answers). The check
must not require Orion; it may add "orion-node READY" when Orion is
installed. A failed check restarts the board at once (`UPDATE_REBOOT_ON_FAIL`),
which boots the old slot. A trial that hangs is reset by the hardware
watchdog (systemd `RuntimeWatchdogSec=15s`), with the same result.

The identity endpoint advertises it: `"update_methods": ["image-write", "ab-tryboot"]`
(the second only when `status` has seen an A/B layout) and `"update"`, the
`status` object.

## Safety nets, and what each covers

| Failure on the new slot | What catches it |
|---|---|
| `apply`'s restart doesn't happen | The writer marks `trying` only once `systemctl reboot --reboot-argument='0 tryboot'` is accepted. If it isn't, the tryboot section is removed and the update stays `staged`. (systemd 258 dropped the positional `"0 tryboot"`.) |
| The kernel panics before userspace (e.g. it can't mount root) | `panic=5` on the OS's cmdline restarts the board. The tryboot flag is one-shot, so the bootloader boots the default (old) slot. |
| A hang after systemd starts | The hardware watchdog (`dtparam=watchdog=on` in raze-device.txt, armed by systemd's `RuntimeWatchdogSec=15s`) resets the board into the old slot. |
| A hang before systemd starts, without a panic | Not caught automatically: power-cycle it, and the old slot boots, because tryboot is one-shot. |
| The image's kernel doesn't match its root's modules | `stage` refuses it: the kernel release is read from the boot slot's kernel image and compared with `/lib/modules` on the new root. |
| The new slot boots but its drivers or management link are broken | `confirm` runs the package's checks before the OS's `update-health`: the running kernel has modules on this root, the USB gadget is bound, and none of `UPDATE_CRITICAL_UNITS` failed. A failure leaves the trial unconfirmed and restarts into the old slot. |
| A confirmed slot later loses its management link | `board-update-link.service` runs the same checks after every boot (`UPDATE_LINK_DELAY`). After `UPDATE_LINK_MAX_BAD` bad boots in a row it switches the default back to the previous good slot (once, with no ping-pong) and restarts. |

OS cmdline guidance: `panic=5 rootwait` (plus `rootfstype=erofs ro` for an
EROFS root). The kernel's soft-lockup and hung-task detectors can be made to
panic as well (`softlockup_panic=1`), which turns some silent hangs into a
restart into the old slot.

### Hardware test procedure (one board, USB connected)

1. Flash the current image over USB boot, and check that `update status`
   shows slot A, `idle`.
2. **Healthy update:**
   - copy the new `.img.xz` to `/data`;
   - run `update stage /data/x.img.xz --sha256 $(sha256sum < /data/x.img.xz | cut -d' ' -f1)`
     and check it reaches `staged` and that `cmdline.txt` on p3 names p6;
   - run `update apply`: the board restarts into B;
   - after `UPDATE_CONFIRM_DELAY`, check `status` shows `confirmed` on B and
     autoboot.txt's `[all] boot_partition=3`.
3. **Failed restart:** `UPDATE_REBOOT=false update apply` must leave it
   `staged` with no `[tryboot]` section.
4. **Trial that fails before root mounts:**
   - stage an image, then on p3 (slot B's boot) edit `cmdline.txt` to
     `root=/dev/mmcblk0p9`, a partition that doesn't exist;
   - run `apply`;
   - with `panic=5`, the kernel panics, restarts, and boots A;
   - check `confirm` on A records `rolled-back`.
5. **Trial whose drivers are broken:**
   - stage, then remove `lib/modules/<release>` from slot B's root (it is
     EROFS, so build a test image with a different kernel release in its
     boot slot instead, which `stage` refuses; to test `confirm`, set
     `UPDATE_UNAME_R=wrong` in `/data/board/update.env` on B);
   - run `apply`;
   - check `confirm` restarts into A and `status` shows the reason.
6. **Link loss after a confirmed update:** on B, mask
   `board-usb-gadget.service` and reboot twice; on the second boot,
   `check-link` must switch back to A (`rolled-back`, with the reason).
7. **Watchdog:** on a trial boot, `echo c > /proc/sysrq-trigger` (panic) and
   a userspace hang (`kill -STOP 1` is caught by systemd's watchdog) must
   both end on A.

## Read-only root (EROFS)

Root slots hold a read-only, compressed EROFS filesystem. `CONFIG_EROFS_FS`
is built into the Raze kernel (`buildroot-external/linux/raze.config`, with
LZMA and ZSTD), so it mounts without an initramfs; the OS's cmdline adds
`rootfstype=erofs ro`. The writer copies slots raw, so nothing changes for
updates; it reads the new root's `os-release` and `board-package.env`
through a read-only mount, which auto-detects EROFS.

Nothing in the device package writes to `/` at runtime:

| What | Where |
|---|---|
| identity, update status, LED state, action requests, gadget DHCP leases | `/run/board/` |
| SSH keys from the boot partition | `/run/board/ssh/authorized_keys`, read by sshd through `/etc/ssh/sshd_config.d/50-board.conf` (the OS's sshd_config must `Include /etc/ssh/sshd_config.d/*.conf` before any `AuthorizedKeysFile`) |
| update state | p1 (`board-update.env`), shared by both slots |
| user overrides of the package's `*.env` settings | `/data/board/` (read after `/etc/board/`) |
| hostname | the kernel's transient hostname; `/etc` is never written |

The OS provides: `/data` mounted early (from the data partition, p7, which
the package grows to the end of the eMMC and formats on a fresh flash:
`board-data-setup.service`; the image's build writes a flash id, any line
new for each build, to `board/flash-id` on p1, and a reflash then resets
`/data` while updates keep it), `/var/log/journal` on `/data` (the package
keeps the journal there, bounded, under a stable machine id:
`board-machine-id.service`; the kernel log a reset left in RAM goes to
`/data/board/pstore/`: `board-pstore.service`), SSH host keys on `/data`, and its own writable
paths.

## Board clock

A Raze has no RTC battery, and over the USB gadget there is usually no NTP:
the host is the board's DHCP client, not a time server, and desktops rarely
run one. So the board can boot with a date months old (systemd's build
epoch). The identity reports `"time"` (Unix seconds); when it differs from
the computer's by more than 2 s, Atlas shows the offset and offers "Set clock
from this computer" (`set-clock`: Orion's `clock.set`, else over SSH
`date -u -s`). While it watches, Atlas also sets it by itself, per the
"Keep board clocks synced" preference: boards on USB (the default), all
boards, or off; at most once per board every 10 minutes, logged in the fleet
history. A 3 h offset (seen on hardware) makes every board-side time
misleading, even with events ordered by seq.

OS images: keep systemd-timesyncd's clock file on /data
(`/var/lib/systemd/timesync/clock`, e.g. a symlink or bind into
`/data/timesync/`), so after a reboot the clock starts at the last known
time and never goes backwards, and let timesyncd sync whenever a network
with NTP is reachable.

## Board awareness: who did what

Atlas shows what a board is doing whoever started it: Atlas over SSH, Orion
through board-agent, or someone typing on the board.

- **Event log.** The package's scripts append to
  `/data/board/events.jsonl`, one line per event:
  `{"t":<unix s>,"boot_id":..,"seq":<n>,"uptime_s":<s>,"kind":"update.staged","source":"atlas|orion|local","message":..,"data":{..}}`.
  `t` is the board's clock, which can be hours off or step back (no RTC);
  `seq` numbers the events in the order they were written, across boots
  (`/data/board/event-seq`, allocated with the append under one lock), so
  a reader catches up by seq whatever the clock did.
  The source is `BOARD_EVENT_SOURCE`: Atlas exports `atlas` on every SSH
  command, board-agent sets `orion`, anything else is `local`. The writer
  records who started an update as `started_by` in `update.json`, and a
  trial boot's outcome is logged under that source.
- **Boot record.** `board-boot.service` counts boots and logs `boot` and, at a
  clean shutdown, `shutdown`; a boot with no `shutdown` before it reports
  `previous_clean: false`.
- **Status.** `/usr/lib/board/status --json`: boot, failed units,
  temperatures, fan, `update.json`, clock and drift. Drift compares the
  running slot's boot files with the hashes the writer recorded on p1 when it
  staged that slot (`board-boot.<slot>.sha256`), else with what the board first
  saw, lists `/etc/board` and `/data/board` overrides, and on a writable root
  checks `sshd_config` and `/etc/board`; root's `board-drift.timer` does the
  hashing and keeps the result in `/run/board/drift.json`. Failed units come
  from root too (`board-health.timer`, every 30 s, `/run/board/failed.json`):
  the endpoint's sandboxed user can't ask systemd. Without a fresh list
  `failed_units` is `null` (unknown), never an empty list.
- **Hardware.** The same root job (every 10 s) keeps lemnosd's view of the
  board in `/run/board/hardware.json`: each device from `lemnos-ctl list`
  (id, class, model, status: available, degraded, faulted or missing, and
  controls) with one reading from `lemnos-ctl read` (name, value or null,
  unit). `GET /status` serves it as `hardware` while under a minute old, else
  `null`. Atlas shows it on the device's Hardware tab, a card per device with
  short trends while the tab is open. When Orion lists the board's
  `lemnos.device` resources (Lemnos's bridge, its `docs/orion.md`), Atlas
  takes the hardware from there instead (live, up to 2 Hz per device, with
  each control's value and range) and the controls work: a slider sets a
  control when let go, Restore undoes this computer's writes, and a fan can
  be handed back to the board's cooling. These are Orion request/response
  actions (`set`, `restore`, `release` on the device's resource): the node
  holds its answer until the bridge has one. Lemnos applies the device's
  `writers` to the caller `orion:<requested_by>`, which for Atlas is
  `orion:operator:atlas-<host>`. The Raze's fan lists `orion:*` (a prefix
  match), usb-a-power lists no writers (anyone); a device whose list doesn't
  match refuses the write, and the card says so.
- **Endpoints.** `GET /status`, `GET /events?since=<t>&limit=<n>` (the
  newest events by time) and `GET /events?after_seq=<n>&limit=<n>` (the
  oldest events after a seq, to page forward; both give the board's clock,
  boot and newest seq) on the identity endpoint (port 5899), read-only and
  listed in the identity's `endpoints`.
- **Push channel.** `board-stream` (crates/board-stream, socket-activated per
  connection on TCP 5898, read-only, listed as `"stream":":5898/stream"` in
  the identity's `endpoints`): `GET /stream[?topics=events,update][&after_seq=<n>]`
  answers with Server-Sent Events: a `hello` (boot id, newest seq, time),
  the events after `after_seq` (or `Last-Event-ID`), the update state, then
  every new event (`id: <seq>`) and update-state change as it is written,
  and a keepalive every 10 s. It looks at the files every 200 ms (size and
  mtime), so nothing depends on inotify. While Atlas watches, it follows
  each board's stream: a pushed event or reconnect fetches the board's new
  events (by seq) and tells the UI to re-read its status at once; an
  update-state change also re-scans, so the identity is current. A stream
  that drops (a reboot) re-scans at once and then every 2 s for 2 minutes,
  so the board is seen going and coming back; boards without the stream are
  polled as before.
- **Live readings.** The same stream's `hardware` topic:
  `GET /stream?topics=hardware&hardware=imu:10,power:100`
  (device:period ms, at least 5 ms, at most 8 devices) opens one lemnosd
  connection (`board-stream`) for that viewer and subscribes those devices
  only while it is connected, so nothing is read for nobody. lemnosd reads
  a device at the fastest period any client asked for (and at least its
  board `poll_ms`; the Raze's IMU is 10 ms). It sends `hardware` (each
  device's channels and units), then `samples` every 16 ms
  (`[[t_us, v, ..], ..]`, lemnosd's monotonic µs), and `hardware-gone` if
  lemnosd goes. In Atlas, the Hardware tab opens this stream while it is
  shown (motion sensors at 10 ms, the rest at 100 ms) and draws a canvas
  chart per unit for each device at the display's rate from a ring buffer
  (40 s at 100 Hz), with a 2, 10 or 30 s window; hovering pauses it and
  shows the nearest sample's values and when it was read. A board without
  the stream shows the 10 s snapshot as before. Orion's status lane stays
  the low-rate summary (at most
  `LEMNOS_ORION_RATE_HZ`, deadbanded): it isn't meant for live viewing, and
  a remote operator can only poll it.
- **Atlas.** atlas-driver-board reads both (a `status` capability; without a
  metrics endpoint the temperatures and fan are its telemetry). atlas-core
  keeps each board's events (by board serial, at most 500, saved with the
  inventory), fetches new ones when the device's status is read and at most
  every minute after a scan: after the last seq fetched (paging until caught
  up; from the start again when the board's counter started over), or by
  time from a board that doesn't number its events. It dedupes by boot and
  seq (boot, time, kind and message without a seq), orders by seq, places
  each event in this computer's time by its boot's clock offset when that
  boot answered, and merges them with its own entries into the device's
  history. New events
  someone other than Atlas caused that matter (an update's outcome, an
  unclean boot, new SSH keys) also get a fleet-history line. For a board
  without the endpoints, Orion's `update.*` keys and host facts fill the
  status.
- **Controls.** Over SSH: `reboot`, `power-off` (`systemctl`), and on A/B
  boards `update.cancel` and `update.rollback` (the writer), next to
  `usb-boot` and `set-clock`; the ids are Orion's, so the device panel shows
  one control whichever transport offers it.

## Fresh installs without the button

A fresh install (new layout, wiped /data) needs USB boot. On a running CM5,
`/usr/lib/board/usb-boot` sets a one-time boot order of RPIBOOT through
the firmware mailbox (`set_reboot_order`, tag 0x0003808b) and reboots; the
bootloader's own `BOOT_ORDER` never changes, and the next normal power-up
boots the eMMC. RPIBOOT has no timeout, so the board waits for the host
until it is flashed or power-cycled. The identity lists `usb-boot-reboot`
when the board supports it, and Atlas offers "Restart into USB boot" over
SSH (Orion later). It is never on the unauthenticated identity endpoint.

## Reaching a board over USB

SSH and the identity endpoint work over the USB gadget network as well as
Ethernet. Each board's gadget network is a /29 of its own inside
172.31.0.0/16, picked by a hash of its board serial (scheme `serial-hash-v1`,
`capabilities.gadget-net.addressing` in the manifest). The gadget's USB
serial string is the board serial, so Atlas computes every plugged-in
board's address from its USB descriptor and probes them all; an identity
counts only when its serial matches the USB serial. Images from before
per-board addressing sit at 172.31.250.1 (left out of the hashed range) and
are reached there while they are the only board plugged in. The scheme,
collision odds and host-side notes are in `devices/README.md`
("USB gadget network").

## Transports

Image bytes never travel over Orion. Atlas serves the image over HTTP
from the computer running it, and the device pulls `image_url` and checks
sha256, size, and (when required) the signature itself. Over SSH
(`atlas-driver-board`, `ssh.rs`), Atlas streams the image to `/data/board/update/`
and runs `update stage <file> --sha256 <hex>` (the board checks the copy),
reading progress from
`update status`, and `update apply`. It waits for a new boot id and polls
`status` until `confirmed` or `rolled-back`. It runs the system OpenSSH as
root with the key chosen in Settings. Host keys are trusted on first use and
pinned per board (`HostKeyAlias=board-<model>-<serial>` in Atlas's own
known_hosts), because one address can belong to different boards over time
(older images all share 172.31.250.1, and two boards can collide on a
per-board gadget subnet). Atlas
offers it for boards whose identity lists `ab-tryboot`.

### The image server

For Orion updates, Atlas runs a small HTTP server (`crates/atlas-image-server`)
that serves registered images only. Each update registers the chosen image
and gets a URL `http://<host>:<port>/images/<token>/<file name>`: the token
is 32 random bytes in hex, new for every registration, and the name must
equal the file's name. Unknown tokens, other names, traversal attempts, and
directory paths all get 404; nothing is ever listed. It answers `GET` and
`HEAD` with `Content-Length`, ranges (`Range`/`If-Range`, so a board can
resume), and a strong ETag (the image's SHA-256), streaming from disk so
multi-gigabyte `.img`/`.img.xz`/`.img.zst` files never sit in memory. A
registration expires a day after its last request (a running download keeps
it alive) and allows four downloads at once; a fifth gets 503 with
`Retry-After`. Requests and finished downloads go to `atlas.log` (`IMAGES`
lines) with the token cut to 8 characters.

- **Port:** TCP 7700 on every IPv4 interface by default (Settings › Orion ›
  Image server), because boards reach Atlas over the robot network or the
  USB gadget. It starts when Orion is set up, or on the first Orion update.
- **Host in the URL:** the local address of the route to the board (Atlas
  connects a UDP socket to the board's address from its identity, which
  sends nothing, and reads the local address), else the configured image
  host. With neither, the update stops and asks for an image host.
- **Firewall:** a host firewall may block the port (firewalld on Fedora:
  `sudo firewall-cmd --permanent --add-port=7700/tcp && sudo firewall-cmd
  --reload`; ufw: `sudo ufw allow 7700/tcp`). The health screen's "Image
  server" check says whether it listens and on which addresses, or why it
  can't (port taken).
- The token is the only credential: anyone on the network who learns a URL
  can download that image until it expires. Images are not secret, and the
  board checks size and SHA-256 itself, so a tampered transfer is refused.

| Path | Needs | Auth | Used when |
|------|-------|------|-----------|
| Orion (intent) + Atlas URL (bytes) | orion-node and board-agent on the device | Orion's | Default for fleets |
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
| Preflight | `status`: compatible model/revision, enough space, not mid-update (`staging`, `rebooting`, `trying`) |
| Transfer | the device pulls `image_url` from Atlas (Orion: `update.state = staging`, progress 0-500), or Atlas copies it over SSH and runs `update stage <file> --sha256` |
| Apply | `stage` verify + slot write (Orion: progress 500-1000, then `staged`), then `apply` |
| Reboot | wait for the device to drop and come back (Orion: `rebooting`, then a new `boot_id` in the node's host facts) |
| Confirm | durable state after boot: `update.state` = `confirmed` with the new version → `Verified`; `rolled-back` → `RolledBack` (Orion: only keys whose `update.boot_id` is the new boot count) |

The outcome is never taken from an action result. The `update` action is
asynchronous: it ends `Succeeded` with `phase = "staging"` once the board has
started the download and stage, because a large image can outlast any action
deadline, and action records live in orion-node's memory and don't survive
the reboot. Everything after that is read from durable state: the `update.*`
keys board-agent keeps published and the node's host facts (boot id, OS
version, `board_serial`). A stage that fails ends the job with the board's
`update.error`; cancelling the job while the board stages sends
`update.cancel`. When a node's agent publishes `update.state`, Atlas also
offers the device actions `update.cancel` ("Cancel update") and
`update.rollback` ("Go back to the previous version", confirmed first).
Atlas keeps its Orion capability behind one interface, so moving from
actions to Orion's later durable UpdateIntent/UpdateStatus records
(milestone U3) changes nothing in these steps.

Concurrency is `Parallel`: each board updates itself, so staged rollout
(one board first) and bulk updates work as they do today.

### Self-test after an install

A verified flash (USB boot) or update (SSH, Orion) ends its job as soon as
the board is written or confirmed. atlas-core then remembers the board by its
board serial for 15 minutes. When a scan finds that board running, with the
board driver and a `self-test` capability (its identity lists
`"diagnostics": ["selftest"]`), Atlas runs
`/usr/lib/board/selftest --json` over SSH once:

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

- **Intent:** `ActionRequest { target: Node, action: "update", args: { image_url, sha256, size } }`,
  plus `update.cancel` and `update.rollback` (no args).
- **Progress while it runs:** `update.state` and `update.progress` under the
  Node subject; board-agent also mirrors each action into
  `action.<action_id>.state|error`.
- **Durable status across reboots:** stable keys under the Node subject,
  republished by board-agent after every boot, on change and every 5 s:
  `update.state`, `update.version_active`, `update.version_staged`,
  `update.slot_active`, `update.slot_staged`, `update.progress`,
  `update.error`, `update.boot_id`, `update.started_by`,
  `update.version_previous` (and the newest event's `update.event_*`).
- **Action result:** "staging started" only (see above).
- **Facts:** `board_serial` raw from `/proc/device-tree/serial-number`
  (DMI as a fallback), `board_model`, and `machine_id`. Atlas normalizes
  `board_serial` to its matching rule (the last 8 hex digits, lowercase) to
  merge the Orion record with the board identity and USB records of a board.

How Atlas reaches Orion: as an enrolled peer (orion+tcp, signed), with
actions forwarded across nodes, ideally through an Orion client library
compiled into Atlas behind an optional feature, rather than a separate
orion-node on the user's computer. The HTTP control API is the fallback.

## Open questions

- Where an image signing key lives (device package vs OS), if images get signed.
- Whether HeliOS images adopt the same layout, so one writer serves both.
