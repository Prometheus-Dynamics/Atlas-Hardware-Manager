# Device packages

A device package is everything one hardware model needs, kept once and shared
by every OS that ships images for it and by Atlas:

- the OS builds import its Gaia layer to get the kernel, drivers, boot
  configuration, overlays and device services;
- Atlas reads its manifest to recognise a device on USB or the network, to
  recover it, and to flash images and bootloader updates.

A package carries device support only: no OS policy (init system, root
filesystem, update agent) and no application (vision software, frontends).

```
devices/
  README.md                  this file
  schema/                    JSON Schema 2020-12 for the files below
    manifest.schema.json
    compat.schema.json
    identity.schema.json     the document served by the identity endpoint
  tools/sync-device.sh       vendor a package into an OS repo, and check it
  tools/gen-raze.py          render the Raze config from its manifest; --check lints
  <model>/
    manifest.json            read by Atlas (and people)
    compat.json              which OS images are known to work (informational)
    CHANGELOG.md
    fetch.lock               files fetched by pinned URL instead of kept in git
    eeprom/                  bootloader images (small, versioned)
    gaia/                    the Gaia v2 layer, OS-neutral
```

## The rule: never block the user

Nothing in a package may stop a user from doing what they want with their
device.

- Every behaviour is a default. Hostname, fan curve, port power, the USB
  gadget, the identity endpoint and mDNS can each be changed or turned off by
  the OS or the user without editing package files (see "Overrides").
- Checksums are advisory. Tools may verify `sha256` values in the manifest or
  `fetch.lock` and warn, but must let the user continue. Nothing requires a
  signature.
- `compat.json` is information, never a gate.

## Contract version

`contract: 1` in `manifest.json` and in the identity document. Within a
contract version, fields are only added; readers ignore what they do not know.
Removing or changing the meaning of a field needs contract 2.

Contract 1 is:

- `manifest.json` as described by `schema/manifest.schema.json`;
- the identity document (`schema/identity.schema.json`), served unauthenticated
  at `GET http://<device>:5899/.well-known/pd-device` (`application/json`) and
  written at boot to `/run/pd-device/identity.json`;
- the mDNS service `_pd-device._tcp`, port 5899, with TXT
  `contract=1 model=<model> rev=<id> serial=<hex> os=<ID> os_ver=<VERSION_ID> path=/.well-known/pd-device`;
- neutral paths on the device: `/usr/lib/pd-device/` (package scripts and
  defaults), `/etc/pd-device/` (OS and user overrides), `/run/pd-device/`
  (generated state).

## Hardware facts: the manifest is the source

`manifest.json` holds every hardware fact of the board, precise enough to
generate the package's config from (and, later, a hardware daemon's board
definition). Values are numbers and enums with units in the key names, never
prose:

