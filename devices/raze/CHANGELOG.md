# Raze device package changelog

## 1.5.0

Smaller images, and a read-only root.

- **EROFS root:** `CONFIG_EROFS_FS=y` (with LZMA and ZSTD) is built in, so a
  read-only compressed root mounts without an initramfs. The OS's cmdline
  adds `rootfstype=erofs ro`. Root slots target 512 MiB; nothing assumed
  2 GiB (pd-image-slots checks the fit).
- **Nothing writes to `/` at runtime:**
  - gadget DHCP leases go to `/run/pd-device/`;
  - SSH keys from the boot partition go to `/run/pd-device/ssh/authorized_keys`,
    read through `/etc/ssh/sshd_config.d/50-pd-device.conf`. The OS's
    sshd_config must `Include` that directory before any
    `AuthorizedKeysFile`. The home directory copy is kept when writable;
  - `*.env` overrides are also read from `/data/pd-device/`, after
    `/etc/pd-device/`. device-package.env is never read from there.
  - Identity, update and LED state already lived in `/run` or on p1.
- **Kernel trims** (`buildroot-external/linux/raze.config`,
  `BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES`): no Wi-Fi, Bluetooth, NFC,
  802.15.4 or ATM; no analog/digital TV, radio, SDR, RC or gspca; 43 unused
  camera sensors (only the OV9782 via ov9282); no MD RAID, DRBD or NBD; no
  btrfs, xfs, f2fs, nfs/nfsd, cifs, ntfs3, iso9660, udf or hfs. Modules drop
  from 25 MB to 15 MB (1889 to 1410).
  - Sound stays, because DRM_VC4 (display and GPU) depends on it.
  - Also kept: the bridge (USB gadget), CAN, USB cameras and USB Ethernet,
    IIO and hwmon, and overlayfs.
- **No udev hwdb:** `BR2_PACKAGE_SYSTEMD_HWDB` and
  `BR2_PACKAGE_EUDEV_ENABLE_HWDB` are off, about 13 MB.
- Mesa was already only v3d/vc4, plus v3dv in the Vulkan layer.
- `update` takes `UPDATE_SYNC`, so tests can skip whole-system syncs.
- `tests/update.sh` installs an image with a real EROFS (LZMA) root slot
  when erofs-utils >= 1.5 is present. CI runs the device tests on Ubuntu
  24.04 for that.

## 1.4.0

- **One image for everything:** `update stage` takes the same `.img`/`.img.xz`/
  `.img.zst`/`.img.gz` that is flashed over USB, plus `--sha256` of that file.
  The `.pdupdate` bundle is gone; nothing had shipped with it.
  - The writer copies the image's boot slot A (p2) and root slot A (p5) into
    the inactive slot and skips p1, the other slot and /data.
  - It reads the image as one stream through `pd-image-slots`, a new C tool
    in this package (`packages/pd-image-slots`, `BR2_PACKAGE_PD_IMAGE_SLOTS`).
    The tool follows the MBR and EBR chain as they pass, checks the
    partitions fit, and reports progress.
  - A file is checked against `--sha256` before anything is written. stdin
    (`stage - --sha256 <hex> [--format xz]`) is checked at the end, and a
    mismatch leaves the slot unstaged.
  - The new root must name this model, in its `device-package.env`. Its
    os-release gives the version (`IMAGE_VERSION`, else `VERSION_ID`).
  - Images without the A/B layout are refused with that reason.
- **OS hooks** in `/etc/pd-device/update.d/`: `pre-stage`, `post-stage`,
  `pre-reboot`, `post-boot`. A failing pre- hook stops its step.
- The image gets xz-utils (`BR2_PACKAGE_XZ`). busybox `xzcat` still works
  without it.
- `tests/update.sh` now builds a real A/B image with sfdisk, compresses it
  with xz and zstd, and compiles `pd-image-slots` from source.

## 1.3.0

