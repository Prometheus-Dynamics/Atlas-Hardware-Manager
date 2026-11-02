# Atlas rebuild: progress and TODO

Atlas is being rebuilt as a fleet and device manager for all Prometheus
Dynamics gear: connect to a robot over USB, through a device, or over the
network, and manage every device on it. Stack: Rust core, Tauri 2 shell,
Svelte 5 UI. v1 targets Raze/HeliOS; other device families plug in as drivers.

Audited against the code on 2026-11-01. Scope is the 2026-10-27 freeze plus the
user's last additions (2026-10-30); nothing else is added. Each open item:
**what**, **why**, **done when**, **blocked by**. "Written, untested" means the
code exists and its tests pass, and it has never run on a board.

## Releases

Releases are the user's alone: when, and what holds one back. Everything keeps
the dev scheme, `<year>.<major>.<minor>-dev.<count>+<commit>`, the Raze package
included (c26ad85). Board work gets tested on boards before anything ships,
unless the user says otherwise.

## Open: code, no hardware needed

- [ ] **The file-size gate fails on dev.**
  - **What:** `scripts/check-file-sizes.sh` fails on three files over 600 lines:
    `crates/atlas-driver-soc/src/flash.rs` (1004),
    `crates/atlas-driver-rpi/src/remote.rs` (833),
    `crates/atlas-ubi/src/nand_slots.rs` (652). Split them.
  - **Why:** the gate runs before clippy and the tests, so on all three OSes
    CI's `checks` stop there (run 41301714645, 058c53c). That run's `device
    package tests` also failed, on a rustup download of clippy, not on a test.
  - **Done when:** the script exits 0 and a CI run on dev passes.
  - **Blocked by:** nothing.
- [ ] **Release notes kept current.**
  - **What:** the draft, docs/release-notes/2026.1.0.md. It has the flashing and
    raw SPI-NAND writes; it lacks NAND slots on a running board (702b1ae,
    3e7cbf7) and board-lite (058c53c).
  - **Why:** so the notes are ready whenever the user cuts a release.
  - **Done when:** the draft covers everything on dev. It reopens with each
    change.
  - **Blocked by:** nothing.
## Open: the user's call

- [ ] **How Atlas writes to a Luckfox.**
  - **What:** Gaia's Luckfox image has no SSH, Orion or USB console, and
    board-lite is read-only by design. Nothing can call `atlas-ubi nand-slot` on
    a running board, and no Atlas code calls it: `update_methods: ["nand-slots"]`
    in board-lite's identity has no reader. Once there's a path, connect
    Atlas's slot install to it.
  - **Why:** Flame and other NAND boards take slot updates like Raze.
  - **Done when:** `atlas slots` installs, boots once and commits on a NAND
    board, through that path.
  - **Blocked by:** the user's choice between SSH by key with keys added over a
    USB console (recommended; a USB console may need kernel or device-tree
    changes in Gaia), SSH with a key built into a dev-only image, or Orion
    (unmeasured, a doubtful fit on 128 MB). Then Gaia takes the layer.

## Open: Raze on this computer's USB

Each takes a Raze with its regular USB port (the one used for flashing) in this
computer. The bench Raze, e5226d57, is on the SystemCore instead.

- [ ] **Fresh USB-boot install.**
  - **What:** `atlas flash` on a Raze in USB boot: boot, write, verify, SSH key,
    restart, then running.
  - **Why:** the path every new board takes.
  - **Done when:** a board flashed from USB boot runs the image and takes
    `atlas ssh-key` and slots commands, with the timing written down; it shows
    up by USB hotplug without a rescan; "Open as USB disk", "Browse files",
    "Eject and restart" and "Add my SSH key" work on its eMMC.
  - **Blocked by:** the user: power a Raze off, hold its boot button, connect
    that port to this computer, release once 0a5c:2712 appears.
  - Already done on hardware: the first flash from this computer (PhotonVision
    image, 2026-10-04/05), before `atlas flash` (56beff2) and the restart
    through the gadget console (c3a0241, ae07a30).
