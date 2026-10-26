# Devices as data: discovery, info and install methods (design)

Status: design, not built. Input for SystemCore's values: the capture from
a real unit (`scripts/systemcore-capture.sh`).

Today a device family is mostly code: `atlas-driver-board` reads the board
identity contract (`/.well-known/pd-device`) that a device package's boards
serve, the robot driver recognises the roboRIO by NI's page, and how a
board takes software (A/B image, slot, layer, fresh flash) is a set of
capabilities wired by hand. A device package (`devices/<model>/`) already
holds the manifest (`manifest.json`), compat list and hardware list.

The direction: everything that varies per family lives in its device
package as data, and Atlas has one engine for each concern, with as little
per-family code as possible:

1. **Recognise** a device: where to look, and how to tell it is this model.
2. **Read** what it is: identity, versions, team, packages, from declared
   read-only sources.
3. **Install** an artifact on it by one of the methods it declares; each
   method is a small adapter in Atlas.

SystemCore becomes `devices/systemcore/`. Raze keeps working unchanged
while it moves over.

## 1. Manifest additions (schema `urn:board:manifest:1`, all optional)

Old manifests stay valid: every new section is optional, and a package
without `discovery` is found as today (the identity contract over mDNS and
the USB gadget).

```jsonc
{
  "model": "systemcore",
  "display_name": "SystemCore",
  "key": "team",                      // what keys the device: "serial" (Raze), "team", "mac"

  "discovery": {
    // Where to look, besides what Atlas already browses.
    "addresses": [
      { "template": "10.{TE}.{AM}.2", "link": "robot-net" },
      { "ip": "172.27.0.1", "link": "usb-network", "host_os": ["linux", "macos"] },
      { "ip": "172.26.0.1", "link": "usb-network", "host_os": ["windows"] },
      { "ip": "172.30.0.1", "link": "wifi-ap" }
    ],
    "hostnames": ["robot.local"],
    "mdns": [{ "service": "_SystemCore._tcp" }],   // from the capture; Raze: _pd-device._tcp with txt model=raze
    "usb": []                                      // vid/pid/manufacturer, as identity.usb_gadget is today
  },

  // How to tell it is this model once something answers. All of a
  // fingerprint's checks must hold; the first fingerprint that holds wins.
  // GETs only.
  "fingerprints": [
    {
      "id": "web-ui",
      "get": { "port": 80, "path": "/" },
      "expect": {
        "status": 200,
        "header": { "server": "<regex>" },
        "body_contains": ["<title>…</title>"]    // from the capture
      }
    }
  ],

  // What to read, once recognised: each value from one source. GET only,
  // and only to the address it was found at.
  "info": {
    "version":  { "http_json": { "port": 80, "path": "<from capture>", "pick": "$.release" } },
    "os_commit": { "http_json": { "port": 80, "path": "<from capture>", "pick": "$.commit" } },
    "team":     { "address": "10.{TE}.{AM}.2" },          // or an http_json source
    "hostname": { "mdns": "hostname" },
    "packages": { "http_json": { "port": 80, "path": "<from capture>",
                  "list": "$.packages[*]", "fields": { "name": "$.name", "version": "$.version" } } },
    "services": { "tcp_open": [22, 80, 5810, 1735, 1740] },
    "manage_url": { "template": "http://{address}/configure" }
  },

  // How it takes software. Each entry names an adapter in Atlas and
  // configures it; the order is the preference.
  "install": [
    { "method": "package", "manager": "opkg", "artifacts": ["ipk"],
      "via": [{ "web_upload": { "port": 80, "path": "<from capture>" } }] }
  ]
}
```

Raze's `install` would read:

```jsonc
"install": [
  { "method": "slot",     "artifacts": ["slot-bundle", "slots-image"], "when": "update_methods has slots-tryboot",
    "via": ["orion", "console-pull", "ssh"] },
  { "method": "ab-image", "artifacts": ["ab-image"], "when": "update_methods has ab-tryboot",
    "via": ["orion", "ssh"] },
  { "method": "layer",    "artifacts": ["layer"], "when": "slot is layered", "via": ["ssh", "console"] },
  { "method": "fresh",    "artifacts": ["disk-image"], "via": ["usb-boot"] }
]
```

### Value sources (closed set, read-only)

| Source | What it does |
|---|---|
| `contract` | The board identity contract (`/.well-known/pd-device`): Raze and every package board. Fills identity, versions, endpoints at once. |
| `http_json` | `GET http://<found address>:<port><path>`, then `pick` with a small JSONPath subset (`$.a.b`, `$.list[*]`). |
| `http_text` | The same, with a regex and its first group. |
| `address` | A value from the address it was found at (`10.{TE}.{AM}.2` gives the team). |
| `mdns` | A TXT key or the advertised hostname. |
| `nt4` | One NetworkTables topic, read once. |
| `tcp_open` | Which of the listed ports accept a connection. |
| `template` | A string built from other values (`{address}`, `{team}`). |