- **Restart into USB boot without the button:** `/usr/lib/pd-device/usb-boot`
  sets a one-time boot order of RPIBOOT through the firmware mailbox
  (`set_reboot_order`, tag 0x0003808b; Pi 5/CM5 only), then reboots. The
  bootloader's own BOOT_ORDER is untouched, and the next normal power-up
  boots the eMMC again. `usb-boot --check` reports whether the board
  supports it.
  - RPIBOOT has no timeout: the board waits for a USB host until it is
    flashed or power-cycled. So the script is root-only and Atlas runs it
    over SSH (Orion later). It is never offered on the unauthenticated
    identity endpoint.
- The identity's `update_methods` gains `usb-boot-reboot` when the board
  supports it (a BCM2712 with vcmailbox).
- `rpi-utils` now also builds `vcmailbox` (one C file, compiled directly).
- Test: `devices/raze/tests/usb-boot.sh`, with a fake mailbox and device tree.
- Not yet tried on hardware.

## 1.2.0

- New optional layer `gaia/gpu-vulkan.toml`, imported after `gpu.toml`. It
  adds Vulkan on the V3D GPU: Mesa's v3dv driver
  (`BR2_PACKAGE_MESA3D_VULKAN_DRIVER_BROADCOM`), the Vulkan loader and
  headers, and vulkan-tools (vulkaninfo, vkcube; pulls in vulkan-sdk). An
  OS that only needs GLES doesn't import it and doesn't pay for it.

## 1.1.1

- **Toolchain:** Bootlin aarch64 glibc **bleeding-edge** 2025.08-1 (gcc 15,
  kernel headers 5.15) instead of the defconfig's stable one (gcc 14, headers
  5.4). libpisp 1.7.0 needs `linux/dma-heap.h` (5.6), and PhotonVision's
  natives need glibc >= 2.38.