- [ ] **USB gadget rebind timing.**
  - **What:** a natural link stall recovers by the watch's rebind (79389f3).
  - **Why:** the ECM stall's recovery is the field fix (its root cause is cut).
  - **Done when:** `usb-gadget.rebound` lands about 15 s after TX stops, in
    events.jsonl and the host log.
  - **Blocked by:** the user: a running Raze left on that port until a stall
    comes (the gap has been as long as 14 h).

## Open: Raze on the bench

Windows are booked through HeliOS.

- [ ] **A/B in place.**
  - **What:** an A/B image installed and confirmed on a Raze.
  - **Why:** the update path for images without slots.
  - **Done when:** stage, apply, confirm and a rollback on hardware.
  - **Blocked by:** HeliOS: an A/B image and a bench window.
- [ ] **Keyless install over robot Ethernet (Orion, no controller).**
  - **What:** a slot install with security off; with security on it must refuse
    or fall back.
  - **Why:** the default FRC path when there is no SystemCore.
  - **Done when:** both cases on hardware, the board pulling the bundle from
    Atlas's image server (5da10e3, a744d40), and an Orion update cancelled and
    one rolled back through board-agent (17c2475; until now tested only against
    a real orion-node with a fake writer).
  - **Blocked by:** the user or HeliOS: a Raze's Ethernet on a 10.x network with
    this computer, not behind the SystemCore (off, the node auto-enrolls only
    the USB gadget network and `ORION_NODE_ROBOT_NETS`, 10/8 by default; the
    bench's 172.31.12.8x isn't one). Security on is the `security.env` switch,
    which exists (docs/ota.md).
  - Already done on hardware (2026-10-30): unhealthy-trial fallback, layered
    slot plus push (over SSH), keyless install through the SystemCore, support
    bundles.
- [ ] **HEVC decoder binds.**
  - **What:** `CONFIG_VIDEO_RPI_HEVC_DEC=m` (4719dba); written, untested.
  - **Why:** the Pi 5's decoder node was in the device tree but unbound.
  - **Done when:** on a Raze, rpi-hevc-dec is loaded and bound to
    brcm,bcm2712-hevc-dec.
  - **Blocked by:** HeliOS: a slot with this kernel, then a bench window.
- [ ] **"Last boots" on the device page.**
  - **What:** the panel, `pstore --show <boot id>` and board-agent's
    `update.boot_*` keys (facecb9); written, untested.
  - **Why:** the first thing to read after a reset in the field.
  - **Done when:** after a real reset, the panel shows its cause and kept log.
  - **Blocked by:** HeliOS: a slot carrying facecb9, then a bench window.
