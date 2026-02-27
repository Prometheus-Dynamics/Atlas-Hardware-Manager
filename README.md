# Atlas Hardware Manager

Atlas Hardware Manager is a desktop app for discovering, inspecting, updating, and recovering Atlas/HeliOS devices over network and USB paths.

It is built with Tauri 2 (Rust backend + native packaging) and SvelteKit (frontend).

## What This Project Does

- Discovers devices over USB networking, standard network interfaces, and USB bootloader mode.
- Shows device identity and runtime metadata (IP/MAC, interface, USB location, firmware/OS/runtime fields).
- Runs update flows:
  - Mount-only bootloader flow (`rpiboot` + target remount).
  - Mount + flash flow (raw image write to exposed storage target).
  - OTA over API.
  - OTA over USB recovery serial protocol.
- Tracks live updater progress and step state in the UI.
- Detects existing in-progress OTA sessions and can re-attach to monitor them.
- Streams runtime telemetry from compatible devices.
- Fetches and manages device logs, including roboRIO WPILib log actions.
- Includes host setup checks and repair actions for required runtime tools/privileges.

## Tech Stack

- Frontend: SvelteKit 2, TypeScript, Tailwind CSS 4, Skeleton UI
- Backend: Rust + Tauri 2
- JS runtime/package manager: Bun (pinned to `1.2.9`)
- Native bundles: Tauri bundler (Linux/Windows release artifacts in CI)

## Repository Layout

- `src/`: SvelteKit frontend
- `src-tauri/`: Rust backend + Tauri config
- `scripts/`: build/tooling scripts (`run-build-target`, `rpiboot` bundling, verification)
- `tools/`: bundled runtime binaries/assets (`rpiboot`, `mass-storage-gadget64`, profile folders)
- `.github/workflows/`: CI workflows

## Prerequisites

## Required on all platforms

- Bun `1.2.9`
- Rust stable toolchain (`cargo`, `rustup`)
- Tauri build prerequisites for your OS (`bun run tauri:info` to validate)

Install dependencies:

```bash
bun install
```

Validate local environment:

```bash
bun run tauri:info
```

## Linux

Install Tauri/WebKit/GTK build deps:

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  librsvg2-bin \
  libusb-1.0-0-dev \
  build-essential