- libpisp and libcamera's rpi/pisp pipeline now `depends on
  BR2_TOOLCHAIN_HEADERS_AT_LEAST_5_6`, so an older toolchain fails at
  configure time instead of mid-build.
- libpisp and libcamera are pinned by commit. libpisp has both a tag and a
  branch named v1.7.0. Both packages now ship `.hash` files (tarball and
  license files).

## 1.1.0

Platform upgrade: every layer moves to its newest release. Validated off the
board (see below); not yet run on hardware.

- **Buildroot 2026.08** (from 2025.11.3). `raspberrypicm5io_defconfig` and
  `board/raspberrypi` are unchanged, and every `config_overrides` symbol
  survives `olddefconfig` with the packages staged as Gaia does.
  - OpenJDK's default becomes 25.
  - Mesa moves to 26.1.8, and dnsmasq to 2.93.
- **Kernel:** raspberrypi/linux `rpi-7.2.y` at 53679a5 (7.2.9) replaces
  `stable_20250916` (6.12). It still uses bcm2712 with 16K pages.
  - The ov9782 patch is ported to 7.2's CCI-regmap ov9282 driver, with the
    same behaviour.
  - `make Image modules dtbs` builds cleanly.
  - Every module and symbol the OS uses is present: RP1 CFE, PiSP BE, rp1-pio,
    ws2812-pio-rp1, dwc2/libcomposite/configfs gadget functions, pwm-fan with
    pwm-rp1, i2c, and ov9282.
  - Behaviour change: the CFE driver for `raspberrypi,rp1-cfe` is now
    `rp1-cfe-downstream.ko`. `rp1-cfe.ko` is the upstream driver and binds
    only `-upstream`. Both register as "rp1-cfe"; nothing in the package names
    the module.
- **Firmware:** rpi-firmware 1.20260915 (`packages/rpi-firmware` override;
  the tarball is 182 MB).
  - The stock overlays raze-device.txt loads are now built from the kernel
    (`images/raze-overlays/`, external.mk) instead of being taken from the
    firmware. The firmware's overlays come from its own kernel (6.18), and
    7.2's `ws2812-pio` changed.
- **libcamera** v0.7.2+rpt20260817 and **libpisp** v1.7.0.
  - The ov9782 patches are rebased and apply cleanly.
  - The ov9782 tuning still parses under 0.7.2's controller.
  - The soname moves from libcamera.so.0.6 to 0.7: rebuild anything linked
    against it, such as PhotonVision's libcamera GL driver.
- **rpi-userland removed** (Buildroot dropped it). `vcgencmd` now comes from
  `packages/rpi-utils` (raspberrypi/utils e0484c8, vcgencmd only). The
  external tree no longer hides rpi-userland's EGL headers from staging.
  Other userland tools (vcmailbox, dtoverlay) are gone; nothing here used
  them.

## 1.0.12

- `update` takes a lock (`/run/pd-device/update.lock`), so two commands never
  write at once. `status` still answers while another command runs, from the
  last `update.json`, which is how Atlas shows staging progress. A `staging`
  state left by an interrupted stage can now be staged again; only a running
  trial (`trying`) blocks a new stage.

## 1.0.11

- `ssh-keys` finds the boot partition from `root=` on the kernel command line
  (PARTUUID/UUID/LABEL resolved), so an overlay or squashfs root works; with
  no block device behind `/` it falls back to `/dev/mmcblk0p1`.
  `SSH_KEYS_BOOT_PARTITION` still overrides. On the A/B layout that is p1,
  shared by both slots, so keys survive updates.
- `lib.sh`: `pd_root_device` and `pd_sibling_partition`, shared by `ssh-keys`
  and `update`.
- `PD_DEVICE_PACKAGE_COMMIT`: an OS that imports the layer straight from git
  sets it in `/etc/pd-device/device-package.env` (Gaia:
  `${source.<id>.commit}`), which is read after the package's file.

## 1.0.10

- LEDs: the ring is SK6812-EC20, 24-bit RGB, not RGBW. The `ws2812-pio`
  overlay no longer passes `rgbw` (32 bits per LED smeared the colours around
  the ring). `raze-leds` keeps writing 4 bytes per LED, the driver's
  userspace layout; W is dropped on the wire, so white is `color 255 255 255`.
  `RAZE_LEDS_ORDER` must stay four letters. Verified on a Raze: red, green,
  blue and white on all 16 LEDs.
- Fan: 70 % minimum, levels 179/212/245/255/255 (70, 83, 96, 100 %), normal
  polarity. Verified on a Raze: 83 % at 58 °C.

## 1.0.9

- A/B updates: `/usr/lib/pd-device/update` (status, stage, apply, confirm,
  rollback) writes a `.pdupdate` bundle (`manifest.env`, `boot.vfat.zst`,
  `rootfs.ext4.zst`) to the inactive slot after checking every SHA-256,
  boots it once with tryboot, and keeps it only when
  `pd-device-update-confirm.service` finds it healthy. Otherwise the board
  restarts into the previous version. The layout is p1 autoboot.txt,
  p2/p3 boot, p5/p6 root (docs/ota.md). Images built without that layout
  are unaffected.
- The identity reports `update_methods: ["image-write", "ab-tryboot"]` and an
  `update` object (state, slots, versions, progress, error) on an A/B layout.
- The hardware watchdog is armed (RuntimeWatchdogSec=15s), so a hung trial
  boot resets back to the confirmed slot.
- The image gets zstd (`BR2_PACKAGE_ZSTD`).
- Settings live in `update.env`; an OS adds its health check as
  `/etc/pd-device/update-health`.

## 1.0.8

- Fan: normal PWM polarity and a 50 % minimum. On a Raze Gen 1 the inverted
  default left the fan "basically not moving" (the 88 % low level came out
  near 12 %, and full speed would have stopped it). Levels are now 128/160/
  200/255/255 (50 %, 63 %, 78 %, 100 %). `dtoverlay=raze-fan,polarity=1`
  restores inverted drive for a revision that needs it.

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