- [ ] **SSH key re-pin after a slot install.**
  - **What:** a new OS's host key is re-pinned, and "Trust its new SSH key"
    clears a stale pin (5f1e8f9); written, untested.
  - **Why:** a slot from an older image mustn't lock Atlas out.
  - **Done when:** both on a board whose host key changes.
  - **Blocked by:** HeliOS: a bench window that changes the key (the bench keeps
    its keys in /data/ssh, so an install alone doesn't).
- [ ] **Orion live stream numbers.**
  - **What:** CPU, forks and backfill time on a board for Orion's streams
    (2d46378). Then point HardwareTab at `board.hardware` (the driver follows it;
    the tab reads board-stream) and retire board-http and board-stream.
  - **Why:** always-live charts without a process per request.
  - **Done when:** the numbers are written down and the old paths are gone or
    kept for a stated reason.
  - **Blocked by:** HeliOS: the measurement on a board.
- [ ] **Raze facts the manifest still marks unverified.**
  - **What:** in `devices/raze/manifest.json`: LED count and index direction, the
    INA238 shunt (10 or 20 mOhm) and its max current, the sensor bus's select,
    the IMU's mount angles, the camera tuning, the USB-A power line's GPIO chip
    and control, the kernel's built-in list and version;
    `selftest --interactive` with HeliOS running.
  - **Why:** the generated board files and the self-test trust them.
  - **Done when:** each has a `verified` note and
    `devices/tools/gen-raze.py --check` passes (it passes today, with them open).
  - **Blocked by:** a bench window; the shunt needs the user to read the
    resistor marking (R010 or R020) or compare with a USB-C meter.

## Open: Raze behind the SystemCore

All three start with the same move. Written, tested off the board, never run
through a real flash.

- [ ] **The first real flash through the USB agent.**
  - **What:** step 1 is built (456e4cc, 04d7931): the agent as a service from its
    own `.ipk` on :5809, one-click install and update, boards behind the
    SystemCore in `atlas ls`, `atlas flash` through it. The user approved
    flashing e5226d57 with dev.811.
  - **Why:** boards on a robot are plugged into the SystemCore, not the PC.
  - **Done when:** `atlas flash --image <dev.811 image> --device rpi:e5226d57`
    completes; the board boots, takes SSH with its key and gets a 172.31.12.8x
    address.
  - **Blocked by:** the user: move e5226d57 from 3-1.1.4.1 (behind two Genesys
    hubs, where it dropped off USB mid-write in 2 of 3 runs) to a TUSB8041 3-1.x
    port on the SystemCore. HeliOS says when it's there.
- [ ] **Re-attach after Atlas restarts.**
  - **What:** built (a72418a): the remote flash remembers the agent job
    (`<data>/agent-jobs/<board>.json`) until it ends. After a scan, Atlas starts
    a job for any update that outlived it (`UpdateCapability::resumable`), and
    that job follows the agent's job instead of flashing again. A job the agent
    no longer has is said so and forgotten; a lost agent keeps the record for the
    next Atlas. Tested: core with the mock fleet, the driver against a loopback
    agent.
  - **Why:** a flash outlives Atlas closing or crashing; its result must still
    reach the job history.
  - **Done when:** killing Atlas mid-flash and starting it again shows the flash
    running, then its result, on a board behind the SystemCore.
  - **Blocked by:** the same move of e5226d57.
- [ ] **"Behind SystemCore" in the app.**
  - **What:** built (722f36a): the device list and cards put each device right
    under the gateway Atlas reaches it through, indented with a tree mark. The
    SystemCore's row shows its USB agent's version, with Install or Update when
    Atlas carries another. Checked in the browser mock only.
  - **Why:** on a robot that's where the boards are; the user must see the path
    Atlas reaches them by.
  - **Done when:** a SystemCore shows its boards under it, and a flash or update
    is started there.
  - **Blocked by:** the same move of e5226d57.
- [ ] **PIT/ROBOT end to end.**
  - **What:** the PIT/ROBOT view on a real robot: controller, radio, boards
    behind the SystemCore, readiness.
  - **Why:** the main screen at events.
  - **Done when:** a robot run shows every device with its state, and one install
    from the view.
  - **Blocked by:** the user: a robot to run it on.
- [ ] **The USB agent under Orion, with detached flashes.**
  - **What:** step 2: the agent registers with the SystemCore's Orion node and
    takes images over Orion. The memory-capped service and the detached flash
    exist from step 1; the SSH-key install (opkg) goes away. Not started.
  - **Why:** Phoenix Tuner works this way on the roboRIO; a robot needs no SSH
    step.
  - **Done when:** from a clean SystemCore, `atlas flash` on a board in USB boot
    behind it installs the agent, flashes, verifies and the board comes up
    running, with no SSH step; pulling the PC's cable mid-write doesn't stop the
    flash.
  - **Blocked by:** Orion's blob push, job API, non-HeliOS providers and large
    status values (HeliOS has these four).

## Open: the new boards

All on order: written, untested on a board, and shipped with
`"status": "untested"`, which Atlas says before it flashes. Vendor loaders are
no longer a blocker: Gaia's builds put each one beside its image (006284a,
3d1f249). Each record's `todo` lists what's left to settle.

