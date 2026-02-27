# Bundled Tooling

Atlas Hardware Manager can execute host tools from packaged app resources before falling back to `PATH`.

`tools/manifest.json` defines required bundled tools per deployment target.
`scripts/build-rpiboot-from-source.mjs` pulls and builds `rpiboot` from https://github.com/raspberrypi/usbboot and syncs binaries + `mass-storage-gadget64` into `tools/`.

## Lookup order

1. `ATLAS_TOOL_DIR` (if set)
2. App resource directory:
- `tools/<platform>-<arch>/`
- `tools/<platform>/`
- `tools/`
- `bin/`
3. System `PATH`

## Supported tags

- Platforms: `linux`, `macos`, `windows`
- Architectures: `x64`, `arm64`, `x86`

Examples:

- `tools/linux-x64/rpiboot`
- `tools/linux-arm64/rpiboot`
- `tools/windows-x64/rpiboot.exe`
- `tools/macos-arm64/rpiboot`

Repository includes target folders you can drop binaries into:

- `tools/linux-x64/`
- `tools/linux-arm64/`
- `tools/macos-x64/`
- `tools/macos-arm64/`
- `tools/windows-x64/`
- `tools/windows-arm64/`

Each profile must include:

- `rpiboot` (or `rpiboot.exe` on Windows)
- `mass-storage-gadget64/` boot files directory

## Tools currently invoked by backend

- `rpiboot` (bootloader mount/flash)
- `ip` (neighbor discovery)
- `lsusb` (USB enumeration)
- `lsblk` (flash target discovery)
- `dd` (image flashing)
- `xz` (compressed image streaming)
- `sudo` / `sync` (privileged operations on Linux)

## Notes

- Keep Unix binaries executable (`chmod +x`).
- Prefer bundling app-specific tools (`rpiboot`) and using native Rust APIs for generic host discovery over time.
- Bundling core OS utilities (`sudo`, `dd`, `ip`, `lsusb`, `lsblk`) is generally not recommended; rely on host OS for those.
- Validate bundle readiness with `bun run tools:verify:<target>`.
- Build and bundle rpiboot from source with `bun run rpiboot:build`.
- `rpiboot:build` compiles for the current host architecture. Build release artifacts on native builders for each target architecture.
