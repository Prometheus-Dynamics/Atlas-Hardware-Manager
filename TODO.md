# Atlas rebuild: progress and TODO

## Next release (2026.1.0)

The version is the workspace's `release` (Cargo.toml
`[workspace.metadata.atlas]`); dev builds are `2026.1.0-dev.<count>+<commit>`.
Scope is the 2026-10-27 freeze plus the user's last additions (2026-10-30);
nothing else is added. Each item: **what**, **why**, **done when**,
**blocked by**.

### 1. Freeze items still open

- [ ] **Raze hardware proofs: fresh USB-boot install.**
  - **What:** `atlas flash` on a Raze in USB boot plugged into this computer:
    boot, write, verify, SSH key, restart, then running.
  - **Why:** the path every new board takes.
  - **Done when:** a board flashed from USB boot runs the image and takes
    `atlas ssh-key`/slots commands, with the timing written down; the board
    shows up by USB hotplug without a rescan; "Open as USB disk", "Browse
    files", "Eject and restart" and "Add my SSH key" work on its eMMC.
  - **Blocked by:** a window with a Raze on this computer's USB (user).
  - Already done on hardware: the first flash from this computer
    (PhotonVision image, 2026-10-04/05), before `atlas flash` (56beff2) and
    the restart through the gadget console (c3a0241, ae07a30).
- [ ] **Raze hardware proofs: USB gadget rebind timing.**
  - **What:** a natural link stall recovers by the watch's rebind.
  - **Why:** the ECM stall's recovery is the field fix (its root cause is cut).
  - **Done when:** `usb-gadget.rebound` lands about 15 s after TX stops, in
    events.jsonl and the host log.
  - **Blocked by:** the same USB window.
- [ ] **Raze hardware proofs: A/B in place.**
  - **What:** an A/B image installed and confirmed on a Raze.
  - **Why:** the update path for images without slots.
  - **Done when:** stage, apply, confirm and a rollback on hardware.
  - **Blocked by:** a bench window and an A/B image from HeliOS.
- [ ] **Raze hardware proofs: keyless install over robot Ethernet (Orion, no
      controller).**
  - **What:** a slot install with security off; with security on it must
    refuse or fall back.
  - **Why:** the default FRC path when there is no SystemCore.
  - **Done when:** both cases on hardware, the board pulling the bundle from
    Atlas's image server (5da10e3, a744d40), and an Orion update cancelled
    and one rolled back through board-agent (17c2475; until now tested only
    against a real orion-node with a fake writer).
  - **Blocked by:** a window with the board reachable directly, and a way to
    turn board security on.
  - Already done on hardware (2026-10-30): unhealthy-trial fallback, layered
    slot plus push, keyless install through the SystemCore, support bundles.
- [ ] **PIT/ROBOT end to end.**
  - **What:** the PIT/ROBOT view on a real robot: controller, radio, boards
    behind the SystemCore, readiness.
  - **Why:** the main screen at events.
  - **Done when:** a robot run shows every device with its state, and one
    install from the view.
  - **Blocked by:** a robot (user).
- [x] **Devices as data, step 3: SystemCore from data.** Built:
  `devices/systemcore/manifest.json` (the `/api/team` fingerprint, key
  `team`, model, name and manage URL) recognises and reads it through the
  engine, tested on its real answer. docs/devices-as-data.md "As built
  (step 3)".
- [ ] **Installer proofs on Windows and macOS (HELD).**
  - **What:** NSIS and dmg installed and run, USB access working.
  - **Why:** teams use both.
  - **Done when:** a flash or update from each, with USB hotplug and mDNS
    watching finding the board on its own.
  - **Blocked by:** the user's hold.
- **Pinned for later, not this release:** the release signing key (user
  decision 2026-10-30). Releases stay usable unsigned (signatures are
  checked when present, never required).

### 2. Additions (the user's last, 2026-10-30)

- [ ] **Atlas on the SystemCore: self-install, under Orion, detached
      flashes.**
  - **What:** Atlas sees a SystemCore and installs or upgrades its on-robot
    agent with one click: a SystemCore `.ipk` built by Gaia (fallback: a
    binary in persistent storage with a systemd unit; never /tmp). The
    agent runs as a memory-capped service and registers with the
    SystemCore's Orion node. It lists the USB devices behind the SystemCore
    (boards in USB boot, gadgets, serials and ports). It takes images over
    Orion and flashes detached: the flash goes on if Atlas's link drops,
    and Atlas re-attaches to its progress. `atlas flash` and the GUI use it
    for any board behind a robot controller, shown in PIT/ROBOT.
  - **Why:** boards on a robot are plugged into the SystemCore, not the PC
    (Phoenix Tuner works the same way on the roboRIO).
  - **Done when:** from a clean SystemCore, `atlas flash` on a board in USB
    boot behind it installs the agent, flashes, verifies and the board
    comes up running, with no SSH step; pulling the PC's cable mid-write
    doesn't stop the flash.
  - **Blocked by:** step 2 (under Orion): Orion's blob push, job API,
    non-HeliOS providers and large status values (HeliOS has taken these). Step 1
    is built (456e4cc, 04d7931): the agent as a service from its own
    `.ipk` on :5809, one-click install and update, boards behind the
    SystemCore in `atlas ls`, `atlas flash` through it. The first real
    flash waits on the Raze moving off a faulty hub (user).
