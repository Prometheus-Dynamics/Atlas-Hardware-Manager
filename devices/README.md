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
   PhotonVision's libcamera GL driver does. The device external tree then
   keeps rpi-userland out of staging so Mesa's EGL/GLES headers stay intact.

The device units are installed in `/usr/lib/systemd/system` and enabled by
`/usr/lib/systemd/system-preset/70-pd-device.preset` when Buildroot runs
`systemctl preset-all` at image build:

| Unit | What it does |
| --- | --- |
| `pd-device-hostname.service` | default hostname `raze-{serial8}`, only over an unset or stock hostname |
| `pd-device-identity.service` | writes `/run/pd-device/identity.json` and `/run/systemd/dnssd/pd-device.dnssd` before resolved starts |
| `pd-device-http.socket` (+ `pd-device-http@.service`) | identity endpoint on TCP 5899 |
| `pd-device-usb-gadget.service` | USB gadget (ECM/RNDIS/ACM), gadget-only bridge `usbbr0` at 172.31.250.1/24 |
| `pd-device-usb-gadget-dhcp.service` | dnsmasq DHCP on `usbbr0` only, DNS off, no default route |
| `raze-leds-reprobe.service` | re-probes the WS2812 PIO driver if `/dev/leds0` is missing |

Fan, port power and LEDs need no service: they are device tree overlays in
`raze-device.txt`, plus `/usr/lib/udev/rules.d/60-raze-usb-power.rules`.

## Overrides

| To change | Do this |
| --- | --- |
| Hostname | Set `/etc/hostname` (always wins), or set `PD_HOSTNAME_POLICY=never`, `PD_HOSTNAME_PATTERN` or `PD_HOSTNAME_STOCK` in `/etc/pd-device/hostname.env`. An OS whose default name should give way to `raze-{serial8}` adds it: `PD_HOSTNAME_STOCK="$PD_HOSTNAME_STOCK photonvision"`. |
| USB gadget | `/etc/pd-device/usb-gadget.env` (any key from `/usr/lib/pd-device/usb-gadget.env`; `USB_GADGET_ENABLED=0` turns it off, `USB_GADGET_NET=none` leaves networking to the OS). Extra dnsmasq settings in `/etc/pd-device/usb-gadget-dnsmasq.d/*.conf`. |
| mDNS advertisement | `PD_MDNS=0` in `/etc/pd-device/identity.env`. |
| Identity endpoint | `disable pd-device-http.socket` in an OS preset, or mask it. |
| Update methods / manage URL | One id per line in `/etc/pd-device/update-methods.d/<file>` (added after `image-write`); the URL in `/etc/pd-device/manage-url` (an empty file means `null`). |
| Board revision | The revision id in `/etc/pd-device/rev`. |
| Fan, port power, LEDs, camera | Copy the lines you want from `raze-device.txt` into your `config.txt` instead of including it, and change their parameters (`raze-fan`: `level0`..`level4`, `period_ns`, `polarity`; `raze-usb-power`: `usba=off`, `usbc=off`, `hog=off`). |
| Any unit | A preset file that sorts before `70-pd-device.preset`, a drop-in, or a mask. |
| Any Buildroot option or default in the layer | Set it in a Gaia layer imported after the device layer. |
