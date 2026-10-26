# The robot network: boards behind a USB route, the robot controller and the radio

Status: the read-only parts are implemented and were tried on team 6390's
robot (October 2026). They need no credentials:

- discovery of boards, the robot controller and the radio behind a board's
  USB route;
- the robot controller (`roborio`) and radio (`frc-radio`) families;
- `atlas robot …`;
- the connection map's real topology.

Anything that needs a login is designed below but not built. That means the
roboRIO's log files, its system facts and SystemCore.

## What is on a robot network

An FRC robot's network is `10.TE.AM.0/24` for team TEAM, so team 6390 uses
`10.63.90.0/24`. The robot controller is `.2`. The radio is `.1` as the
robot radio, or `.4` as a VH-109 used as the team's access point. The team
assigns the rest (cameras, coprocessors, the driver station). The code for
this is `atlas_driver::robot_net`: `team_of`, `team_net`, `Net` and
`robot_nets_in`.

### Found on team 6390's robot (read-only GETs and port checks, no logins)

**10.63.90.2: roboRIO, FRC 2026 image** (NI Linux RT; DNS name
`roboRIO-6390-FRC.lan` on the radio)

| Port | What | Without credentials |
|---|---|---|
| 80, 443 | NI web configuration (`Server: Embedthis-http`, title `HTML Device Configuration`, built 2026-01-05) | The page itself. Its system facts (serial, image, firmware) come from `POST /nisysapi/server` (`SearchForItemsAndProperties`), which the page sends on load. A GET is a 404. Atlas sends no POSTs, so it doesn't read them. |
| 80 `/files/` | WebDAV over the filesystem | **401**, Digest realm `NIAuth`: needs a login |
| 1250 | CTRE Phoenix diagnostics server, part of the robot program when it uses Phoenix | `GET /?action=getversion` returns `{"System":"roboRIO","ReleaseInfo":"FRC2026","Version":"2026.3.0 …"}`. `GET /?action=getdevices` lists every CTRE device on every bus: 17 here (Talon FX, CANcoder, Pigeon 2), with bus (CANivore `can` or the RIO's own), ID, name, model, firmware and serial. |
| 1741 | The robot program's console (RioLog), streamed from the moment you connect, with no history | Prints and error/warning reports. The framing is documented in `crates/atlas-driver-robot/src/console.rs` and checked against a capture. |
| 5810, 1735 | NetworkTables 4 and 3, served by the robot program | NT4 over WebSocket. The desktop app's NetworkTables page already reads it (a team number means `10.TE.AM.2`). |
| 1740 | Driver Station TCP | Not touched: it is the DS's connection. |
| 22, 21 | SSH (OpenSSH 8.3) and FTP (vsftpd, "LabVIEW Real-Time Linux FTP Server") | Both are logins. They were not used. |
| 3580 | NI Service Locator | Lists services (`NI-RIO`, `Web Servers`). Not used. |

**10.63.90.4: Vivid-Hosting VH-109 radio, firmware `2.0.1-02062026`**,
mode `TEAM_ACCESS_POINT`, channel 53, DNS name `radio.lan`

| Port | What | Without credentials |
|---|---|---|
| 80 `/` | 302 to `/configuration`, the configuration page | The page |
| 80 `/status` | JSON | Mode, channel, team, status and version. For each band (2.4 and 6 GHz): SSID, enabled, linked, peer MAC, signal, noise, SNR, rx/tx rates and packets, bandwidth used, quality. It also has the 6 GHz bypass, QoS and SystemCore flags. **It also carries each band's WPA key, hashed and salted. Atlas drops these fields and never shows or stores them.** |
| 80 `POST /configuration`, `POST /firmware` | Change settings, upload firmware | Never sent |
| 22 | SSH (dropbear) | A login |
| 53 | DNS for the robot network (`roboRIO-6390-FRC.lan` resolves to .2) | Lookups only |

Neither device reports a serial number without a login, so Atlas keys both
by team number: `roborio:6390` and `frc-radio:6390`. A robot network has
one of each. A practice robot with the same team number would share the key
when it isn't on at the same time; give it its own label.

## Reaching the robot network

There are two paths:

1. **Directly.** This computer is on the radio's Wi-Fi or Ethernet, with a
   `10.TE.AM.x/24` address. mDNS works there, and the board driver also
   sweeps the network for boards (`sweep.rs`). The robot drivers probe `.1`,
   `.2` and `.4` (`own_robot_nets`). These devices are on the network link
   (`ethernet`).
2. **Through a board's USB route** (a Raze's `usb-route`). The board NATs
   its USB link onto its other networks and gives this computer a DHCP
   route to each (options 121 and 249). The identity says so in `usb_route`
   {enabled, active, subnets, addresses}. mDNS doesn't cross that router, so
   until now nothing behind the route showed up: a camera behind it was
   "offline" while it answered.