- [ ] **Rockchip flashing on the boards.**
  - **What:** run what's written (see Done). Then LEDs during maskrom and
    flashing, and on the ROCK 4D, writing the SPI NOR bootloader (not wired in).
  - **Why:** the new boards.
  - **Done when:** each board flashed from maskrom and running, its record's
    `todo` settled (PIDs, the RK3576's above all; RV1106's RC4 and SPI-NAND LBA
    mapping and storage id; eMMC or UFS on the 4D) and its status `verified`.
  - **Blocked by:** the boards. On arrival, with the loader beside the image:
    - ROCK 5C: microSD out, eMMC fitted; short the Maskrom pin hole with a DuPont
      wire, connect the top USB 3.0 port here, power on, remove the wire.
    - ROCK 4D: microSD out; hold Maskrom, power on, release, connect the OTG port
      here. Its `lsusb` settles the guessed 2207:350e.
    - Luckfox Pico Pro: hold BOOT, connect USB-C here, release.
- [ ] **K230D flashing on the board.**
  - **What:** run what's written (see Done).
  - **Why:** flash a blank K230D board from scratch.
  - **Done when:** a blank board flashed and running, from Linux, Windows and
    macOS; the record's `todo` settled (load address 0x80300000, medium codes,
    the verify reply, whether the board has SPI-NAND at all) and its status
    `verified`.
  - **Blocked by:** the BPI-CanMV-K230D-Zero. On arrival: connect its OTG USB-C
    here and power it from the other (which is which is in the `todo`). Windows
    and macOS also wait on the installers' hold below.
- [ ] **NAND slots on a board.**
  - **What:** Gaia's contract (942746b) is carried out by `atlas-ubi nand-slot`,
    and Gaia's image holds it (see Done); written, untested.
  - **Why:** Flame and other NAND boards take slot updates like Raze.
  - **Done when:** the kernel attaches a UBI image atlas-ubi wrote; a trial of
    each slot, killed in Linux, comes back on the committed slot; `mtd list` at
    U-Boot names spi-nand0; a commit sticks; `data` formats itself on first mount.
  - **Blocked by:** the Luckfox Pico Pro. For the attach alone, the user's yes to
    `sudo modprobe nandsim` on this computer. Driving it from Atlas waits on the
    write path above.
