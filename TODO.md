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
    `atlas ssh-key`/slots commands, with the timing written down.
  - **Blocked by:** a window with a Raze on this computer's USB (user).
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
  - **Done when:** both cases on hardware.
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
- [ ] **Devices as data, step 3: SystemCore from data.**
  - **What:** `devices/systemcore/` (fingerprints, info) like
    `devices/roborio/` (step 2, 2f96f61).
  - **Why:** new controllers without new code.
  - **Done when:** the SystemCore's recognition and info come from its
    package and its tests pass on the captured answers.
  - **Blocked by:** nothing (its API is known: `:4800/api/team`).
- [ ] **Installer proofs on Windows and macOS (HELD).**
  - **What:** NSIS and dmg installed and run, USB access working.
  - **Why:** teams use both.
  - **Done when:** a flash or update from each.
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
  - **Blocked by:** Orion: a blob or stream transfer for images and a
    long-running job API (exact ask sent to HeliOS); Gaia: the `.ipk`
    recipe. Today's step, `atlas-usb-agent` (b912fef), boots and writes
    over SSH. A board behind two cascaded hubs keeps dropping off USB
    mid-write (a hub or cable problem, user).
- [ ] **K230D flashing.**
  - **What:** its BootROM USB download protocol in Atlas's Rust USB stack:
    load the loader, then write the storage.
  - **Why:** flash a blank K230D board from scratch.
  - **Done when:** a blank board flashed and running, from Linux, Windows
    and macOS.
  - **Blocked by:** a K230D board; the protocol from the vendor tool's
    source.
- [ ] **Rockchip flashing: RK3588S, RK3576, RK3562, RK3566 modules and Flame
      (RV1106G2).**
  - **What:** Rockusb in maskrom and loader modes, in Atlas's own Rust
    flasher (no rkdeveloptool or upgrade_tool at runtime): download the
    loader, write the storage (eMMC, or SPI NAND on Flame), verify, reset.
    LEDs while flashing, as on Raze.
  - **Why:** the new boards.
  - **Done when:** each board flashed from maskrom and running.
  - **Blocked by:** the boards; their loaders from Gaia.
- [ ] **NAND/UBI slot writing per Gaia's G8 layout.**
  - **What:** slot installs onto raw NAND (UBI volumes) where the board has
    no eMMC.
  - **Why:** Flame and other NAND boards take slot updates like Raze.
  - **Done when:** install, boot-once, confirm and fallback on a NAND
    board.
  - **Blocked by:** Gaia's G8 layout; a NAND board.
- [ ] **Board records for the new boards.**
  - **What:** `devices/<model>/` for each (recognition, recovery ids,
    flashing method), so they show in PIT/ROBOT like Raze.
  - **Why:** devices as data: no per-board code beyond the flasher.
  - **Done when:** each is recognised in recovery and running, from its
    package.
  - **Blocked by:** each board's USB ids and identity contract
    (board-gated).

### 3. Release mechanics

- [ ] **Changelogs.**
  - **What:** release notes from the dev history, and
    `devices/raze/CHANGELOG.md`'s Unreleased cut to a version.
  - **Why:** teams need to know what changed.
  - **Done when:** both written and reviewed by the user.
  - **Blocked by:** nothing.
- [ ] **Versions.**
  - **What:** crates and Raze package from `-dev` to their release
    versions; packaging checks pass (`scripts/tests/package-versions.sh`).
  - **Why:** a release must not report `-dev`.
  - **Done when:** `atlas --version` says `2026.1.0` on the tagged build.
  - **Blocked by:** the items above.
- [ ] **dev into main.**
  - **What:** merge.
  - **Why:** releases come from main.
  - **Done when:** main has the release commit and CI is green on it.
  - **Blocked by:** the user's OK.
- [ ] **Tag and build.**
  - **What:** `v2026.1.0`; CI builds deb, rpm, AppImage, NSIS and dmg.
  - **Why:** the installers people download.
  - **Done when:** the tagged installers install and run (Linux here;
    Windows and macOS as their proofs allow).
  - **Blocked by:** the user (Atlas sessions never tag).


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
- [x] Device packages in `devices/` (Raze, now 1.0.7, owned here): catalog
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

