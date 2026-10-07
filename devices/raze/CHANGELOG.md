# Raze device package changelog

## Unreleased

Development work since 1.0.7, not yet validated on hardware as a release.
The package version stays 1.0.7 until a real release. OSes pin this layer by
commit; the commits are listed per area.

### Breaking and ABI notes

- **Platform:** Buildroot 2026.08, kernel `rpi-7.2.y` (7.2.9, bcm2712, 16K
  pages) and rpi-firmware 1.20260915 (`fd52491`). Buildroot 2026.08 removed
  rpi-userland, so `BR2_PACKAGE_RPI_USERLAND` must go from OS overrides.
  OpenJDK's default became 25.
- **Toolchain:** Bootlin aarch64 glibc bleeding-edge 2025.08-1 (gcc 15,
  kernel headers 5.15) replaces the defconfig's stable one, which has 5.4
  headers. libpisp needs `linux/dma-heap.h` (5.6); libpisp and libcamera's
  rpi/pisp pipeline now refuse an older toolchain at configure time
  (`c881c60`).
- **libcamera 0.7.2** (soname `libcamera.so.0.7`) and libpisp 1.7.0, pinned by
  commit with `.hash` files. Rebuild anything linked against 0.6, such as
  PhotonVision's libcamera GL driver (`fd52491`, `c881c60`).
- **Kernel modules:** the CFE driver for `raspberrypi,rp1-cfe` is now
  `rp1-cfe-downstream.ko`. A Raze kernel fragment
  (`buildroot-external/linux/raze.config`, set through
  `BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES`) removes:
  - Wi-Fi, Bluetooth, NFC, 802.15.4 and ATM;
  - TV, radio, SDR, RC and gspca;
  - the 43 camera sensors other than the OV9782;
  - MD RAID, DRBD and NBD;
  - btrfs, xfs, f2fs, nfs/nfsd, cifs, ntfs3, iso9660, udf and hfs.

  An OS adds its own fragments through `BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES`;
  the package's are merged after them (`695d172`).
- **Kernel page size: 4 KiB, explicit.** Buildroot sets the kernel's page size
  from `BR2_ARM64_PAGE_SIZE_*` after all fragments, so every Raze image has
  run 4K. bcm2712_defconfig's 16K and the kernel's "-v8-16k" name never
  applied. kernel.toml now sets `BR2_ARM64_PAGE_SIZE_4K`, raze.config states
  it, and the build fails if the kernel config disagrees. Moving to 16K needs
  a hardware test of the OS's prebuilt native libraries first.
