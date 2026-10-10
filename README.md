# Atlas Hardware Manager (rebuild)

Atlas is the one tool for Prometheus Dynamics hardware: connect to a robot
once, see every board on it, and update, configure, or recover any of
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
  and a web UI link appear only when a device offers them. A running
  device opts in by listing `endpoints` (`metrics`, `logs`, `actions`),
  `actions`, and `camera_stream` in its identity document; responses are read
  leniently (see `crates/atlas-driver-board/src/live.rs`). Nothing is required.
- **Fleet history**: devices found, lost, updated, and acted on, kept
  between sessions; a board's own event log (by seq, so a board clock that is
  off can't hide events), pushed as it happens by boards with `board-stream`.
- **Live hardware**: a board's sensors at device rate (the Raze's IMU at
  100 Hz) as live charts on its Hardware tab, while the tab is open.
- **NetworkTables**: view any NT4 server live (a team's robot by its number,
  or a camera's own server by address): the topic tree with values, search,
  and graphs of number topics. Or start a plain NT4 server on this computer,
  with topics you create and edit, to test a PhotonVision or HeliOS camera
  without a roboRIO; the page says which address to give each camera. On
  Orion's `orion-nt4`.
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
| `crates/atlas-driver-board` | Running boards via mDNS and the identity endpoint |
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

The installers put the CLI next to the app (`/usr/bin/atlas` on Linux), where
it finds `atlas-helper`, the Pi boot files and the device packages the way the
app does. A fresh install from a terminal, the same job as the app's Flash:

```bash
# A board already in USB boot (boot button, or a blank eMMC):
atlas flash --image helios-raze.img.xz --ssh-key ~/.ssh/id_ed25519.pub
# A running board: restart it into USB boot over SSH first, then flash.
atlas flash --usb-boot raze-8f3a1c2d --image helios-raze.img.xz --ssh-key ~/.ssh/id_ed25519.pub
```

It boots the board's mass-storage gadget, writes the one USB disk that appears
(it refuses when none or several do, and never writes a disk that was already
there), reads it back, puts the keys on the boot partition and ejects. The
write runs `atlas-helper` through pkexec: over SSH, use a terminal (`ssh -t`)
and type your password when polkit asks. `atlas doctor` checks USB access
first (`atlas fix usbboot.install-access` installs the udev rule).

## Build installers

```bash
cd apps/desktop
bun run bundle
```

This fetches the boot files, builds `atlas-helper` and the `atlas` CLI as
sidecars, and runs
`tauri build` with `src-tauri/tauri.bundle.conf.json`. CI does the same on
Linux, Windows, and macOS for tags and manual runs.

Windows needs the WinUSB driver bound to the Pi boot device (the Raspberry Pi
rpiboot installer provides it); `atlas doctor` and the Settings screen say so
when it is missing.

## Development

```bash
./scripts/ci.sh
```

That runs `cargo fmt --check`, the 600-line file-size check, the device
package tests (in parallel), clippy with `-D warnings`, and the tests. In
`apps/desktop`, `bun run check` type-checks the UI. CI runs all of it, plus
CLI smoke tests, on Linux, and on Windows and macOS when a change touches more
than the device package and docs.

Keeping it quick:

- Install `cargo-nextest` (`cargo install cargo-nextest --locked`): `ci.sh` uses
  it, and it runs every test binary's tests at once (6 s instead of 24 s).
- Put `target/` on a fast internal disk. On a slow or busy drive a build can sit
  for an hour waiting on I/O; on an NVMe the whole workspace's tests build cold in
  about 80 s, and a change to a core crate rebuilds in about 3 s. Either export
  `CARGO_TARGET_DIR`, or give the checkout a git-ignored `.cargo/config.toml`:

  ```toml
  [build]
  target-dir = "/path/on/a/fast/disk/atlas-target"
  ```

  The scripts (`ci.sh`, `bundle.sh`) follow it; installers then land under that
  directory instead of `target/`.
- Dev builds carry line tables only (and no debug info for dependencies), so
  `target/` stays about 3.4 GB instead of 8. For a debugger session, build with
  `CARGO_PROFILE_DEV_DEBUG=true`.