```

For release packaging on Linux hosts, also install `rpm`.

Runtime command dependencies expected on Linux for flashing/discovery:

- `ip`, `lsusb`, `lsblk`, `dd`, `xz`, `sudo`, `sync`

## Windows

- Visual Studio Build Tools (MSVC toolchain + Windows SDK)
- WebView2 Runtime

For local Windows release builds that bundle `rpiboot` from source, MSYS2 toolchain packages are required:

- `make`
- `mingw-w64-x86_64-gcc`
- `mingw-w64-x86_64-libusb`
- `mingw-w64-x86_64-pkgconf`

## macOS

macOS scripts remain available for local/native builds, but CI release artifacts are currently Linux + Windows only.

## Quick Start (Development)

Run the desktop app in dev mode:

```bash
bun run tauri dev
```

The root route redirects to `/network` (device operations), with `/settings` for host tooling/setup status.

## Feature Areas

## Device Discovery

- Scans and merges device candidates from USB and network paths.
- Dedupe/stability logic is applied to avoid stale or duplicate identity churn.
- Continuous discovery progress events are emitted to the UI.

## Updater and Recovery

- `mount`: bootloader detect -> `rpiboot` -> target rediscovery.
- `flash`: bootloader detect -> `rpiboot` (if needed) -> image write.
- `ota` over API: upload -> apply -> monitor -> reconnect.
- `ota` over USB recovery: serial transport upload/apply.
- Existing OTA probe + attach support when a device reconnects mid-update.
- Cancel and recovery session handling are built into backend commands.

## Logs and Telemetry

- Device log fetch via runtime API probing.
- roboRIO WPILib log listing/download/delete flows.
- Live telemetry stream start/stop with reconnect handling in UI.

## Host Setup and Repair

- Runtime checks report required/advisory tool readiness.
- Repair command attempts to seed/fix missing bundled tooling paths.
- Elevation-aware relaunch helper exists for Windows/macOS.

## Common Commands

## App/frontend

```bash
bun run dev
bun run build
bun run preview
bun run lint
bun run check
```

## Rust quality

```bash
cargo fmt --all --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo check --manifest-path src-tauri/Cargo.toml
```

## Tool bundling/verification

```bash
bun run rpiboot:build
bun run tools:verify:host
bun run tools:verify:linux:x64
bun run tools:verify:linux:arm64
bun run tools:verify:windows:x64
bun run tools:verify:windows:arm64
```

## Build and Packaging

Build scripts are host-aware and run through `scripts/run-build-target.mjs`.

## Linux

```bash
bun run build:linux
bun run build:linux:x64
bun run build:linux:arm64
bun run build:linux:appimage:x64
bun run build:linux:appimage:arm64
```

## Windows

```bash
bun run build:windows:x64
bun run build:windows:arm64
```

## macOS (local/manual only)

```bash
bun run build:macos:x64
bun run build:macos:arm64
bun run build:macos:universal
```

Notes:

- Linux/Windows installer builds require native hosts for full installer output.
- When not on the native host, some targets fall back to Docker cross builds and output raw binaries instead of installers.
- AppImage packaging is gated to Debian/Ubuntu-like Linux in the native path.

## Output Locations

- Native bundles: `src-tauri/target/<target>/release/bundle/` (or host default bundle path)
- Normalized build exports: `outputs/<task>/`
- Docker cross binaries: `dist/cross/<target-triple>/`

## Bundled Tooling (`tools/`)

Atlas Hardware Manager resolves tools in this order:

1. `ATLAS_TOOL_DIR` (if set)
2. Bundled app resources (`tools/<platform>-<arch>`, `tools/<platform>`, etc.)
3. System `PATH`

`tools/manifest.json` defines required bundled tools per target profile.

`rpiboot` + `mass-storage-gadget64` are treated as required bundled assets for supported profiles.

See [`tools/README.md`](tools/README.md) for profile layout details.

## CI Workflows

## Quality checks

Workflow: `.github/workflows/quality-checks.yml`

Triggers: push, pull_request, manual

Runs:

- frontend lint + type checks
- Rust fmt check
- Rust clippy with `-D warnings`

## Release installers

Workflow: `.github/workflows/release-installers.yml`

Triggers: tag push (`v*`), manual

Current release build matrix:

- `linux-x64`
- `linux-arm64`
- `windows-x64`
- `windows-arm64`

Release workflow publishes assets + `SHA256SUMS.txt` on tag runs.
On tag runs, CI also derives the app version from the tag name (`v2026.0.0` -> `2026.0.0`), applies it to version manifests before building, sets an MSI-compatible WiX version for Windows bundling, and commits those version-file updates back to the default branch when needed.

## Windows signing secrets (optional in release CI)

- `ATLAS_WINDOWS_CERTIFICATE_PFX_BASE64`
- `ATLAS_WINDOWS_CERTIFICATE_PASSWORD`
- `ATLAS_WINDOWS_TIMESTAMP_URL`

## Environment Variables

- `HELIOS_WORKSPACE_PATH`: override workspace path used for local HeliOS image discovery logic.
- `ATLAS_TOOL_DIR`: override bundled tool lookup root.
- `ATLAS_OUTPUT_DIR`: override normalized build output directory (default `outputs`).
- `ATLAS_STRICT_HOST_BUILDS=1`: fail instead of soft-skip on unsupported host/platform build invocations.

Build/signing-related variables are consumed by scripts/workflows as needed (`ATLAS_WINDOWS_*`, `ATLAS_RPIBOOT_*`, etc.).

## Troubleshooting

## Linux serial permission denied (`/dev/ttyACM*`)

If USB OTA serial access fails with permission denied:

```bash
sudo usermod -aG dialout $USER
```

Then sign out/in (or reboot) so group membership applies.

## Privileged operations fail

`rpiboot`, raw flashing, and some mount operations may require elevation depending on host policy.

- Linux: ensure sudo policy allows required operations.
- Windows: run elevated when needed.
- macOS: run with elevated privileges when needed.

## rpiboot build issues on Windows

The build script already applies Windows-specific compiler and `fmemopen` fallback handling for MSYS2 builds. If this still fails, verify MSYS2 packages listed above are installed and available in the build shell.

## License

MIT (see `package.json`).