- **Kernel fragment policy:** `BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES` belongs
  to the OS (empty by default, which also drops the defconfig's 4K fragment).
  The package's raze.config is appended after the OS's fragments in
  external.mk, so the package's decisions win. The build also fails if EROFS
  isn't built in.
- **Read-only root:** EROFS is built into the kernel, and nothing in the
  package writes to `/`. SSH keys from the boot partition go to
  `/run/pd-device/ssh/authorized_keys`, which needs the OS's sshd_config to
  `Include /etc/ssh/sshd_config.d/*.conf` before any `AuthorizedKeysFile`
  (`695d172`).
- **Update input:** `update stage` takes the same whole-disk image that is
  flashed over USB, plus `--sha256`. There is no separate update format (an
  interim `.pdupdate` bundle never shipped) (`696d3ad`).

### Updates (A/B with tryboot)

- `/usr/lib/pd-device/update`: `status`, `stage`, `apply`, `confirm`,
  `rollback`.
  - **stage:** `stage <image> --sha256 <hex>`, or
    `stage - --sha256 <hex> [--format xz|zst|gz|raw]` from stdin.
    - It copies the image's boot slot A (p2) and root slot A (p5) into the
      inactive slot in one streaming pass through `pd-image-slots`, a small C
      tool in this package. The rest of the image is skipped.
    - A file is checked against its SHA-256 before anything is written.
    - The new root must name this model. Its os-release gives the version
      (`IMAGE_VERSION`, else `VERSION_ID`).
    - Images without the A/B layout are refused.
  - **apply:** boots the new slot once with `reboot "0 tryboot"`.
  - **confirm:** `pd-device-update-confirm.service` keeps the new slot after
    the OS's `update-health` passes. A failed check restarts into the old
    slot, and the hardware watchdog (RuntimeWatchdogSec=15s) covers a hung
    trial.
  - **Locking:** a lock lets only one writer run. `status` answers from the
    last state while another command runs.
  - Commits: `e12e22d`, `5baf83e`, `696d3ad`.
- OS hooks in `/etc/pd-device/update.d/`: `pre-stage`, `post-stage`,
  `pre-reboot`, `post-boot`. A failing pre- hook stops its step (`696d3ad`).
- Layout: p1 autoboot.txt (and the update state), p2/p3 boot, p5/p6 root
  (512 MiB target, EROFS), p7 /data. See docs/ota.md.
- The identity lists `ab-tryboot` in `update_methods`, and an `update` object
  with the state, slots, versions, progress and error.
- The image gets zstd, xz-utils and pd-image-slots.

### Fresh installs without the boot button

- `/usr/lib/pd-device/usb-boot` restarts a running CM5 straight into USB boot
  (RPIBOOT). It uses the firmware's one-time `set_reboot_order`, so the
  bootloader's own BOOT_ORDER never changes. It is root-only and run over SSH
  (later Orion), never through the open identity endpoint.
- The identity lists `usb-boot-reboot` when the board supports it.
- `rpi-utils` builds `vcgencmd` and `vcmailbox` (`194a159`).

### Hardware

- **LED ring:** SK6812-EC20, 24-bit RGB. The `ws2812-pio` overlay no longer
  passes `rgbw`. `raze-leds` keeps the driver's 4-byte layout, so white is
  `color 255 255 255` (`f98489f`). Verified on a Raze.
- **Fan:** normal polarity, levels 179/212/245/255/255 (a 70 % minimum)
  (`ff18ab8`, `f98489f`). Verified on a Raze. `dtoverlay=raze-fan,polarity=1`
  restores inverted drive.
- **Overlays:** the stock overlays `raze-device.txt` loads are built from the
  kernel being built, not taken from the firmware (`fd52491`).

### Optional layers

- `gaia/gpu-vulkan.toml`, after `gpu.toml`: Mesa's v3dv driver, the Vulkan
  loader and vulkan-tools (`e5f9087`).

### Smaller image

- No udev hwdb (systemd or eudev), about 13 MB.
- Kernel modules drop from 25 MB to 15 MB; sound stays, because DRM_VC4
  depends on it (`695d172`).

### Runtime paths and settings

- **SSH keys:** `ssh-keys` finds the boot partition from `root=` on the
  kernel command line, so an overlay root works. On the A/B layout that is
  p1, shared by both slots (`a6dec52`).
- **Overrides:** `*.env` settings are read from `/usr/lib/pd-device`, then
  `/etc/pd-device`, then `/data/pd-device`. device-package.env is never read
  from `/data` (`695d172`).
- **Commit stamp:** `PD_DEVICE_PACKAGE_COMMIT` comes from the OS, in
  `/etc/pd-device/device-package.env`, for git imports (`a6dec52`).
- **DHCP leases** for the USB gadget live in `/run` (`695d172`).

### Tests

- `devices/raze/tests/`:
  - `update.sh` uses a real sfdisk A/B image, compressed with xz and zstd,
    with an EROFS root slot when erofs-utils is new enough.
  - `usb-boot.sh` uses a fake mailbox.
  - `root-device.sh` checks finding the root device.
- CI runs them on Ubuntu 24.04.

## 1.0.7

- LED ring helper `/usr/lib/pd-device/raze-leds`: `on`, `off`, `set <0|1>`,
  `dim <0-100>`, `color <r> <g> <b> [<w>]`, `status <ok|warn|error|busy|off>`,
  `blink [<hz>]`, `locate [<seconds>]`, `show`. Writes 16 x RGBW frames to
  `/dev/leds0`; state in `/run/pd-device/leds.state`. `set` and `dim` fit
  PhotonVision's custom LED commands (`raze-leds set {v}`, `raze-leds dim {v}`).
- Locate over the identity endpoint: the identity JSON lists
  `"endpoints": {"actions": "/actions"}` and a `locate` action; `POST
  /actions/locate` on port 5899 blinks the ring for 10 s. The endpoint stays
  unprivileged: it drops a request file that `pd-device-locate.path` acts on.
  `PD_ACTIONS` in `identity.env` lists the offered actions.
- Opt-in SSH keys: `pd-device-ssh-keys.service` adds the public keys in
  `<boot partition>/pd-device/authorized_keys` to root's `authorized_keys` at
  boot. Nothing is removed and password login is never enabled.
- Serial recovery console on the USB gadget's ACM ports (`ttyGS0`, `ttyGS1`):
  root autologin, started by the gadget service once the ports exist. Mask
  `serial-getty@ttyGS0/1.service` to turn it off.
- Identity schema: documents the optional `endpoints`, `actions`, and
  `camera_stream` fields Atlas reads.

## 1.0.6

- Toolchain: pass OpenJDK's target binutils as configure arguments. 1.0.5 set
  them in the environment, which OpenJDK's configure ignores ("Use command
  line variables instead"), so it still used the host objcopy/strip.

## 1.0.5

- Toolchain: OpenJDK's configure now gets the target objcopy, strip, nm and
  ar explicitly. With the Bootlin external toolchain (tools named
  `aarch64-linux-*`) it fell back to the host's x86 objcopy, and jlink failed
  to strip the aarch64 runtime. Only applies when OpenJDK is selected.

## 1.0.4

- GPU: with `gpu.toml` (Mesa EGL), the external tree keeps rpi-userland out of
  staging so its old Broadcom EGL/GLES headers can't replace Mesa's. OSes no
  longer need their own copy of this fix.
- I2C: `i2c-dev` is loaded by the package (`/usr/lib/modules-load.d/raze-i2c-dev.conf`),
  so the I2C buses it enables have userspace device nodes.
- README: OSes that manage Ethernet with NetworkManager should clear
  `BR2_SYSTEM_DHCP`, or systemd-networkd runs a second DHCP client on it.

## 1.0.3

- EEPROM: `eeprom/recovery.bin` is now the one from rpi-eeprom release
  `v2025.12.08-2712` (`cb1a22e`, sha256 `ad66b296…1d3d`), matching the bundled
  2025-12-08 pieeprom, instead of the 2025-08-27 build usbboot pins.

## 1.0.2

- EEPROM: `eeprom/recovery.bin` added (rpi-eeprom `360324a`, BCM2712
  2025-08-27, sha256 `7993e58a…218c`), so bootloader updates over USB boot
  work without a separate fetch.
- Recovery instructions: the status LED is red while the boot button is held
  with power applied, then turns green and the fan starts once the USB boot
  begins.

## 1.0.1

- Revisions: the placeholder rev `a` is now `gen1`. First-generation boards
  carry no revision marker, so a board without one is `gen1`; later revisions
  will be provisioned with a marker.
- Recovery instructions: hold the boot button and connect the regular USB port
  (the flashing port). Storage is eMMC on every current board.
- LEDs: the ring is SK6812 RGBW, so the `ws2812-pio` overlay now passes `rgbw`.

## 1.0.0 (contract 1)

First shared package, reconciled from HeliOS (`gaia/` on
`architecture-overhaul`), photon-image-modifier `gaia-build-fix`
(`helios/raze/`) and the old shell build in photon-image-modifier `main` at
`45d4b82` (`helios-raze/`).

- Kernel: Raspberry Pi `stable_20250916`, `raspberrypicm5io_defconfig`, OV9782
  as a variant of the upstream ov9282 driver. The kernel tarball hash is wired
  in through the external tree, so no `BR2_GLOBAL_PATCH_DIR` path is needed.
- Camera: libcamera `v0.6.0+rpt20251202` and libpisp `pios/1.3.0-1` package
  overrides with the OV9782 patches, IPA module signing and opt-in LTTng
  tracing; the PiSP pipeline; the OV9782 PiSP tuning. libcamera depends on the
  standard Buildroot `jpeg` package (HeliOS used its own `turbojpeg`).
- Boot: `raze-device.txt`, included from the OS `config.txt`. The OV9782 overlay
  loads on CAM0 with a continuous clock, as in HeliOS and the old build.
- Fan: new `raze-fan` overlay. The kernel thermal governor drives the fan at
  88-100 % (levels 224/240/248/255/255) with inverted PWM on RP1 PWM1 channel 3,
  41566 ns. Replaces `helios-fan-overlays.sh` and `helios-cm5-fan.dts`.
- USB port power: new `raze-usb-power` overlay with GPIO hogs (USB-A GPIO20,
  USB-C GPIO16, active high) and a udev rule that keeps USB devices out of
  runtime suspend and USB2 LPM. Replaces `helios-usb-power.dts`,
  `helios-usb-power-setup.sh` and its service.
- LED ring: `ws2812-pio` on GPIO13, 16 LEDs, the `rp1_pio` soft dependency and
  `raze-leds-reprobe.service`.
- USB gadget: Prometheus Dynamics / Raze, `1d6b:0104`, serial = board serial.
  ECM + RNDIS + ACM, MACs derived from the serial with the locally administered
  bit (the same addresses as before), falling back to `/etc/machine-id`, never
  a constant. The gadget bridge carries only gadget interfaces: the old build
  added `end0`/`end1` to it, which put 172.31.250.1 and a DHCP server on the
  robot network. DHCP serves the USB link only, without DNS or a default route.
- Identity (contract 1): `/run/pd-device/identity.json`, the HTTP endpoint on
  port 5899 and the `_pd-device._tcp` mDNS advertisement through
  systemd-resolved. Plain systemd units and POSIX sh.
- Hostname: `raze-{serial8}` as a default only, applied over an unset or stock
  hostname.
- EEPROM: bootloader 2025-12-08 (`2226a853`), unchanged from the old build.

### Renamed from HeliOS

| HeliOS | Package |
| --- | --- |
| `helios-usb-gadget.service`, `/usr/local/bin/usb-gadget-setup.sh` | `pd-device-usb-gadget.service`, `/usr/lib/pd-device/usb-gadget-setup` |
| `helios-dnsmasq.service`, `/etc/dnsmasq.d/usb0.conf` | `pd-device-usb-gadget-dhcp.service`, generated `/run/pd-device/usb-gadget-dnsmasq.conf` |
| `/etc/helios/gadget.env` | `/usr/lib/pd-device/usb-gadget.env`, overrides in `/etc/pd-device/usb-gadget.env` (keys now prefixed `USB_GADGET_`) |
| `/etc/systemd/network/10-usbbr0.netdev`, `10-usb0.network`, `11-usb-gadget-slaves.network`, `12-usbbr0.network` | set up by `usb-gadget-setup`; networkd and NetworkManager are told to leave the gadget link alone |
| `/etc/modules-load.d/usb-gadget.conf` | `/usr/lib/modules-load.d/pd-device-usb-gadget.conf` |
| `helios-usb-power.service`, `helios-usb-power-setup.sh`, `/etc/helios/usb-power.env`, `99-helios-usb-power.rules`, `helios-usb-power.dts` | `raze-usb-power` overlay, `/usr/lib/udev/rules.d/60-raze-usb-power.rules` |
| `helios-cm5-fan.dts`, `helios-fan-overlays.sh` | `raze-fan` overlay |
| `ws2812-reprobe.service`, `/etc/modprobe.d/ws2812-pio.conf` | `raze-leds-reprobe.service`, `/usr/lib/modprobe.d/raze-ws2812-pio.conf` |
| `/etc/helios/sensors.toml` | `/usr/share/pd-device/raze/sensors.toml` (same content) |
| `ov9782-overlay.dtbo` | `ov9782.dtbo` |
| `/usr/share/helios/bootloader/pieeprom-2025-12-08.{upd,sig}`, `/etc/helios/bootloader.conf` | `eeprom/` in the package, for Atlas; no in-OS updater is shipped |

Not carried over: `/etc/helios/fan.toml` and `leds.toml` (configuration for
HeliOS's own peripherals daemon), the gadget watchdog and reset scripts,
`helios-stage-bootloader-update`, PhotonVision camera seeding and
`hardwareConfig.json`, the Python LED helper, and the vc4 (Pi 4) OV9782
tuning, which a CM5 does not use.