- [ ] **Atlas re-attaches to an agent's flash after its own restart: prove
      it on the board.**
  - **What:** built: the remote flash remembers the agent job
    (`<data>/agent-jobs/<board>.json`) until it ends. After a scan, Atlas
    starts a job for any update that outlived it
    (`UpdateCapability::resumable`), and that job follows the agent's job
    from its first event instead of flashing again. A job the agent no
    longer has is said so and forgotten; a lost agent keeps the record for
    the next Atlas. Tested: core with the mock fleet, the driver against a
    loopback agent.
  - **Why:** a flash outlives Atlas closing or crashing; its result must
    still reach the job history.
  - **Done when:** killing Atlas mid-flash and starting it again shows the
    flash running, then its result, on a board behind the SystemCore.
  - **Blocked by:** the Raze replug onto a 3-1.x port (the same window as
    the first real agent flash).
- [ ] **"Behind SystemCore" in the app: check on a robot.**
  - **What:** built: the device list and cards put each device right under
    the gateway Atlas reaches it through (a SystemCore's boards through
    HeliOS and, in USB boot, through its agent), indented in the list with
    a tree mark. The SystemCore's row shows its USB agent's version, with
    Install or Update when Atlas carries another (the `usb-agent.install`
    action). Checked in the browser mock with a SystemCore and a Raze in
    USB boot behind it.
  - **Why:** on a robot that's where the boards are; the user must see the
    path Atlas reaches them by.
  - **Done when:** a robot with a SystemCore shows its boards under it, and
    a flash or update started there.
  - **Blocked by:** a robot window with the SystemCore (the bench one, with
    the Raze replugged).

- [ ] **K230D flashing: debug on the board.**
  - **What:** written: the boot ROM stage and Canaan's burner protocol
    (atlas-socflash `k230`), the K230 driver (atlas-driver-soc), the
    CanMV K230D Zero record. Left: run it.
  - **Why:** flash a blank K230D board from scratch.
  - **Done when:** a blank board flashed and running, from Linux, Windows
    and macOS; the record's `todo` settled (load address, medium codes,
    the verify reply, whether the board has SPI-NAND) and its status
    `verified`.
  - **Blocked by:** the BPI-CanMV-K230D-Zero (on order); Canaan's burner
    loader file next to the image (from Gaia's K230 SDK fetch).
- [ ] **Rockchip flashing: debug on the boards.**
  - **What:** written: maskrom download, rockusb commands, image writes read
    back, the FlashHead idblock, SPI-NAND through LBAs (atlas-socflash
    `rockchip`), the Rockchip driver, records for ROCK 5C (RK3588S2), ROCK
    4D (RK3576) and Luckfox Pico Pro (RV1106G2). Left: run it; LEDs while
    flashing, as on Raze; RK3562/RK3566 records when those modules come.
  - **Why:** the new boards.
  - **Done when:** each board flashed from maskrom and running, its
    record's `todo` settled (PIDs, loader names, the trailer CRC
    polynomial, SPI-NAND LBA mapping) and its status `verified`.
  - **Blocked by:** the boards (on order); their loader files from Gaia's
    rkbin fetch.