### Tooling and install
- [x] `atlas` CLI covering every flow, with `--sim` fleets.
- [x] Installers via `scripts/bundle.sh`: deb/rpm (udev rule included),
      AppImage, NSIS (WinUSB driver install), dmg. USB access fix button.
- [x] CI on Linux, Windows, macOS; installers on tags or manual runs.

## Not yet tested

- [x] Real Raze end to end: USB boot, eMMC write, verify, helper elevation
      (PhotonVision image, 2026-10-04/05).
- [ ] Open as USB disk / Browse / Eject / Add my SSH key on hardware.
- [ ] Raze 1.0.7 on hardware: LED colour order, locate, SSH keys, serial
      console. Fan polarity (`dtoverlay=raze-fan,polarity=0`) still to test.
- [ ] Raze with lemnosd (Lemnos cca50f7, `d5302db`) on hardware: an OS build
      with the `lemnos` source; the sensors binding through the bus
      selectors (`i2c:compatible=i2c-gpio`,
      `i2c:of=/axi/pcie@1000120000/rp1/i2c@74000`) and their readings; the
      udev rules (lemnosd as `lemnos`, not root); `raze-leds` and locate as
      lemnosd clients; `selftest --interactive` (frames above other
      clients' colours, but below a status: check with HeliOS running); the
      fan check's duties and hand-back; the LED direction; the INA238 shunt
      against the schematic; the RP1 GPIO chip label (`pinctrl-rp1`).
- [ ] USB hotplug and mDNS watching on real hardware (Linux, Windows, macOS).
- [ ] Windows and macOS installers on real machines.

## Next

- [x] atlas-driver-orion against Orion v4 (c22fa42): Orion adds readings,
      locate/reboot, and A/B updates to the device with the same board
      serial (capability sources in the core); tested against a fake Orion.
- [ ] Orion transport: Orion's embeddable operator client (signed
      orion+tcp, per-identity action rights). HTTP is query-only in v4, so
      the app registers no Orion directory until it lands.
- [x] Bundle host: `atlas-image-server` serves update images over HTTP
      (port 7700, per-update tokens, ranges, ETag) for boards to pull; the
      app passes it to the Orion directory, so Orion-reachable boards get
      updates. Still to test end to end with a real board's agent.
- [x] A/B writer in the device package (Raze 1.0.9–1.0.12, tested
      off-device in `devices/raze/tests/update.sh`).
- [x] SSH update transport for boards listing `ab-tryboot`.
- [ ] Verify A/B on a real Raze with HeliOS's PhotonVision image (they test
      stage/apply/confirm/rollback first).
- [x] `board-agent` for Orion-driven updates (crates/board-agent, Orion
      f3efb26; tested against a real orion-node with a fake writer).
- [ ] Run board-agent on a Raze with orion-node: a full Orion update,
      cancel and rollback, and the Gaia artifact (gaia/board-agent.toml) in
      a full image build.
- [ ] Lemnos asks from the Raze integration: a fan "release" request that
      hands one fan back while lemnosd runs (today only `fan restore`,
      root, via sysfs; lemnosd keeps the first write's governor state); a
      test/diagnostic LED layer above status for the self-test; the Raze's
      RP1 path (`/axi/pcie@1000120000/rp1/...`) in Lemnos's docs example.
- [ ] HeliOS secured mode (helios-api `docs/docs/api/http.md`): when
      `/v1/identity` reports `helios.auth.mode = secured`, send
      `Authorization: Bearer helios_<64 hex>` on everything but health,
      identity and auth/status (incl. `/v1/ota/*` and the `/v1/device/os`
      reconnect probe). Store tokens per board; prompt to paste one on 401
      (`WWW-Authenticate: Bearer`). Off by default; not used by PhotonVision.

- [ ] UI direction: confirm the current style fits; else compare 2-3
      directions side by side.
- [ ] Rebuild installers so they carry Raze 1.0.6.

## Later

- [ ] Device side (on hold): metrics, logs, and actions endpoints (locate via
      the LED ring, restart) and a camera stream in the Raze package.
- [ ] HeliOS driver / OTA updates for running devices.
- [ ] STM32 driver for the custom USB bootloader.
- [ ] Storage capacity "fits" check before flashing.
- [ ] Job ETA and throughput from the backend; friendlier failure reasons.
- [ ] Job history across restarts.
- [ ] Light theme.
- [ ] Tagged releases, code signing, Tauri self-updater.