### Behind a USB route: the board's children

The board driver's `children()` lists what is behind the board this
computer reaches **at its USB gadget address**. The core gives everything
it finds `LinkKind::Gateway { via: <that board> }`, so the inventory
records the real path. The link id is `robot-net`.

- **Which networks.** Each routed subnet of `/24` or narrower is used as it
  is. A wider subnet is narrowed with `robot_nets_in`. A Raze with a static
  `10.63.90.12/8` routes all of `10.0.0.0/8`, and nobody sweeps 65 536
  /24s. Atlas keeps the /24s it can name inside the route:
  - the board's own addresses on it: `usb_route.addresses`, new in this
    change (device package; `usb-route status`);
  - the teams Atlas knows (`RobotTeams`): the user's teams (desktop Settings
    > Robot teams, or the CLI's `--team N` / `ATLAS_TEAM`), plus every team
    whose network a device was seen on. The core notes these from each
    sighting's addresses and from the saved inventory at start-up, so once
    a camera has been seen on `10.63.90.x`, team 6390 is known after a
    restart.
- **Boards.** Atlas sweeps `:5899/.well-known/pd-device` on each of those
  /24s, with one sweeper per network: 48 addresses at once, 600 ms each,
  about 4 s for a /24. Each network's first sweep is awaited, so a one-off
  `atlas ls` sees what is there. After that, a background refresh runs at
  most once a minute. The routing board finds itself at its robot address,
  and Atlas drops that answer by serial. Each board found goes to its
  model's driver.
- **Robot controller and radio.** Atlas makes a TCP connect to `:80` on
  `.2`, `.1` and `.4` (`RobotProbe`, cached for 20 s, at most 700 ms), and
  the robot drivers identify whatever answers.
- **Actions, updates, streams and SSH** use the device's own address
  (`10.63.90.11`). The host's route carries them; no proxy is involved.

Not done: a board that routes but was reached only over Ethernet has no
children. This computer is then on that network itself, and path 1
applies. The sweep is IPv4 only.

## The new families

The crate is `crates/atlas-driver-robot`. Both drivers are read-only and
registered in the CLI and the desktop app.