- [ ] **The four boards running: identity on the board.**
  - **What:** each record's `gaia/device.toml` imports board-lite (identity on
    :5899 and `_pd-device._tcp`, read-only), built for its arch: armv7 Luckfox
    (058c53c), aarch64 ROCK 5C and 4D, static riscv64 K230D (riscv64 added to
    `rust-toolchain.toml`). Written, untested: all four builds run under
    qemu-user, and a repo test ties each record to its package.
  - **Why:** devices as data: no per-board code beyond the flasher, and Atlas
    lists a running board the same way as a flashed one.
  - **Done when:** each board is listed by `atlas ls` running, from its package.
  - **Blocked by:** the boards. Before that: Gaia importing the ROCK and K230D
    packages into its examples (the Luckfox's is in, 17c20df). The K230D has
    no Ethernet and Gaia's image sets up no Wi-Fi, so it has nothing to answer
    on until one of them does.
- [ ] **RK3562 and RK3566 records.**
  - **What:** board records for these two, in the user's additions. No record
    exists and no board is on order. The driver takes a new record as data.
  - **Why:** the module families the user named.
  - **Done when:** a record for each, flashed from maskrom on a board.
  - **Blocked by:** which modules: the user names them when they come.

## Open: other computers

- [ ] **Installer proofs on Windows and macOS (HELD).**
  - **What:** NSIS and dmg installed and run, USB access working; rpm and
    AppImage from the same freeze item, on real machines.
  - **Why:** teams use both.
  - **Done when:** a flash or update from each, with USB hotplug and mDNS
    watching finding the board on its own.
  - **Blocked by:** the user's hold.
- [ ] **The self-updater's first real run.**
  - **What:** built (a2b94b7). v2026.0.0 predates it, so it
    first runs from the first release that carries it to the one after.
  - **Why:** a real update is the only proof of the path.
  - **Done when:** an installed Atlas updates itself from GitHub and restarts on
    the new version.
  - **Blocked by:** two releases, whenever the user cuts them.

## Later, behind the freeze

- [ ] **Layered push to a board behind a controller.**
  - **What:** `atlas slots push` runs over SSH only (board driver `push_layer`).
  - **Why:** a board behind a SystemCore has no SSH from Atlas; its slots already
    install through the gateway (3e350dd).
  - **Done when:** a push and a revert through the SystemCore on hardware.
  - **Blocked by:** the freeze.
- [ ] **HeliOS secured mode.**
  - **What:** when `/v1/identity` reports secured, send a HeliOS API token
    (`Authorization: Bearer`) on everything but health, identity and auth status,
    kept per board, asked for on a 401. Not started.
  - **Why:** a secured board refuses Atlas today; FRC robots run open.
  - **Done when:** a secured board works from Atlas with its token.
  - **Blocked by:** the freeze.
- [ ] **STM32 driver.**
  - **What:** a driver for the custom USB bootloader. Only the mock names STM32.
  - **Why:** boards on that bootloader can't be flashed from Atlas.
  - **Done when:** a board flashed through it.
  - **Blocked by:** the freeze and the bootloader's protocol.
- [ ] **Storage "fits" check before flashing.**
  - **What:** today the writer stops with "image larger than disk" after writing
    part of it (atlas-blockdev writer.rs).
  - **Why:** a flash that can't fit shouldn't touch the disk.
  - **Done when:** it's refused before the first write.
  - **Blocked by:** the freeze.
- [ ] **Job throughput and friendlier failure reasons.**
  - **What:** the app shows a stage's ETA (StageProgress.svelte) but no
    throughput, and failure reasons as the drivers word them.
  - **Why:** people watch long flashes and act on the failures.
  - **Done when:** a running job shows MB/s, and each driver failure says what to
    do next.
  - **Blocked by:** the freeze.
- [ ] **Job history across restarts.**
  - **What:** jobs live in memory; update results already reach the fleet
    activity, which is kept. An agent's flash is followed again (a72418a).
  - **Why:** the Jobs page empties on restart.
  - **Done when:** finished jobs are still listed after a restart.
  - **Blocked by:** the freeze.
- [ ] **Light theme.**
  - **What:** a second block of tokens (themes/glass.css allows one).
  - **Why:** the tokens already allow it; nobody has asked yet.
  - **Done when:** the app switches and every page reads in it.
  - **Blocked by:** the freeze.
- [ ] **Signed releases and installers.**
  - **What:** the self-updater verifies a signed manifest once
    `update-keys.txt` lists a key (a2b94b7); it lists none, so releases stay
    usable unsigned (checked when present, never required).
  - **Why:** so an update can be proved to be ours.
  - **Done when:** a key is listed and a signed release verifies; a bad signature
    is refused.
  - **Blocked by:** Gaia's signing proof: minisign and cosign are approved for
    Gaia (user), and Atlas's key follows it.

## Done

### Board flashing and NAND, written and untested on a board

Tests pass, on simulated hardware and on Gaia's real loaders and images (run
again 2026-11-01); the board proofs are the open items above.

- [x] Rockchip maskrom, rockusb, read-back writes, FlashHead idblock, SPI-NAND
      through LBAs; K230 boot ROM and Canaan's burner protocol: atlas-socflash
      (d29f258).
- [x] UBI on raw NAND: atlas-ubi reads Gaia's images, writes as ubiformat does,
      updates slot volumes (ebbf3b8).
- [x] The Rockchip and K230 drivers and the four board records, atlas-driver-soc
      (0cadabd).
