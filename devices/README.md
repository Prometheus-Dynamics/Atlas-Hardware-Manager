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
    lemnos-board.schema.json Lemnos's board definition (vendored from the pinned Lemnos)
  tools/sync-device.sh       vendor a package into an OS repo, and check it
  tools/gen-raze.py          render the Raze config from its manifest; --check lints
  tools/raze_lemnos.py       its lemnosd board.toml renderer and validator
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
  written at boot to `/run/board/identity.json`;
- the mDNS service `_pd-device._tcp`, port 5899, with TXT
  `contract=1 model=<model> rev=<id> serial=<hex> os=<ID> os_ver=<VERSION_ID> path=/.well-known/pd-device`;
- neutral paths on the device: `/usr/lib/board/` (package scripts and
  defaults), `/etc/board/` (OS and user overrides), `/run/board/`
  (generated state).

## Hardware facts: the manifest is the source

`manifest.json` holds every hardware fact of the board, precise enough to
generate the package's config and lemnosd's board definition from. Values are numbers and enums with units in the key names, never
prose:

| Section | Facts |
| --- | --- |
| `kernel` | source, branch, commit, version, defconfig, `page_size_kib`, `builtin`, the package fragment and `fragment_merge` (OS fragments first, the package's last, so the package wins) |
| `capabilities.leds` | `part`, `gpio`, `count`, `device`, `brightness`, overlay `name`/`dev_name`, `wire_format` (`grb24`: bits on the data line), `userspace` (`layout` `rgbw`, `bytes_per_led` 4, `ignored_channels`), `index` (`offset`, `direction`: logical LED i is driver slot (offset + direction * i) mod count) |
| `capabilities.fan` | `pwm` (`controller`, `channel`, `period_ns`, `polarity`), `cooling_levels`, `min_level` (the duty floor), `trips_c`, `thermal_zone`, `cooling_device_type`, `hwmon_name` |
| `capabilities.camera` | `part`, `csi` (port, receiver), `i2c` (Linux `bus` 10 from the kernel DT, `bus_dt_label`, `controller`, `address`), `chip_id`, kernel driver and compatible, tuning files |
| `capabilities.i2c` | `buses` (Linux `bus` on the 7.2.9 image as a hint, kind, `sda_gpio`/`scl_gpio`, overlay, and `select`: what finds the bus whatever probe order numbered it, in the keys of a Lemnos `i2c:` selector, `compatible`, `of` (device-tree path), `node` or `name`) and `devices` (`id`, `part`, `bus`, 7-bit `address` as a number, `chip_id` register, width, value and power-control bit, and `lemnosd`: the board.toml device that drives it) |
| `capabilities.*.lemnosd` | the board.toml device for the ring (`device`, the `look` defaults), the fan (`device`, `thermal_device`, `poll_ms`, `writers`) and the USB-A power line |
| `capabilities.hardware-service` | lemnosd: its board definition, socket, update status file, the backends, and the pinned Lemnos Gaia import (`gaia_import.rev`) |
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
- whole files with a "Generated" header: `usr/lib/board/leds.env`
  (raze-leds defaults), `usr/lib/board/hardware.env` (selftest facts),
  `usr/share/board/raze/sensors.toml`, the systemd watchdog drop-in and
  `etc/lemnos/board.toml`, lemnosd's board definition (see below).

Edit the manifest and run the tool; never edit generated parts by hand.
`gen-raze.py --check` changes nothing: it prints a diff and exits 1 when a
file disagrees, when the manifest breaks a rule (levels, offsets, addresses,
missing `verified` notes, rgbw vs wire format) or when the kernel facts
disagree with `gaia/kernel.toml` and the fragment, which it only reads. It
also checks the generated board.toml against Lemnos's JSON Schema
(`schema/lemnos-board.schema.json`, with a small built-in validator) and
against what Lemnos's driver registry accepts per driver, parses it back, and
runs `lemnos-ctl validate` on it when it finds a `lemnos-ctl`
(`--lemnos-ctl`, `$LEMNOS_CTL` or `PATH`; a host build is
`cargo build -p lemnosd --bin lemnos-ctl` in a Lemnos checkout). The Lemnos
pin must agree between the manifest, `gaia/lemnos.toml`, `gaia/device.toml`
and this file. The output is committed (Gaia has no build-time script step);
`devices/raze/tests/manifest-lint.sh` runs the check in CI.

## lemnosd: the hardware service

The package ships [Lemnos](https://github.com/Prometheus-Dynamics/Lemnos)'s
`lemnosd`, the one service that owns the board's LED ring, fan, sensors and
GPIO, and its client `lemnos-ctl`. HeliOS, PhotonVision, the package's own
scripts and anything else talk to it over `/run/lemnos/lemnosd.sock` (group
`lemnos`; a non-root client joins that group) instead of opening
`/dev/leds0`, `/dev/i2c-*` or the fan's sysfs files.

- **Gaia.** `gaia/lemnos.toml`, imported by `device.toml`, imports Lemnos's
  own layer `packaging/gaia/lemnosd.toml` from the `lemnos` source: it builds
  static aarch64 musl `lemnosd` and `lemnos-ctl` (in Docker), installs them in
  `/usr/bin` and stages `lemnosd.service`, the `lemnos` sysusers entry, a preset
  that enables the service and `/etc/default/lemnosd.env`. The package
  redeclares that env file (`gaia/assets/lemnos/lemnosd.env`) with
  `LEMNOSD_UPDATE_STATUS=/run/board/update.json`, so lemnosd shows the update
  writer's states and copy progress on the ring. The OS declares the `lemnos`
  source (see "Consuming a package").
- **Board definition.** `/etc/lemnos/board.toml`, generated from the manifest:
  the status ring (`ws2812` on `/dev/leds0`, 16 LEDs, `wire = "rgb"`, offset
  5, direction from the manifest, fade 250 ms ease-in-out, status effect
  breathe), the fan (`hwmon-fan` matched by hwmon name `pwmfan`, no
  `restore_mode`: pwm-fan goes back through its cooling device), the CPU
  thermal zone, the BMI088 (`bmi088`, accel 0x18 and gyro 0x68 on
  `pio-i2c:sda=8,scl=7`: lemnosd runs that bus on the RP1 PIO block at
  400 kHz, so no overlay claims GPIO8/GPIO7 and there is no `/dev/i2c` node for
  it), the BMM150 and the INA238 (on
  `i2c:of=/axi/pcie@1000120000/rp1/i2c@74000`, the RP1 DesignWare controller
  i2c1-pi5 enables) and the USB-A and USB-C power switches
  (`gpio-power-switch`, `pinctrl-rp1` lines 20 and 16). Comments in the file mark what is unverified on hardware. An OS
  replaces the file by staging its own `/etc/lemnos/board.toml` in a later
  layer (item `raze-lemnos-board`).
- **Access.** `/usr/lib/udev/rules.d/60-board-lemnosd.rules` gives the `lemnos`
  group write access to the fan's `pwm1`/`pwm1_enable`, the pwm-fan cooling
  device's `cur_state` and the thermal zones' `policy` (Lemnos's
  `packaging/README.md`), and the device nodes to the groups lemnosd runs
  with (`i2c-*` to `i2c`, `gpiochip*` to `gpio`, `leds*` to `video`), and
  `/dev/pio0` (the RP1 PIO block, for the IMU's bus) to the `lemnos` user
  alone, mode 0600.
- **The fan** stays the kernel governor's (`cooling_levels`, trips). A client
  may set a duty; lemnosd records the governor's state before the first write
  and, when it stops (or crashes: `ExecStopPost=+lemnos-ctl fan restore
  --all`), puts it back and makes the thermal zone re-evaluate. The self-test
  ends its fan steps the same way with `lemnos-ctl fan restore`.
- **USB port power.** `raze-device.txt` drives both enables (GPIO20 USB-A,
  GPIO16 USB-C) high from the firmware (`gpio=16,20=op,dh`), and lemnosd's
  `usb-a-power` and `usb-c-power` (`gpio-power-switch`, `default_on`) take
  the lines already on, so the ports never pass through off at boot. Orion
  callers and Atlas may switch them (`power.set`); every boot starts on
  (`persist` off: the root is read-only), and they stay as they are when
  lemnosd stops. The firmware's hold on RP1 lines until lemnosd is
  unverified on hardware. An OS without lemnosd loads
  `dtoverlay=raze-usb-power` instead (kernel hogs), never both.

### Hardware backends

The package's scripts reach the hardware through `/usr/lib/board/hw.sh`,
which has the parts every backend shares (I2C probes and chip ids, the
camera, the watchdog, the gadget) and loads one backend for the ring and the
fan:

| `BOARD_HW_BACKEND` | Backend |
| --- | --- |
| `lemnosd` (the default when `lemnos-ctl` is installed) | `hw-lemnosd.sh`: LED fills and frames are `lemnos-ctl led color`/`led frame` intents of the caller's client, fan duties `lemnos-ctl set fan duty`, readings `lemnos-ctl read fan`, the hand-back `lemnos-ctl fan restore`, sensors lemnosd's device status |
| `sysfs` (the default without `lemnos-ctl`) | `hw-sysfs.sh`: 4-byte frames written to `/dev/leds0`, the pwm-fan cooling device set directly with its thermal zone paused, I2C probes only |

Set it in the environment or in `/etc/board/hw.env`. A path instead names a
replacement backend file that defines the same `hw_*` functions.

`raze-leds` keeps its commands (`on`, `off`, `set`, `dim`, `color`, `status`,
`blink`, `locate`, `pixel`, `refresh`, `show`) and its state file; with
lemnosd it is a thin wrapper over `lemnos-ctl --client raze-leds led ...`
(`status` uses lemnosd's status look, `blink` its blink effect, `locate` its
locate effect, `off` drops raze-leds' intents so other clients' looks show).
`board-locate.service` runs `raze-leds locate 10`.

## Self-test

`/usr/lib/board/selftest [--interactive] [--json]`, root only, one run at
a time. Each check is `ok`, `skip` or `fail` with a message and data:

| Check | What it does |
| --- | --- |
| `leds` | lemnosd has the ring available (lemnosd backend), or `/dev/leds0` takes a frame (sysfs: redraws raze-leds' state, or only opens the device when another program owns the ring). `--interactive`: red, green, blue, a W-only frame a 24-bit ring must drop, and a walk round the ring by index with the offset applied (as `board-selftest` frames at priority 100 through lemnosd); the operator answers on the terminal. |
| `fan` | sets the duty of each cooling state in turn (lemnosd: `set fan duty`; sysfs: the cooling device, with the thermal zone paused), reads the duty (and rpm with a tachometer) back against `cooling_levels`, then hands the fan back to the governor (lemnosd's hand-back, or the state and governor restored) |
| `camera` | the sensor's I2C client is bound to its driver (which checked the chip id), and rp1-cfe video nodes, `media-ctl` or `cam -l` see it |
| `i2c` | the manifest's devices answer on their bus, found by its `select` (else its number): a device lemnosd has available counts as checked (its driver read the chip id); otherwise `i2cdetect -r` probes it and `i2cget` reads its chip id against the manifest. The BMM150 boots suspended: without lemnosd its power bit is set for the read and put back; with lemnosd it is left alone. |
| `watchdog` | `/dev/watchdog0` exists and systemd's `RuntimeWatchdogUSec` is set |
| `gadget` | the configfs gadget is bound to a UDC and `usbbr0` is up with an address |

`--json` prints one object and writes it to `/run/board/selftest.json`:

```json
{"version":1,"board_serial":"10000000a317bcbe","model":"raze","package_version":"1.0.7",
 "at":1791347363,"interactive":false,"ok":true,
 "checks":[{"id":"fan","status":"ok","message":"...","data":{"steps":[...]}}]}
```

The report also names the backend (`"backend": "lemnosd"`); checks add
their own data (`backend`, `status`, and per I2C device `bus_found_by`,
`chip_id`, `lemnosd`). With `--json` the exit status is 0 whenever a report was made; without it,
1 when a check failed. The fan and the LED ring are restored on exit and on
interrupt. All hardware access goes through `hw.sh` and its backend
(`hw_leds_write_frame`, `hw_fan_set_state`, `hw_fan_read`, `hw_i2c_probe`,
`hw_camera_list`, ...; see "Hardware backends"), so the checks and the JSON
are the same with lemnosd or without. The identity lists `"diagnostics": ["selftest"]`; Atlas
runs it over SSH after a flash or update and on request (docs/ota.md).
`devices/raze/tests/selftest.sh` tests it off-device with a fake sysfs.

## Consuming a package from an OS build

### Gaia import from the Atlas git source (preferred)

Declare Atlas as a pinned git source in the OS build (never inside the layer)
and import the layer's entry file. The layer imports Lemnos's lemnosd layer
from a second source, `lemnos`; Gaia lets only local files declare import
sources, so the OS build declares that one too, at the commit the package is
tested with (`capabilities.hardware-service.gaia_import.rev`). Requires Gaia >= 2.0.0
with Rust build groups.

```toml
gaia_version = ">=2.0.0"

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

[[sources]]
id = "lemnos"
kind = "git"
repo = "https://github.com/Prometheus-Dynamics/Lemnos.git"
rev = "c43d207b06ea4a7a42dabd51065746154207125a"
```

For local development against an Atlas checkout:
`gaia run build.toml --set sources.atlas.path=/path/to/Atlas-Hardware-Manager`
(and `--set sources.lemnos.path=/path/to/Lemnos`).

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

A vendored copy still needs the `atlas` source declared in the OS build:
`board-agent` is a Gaia artifact (`gaia/board-agent.toml`) built from
`crates/board-agent` in that source.

`sync-device.sh` writes `<dest>/.device-lock` with the Atlas commit and a
content hash, and stamps the commit into the package's `board-package.env` so
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
   (`/usr/lib/systemd/resolved.conf.d/60-board-mdns.conf`) and mDNS on the
   USB link; mDNS must also be enabled per Ethernet link:
   - systemd-networkd (HeliOS): add `MulticastDNS=yes` to the `[Network]`
     section of the Ethernet `.network` file;
   - NetworkManager (PhotonVision): the package ships
     `/usr/lib/NetworkManager/conf.d/60-board.conf` with
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
6. **What lemnosd needs** (Lemnos `packaging/README.md`): Docker on the build
   host (Lemnos's layer builds the binaries in a small Rust image) and its
   own clients (HeliOS, PhotonVision) in the `lemnos` group. The `lemnos`
   user comes with the package: `gaia/lemnos.toml` hands Lemnos's
   `packaging/buildroot/lemnos-users.table` to the external tree, which adds
   it next to the OS's own `BR2_ROOTFS_USERS_TABLES` (it doesn't replace
   them), so the read-only root has the user without systemd-sysusers. An OS
   drops any copy of that table it keeps itself.

The device units are installed in `/usr/lib/systemd/system` and enabled by
`/usr/lib/systemd/system-preset/70-board.preset` when Buildroot runs
`systemctl preset-all` at image build:

| Unit | What it does |
| --- | --- |
| `board-hostname.service` | default hostname `raze-{serial8}`, only over an unset or stock hostname |
| `board-identity.service` | writes `/run/board/identity.json` and `/run/systemd/dnssd/board.dnssd` before resolved starts |
| `board-http.socket` (+ `board-http@.service`) | identity endpoint on TCP 5899 |
| `board-usb-gadget.service` | USB gadget (ECM/RNDIS/ACM) with the board serial as its USB serial, gadget-only bridge `usbbr0` on a per-board /29 (see "USB gadget network") |
| `board-usb-gadget-dhcp.service` | dnsmasq DHCP on `usbbr0` only, DNS off, no default route |
| `raze-leds-reprobe.service` | re-probes the WS2812 PIO driver if `/dev/leds0` is missing |
| `lemnosd.service` (Lemnos's unit and `80-lemnosd.preset`) | the hardware service: LED ring, fan, sensors, GPIO; `/etc/lemnos/board.toml` |
| `board-locate.service` (+ `.path`) | the identity endpoint's locate action: `raze-leds locate 10` |
| `board-agent.service` | Orion device agent (a Gaia artifact, `gaia/board-agent.toml`: a static aarch64 musl binary built from `crates/board-agent`): claims `update`, `update.cancel`, `update.rollback`, `reboot` and `locate` on orion-node's local IPC and runs them with `/usr/lib/board/update` (docs/ota.md). Waits when the OS has no orion-node |

**orion-node local auth.** The package ships
`orion-node.service.d/50-board-agent.conf`, setting
`ORION_NODE_LOCAL_AUTH_ALLOW=root` so board-agent (root) may use the node's
local IPC. It is an allow-list on top of the node's local auth mode, and
gives root nothing it lacks on the board. systemd can't merge it with
another value: an OS that sets its own `ORION_NODE_LOCAL_AUTH_ALLOW` replaces
it and must list `root` too (e.g. `root,photonvision`).

Fan, port power and LEDs work without a service: they are device tree
overlays in `raze-device.txt`, plus `/usr/lib/udev/rules.d/60-raze-usb-power.rules`;
lemnosd adds control over them on top.

## USB gadget network

Each board puts its USB network on a subnet of its own, so several boards
plugged into one computer all work. The address derives from the gadget's
USB serial string (iSerialNumber), which is the board serial, so the host
can compute it from the USB descriptor without any network traffic. The
scheme, `serial-hash-v1`, is described in `manifest.json`
(`capabilities.gadget-net.addressing`); the device computes it in `lib.sh`
(`board_gadget_subnet`) from the same parameters in `usb-gadget.env`, and Atlas
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
| Hostname | Set `/etc/hostname` (always wins), or set `BOARD_HOSTNAME_POLICY=never`, `BOARD_HOSTNAME_PATTERN` or `BOARD_HOSTNAME_STOCK` in `/etc/board/hostname.env`. An OS whose default name should give way to `raze-{serial8}` adds it: `BOARD_HOSTNAME_STOCK="$BOARD_HOSTNAME_STOCK photonvision"`. |
| USB gadget | `/etc/board/usb-gadget.env` (any key from `/usr/lib/board/usb-gadget.env`; `USB_GADGET_ENABLED=0` turns it off, `USB_GADGET_NET=none` leaves networking to the OS, `USB_GADGET_ADDRESS=<address>/<prefix>` pins the address instead of the per-board one). Extra dnsmasq settings in `/etc/board/usb-gadget-dnsmasq.d/*.conf`. |
| mDNS advertisement | `BOARD_MDNS=0` in `/etc/board/identity.env`. |
| Identity endpoint | `disable board-http.socket` in an OS preset, or mask it. |
| Update methods / manage URL | One id per line in `/etc/board/update-methods.d/<file>` (added after `image-write`); the URL in `/etc/board/manage-url` (an empty file means `null`). |
| Board revision | The revision id in `/etc/board/rev`. |
| LED byte order, index offset and direction | With lemnosd: `offset`/`direction` in `/etc/lemnos/board.toml`. Sysfs backend: `RAZE_LEDS_ORDER`, `RAZE_LEDS_OFFSET`, `RAZE_LEDS_DIRECTION` in `/etc/board/raze-leds.env` (defaults from the generated `leds.env`). |
| Hardware backend | `BOARD_HW_BACKEND=sysfs` (or `lemnosd`) in `/etc/board/hw.env`. |
| lemnosd | Its board definition: stage your own `/etc/lemnos/board.toml` (item `raze-lemnos-board`); its settings: redeclare `lemnosd-env` (`/etc/default/lemnosd.env`) in a later layer; the unit: drop-ins in `/etc/systemd/system/lemnosd.service.d/`, or `disable lemnosd.service` in a preset. |
| Fan, port power, LEDs, camera | Copy the lines you want from `raze-device.txt` into your `config.txt` instead of including it, and change their parameters (`raze-fan`: `level0`..`level4`, `period_ns`, `polarity`; port power: the `gpio=16,20=op,dh` line, or `dtoverlay=raze-usb-power` with `usba=off`, `usbc=off` when lemnosd doesn't run). |
| Any unit | A preset file that sorts before `70-board.preset`, a drop-in, or a mask. |
| Any Buildroot option or default in the layer | Set it in a Gaia layer imported after the device layer. |