| | `roborio` (ControllerDriver) | `frc-radio` (RadioDriver) |
|---|---|---|
| Identified by | `Server: Embedthis…` and NI's page title on `/` | `GET /status` JSON with a version or mode |
| Model, versions | `roboRIO` (Phoenix's `System`); `phoenix` 2026.3.0, `season` 2026 | `VH-109`; `firmware` 2.0.1-02062026 |
| Name | `roboRIO-6390-FRC` | `radio-6390` |
| Attributes | `team`, `hostname`, `manage_url` (`http://10.63.90.2/`), `robot_code` (running when NT4 or Phoenix answers), `services` (ssh, ftp, nt4, nt3, ds, console, phoenix), `can.devices`, `can.buses` | `team`, `manage_url`, `radio.mode`, `radio.status`, `radio.channel`, `radio.2.4.ssid`, `radio.2.4.link` ("linked, -65 dBm, excellent"), `radio.6.*`, `radio.6ghz_bypassed`, `radio.qos`, `radio.systemcore` |
| Capabilities | Logs: the console, through a background tap that keeps 2000 lines while someone reads it (it disconnects after 2 min idle) | Telemetry: each linked band's signal, noise, SNR, rx/tx rate and bandwidth used |

In the desktop app, both appear as device cards:
- the radio has the router icon and the controller the robot icon;
- "Open web UI" comes from `manage_url`;
- the attributes appear in Settings > Identity;
- the console appears in Logs, and the radio's readings in charts.

### The connection map

`layout.ts` `parentOf()` draws:

- a device behind a board's route hangs off that board ("Robot net");
- when the radio was found the same way, the other devices hang off the
  radio instead ("Network"), and the radio hangs off the board;
- on a robot network this computer reaches directly, the radio's /24 hangs
  off the radio, and the radio hangs off this computer.

`layout.test.ts` covers the bench as found:

```
this computer --USB--> right --Robot net--> radio-6390 --Network--> roboRIO-6390-FRC, left, front
```

## CLI

```
atlas ls                                  # the robot's devices behind a USB route, link "gateway"
atlas --team 6390 ls                      # the first time, when the route is wider than a /24
atlas robot info roborio:6390             # or frc-radio:6390, a name, or an IP
atlas robot can roborio:6390              # CTRE devices on every bus
atlas robot console roborio:6390          # RioLog, until Ctrl-C (--seconds N)
atlas robot logs roborio:6390 --out DIR   # roborio.json, can-devices.json, console.txt (--seconds 10)
atlas robot logs frc-radio:6390 --out DIR # radio-status.json (WPA key fields left out)
atlas robot open frc-radio:6390           # the web page's address
```

## Needs credentials or a decision (designed, not built)

- **The roboRIO's log files.** These are the `.wpilog` data logs and
  console logs in `/home/lvuser/logs` and `/u` on a USB stick, and NI's logs
  in `/var/local/natinst/log`. Two ways in, both needing a login:
  1. **SFTP/SCP with a key** (recommended). The user puts Atlas's SSH key on
     the RIO once, as `lvuser` or `admin`. roboRIO images ship with
     `admin`/blank and `lvuser`/blank, and Atlas must never log in with
     those on its own; the user runs the key install themselves. Atlas then
     reuses its `SshAccess` (known_hosts in the data directory) for
     `atlas robot logs --ssh` and a Files tab that downloads logs. This
     needs a per-device stored user and key choice.
  2. **WebDAV** at `http://<rio>/files/` (Digest, realm `NIAuth`), with a
     user and password the user gives Atlas, kept in the OS keychain and
     never in settings.json. This one needs no key setup, but Atlas would
     have to store a password.
- **The roboRIO's system facts** (serial, image version, firmware, free
  space). These come from the `POST /nisysapi/server` read query that NI's
  own page sends unauthenticated on load. It is a read, but it is a POST.
  Decide whether Atlas may send it. If yes, the key could become the RIO's
  serial instead of the team number.
- **SystemCore (2027).** It is expected at `10.TE.AM.2` too, at
  `robot.local` (mDNS), at `172.27.0.1` over USB from Linux and macOS
  (`172.26.0.1` from Windows; `.28`/`.29` on images 9 and earlier) and
  `172.30.0.1` on its own Wi-Fi. The controller driver rejects anything that
  isn't NI's page, so SystemCore needs its own driver. Its web UI is a
  dashboard at `/` with `/configure` (team number, web OTA from a
  `.llupdate`) and a Package Manager (opkg `.ipk`; a package's `X-Https` and
  `X-Services` control fields shape the UI). Neither Limelight's
  systemcore-os-public nor wpilibsuite/SystemcoreTesting documents an HTTP
  API, and its SSH login is a password (`systemcore`), which Atlas doesn't
  use. So the driver is written against what a unit serves without a login:
  `scripts/systemcore-capture.sh [host]` saves that (GETs only: the pages,
  their scripts, the API-like paths those name minus anything that writes,
  and which FRC ports answer). The radio's `systemcoreEnabled` flag is
  already read. Later (not built): installing and updating HeliOS on a
  SystemCore as an `.ipk`, through the same Package Manager API.
- **Radio configuration** (team, SSID, channel, firmware). This would be
  writes (`POST /configuration`, `/firmware`) and is deliberately left out.
- **The board side.** `usb_route.addresses` ships with the device package.
  Until a board runs it, a board with a static address on a wide network
  relies on known teams.

## Tests

- `robot_net`: team math, `/8` narrowing, known teams.
- `atlas-driver-robot`: parsers against trimmed real responses in
  `tests/fixtures` (the WPA key hashes are redacted). These cover the NI
  page, Phoenix getversion and getdevices, VH-109 status, and two real
  console frames, plus probes and captures on the loopback.
- Board driver: a wide route narrows to the board's robot network, a /24
  route is used as it is, and a board not on USB or not routing has no
  children.
- Core: saved addresses name their teams.
- `devices/raze/tests/usb-route.sh`: `addresses`, including a static `/8`.
- Desktop: `layout.test.ts` (bun test).
