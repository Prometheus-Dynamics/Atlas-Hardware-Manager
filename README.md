# Atlas Hardware Manager (rebuild)

Atlas is the one tool for Prometheus Dynamics hardware: connect to a robot
once, see every PD device on it, and update, configure, or recover any of
them, on Linux, Windows, and macOS.

This branch is a ground-up rebuild. The previous app lives on `dev` and
`main` until this one replaces it.

## What works today

- **Desktop app** (`apps/desktop`): an overview per robot (readiness, live
  summary, how everything is connected, recent activity), inventory, a device
  page with live readings, camera view, logs, and quick actions, update and
  recovery flows with a plan preview, bulk jobs with staged rollout, robot
  profiles ("is this robot ready?" and one-click "Make ready"), a release
  catalog, and host health checks.
- **Live discovery, no polling**: USB hotplug and mDNS announcements trigger
  a look the moment something changes; a slow safety-net rescan catches
  devices that leave without saying so.
- **Features follow the device**: metrics, logs, actions, a camera stream,
  and a web UI link appear only when a device offers them. A running PD
  device opts in by listing `endpoints` (`metrics`, `logs`, `actions`),
  `actions`, and `camera_stream` in its identity document; responses are read
  leniently (see `crates/atlas-driver-pd/src/live.rs`). Nothing is required.
- **Fleet history**: devices found, lost, updated, and acted on, kept
  between sessions.
- **Raspberry Pi flashing, no rpiboot binary**: a compute module in USB boot
  mode appears as a recovery device. Recovering it boots it over USB with a
  pure-Rust port of the rpiboot protocol, waits for its eMMC to appear as a
  disk, writes the image through an elevated helper, and verifies it by
  reading it back.
- **Device packages** (`devices/<model>/`): each hardware model's manifest,
  informational compatibility list, EEPROM files, and the Gaia layer every
  OS build imports. Atlas reads the manifests to name boards in recovery,
  show their recovery steps, and offer bootloader updates.
- **Any OS, one contract**: running devices are found over mDNS
  (`_pd-device._tcp`) and identified through `/.well-known/pd-device`,
  keyed by model and serial; the OS is just an attribute.
- **Releases**: local image files and remote manifests. Signatures and
  checksums are used when available and never required: unsigned images get
  a warning, a checksum mismatch stops by default with "flash anyway".
- **`atlas` CLI**: every flow from a terminal, for scripts and CI.
- **Simulated devices** for demos, UI work, and CI.

Not yet: the HeliOS driver (waiting on the HeliOS device API), the STM32
bootloader driver, job history across restarts, and a signed Atlas
self-update.

## Workspace

| Path | What it is |
| --- | --- |
| `crates/atlas-driver` | The driver contract: identity (`family:serial`), links, capabilities, health checks, registry |
| `crates/atlas-core` | Inventory, scanning, jobs, robot profiles, events, persistence |
| `crates/atlas-release` | Release catalog, signed manifests, verified downloads |
| `crates/atlas-usbboot` | Raspberry Pi USB boot (rpiboot protocol) over `nusb` |
| `crates/atlas-blockdev` | Disk listing, safety checks, verified image writes, helper client |
| `crates/atlas-helper` | The only privileged binary: writes one image to one removable disk |
| `crates/atlas-devices` | Device package manifests and compatibility lists |
| `crates/atlas-driver-rpi` | Pi compute modules in USB boot mode, EEPROM updates |
| `crates/atlas-driver-pd` | Running PD devices via mDNS and the identity endpoint |
| `devices/` | Device packages, shared with the OS builds |
| `crates/atlas-driver-mock` | Simulated robot with gateways, recovery mode, and failure injection |
| `crates/atlas-cli` | The `atlas` command |
| `apps/desktop` | Tauri 2 shell (`src-tauri`) and SvelteKit UI (`src`) |

Design rules: devices are keyed by `family:serial`, never by IP or port.
Hosts only send intents and render the event stream. A device family is a
driver crate and nothing else. USB, serial, and disk access live in Rust.

## Run it

Prerequisites: Rust 1.99 (pinned in `rust-toolchain.toml`), Bun 1.4.2, and
the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
cd apps/desktop
bun install
bun run app:sim   # the desktop app with simulated devices
bun run app       # the desktop app with real hardware
bun run dev       # the UI alone in a browser, with an in-browser mock backend
```

For real hardware, fetch the pinned USB boot files once:

```bash
scripts/fetch-usbboot-files.sh
```

On Linux, allow your user to open Pi boot devices (`atlas doctor` prints the
exact rule and fixes for anything else it finds):

```bash
echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="0a5c", ATTR{idProduct}=="2711|2712|2763|2764", MODE="0660", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/60-atlas-usbboot.rules
```

The CLI:

```bash
cargo run -p atlas-cli -- doctor
cargo run -p atlas-cli -- disks
cargo run -p atlas-cli -- ls
cargo run -p atlas-cli -- update rpi:port-1-2 --image helios-cm5.img.xz
cargo run -p atlas-cli -- --sim demo update --all --release sim-helios=2026.3.1 --release sim-mcu=1.5.0
```

## Build installers

```bash
cd apps/desktop
bun run bundle
```

This fetches the boot files, builds `atlas-helper` as a sidecar, and runs
`tauri build` with `src-tauri/tauri.bundle.conf.json`. CI does the same on
Linux, Windows, and macOS for tags and manual runs.

Windows needs the WinUSB driver bound to the Pi boot device (the Raspberry Pi
rpiboot installer provides it); `atlas doctor` and the Settings screen say so
when it is missing.

## Development

```bash
./scripts/ci.sh
```

That runs `cargo fmt --check`, the 600-line file-size check, clippy with
`-D warnings`, and the tests. In `apps/desktop`, `bun run check` type-checks
the UI. CI runs all of it, plus CLI smoke tests, on Linux, Windows, and macOS.
