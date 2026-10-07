# Atlas rebuild: progress and TODO

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
- [x] PD driver for running devices: mDNS `_pd-device._tcp` +
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
- [ ] USB hotplug and mDNS watching on real hardware (Linux, Windows, macOS).
- [ ] Windows and macOS installers on real machines.

## Next

- [x] atlas-driver-orion against Orion v4 (3cc974e): Orion adds readings,
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
- [ ] `pd-device-agent` for Orion-driven updates.
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