- [x] Gaps closed against Gaia's real outputs (006284a, 3d1f249):
  - loader globs as a list, `pid_unconfirmed` (27b5a1e);
  - loaders parsed, trailer CRC polynomial 0x04C10DB7 confirmed, chip checked
    (da82771);
  - RK3576's idblock from its table, the Luckfox partition set from env.img's
    `mtdparts`, K230 firmware heads checked, NAND erased before raw writes
    (54e7bb2);
  - idblock byte for byte the vendor's on the Luckfox, K230D load address
    0x80300000 (90ad531).
- [x] `atlas flash` and Windows reach the new boards (WinUSB for
      `SOC_BOOT_IDS`); a guessed PID needs the chip's word (4e57c66).
- [x] Gaia's NAND slot contract (Gaia 942746b): `atlas-ubi nand-slot` with
      power-cut tests at each write (702b1ae); the Luckfox slot layer, installs
      from Gaia's `slot-bundle.tar` (3e7cbf7). Gaia 720c211 enabled the watchdog
      node, `/data` and the slot bundle.
- [x] board-lite: identity on :5899 and `_pd-device._tcp` for a small board, the
      Luckfox package, `atlas ls` waiting for mDNS's first answers (058c53c); its
      `update_methods` carry `image-write`, as the identity schema requires.
      Found by the real `atlas ls` across two network namespaces. In Gaia's full
      Luckfox image (Gaia 17c20df; root filesystem 2,043,904 bytes).

### Core and contract
- [x] Driver contract (`atlas-driver`): identity, links, capabilities
      (update, actions, telemetry, logs), health checks with one-click fixes,
      driver registry.
- [x] Core (`atlas-core`): inventory, parallel scan with gateway traversal and
      best-link ranking, update jobs (staged rollout, exclusive resources,
      cancel), robot profiles and readiness, typed event stream, JSON store.
- [x] Fleet activity history (found, online, offline, version and mode
      changes, update results, actions), kept between sessions.
- [x] Event-driven discovery: USB hotplug (nusb) and mDNS changes trigger a
      scan at once; a slow safety-net rescan replaces polling.
- [x] Devices as data: the SystemCore recognised and read from
      `devices/systemcore/manifest.json` (fad5bcc), after the roboRIO (2f96f61).
      docs/devices-as-data.md "As built".

### Hardware
- [x] Pure-Rust rpiboot (`atlas-usbboot`) over nusb, Windows included.
- [x] Safe disk writing (`atlas-blockdev` + `atlas-helper`): refuses system
      and internal disks, streams xz/zstd/gz, verifies by reading back, one
      elevated helper per OS (pkexec, osascript, RunAs).
- [x] Raspberry Pi recovery driver: USB boot, wait for the eMMC, write,
      verify; EEPROM (bootloader) update action.
- [x] Device packages in `devices/` (Raze, roboRIO, SystemCore and the board records; owned here): catalog
      naming, recovery steps, EEPROM files, the `raze-leds` LED helper, a
      locate action on the identity endpoint, opt-in SSH keys from the boot
      partition, and a serial console on the USB gadget.
- [x] board driver for running devices: mDNS `_pd-device._tcp` +
      `/.well-known/pd-device`. Optional `endpoints` (metrics, logs,
      actions), `actions`, and `camera_stream` are used when a device lists
      them and read leniently; nothing is required.
- [x] Raze: the Gaia layers on config v3 (3c62ba1); the IMU at 200 Hz (20cd0c4).

### Releases
- [x] Local files and remote manifests; ed25519 signatures and checksums are
      used when present, never required (unsigned warns, mismatch offers
      "Download again" / "Flash anyway").
- [x] Every version on the dev scheme, the Raze package included (c26ad85).

### Desktop app
- [x] Overview: robot readiness with one next step, live summary tiles,
      connection map, recent activity.
- [x] Devices inventory (cards and list), selection, bulk update.
- [x] Device page: live readings and trends, camera view, logs with support
      bundle, quick actions (Find it, Restart, Web UI), update, flash,
      history.
- [x] Robots, Jobs, Releases, Settings (health checks with Fix buttons).
- [x] "Glass" visual style; fast motion system (no blur, no looping
      animations, data snaps) for older hardware.