The data never holds a command to run, a POST for a read, or a host other
than the device's own address. Writes exist only inside install adapters,
which are code.

## 2. One install model

The user picks an artifact for a device. Atlas works out every method the
device can take it by, picks the first in the package's order (the others
stay offered), plans it, and runs it with the same progress, cancel and
history as every install.

```rust
/// What an artifact is, by its content (never only its name).
pub enum ArtifactKind { SlotBundle, SlotsImage, AbImage, DiskImage, Layer, Ipk, Deb, Firmware }

pub struct InstallRequest {
    pub artifact: Artifact,          // path, name, sha256, size (as today)
    pub kind: ArtifactKind,
    pub options: InstallOptions,     // boot_after, pin, keep_settings, …: a method ignores what it doesn't use
}

pub struct InstallPlan {
    pub method: &'static str,        // "slot", "ab-image", "layer", "package", "fresh"
    pub steps: Vec<UpdateStep>,      // as today: Preflight, Transfer, Apply, Reboot, Confirm
    pub erases: bool,                // a fresh install: the UI asks first
    pub restarts: bool,
    pub summary: String,
}

#[async_trait]
pub trait InstallMethod: Send + Sync {
    fn id(&self) -> &'static str;
    /// The artifact kinds this method installs.
    fn kinds(&self) -> &'static [ArtifactKind];
    /// Can it run on this device now (its state, not just its model)? Err says why not.
    fn plan(&self, device: &Identity, request: &InstallRequest) -> Result<InstallPlan, DriverError>;
    async fn run(&self, device: &Identity, request: &InstallRequest,
                 progress: &ProgressSink, cancel: &CancellationToken) -> Result<UpdateOutcome, DriverError>;
}

/// How a method reaches the device: the "via" in the manifest. A method
/// asks for what it needs (run a command, move bytes) and takes the first
/// way that works, falling back only before anything changed.
#[async_trait]
pub trait DeviceChannel: Send + Sync {
    fn id(&self) -> &'static str;    // "orion", "console", "ssh", "web_upload", "usb-boot"
    async fn run(&self, script: &str) -> Result<Vec<u8>, DriverError>;          // where it can
    async fn deliver(&self, artifact: &Artifact, into: Delivery) -> Result<(), DriverError>;
}
pub enum Delivery { Stdin(String) /* command reading it */, Pull /* device fetches from BundleHost */, Upload(HttpTarget) }
```

The adapters, mostly moving code that exists:

| Method | Kinds | Today's code |
|---|---|---|
| `slot` | `SlotBundle`, `SlotsImage` (made into a bundle) | `BoardSlots::install` / `OrionSlots::install`, `slot_bundle_from_image` |
| `ab-image` | `AbImage` | `SshUpdate` / `OrionUpdate` (`UpdateCapability`) |
| `layer` | `Layer` | `push_layer` |
| `fresh` | `DiskImage`, `SlotsImage` | the rpi driver's USB-boot flash |
| `package` (opkg) | `Ipk` | new: the SystemCore Package Manager's upload, from the capture |
| `deb` (later) | `Deb` | new: over Orion |

The channels are also existing code: Orion (`BoardNodes`/`RemoteTransport`
plus `BundleHost` for pulls), the USB console (`Console`), SSH
(`SshUpdate::run`/`upload_to`), USB boot (the flasher). `web_upload` is new.
The fallback rules that are scattered today (Orion first, then the console
pull, then SSH with the key added over the console) become one rule in the
engine: try `via` in order, and move on only on an error that changed
nothing.

`ArtifactKind` is sniffed: a `.slot.tar` has `slot.json`; a slots image has
`slots/` on its FAT; an A/B image has the A/B partition layout; an ipk is an
`ar` archive with `control.tar.*`. A file that fits no method the device
takes is refused with what it is and what the device takes.

## 3. Recognition engine

A generic `PackageDriver` per device package, from its manifest:

1. Candidates: its `discovery` addresses and hostnames on this computer's
   links, its mDNS services, its USB ids, plus what other drivers already
   found (the robot-net probe of `.2`).
2. Fingerprint: the first `fingerprints` entry whose checks all hold.
3. Identity: `info` values; the key from `key` (`systemcore:6390`).
4. Capabilities: info as read-only status; install methods from `install`.

The board identity contract is one more source (`contract`), so Raze's
driver is the same engine with its sources set to `contract`. The roboRIO
moves into `devices/roborio/` the same way (fingerprint: NI's page and
`Server: Embedthis`).

## Order of work

1. `InstallMethod` + `DeviceChannel` in `atlas-driver`, with today's slot,
   A/B, layer and fresh paths behind them (no behaviour change), and the
   UI's Software tab on one "Install" that shows the chosen method and the
   others.
2. Manifest sections and the recognition engine, with the roboRIO moved to
   data first (it has real fixtures), then Raze's discovery.
3. `devices/systemcore/` from the capture: fingerprints, info sources, the
   `package` method over `web_upload`.
4. Later: HeliOS as an `.ipk` on SystemCore, `deb` over Orion.