- [ ] **NAND/UBI: Atlas's slot install, then the board.**
  - **What:** written: atlas-ubi reads Gaia's G8 images, writes them as
    ubiformat does (used by the Rockchip flash), and carries out Gaia's NAND
    slot contract on a board (Gaia 942746b; `atlas-ubi nand-slot`: install from
    `slot-bundle.tar`, trial-check, watchdog, commit;
    docs/soc-flashing.md). `devices/luckfox-pico-pro/gaia/nand-slot.toml` puts
    it and its init hook in the image. Left: Atlas's slot install calling it.
  - **Why:** Flame and other NAND boards take slot updates like Raze.
  - **Done when:** the kernel attaches an image atlas-ubi wrote; install,
    boot-once, confirm and fallback on a NAND board; `data` formats itself on
    first mount.
  - **Blocked by:** for the rest of the code, a way for Atlas to reach a board
    with no SSH, Orion or USB console (the user's call: see soc-flashing.md).
    For the proof, a NAND board (Luckfox Pico Pro, on order),
    or the user's yes to `sudo modprobe nandsim` for the attach; and Gaia's
    commit enabling the watchdog's device-tree node.
- [ ] **Board records: the running side.**
  - **What:** written for the Luckfox: board-lite serves the identity contract
    and its mDNS advertisement (`devices/luckfox-pico-pro/gaia/device.toml`), so
    Atlas lists it running, its version and slot. Left: the other three.
  - **Why:** devices as data: no per-board code beyond the flasher.
  - **Done when:** each is recognised in recovery (on the board) and running,
    from its package.
  - **Blocked by:** the boards; an OS image for each (HeliOS/Gaia).

### 3. Releases

Releases are the user's call alone: no dev into main and no tag from here,
and they aren't asks. Everything keeps the dev scheme,
`<year>.<major>.<minor>-dev.<count>+<commit>`, the Raze package included. The
new boards will be tested; nothing ships until they are, or until the user
says otherwise.

- [ ] **Release notes.** Kept current as a draft
  (docs/release-notes/2026.1.0.md) for whenever the user cuts one.
- [ ] **The self-updater's first real run.** Built (a2b94b7); 2026.0.0 predates
  it, so it first runs from the first release that carries it to the one after.
  **Blocked by:** two releases (the user's).


Atlas is being rebuilt as a fleet and device manager for all Prometheus
Dynamics gear: connect to a robot over USB, through a device, or over the
network, and manage every device on it. Stack: Rust core, Tauri 2 shell,
Svelte 5 UI. v1 targets Raze/HeliOS; other device families plug in as drivers.

## Done

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

### Releases
- [x] Local files and remote manifests; ed25519 signatures and checksums are
      used when present, never required (unsigned warns, mismatch offers
      "Download again" / "Flash anyway").

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
- [x] A/B writer in the device package (`devices/raze/tests/update.sh`) and
      the SSH update transport for boards listing `ab-tryboot`.
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

Each was an open item below "Done"; the evidence is what closed it.

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
      keyless freeze item above.
- [x] Lemnos asks from the Raze integration: `lemnos-ctl fan release`, the
      board-selftest layer at priority 100 (Lemnos be8321a, Raze d5302db),
      and the RP1 path in Lemnos's docs/board-definition.md.
- [x] HeliOS OTA for running devices: slots over SSH, Orion and the
      controller gateway (verified 2026-10-27), keyless install through the
      SystemCore (3e350dd, 11.5 s on hardware 2026-10-30), A/B over SSH
      (the A/B proof stays a freeze item). The board driver
      (`/.well-known/pd-device`) serves HeliOS boards; no separate driver is
      needed.
- [x] Device side endpoints in the Raze package: `/status`, `/events`,
      `/actions` (locate offered; reboot through BOARD_ACTIONS), board-stream
      on :5898 and the camera streams list, read by the board driver; logs
      come in the support bundle (`atlas support`, on hardware 2026-10-30).

## Not yet verified on hardware

- [ ] **Raze facts the manifest still marks unverified.**
  - **What:** LED count and index direction, the INA238 shunt (10 or
    20 mOhm), the IMU's mount angles, the USB-A power line's GPIO chip and
    control, the kernel's built-in list and version notes;
    `selftest --interactive` with HeliOS running.
  - **Why:** the generated board files and the self-test trust them.
  - **Done when:** each has a `verified` note in `devices/raze/manifest.json`
    and `gen-raze.py --check` passes.
  - **Blocked by:** a bench window; the shunt needs the resistor marking or a
    USB-C meter (user).

## Later (not in 2026.1.0)

- [ ] HeliOS secured mode: when `/v1/identity` reports secured, send a
      HeliOS API token (`Authorization: Bearer`) on everything but health,
      identity and auth status, kept per board, asked for on a 401. Blocked
      by: the freeze; FRC robots run open.
- [ ] STM32 driver for the custom USB bootloader. Blocked by: the freeze and
      the bootloader's protocol.
- [ ] Storage "fits" check before flashing. Today the writer stops with
      "image larger than disk" when the image outgrows it (atlas-blockdev
      writer.rs), after writing part of it. Blocked by: the freeze.
- [ ] Job ETA and throughput in the app; friendlier failure reasons. Blocked
      by: the freeze.
- [ ] Job history across restarts: jobs live in memory; update results
      already reach the fleet activity, which is kept. Blocked by: the freeze
      (the agent re-attach item above covers flashes behind a SystemCore).
- [ ] Light theme (the tokens allow one: themes/glass.css). Blocked by: the
      freeze.
- [ ] Signed releases and installers. The self-updater verifies a signed
      manifest once update-keys.txt lists a key (a2b94b7). Atlas's signing key
      follows Gaia's: minisign and cosign are approved for Gaia (user), and
      this comes after Gaia's proof. **Blocked by:** Gaia's signing proof.