- [x] Browser mock with a fully simulated robot (`bun run dev`).
- [x] Safer flashing: crashed drivers fail the job instead of hanging it,
      one job per device, a warning when a device goes quiet, Cancel always
      ends a job, host checks on the Flash tab, `atlas.log`.
- [x] After a flash: the eMMC is ejected so the desktop can't mount it, and
      Atlas says to power-cycle. "Open as USB disk", "Browse files", "Eject
      safely", and "Add my SSH key" for a board's eMMC; a half-done flash
      continues without a power-cycle.
- [x] A flashed board replaces its USB-boot record (matched by board serial).
- [x] Hardware self-test: the Raze manifest is the source of the board's
      hardware facts (generated config, `gen-raze.py --check` lint), the
      device package has `selftest`, and Atlas runs it after a flash or
      update, keeps the result per board serial and shows it on a Self-test
      card with "Run again".

### Updates and Orion
- [x] atlas-driver-orion: Orion adds readings, locate/reboot and updates to
      the device with the same board serial (capability sources in the
      core).
- [x] Bundle host: `atlas-image-server` serves update images over HTTP
      (per-update tokens, ranges, ETag) for boards to pull (5da10e3); a
      board on this computer's USB pulls slots from it with no SSH key
      (3b999e3).
- [x] A/B writer in the device package (`usr/lib/board/update`, tested off the
      board by `devices/raze/tests/update.sh`) and the SSH update transport for
      boards listing `ab-tryboot`.
- [x] `board-agent`, the Orion device agent (17c2475; tested against a real
      orion-node with a fake writer).
- [x] Slots: slot installs, boot-once, layered slots and push (verified on
      hardware 2026-10-26/30); the unhealthy-trial fallback (2026-10-30,
      b285300).

### Tooling and install
- [x] `atlas` CLI covering every flow, with `--sim` fleets.
- [x] Installers via `scripts/bundle.sh`: deb/rpm (udev rule included),
      AppImage, NSIS (WinUSB driver install), dmg. USB access fix button.
- [x] CI on Linux, Windows, macOS; installers on tags or manual runs.

### Closed in the TODO audit (2026-10-31)

Each was an open item; the evidence is what closed it.

- [x] Raze 1.0.7 features on hardware: LED colour order and fan polarity
      (verified on a Raze 2026-10-06, manifest `verified` notes, ff18ab8,
      f98489f); locate through lemnosd (lemnosd cd72ad0 on a Raze,
      2026-10-07); SSH keys from the boot partition (found and fixed on
      hardware, used on every bench window since); the gadget serial console
      (the rebind recovery and flash restart use ttyGS0, c3a0241, 3b999e3).
- [x] Orion transport: the operator client (2ff0dd9), each board's own
      node (a744d40); on hardware, live readings over Orion streams
      (2d46378) and slots listed through the SystemCore over Orion
      (3ac887a, 2026-10-27).
- [x] board-agent on a Raze with orion-node, and its Gaia artifact in a full
      image build: HeliOS images ship it, and slots over Orion read from it
      on hardware (2026-10-27). Its update cancel and rollback moved to the
      keyless item above.
- [x] Lemnos asks from the Raze integration: `lemnos-ctl fan release`, the
      board-selftest layer at priority 100 (Lemnos be8321a, Raze d5302db),
      and the RP1 path in Lemnos's docs/board-definition.md.
- [x] HeliOS OTA for running devices: slots over SSH, Orion and the
      controller gateway (verified 2026-10-27), keyless install through the
      SystemCore (3e350dd, 11.5 s on hardware 2026-10-30), A/B over SSH
      (the A/B proof stays open above). The board driver
      (`/.well-known/pd-device`) serves HeliOS boards; no separate driver is
      needed.
- [x] Device side endpoints in the Raze package: `/status`, `/events`,
      `/actions` (locate offered; reboot through BOARD_ACTIONS), board-stream
      on :5898 and the camera streams list, read by the board driver; logs
      come in the support bundle (`atlas support`, on hardware 2026-10-30).