| Section | Facts |
| --- | --- |
| `kernel` | source, branch, commit, version, defconfig, `page_size_kib`, `builtin`, the package fragment and `fragment_merge` (OS fragments first, the package's last, so the package wins) |
| `capabilities.leds` | `part`, `gpio`, `count`, `device`, `brightness`, overlay `name`/`dev_name`, `wire_format` (`grb24`: bits on the data line), `userspace` (`layout` `rgbw`, `bytes_per_led` 4, `ignored_channels`), `index` (`offset`, `direction`: logical LED i is driver slot (offset + direction * i) mod count) |
| `capabilities.fan` | `pwm` (`controller`, `channel`, `period_ns`, `polarity`), `cooling_levels`, `min_level` (the duty floor), `trips_c`, `thermal_zone`, `cooling_device_type`, `hwmon_name` |
| `capabilities.camera` | `part`, `csi` (port, receiver), `i2c` (Linux `bus` 10 from the kernel DT, `bus_dt_label`, `controller`, `address`), `chip_id`, kernel driver and compatible, tuning files |
| `capabilities.i2c` | `buses` (Linux `bus`, kind, `sda_gpio`/`scl_gpio`, overlay) and `devices` (`id`, `part`, `bus`, 7-bit `address` as a number) |
| `capabilities.watchdog` | `device`, `driver`, `runtime_sec`, `reboot_sec` |
| `capabilities.selftest` | where the self-test is, its report format and checks |

The USB gadget (`identity.usb_gadget`, `capabilities.gadget-net`) is a
reference only: its settings live in `usb-gadget.env`.

Each section has `verified`: per fact (a dotted path in the section) either
`{"by", "date", "method"}` or `"unverified"`. Atlas reads the sections
leniently (`atlas-devices`, `hardware.rs`); older manifests with `"0x18"`
address strings still read.

### Generated files and the lint

`devices/tools/gen-raze.py` (python3, standard library) renders, from the
manifest:

- the `gen-raze: i2c`, `leds` and `camera` regions of `raze-device.txt` (the
  `ws2812-pio` line passes `rgbw` only for a 32-bit `wire_format`);
- the `gen-raze: fan` region of `raze-fan-overlay.dts` (`cooling-levels` and
  `pwms`, polarity included);
- whole files with a "Generated" header: `usr/lib/pd-device/leds.env`
  (raze-leds defaults), `usr/lib/pd-device/hardware.env` (selftest facts),
  `usr/share/pd-device/raze/sensors.toml` and the systemd watchdog drop-in.

Edit the manifest and run the tool; never edit generated parts by hand.
`gen-raze.py --check` changes nothing: it prints a diff and exits 1 when a
file disagrees, when the manifest breaks a rule (levels, offsets, addresses,
missing `verified` notes, rgbw vs wire format) or when the kernel facts
disagree with `gaia/kernel.toml` and the fragment, which it only reads. The
output is committed (Gaia has no build-time script step);
`devices/raze/tests/manifest-lint.sh` runs the check in CI.

## Self-test

`/usr/lib/pd-device/selftest [--interactive] [--json]`, root only, one run at
a time. Each check is `ok`, `skip` or `fail` with a message and data:

| Check | What it does |
| --- | --- |
| `leds` | `/dev/leds0` takes a frame (redraws raze-leds' state, or only opens the device when another program owns the ring). `--interactive`: red, green, blue, a W-only frame a 24-bit ring must drop, and a walk round the ring by index with the offset applied; the operator answers on the terminal. |
| `fan` | steps the pwm-fan cooling device through its states with the thermal zone paused, reads the duty (and rpm with a tachometer) back against `cooling_levels`, then restores the state and the governor |
| `camera` | the sensor's I2C client is bound to its driver (which checked the chip id), and rp1-cfe video nodes, `media-ctl` or `cam -l` see it |
| `i2c` | the manifest's devices answer (`i2cdetect -r`, in the image; skipped without it unless a kernel driver owns the address) |
| `watchdog` | `/dev/watchdog0` exists and systemd's `RuntimeWatchdogUSec` is set |
| `gadget` | the configfs gadget is bound to a UDC and `usbbr0` is up with an address |

`--json` prints one object and writes it to `/run/pd-device/selftest.json`:

```json
{"version":1,"board_serial":"10000000a317bcbe","model":"raze","package_version":"1.0.7",
 "at":1791347363,"interactive":false,"ok":true,
 "checks":[{"id":"fan","status":"ok","message":"...","data":{"steps":[...]}}]}
```

With `--json` the exit status is 0 whenever a report was made; without it,
1 when a check failed. The fan and the LED ring are restored on exit and on
interrupt. All hardware access goes through `hw.sh` (`hw_leds_write_frame`,
`hw_fan_set_state`, `hw_fan_read`, `hw_i2c_probe`, `hw_camera_list`, ...),
so another backend can replace it (`PD_HW_BACKEND`) without changing the
checks or the JSON. The identity lists `"diagnostics": ["selftest"]`; Atlas
runs it over SSH after a flash or update and on request (docs/ota.md).
`devices/raze/tests/selftest.sh` tests it off-device with a fake sysfs.

## Consuming a package from an OS build

### Gaia import from the Atlas git source (preferred)

Declare Atlas as a pinned git source in the OS build (never inside the layer)
and import the layer's entry file. Requires Gaia >= 2.1.0.

```toml
gaia_version = ">=2.1.0"

imports = [
  "layers/base-os.toml",
  { source = "atlas", path = "devices/raze/gaia/device.toml", when = { target = "raze" } },
  # OS layers imported after the device layer override its defaults.
  "layers/os-on-raze.toml",
]

[[sources]]
id = "atlas"
kind = "git"
repo = "https://github.com/Prometheus-Dynamics/Atlas-Hardware-Manager.git"
rev = "<pinned commit>"
```

For local development against an Atlas checkout:
`gaia run build.toml --set sources.atlas.path=/path/to/Atlas-Hardware-Manager`.

Inside the layer, paths are written `@self/...` (the directory of the TOML file
that contains them), so the layer works wherever it is mounted. An OS refers to
package files as `@source:atlas/devices/raze/...`.

### Vendored copy (interim)

Until every OS build can import from the git source, copy the package into the
OS repo with `devices/tools/sync-device.sh` and import the vendored
`device.toml` by its relative path; `@self` works the same there.

```sh
# from the OS repo, with an Atlas checkout next to it
../Atlas-Hardware-Manager/devices/tools/sync-device.sh --commit <rev> raze vendor/devices/raze
# in CI (and before committing): fails if the copy was edited or does not match the lock
../Atlas-Hardware-Manager/devices/tools/sync-device.sh --check vendor/devices/raze
```

`sync-device.sh` writes `<dest>/.device-lock` with the Atlas commit and a
content hash, and stamps the commit into the package's `device-package.env` so
the identity document reports it. With a git-source import the commit is not
stamped yet (`device_package.commit` is `null`) until Gaia can pass a source's
revision into the image.

## What the OS still provides (Raze)

The Raze layer (`devices/raze/gaia/device.toml`) needs from the OS:

1. **systemd as init** (`BR2_INIT_SYSTEMD=y`) and **systemd-resolved** (on by
   default in Buildroot) for the mDNS advertisement. Both HeliOS and the
   PhotonVision image already have both.
2. **A boot partition assembly tree with id `boot`**, and `include
   raze-device.txt` at top level of its `config.txt`. The layer puts
   `raze-device.txt`, the device overlays and the stock overlays it loads into
   that tree. The OS keeps its own `config.txt`, `cmdline.txt`, kernel and DTB
   copying, partition layout and root filesystem.
3. **mDNS on Ethernet.** The package turns on the resolved mDNS responder
   (`/usr/lib/systemd/resolved.conf.d/60-pd-device-mdns.conf`) and mDNS on the
   USB link; mDNS must also be enabled per Ethernet link:
   - systemd-networkd (HeliOS): add `MulticastDNS=yes` to the `[Network]`
     section of the Ethernet `.network` file;
   - NetworkManager (PhotonVision): the package ships
     `/usr/lib/NetworkManager/conf.d/60-pd-device.conf` with
     `connection.mdns=2`; NetworkManager hands per-link mDNS to
     systemd-resolved as long as resolved is running (`[main]
     systemd-resolved=true`, the default). Do not also run avahi-daemon on
     port 5353.
     Also clear `BR2_SYSTEM_DHCP` (the CM5 defconfig sets it to `eth0`), or
     systemd-networkd runs a second DHCP client on the link NetworkManager
     manages.
4. **Its own Buildroot external tree, if it has one, combined with the
   device's.** Gaia takes one `external_tree` and the last import wins, so an
   OS with its own tree sets both, device tree first so the `@source:` token
   resolves:
   `external_tree = "@source:atlas/devices/raze/gaia/buildroot-external:gaia/assets/buildroot"`.
   Package overrides in a later tree's `packages/` win over the device's.
5. **Optional GPU userspace.** Import `devices/raze/gaia/gpu.toml` as well when
   the OS processes camera frames on the GPU (Mesa V3D/VC4, EGL, GLES, gbm), as
   PhotonVision's libcamera GL driver does. For Vulkan (Mesa's v3dv, the
   loader and vulkaninfo; wgpu needs it), also import
   `devices/raze/gaia/gpu-vulkan.toml` after `gpu.toml`.

The device units are installed in `/usr/lib/systemd/system` and enabled by
`/usr/lib/systemd/system-preset/70-pd-device.preset` when Buildroot runs
`systemctl preset-all` at image build:

| Unit | What it does |
| --- | --- |
| `pd-device-hostname.service` | default hostname `raze-{serial8}`, only over an unset or stock hostname |
| `pd-device-identity.service` | writes `/run/pd-device/identity.json` and `/run/systemd/dnssd/pd-device.dnssd` before resolved starts |
| `pd-device-http.socket` (+ `pd-device-http@.service`) | identity endpoint on TCP 5899 |
| `pd-device-usb-gadget.service` | USB gadget (ECM/RNDIS/ACM) with the board serial as its USB serial, gadget-only bridge `usbbr0` on a per-board /29 (see "USB gadget network") |
| `pd-device-usb-gadget-dhcp.service` | dnsmasq DHCP on `usbbr0` only, DNS off, no default route |
| `raze-leds-reprobe.service` | re-probes the WS2812 PIO driver if `/dev/leds0` is missing |

Fan, port power and LEDs need no service: they are device tree overlays in
`raze-device.txt`, plus `/usr/lib/udev/rules.d/60-raze-usb-power.rules`.

## USB gadget network

Each board puts its USB network on a subnet of its own, so several boards
plugged into one computer all work. The address derives from the gadget's
USB serial string (iSerialNumber), which is the board serial, so the host
can compute it from the USB descriptor without any network traffic. The
scheme, `serial-hash-v1`, is described in `manifest.json`
(`capabilities.gadget-net.addressing`); the device computes it in `lib.sh`
(`pd_gadget_subnet`) from the same parameters in `usb-gadget.env`, and Atlas
in `atlas-devices` (`GadgetAddressing`):

1. Normalize the USB serial with Atlas's board-serial rule: all hex, at
   least 8 digits, keep the last 8 in lowercase (`a317bcbee5226d57` gives
   `e5226d57`).
2. `h` = the first 4 bytes of `sha256` of those 8 ASCII characters, as a
   big-endian number.
3. Split `172.31.0.0/16` into /29 subnets and leave out the ones in
   `172.31.250.0/24` (the fixed address of older images): 8160 remain. Take
   the one at index `h mod 8160`, counting up from the lowest.
4. The board has the subnet's first address; its DHCP server offers the
   host the other five (a /29 rather than a /30, so a host that comes back
   with another MAC, such as RNDIS after ECM, still gets a lease while the
   old one runs out).

| USB serial | Board address |
| --- | --- |
| `a317bcbee5226d57` | 172.31.209.217/29 |
| `10000000abcdef01` | 172.31.0.113/29 |
| `cfc0641d` | 172.31.254.9/29 (index 8097, past the excluded /24) |

`devices/raze/tests/gadget-address.sh` and the `atlas-devices` unit tests
check the same vectors, so the two sides can't drift.

**Collisions.** Two boards share a subnet with probability 1/8160 (0.012 %).
Among k boards on one computer the chance that any two do is about
k(k-1)/16320: 0.12 % for 5 boards, 0.55 % for 10, 2.3 % for 20. Two boards
that collide behave like two older images: the host reaches only one of
them over the gadget link. Pin one with `USB_GADGET_ADDRESS` (it is then
found by mDNS), or tell them apart over Ethernet.

**Fallback.** A board whose USB serial gives no subnet (not hex, or shorter
than 8 digits) keeps `172.31.250.1/24`, as does a board with
`USB_GADGET_ADDRESS=172.31.250.1/24` pinned. Atlas probes that address only
while exactly one matching gadget is plugged in, after the board's own
address, so images from before per-board addressing still work one at a
time. The identity reports the address in use:
`"gadget": {"address": "172.31.209.217", "prefix": 29, "addressing": "serial-hash-v1"}`
(`pinned` or `fallback` otherwise).

**Atlas** lists every `1d6b:0104` USB device whose manufacturer and product
match a package, reads its USB serial, and probes
`http://<board address>:5899/.well-known/pd-device`. It accepts the answer
only when the identity's serial normalizes to the same 8 digits as the USB
serial, so a stale lease or a collision never shows the wrong board.

**On the host.** Each board is a separate USB network interface. With
NetworkManager every interface gets its own DHCP lease and a route to its
own /29, which is what the distinct subnets are for; no default route or DNS
is offered, so the host's other networks are unaffected. On Windows each
RNDIS adapter does the same. `172.31.0.0/16` is the last of Docker's default
bridge pools (`172.17`-`172.31`); a host with that many Docker networks, or
a VPN that routes `172.31.0.0/16`, shadows the gadget subnets. Changing
`base` means changing it in both `manifest.json` and `usb-gadget.env`.

## Overrides

| To change | Do this |
| --- | --- |
| Hostname | Set `/etc/hostname` (always wins), or set `PD_HOSTNAME_POLICY=never`, `PD_HOSTNAME_PATTERN` or `PD_HOSTNAME_STOCK` in `/etc/pd-device/hostname.env`. An OS whose default name should give way to `raze-{serial8}` adds it: `PD_HOSTNAME_STOCK="$PD_HOSTNAME_STOCK photonvision"`. |
| USB gadget | `/etc/pd-device/usb-gadget.env` (any key from `/usr/lib/pd-device/usb-gadget.env`; `USB_GADGET_ENABLED=0` turns it off, `USB_GADGET_NET=none` leaves networking to the OS, `USB_GADGET_ADDRESS=<address>/<prefix>` pins the address instead of the per-board one). Extra dnsmasq settings in `/etc/pd-device/usb-gadget-dnsmasq.d/*.conf`. |
| mDNS advertisement | `PD_MDNS=0` in `/etc/pd-device/identity.env`. |
| Identity endpoint | `disable pd-device-http.socket` in an OS preset, or mask it. |
| Update methods / manage URL | One id per line in `/etc/pd-device/update-methods.d/<file>` (added after `image-write`); the URL in `/etc/pd-device/manage-url` (an empty file means `null`). |
| Board revision | The revision id in `/etc/pd-device/rev`. |
| LED byte order, index offset and direction | `RAZE_LEDS_ORDER`, `RAZE_LEDS_OFFSET`, `RAZE_LEDS_DIRECTION` in `/etc/pd-device/raze-leds.env` (defaults from the generated `leds.env`). |
| Fan, port power, LEDs, camera | Copy the lines you want from `raze-device.txt` into your `config.txt` instead of including it, and change their parameters (`raze-fan`: `level0`..`level4`, `period_ns`, `polarity`; `raze-usb-power`: `usba=off`, `usbc=off`, `hog=off`). |
| Any unit | A preset file that sorts before `70-pd-device.preset`, a drop-in, or a mask. |
| Any Buildroot option or default in the layer | Set it in a Gaia layer imported after the device layer. |
