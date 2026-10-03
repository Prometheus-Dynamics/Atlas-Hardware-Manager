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
- [x] Device packages in `devices/` (Raze, currently 1.0.6 from the HeliOS
      session's `raze-device` branch): catalog naming, recovery steps,
      EEPROM files.
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

### Tooling and install
- [x] `atlas` CLI covering every flow, with `--sim` fleets.
- [x] Installers via `scripts/bundle.sh`: deb/rpm (udev rule included),
      AppImage, NSIS (WinUSB driver install), dmg. USB access fix button.
- [x] CI on Linux, Windows, macOS; installers on tags or manual runs.

## Not yet tested

- [ ] Real Raze end to end: USB boot, eMMC write, verify, helper elevation.
- [ ] USB hotplug and mDNS watching on real hardware (Linux, Windows, macOS).
- [ ] Windows and macOS installers on real machines.

## Next

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
