# Raze device package changelog

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
