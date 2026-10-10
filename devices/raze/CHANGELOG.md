# Raze device package changelog

## Unreleased

Development work since 1.0.7, not yet validated on hardware as a release.
The package version stays 1.0.7 until a real release. OSes pin this layer by
commit; the commits are listed per area.

### Breaking and ABI notes

- **Platform:** Buildroot 2026.08, kernel `rpi-7.2.y` (7.2.9, bcm2712, 16K
  pages) and rpi-firmware 1.20260915 (`fd52491`). Buildroot 2026.08 removed
  rpi-userland, so `BR2_PACKAGE_RPI_USERLAND` must go from OS overrides.
  OpenJDK's default became 25. The firmware override also installs
  `overlays/hat_map.dtb` next to `overlay_map.dtb`, as Raspberry Pi OS does
  (the firmware uses it only for a HAT+ ID EEPROM; a bare Raze has none).
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
  `/run/board/ssh/authorized_keys`, which needs the OS's sshd_config to
  `Include /etc/ssh/sshd_config.d/*.conf` before any `AuthorizedKeysFile`
  (`695d172`).
- **Update input:** `update stage` takes the same whole-disk image that is
  flashed over USB, plus `--sha256`. There is no separate update format (an
  interim `.pdupdate` bundle never shipped) (`696d3ad`).
- **USB gadget address:** the gadget network moves from the fixed
  `172.31.250.1/24` to a per-board /29 in `172.31.0.0/16`, derived from the
  USB serial (scheme `serial-hash-v1`). `USB_GADGET_ADDRESS` and
  `USB_GADGET_DHCP_RANGE` default to `AUTO`. Anything that hard-codes
  172.31.250.1 (scripts, saved SSH hosts, firewall rules) must use the
  board's address from the identity's new `gadget` field, or pin
  `USB_GADGET_ADDRESS=172.31.250.1/24` in `/etc/board/usb-gadget.env`.
  Atlas needs this commit or later to reach new images over the gadget link
  without mDNS (`e6afcf3`).
- **lemnosd is required for LEDs and fan control by default.** The layer
  imports Lemnos's lemnosd layer (`gaia/lemnos.toml`), so an OS build must
  declare a second git source, `lemnos` at
  `62c3caf677c4bf9cef47fcb02b36d0e98f3248ef` (see `gaia/device.toml`; Gaia
  refuses import sources declared inside a source-imported layer), and needs
  Docker on the build host (Lemnos builds its static binaries in a container)
  and systemd-sysusers (the `lemnos` user). lemnosd owns `/dev/leds0`, the
  sensors and the fan's sysfs controls; `raze-leds`, `board-locate.service`
  and the self-test are its clients when `/usr/bin/lemnos-ctl` exists. Other
  programs that write `/dev/leds0` or the fan directly now race lemnosd: use
  `lemnos-ctl` (or `raze-leds`), and put non-root clients such as
  PhotonVision in the `lemnos` group. `BOARD_HW_BACKEND=sysfs` in
  `/etc/board/hw.env` keeps the old direct access as a fallback; images
  without lemnos-ctl get it automatically (`d5302db`).
- **hw.sh backends:** `BOARD_HW_BACKEND` now names a backend (`lemnosd`,
  `sysfs`, `auto`); a path still names a replacement file. Self-test JSON
  stays version 1 with added fields (`backend` at the top; `backend`,
  `status`, `handed_back`, per I2C device `bus_hint`, `bus_found_by`,
  `chip_id`, `lemnosd`); under lemnosd the fan check's `cooling_device` is
  the board.toml id `fan` (`d5302db`).
- **Board awareness (additive):** `update.json` (and the identity's
  `update`) gains `version_previous` and `started_by`; the identity always
  has `endpoints` with `status` and `events`, also without `BOARD_ACTIONS`;
  the board keeps `/data/board/events.jsonl` (at most 2000 lines),
  `/data/board/boot-count`, `/data/board/drift/` and
  `/data/board/ssh-keys.sum`, and the writer writes
  `board-boot.<slot>.sha256` to p1 when it stages. New units, enabled by the
  preset: `board-boot.service`, `board-drift.timer`. Readers that require an
  exact `update.json` field set must accept the new fields (`019b215`,
  `01d5290`).

### Board awareness: events, status, drift

- **Hardware in the status:** `GET /status` gains `hardware`: lemnosd's
  devices (id, class, model, status, controls) with one reading each (name,
  value, unit), kept by root's `board-health.timer`, now every 10 s, via
  `status --refresh-hardware`; `null` without lemnosd or when stale. A
  faulted sensor (like a BMI088 that fails to start) now shows without
  Orion or SSH. Atlas renders it on a new Hardware tab.
- **Failed units that don't lie** (found on a board): GET /status reported
  `failed_units: []` while sshd and board-locate had failed, because the
  endpoint's sandboxed user can't reach systemd and an empty answer looked
  like "none". Root's `board-health.timer` (every 30 s, enabled by the
  preset) now keeps them in `/run/board/failed.json`; `status` uses that
  when it is under 2 minutes old, else asks systemctl itself, and answers
  `null` when it can't. Atlas shows "Unknown" for null.
- **Event log:** `board_event <kind> <message> [key=value ...]` (lib.sh) and
  `/usr/lib/board/event` append `{t, boot_id, kind, source, message, data}`
  to `/data/board/events.jsonl` (`/run/board` without a writable `/data`),
  keeping the last 2000. `source` is `BOARD_EVENT_SOURCE` (`atlas` on Atlas's
  SSH commands, `orion` for everything board-agent runs) or `local`. Events
  come from the writer (`update.download`, `update.stage`, `update.staged`,
  `update.failed`, `update.apply`, `update.apply-failed`, `update.confirmed`,
  `update.trial-failed` with the reason, `update.rolled-back`,
  `update.cancelled`, `update.rollback`, `update.rollback-failed`,
  `update.link-bad`, `update.link-fallback`; a trial's outcome is attributed
  to whoever started the update), ssh-keys (`ssh.keys`, when the key set
  changes), usb-boot, the selftest (result summary), `boot` and `shutdown`,
  and from Atlas's `reboot`, `power-off` and `clock.set` (`019b215`).
- **Boot record:** `board-boot.service` runs `boot-record`: it counts boots
  (`/data/board/boot-count`), writes `/run/board/boot.json` (id, count, slot,
  kernel, `previous_clean`) and logs `boot`; its ExecStop logs `shutdown`, so
  a boot after a power cut, watchdog reset or panic says the previous one
  didn't end cleanly (`019b215`).
- **Status:** `/usr/lib/board/status --json`: boot (with uptime), failed
  units (`systemctl --failed`), temperatures (thermal zones), the fan
  (through hw.sh, so the lemnosd backend works; sysfs reads when
  unprivileged), `update.json`, the clock (time, NTP) and drift (`019b215`).
- **Drift:** the running slot's `config.txt`, `cmdline.txt`, `kernel*.img`
  and `overlays/*` against the hashes the writer recorded at stage
  (`board-boot.<slot>.sha256` on p1), else against what the board first saw
  (`/data/board/drift/`); `/etc/board` and `/data/board` files listed as
  overrides with their hashes; on a writable root `/etc/ssh/sshd_config(.d)`
  and `/etc/board` against their first-seen hashes (EROFS roots skipped).
  Flags: `cmdline`, `config`, `sshd_config`, `update_env`. Checking mounts
  the boot partitions read-only, so it is root's job (`board-drift.timer`,
  2 min after boot and hourly, `status --refresh-drift`), kept in
  `/run/board/drift.json` (`019b215`).
- **Endpoints:** the identity endpoint answers `GET /status` and
  `GET /events?since=<t>&limit=<n>` (read-only; `since`/`limit` must be plain
  numbers, nothing from the query is evaluated; `limit` at most 2000) and
  lists them in `endpoints` (`019b215`).
- **board-agent:** runs the writer and its commands with
  `BOARD_EVENT_SOURCE=orion` and logs a `reboot` event before an Orion
  reboot (`BOARD_AGENT_EVENT`). Orion's contract (`c22fa42`) has no power-off
  action, so the agent claims none (`01d5290`).
- **Needs hardware:** the clean-shutdown detection across a real reboot and
  a power cut, the drift check's mounts of the running boot slot and p1, and
  `systemctl --failed` / `timedatectl` from the identity endpoint's
  sandboxed user.

### lemnosd, the hardware service

- **Lemnos ae59665** (from cca50f7): the fault reasons (`lemnos-ctl list`
  shows a `why:` line for a device that isn't available; the BMI088 IMU
  tolerates unacknowledged soft resets), the Orion bridge, and trailing `*`
  wildcards in `writers` and `raw_clients`, and a bridge that builds without
  a C cross compiler (no `ring`) and finds orion-node's sockets in
  /run/orion. The board schema and the driver
  registry are unchanged.
- **Who may set what through Orion:** the fan's writers add `orion:*`, so
  Atlas (the bridge writes as `orion:<requested_by>`) can set its duty and
  hand it back. usb-a-power lists no writers, so any client may already;
  the LED ring and the sensors stay out of reach (the ring is the board's
  status display).
- **The Orion bridge:** gaia/lemnos.toml imports Lemnos's optional
  `lemnos-orion.toml` (`/usr/bin/lemnos-orion`, `lemnos-orion.service` as the
  `lemnos` user, on Orion 4fadba9 like board-agent and Atlas), enabled in
  70-board.preset. Its drop-in adds the `orion` group (for /run/orion) and
  starts it only when /run/orion exists; orion-node's drop-in now admits
  `root,user:lemnos`.
- **One Orion node id:** `board-orion-env.service` writes
  /run/board/orion.env (`ORION_NODE_ID=raze-<serial8>`, the name Atlas shows,
  or the OS's own ORION_NODE_ID from /etc/default/orion-node.env) before
  orion-node and lemnos-orion start, and both read it, so the bridge's
  devices land on the node they run on. A board that ran orion-node as
  `node.local` comes up under its new id.
- **Eased blink:** `raze-leds blink [<hz>]` (PhotonVision's blinking
  statuses) is now lemnosd's breathe effect at full depth, ease-in-out, with
  the same period (1000/hz ms): it fades to off and back, so it still reads
  as blinking at 2 Hz without the hard on/off. `blink --hard` keeps the old
  flash. Locate, status and the self-test already used eased effects. The
  sysfs backend (no lemnosd) still flashes.
- **Lemnos cca50f7** (from 62c3caf): raw GPIO, PWM, I2C and SPI through
  lemnosd, with arbitration; claims and writes end with the client's
  connection. In the package:
  - **The `lemnos` user** comes from Lemnos's
    `packaging/buildroot/lemnos-users.table` (groups i2c, gpio, spi,
    video): `gaia/lemnos.toml` sets `BR2_RAZE_LEMNOS_USERS_TABLE` to it and
    `external.mk` adds its lines to `PACKAGES_USERS`, next to the OS's own
    `BR2_ROOTFS_USERS_TABLES`. OSes drop their own copy.
  - **board.toml:** the IMU, magnetometer and power monitor list
    `raw = ["board-selftest"]`. USB-A power stays a `gpio-output` device
    (`initial = true`): its safe state is on, since lemnosd undoes a
    client's write when its connection ends, and a device's line is never
    handed out raw, so no `[[lines]]` entry. The Raze has no free GPIO line,
    PWM channel (the fan's is the kernel's) or spidev to declare, so no
    `[[lines]]`, `[[pwms]]` or PWM/spidev udev rules.
  - **Self-test:** with lemnosd, chip-id registers are read through lemnosd
    (`lemnos-ctl i2c read`, brokered between its own transfers) instead of
    i2cget, also for devices lemnosd has available, so a wrong chip fails
    even while its driver runs. When lemnosd refuses (an older lemnosd) its
    word stands, as before. The report's i2c data gains `reads`
    (`lemnosd` or `i2cget`).
- **Lemnos 62c3caf** (from b4d6cfe, no API changes): its Gaia layer asks
  for `gaia_version >= 2.0.0` (b4d6cfe asked for 2.1.0, which doesn't
  exist, so every OS build importing it failed), and both binaries build in
  one cargo invocation (`build_group`). An OS can import Lemnos's optional
  `packaging/gaia/lemnosd-host.toml` after the package to build them on the
  host instead of in Docker. `gaia/board-agent.toml` asks for 2.0.0 too.
- **Lemnos b4d6cfe** (from 8a126d3): the self-test's LED frames go on
  lemnosd's test layer (`led ... --test --seconds 60`, above every client's
  status; `led off --test` clears only it), and the fan check hands the fan
  back with `lemnos-ctl fan release fan` while lemnosd keeps running,
  instead of the root `fan restore`.

- **Gaia import:** `gaia/lemnos.toml` imports Lemnos's
  `packaging/gaia/lemnosd.toml` (static aarch64 musl `lemnosd` and
  `lemnos-ctl`, `lemnosd.service`, sysusers, preset) pinned at Lemnos dev
  `62c3caf`, and redeclares `lemnosd-env` with
  `LEMNOSD_UPDATE_STATUS=/run/board/update.json`, so the updating animation
  follows the package's update writer (`d5302db`).
- **Board definition:** `/etc/lemnos/board.toml` is generated from the
  manifest by `gen-raze.py`: the status ring (16, `wire = "rgb"`, offset 5,
  direction from the manifest, still unverified; fade 250 ms ease-in-out,
  status effect breathe), the fan (match `name = "pwmfan"`, no
  `restore_mode`), `cpu-thermal`, the BMI088 (accel 0x18, gyro 0x68) on
  `i2c:compatible=i2c-gpio`, the BMM150 (0x10) and INA238 (0x40) on
  `i2c:of=/axi/pcie@1000120000/rp1/i2c@74000`, and the USB-A power line
  (`pinctrl-rp1` 20). The INA238's maximum current is its full scale over
  the shunt (16.384 A), derived; the shunt itself is unverified. The lint
  validates the file against Lemnos's JSON Schema (vendored in
  `devices/schema/`), its driver rules and, with a `lemnos-ctl`,
  `lemnos-ctl validate` (`d5302db`).
- **udev:** `60-board-lemnosd.rules` gives the `lemnos` group the fan's
  `pwm1`/`pwm1_enable`, the pwm-fan `cur_state` and the zones' `policy`, and
  the device nodes to `i2c`, `gpio` and `video` (`d5302db`).
- **Clients:** `raze-leds` keeps its commands and becomes a wrapper over
  `lemnos-ctl --client raze-leds led ...`; locate uses lemnosd's locate
  effect; the self-test sets the fan's duties through lemnosd and ends with
  its hand-back (`lemnos-ctl fan restore`) instead of pausing the thermal
  zone, and draws its interactive frames as `board-selftest` at priority
  100 (`d5302db`).
- **Verified on a Raze with lemnosd cd72ad0** (the coordinator, 2026-10-07):
  the pwmfan fan, cpu-thermal and the ring on `/dev/leds0` (wire rgb,
  offset 5) bind; readings, the progress, updating, breathe and locate
  effects, and the fan hand-back work. Not yet on hardware: the b4d6cfe bus
  selectors, the sensors through lemnosd, the package's udev rules and
  client scripts, the USB-A line (hogged by default).

### I2C facts (verified on a board)

- BMI088 on the i2c-gpio bus (`/dev/i2c-4` there): accel 0x18 (chip id 0x1E),
  gyro 0x68 (0x0F). BMM150 on i2c1-pi5 (`/dev/i2c-1`) at 0x10 (0x32 at
  register 0x40, readable once power control 0x4B bit 0 is set). INA238 at
  0x40 (manufacturer 0x5449, die 0x2381; SMBus words read byte-swapped).
  Checked by chip-id register reads on the PhotonVision image (HeliOS
  coordinator, 2026-10-07). Buses carry a `select` (Lemnos `i2c:` keys)
  besides their number, which follows probe order. The self-test finds buses
  by it and reads the chip ids, waking the BMM150 for the read and putting
  its power control back when lemnosd isn't the owner (`d5302db`).
- **`update rollback` changed meaning.** It used to forget a staged update;
  it now boots the previous confirmed slot (switches autoboot.txt's default,
  records `rolled-back`, restarts). The old behaviour is `update cancel`.
  Scripts that ran `rollback` to clear a staged or failed update must call
  `cancel` (`b2fc1ac`).
- **New update states** `rebooting` (apply's or rollback's restart was
  accepted; `apply` used to record `trying` directly) and `cancelled`, in
  `board-update.env`, `update.json` and the identity's `update` object. The
  values are Orion's `update_action::STATE_*` (`b2fc1ac`).
- **Writer exit status:** a refusal (wrong state, another command holding
  the lock) exits 3 and no longer records `error` (an `apply` with nothing
  staged used to leave the state `error`). A `status` while a stage runs
  prints update.json, which the stage now keeps current, instead of adding
  the copy's progress itself (`b2fc1ac`).
- **Orion:** board-agent is built against Orion `c22fa42` (control protocol
  4 with a new layout fingerprint); the image's orion-node must come from
  the same commit. The package adds an orion-node drop-in,
  `orion-node.service.d/50-board-agent.conf`, that sets
  `ORION_NODE_LOCAL_AUTH_ALLOW=root` so the agent (root) can use the
  node's local IPC; an OS that sets that variable itself must include
  `root` (`17c2475`, `3064d90`).

### USB gadget addressing

- Several Razes on one computer each get a subnet of their own: the USB
  serial (the board serial) picks a /29 of `172.31.0.0/16` outside
  `172.31.250.0/24` by sha256, so the host computes the address from the USB
  descriptor. Parameters in `usb-gadget.env` (`USB_GADGET_ADDR_BASE`,
  `USB_GADGET_ADDR_PREFIX`, `USB_GADGET_ADDR_EXCLUDE`) and in the manifest
  (`capabilities.gadget-net.addressing`); `lib.sh` `board_gadget_subnet`.
- dnsmasq offers the host the rest of the /29 (`.2`-`.6`).
- Without a hex serial, or when pinned, the board keeps 172.31.250.1/24.
- The identity reports `gadget: {address, prefix, addressing}`.
- Atlas probes every matching gadget at its own address and accepts it only
  when the identity's serial matches the USB serial; 172.31.250.1 stays a
  fallback while exactly one board is plugged in.
- Collision odds and host notes: devices/README.md, "USB gadget network".
- Commit: `e6afcf3`.
- **Manifest values:** I2C addresses (`capabilities.i2c.devices[].address`,
  `camera.i2c_address`) are now JSON numbers (24, not `"0x18"`), and
  `camera.kernel_driver` is the driver name (`ov9282`; the prose moved to
  `kernel_driver_source`). The schema still accepts the old strings and
  Atlas reads both; no other reader is known (`47f8769`).
- **Generated files:** `raze-device.txt` (its i2c, LED and camera lines),
  `raze-fan-overlay.dts` (`cooling-levels`, `pwms`), `sensors.toml`,
  `60-board-watchdog.conf`, `leds.env` and `hardware.env` come from
  `manifest.json` through `devices/tools/gen-raze.py`. Edit the manifest,
  not those parts; CI fails when they disagree (`47f8769`).
- **LED index offset:** `raze-leds` now maps LED index i to driver slot
  (5 + i) mod 16, as HeliOS does. Whole-ring commands are unchanged; only
  the new per-LED `pixel` is affected. `RAZE_LEDS_OFFSET=0` restores the
  raw order (`c4bbf6e`).
- **Image:** i2c-tools (`i2cdetect` and friends, small) is added for the
  self-test's I2C scan (`c4bbf6e`).
- **Identity:** an optional `diagnostics` array (`["selftest"]`) (`c4bbf6e`).

### Updates (A/B with tryboot)

- **Found on hardware (fixed):** the first A/B update from an image (PV r5
  to r7, a rollback and a roll-forward) needed manual workarounds:
  - The drift check leaked read-only mounts of p1, p2 and p3 (made in a
    subshell, so its cleanup never saw them; board-health.timer made them
    every 10 s). The updater's own mount of p1 then came up read-only and
    every write to it failed. The drift check now mounts in its own shell
    and unmounts, and only while it holds the update lock shared (it skips a
    round while an update command runs); the updater refuses a read-only p1
    up front (status 3, saying where it is mounted so).
  - `confirm` printed "confirmed" and `stage` "staged" while every write
    failed (a function run in `cmd || rc=$?` has no `set -e`). A failed
    write of the state or `autoboot.txt` now fails the command (status 1,
    the error in `update.json`), and `autoboot.txt` is put back if the state
    couldn't follow it.
  - At boot, `confirm` and `check-link` failed with "another update command
    is running" (board-agent's `status` calls held the lock), and
    `check-link` held the lock through its 25 s delay, so a manual
    `rollback` was refused. Commands that change state now wait for the lock
    (`UPDATE_LOCK_WAIT`, 60 s), `status` answers from `update.json` without
    it, and `check-link` waits before taking it.
  - The kernel probe used `tr -c '[:print:]'`, which busybox doesn't
    support, so `stage` said it couldn't compare the kernel. It now splits
    on NULs (`tr '\000' '\n'`), tested with busybox's tools in CI.
  - `apply` used `systemctl reboot "0 tryboot"`, which systemd 258 rejects;
    it now passes `--reboot-argument='0 tryboot'`.
  - `apply` marked the update `trying` even when no restart happened. It now
    does so only once the restart is accepted; otherwise it removes the
    `[tryboot]` section and stays `staged`.
  - A slot B carrying a stale 6.12 kernel on a 7.2 root (from an image
    assembly bug) passed the OS health check and was confirmed. Three
    safeguards now catch that:
    - `stage` refuses an image whose boot slot's kernel release has no
      matching `/lib/modules` on its root;
    - `confirm` runs the package's own checks first (kernel/modules match,
      USB gadget bound, no failed `UPDATE_CRITICAL_UNITS`);
    - `update check-link` (`board-update-link.service`) runs them after
      every boot and returns to the previous good slot after
      `UPDATE_LINK_MAX_BAD` bad boots in a row, with no USB needed.
- `dtparam=watchdog=on` in raze-device.txt. The OS cmdline should add
  `panic=5` so a panicking trial restarts into the old slot. docs/ota.md has
  a table of what each safety net covers, and a hardware test procedure.

- `/usr/lib/board/update`: `status`, `stage`, `apply`, `confirm`,
  `rollback`.
  - **stage:** `stage <image> --sha256 <hex>`, or
    `stage - --sha256 <hex> [--format xz|zst|gz|raw]` from stdin.
    - It copies the image's boot slot A (p2) and root slot A (p5) into the
      inactive slot in one streaming pass through `board-image-slots`, a small C
      tool in this package. The rest of the image is skipped.
    - A file is checked against its SHA-256 before anything is written.
    - The new root must name this model. Its os-release gives the version
      (`IMAGE_VERSION`, else `VERSION_ID`).
    - Images without the A/B layout are refused.
  - **apply:** boots the new slot once with `reboot "0 tryboot"`.
  - **confirm:** `board-update-confirm.service` keeps the new slot after
    the OS's `update-health` passes. A failed check restarts into the old
    slot, and the hardware watchdog (RuntimeWatchdogSec=15s) covers a hung
    trial.
  - **Locking:** a lock lets only one writer run. `status` answers from the
    last state while another command runs.
  - Commits: `e12e22d`, `5baf83e`, `696d3ad`.
- OS hooks in `/etc/board/update.d/`: `pre-stage`, `post-stage`,
  `pre-reboot`, `post-boot`. A failing pre- hook stops its step (`696d3ad`).
- Layout: p1 autoboot.txt (and the update state), p2/p3 boot, p5/p6 root
  (512 MiB target, EROFS), p7 /data. See docs/ota.md.
- The identity lists `ab-tryboot` in `update_methods`, and an `update` object
  with the state, slots, versions, progress and error.
- The image gets zstd, xz-utils and board-image-slots.
- **cancel, rollback, rebooting, stage-url** (`b2fc1ac`):
  - `update cancel` stops a running stage (its PID is in
    `/run/board/update.pid`; the process tree is frozen, then ended, and the
    lock taken) or forgets a staged update, and prints `cancelled` or
    `idle`. It refuses once the update is past staging.
  - `update rollback [--no-reboot]` boots the previous confirmed slot;
    refused while staging, on a trial boot or without a previous slot. The
    slot it leaves becomes the previous one.
  - `apply` and `rollback` record the boot they were made in: `rebooting`
    read on the next boot reports `trying`, and `confirm` treats a leftover
    `rebooting` like `trying`.
  - `update stage-url <url> --sha256 <hex> --size <n>` downloads to
    `/data/board/update/` with curl or wget (resuming a partial download),
    checks size and SHA-256, then stages; progress 0-500 for the download,
    500-1000 for the stage. An image already staged is done.
  - A stage no longer leaves the slot it overwrites as the previous slot,
    and `status` turns a stage interrupted by a power loss or a killed
    process into an error.
  - Tested in `tests/update.sh`, including a real HTTP server with and
    without range support, with curl and wget.

### Orion device agent

- **Orion 4fadba9** (request/response actions; `ActionRequest.wait_ms`
  changed the wire layout): board-agent and Atlas move together, with every
  orion-node they talk to. Handlers are unchanged. Atlas's own actions wait
  on the node for their reply instead of asking every 500 ms, and its enroll
  command also grants `clock.set`, `set`, `restore` and `release`.
- **board-agent: the writer's own refusal text, also under load.** When the
  writer exited before its last stderr line was read, the action failed
  with "the writer failed (exit status: 3)" instead of the writer's reason
  ("the last update is still on trial ..."). The agent now waits (up to
  1 s) for stderr to end before answering.
- **`clock.set`** (board-agent): sets the board's clock from `unix` or an
  ISO `time` (UTC), refusing times before 2024 or from 2100, and logs a
  `clock.set` event as Orion's. board-agent publishes what it claims as
  `action.claimed`. Atlas offers "Set clock from this computer" through
  Orion when the node lists it and prefers Orion for every action both
  transports offer, falling back to SSH only when Orion was never reached.
- **Orion ec91d0a** (from c22fa42): board-agent and Atlas use the same Orion
  commit as the OS images. Same control protocol (4) and wire fingerprint;
  it adds `TypedConfigValue::F64`, which Atlas shows as text.
- `board-agent` (Atlas `crates/board-agent`, `/usr/bin/board-agent`,
  `board-agent.service`) claims `update`, `update.cancel`,
  `update.rollback`, `reboot` and `locate` on orion-node's local IPC and
  runs them with the writer, per Orion's `docs/device-agent.md`: `update`
  starts `stage-url` and succeeds with `phase = "staging"`, then applies;
  `locate` drops the package's locate request. It publishes the `update.*`
  keys (with the boot id) after connecting, on change and every 30 s, and
  waits without orion-node (`17c2475`).
- Built by Gaia as an artifact (`gaia/board-agent.toml`, imported by
  `device.toml`): a static aarch64 musl binary, cross-built in Docker with
  rust-lld from `crates/board-agent` in the OS build's `atlas` source, and
  installed as `/usr/bin/board-agent`, enabled by the preset. It was first a
  Buildroot package (`3064d90`); the artifact keeps Rust out of the
  Buildroot tree and rebuilds incrementally. `BR2_PACKAGE_BOARD_AGENT` is
  gone: drop it from OS overrides. There is no switch to leave the agent out;
  without orion-node it waits.

### Fresh installs without the boot button

- `/usr/lib/board/usb-boot` restarts a running CM5 straight into USB boot
  (RPIBOOT). It uses the firmware's one-time `set_reboot_order`, so the
  bootloader's own BOOT_ORDER never changes. It is root-only and run over SSH
  (later Orion), never through the open identity endpoint.
- The identity lists `usb-boot-reboot` when the board supports it.
- `rpi-utils` builds `vcgencmd` and `vcmailbox` (`194a159`).

### Hardware

- **Verified on a Raze** (2026-10-07, sysfs on the PhotonVision image,
  package c881c60): camera on I2C bus 10 at 0x60 (bound to ov9282; it
  reports the OV9281 chip id, which is accepted), rp1-cfe and PiSP FE; fan
  cooling device `pwm-fan` and hwmon `pwmfan` (no tachometer); thermal trips
  50/60/67.5/75 °C; watchdog `/dev/watchdog0` (BCM2835); LED offset 5 with
  the 24-bit wire format. Recorded in the manifest's `verified` notes. The
  LED direction is still unverified.
- **Self-test fan restore:** the thermal zone (step_wise, no polling) only
  re-evaluates on a trip crossing, so after stepping the fan the self-test
  now writes the zone's policy back to make the governor run at once.
  Before, the fan stayed at the last stepped level.

- **LED ring:** SK6812-EC20, 24-bit GRB on the wire. The `ws2812-pio`
  overlay no longer passes `rgbw`. `raze-leds` keeps the driver's 4-byte
  layout, so white is `color 255 255 255` (`f98489f`). Verified on a Raze.
- **Fan:** normal polarity, levels 179/212/245/255/255 (a 70 % minimum)
  (`ff18ab8`, `f98489f`). Verified on a Raze. `dtoverlay=raze-fan,polarity=1`
  restores inverted drive.
- **Overlays:** the stock overlays `raze-device.txt` loads are built from the
  kernel being built, not taken from the firmware (`fd52491`).
- **LED index offset:** HeliOS main writes frame index i to driver slot
  (i + 5) mod count (`DEFAULT_LED_INDEX_OFFSET = 5` in
  `backend/src/helios-peripherals/src/lighting.rs`, `HELIOS_LED_INDEX_OFFSET`
  overrides it; static frames only). The package records it as
  `leds.index.offset` 5, `direction` 1; not re-measured here (`47f8769`).

### Manifest as the source of hardware facts

- `manifest.json` describes the hardware precisely (LED part, wire format
  grb24, userspace layout rgbw, index offset/direction; fan PWM
  controller/channel/period/polarity, levels, duty floor, trips; camera CSI,
  I2C and chip ids; I2C buses and devices; watchdog; kernel), each fact with
  a `verified` note: `{by, date, method}` or `"unverified"` (`47f8769`,
  `3f959d0`).
- Verified on a Raze Gen 1 (image 1.0.10) on 2026-10-06 by Mathias and the
  HeliOS coordinator: LED part, wire format and 4-byte layout, LED GPIO; fan
  PWM channel, period, normal polarity and levels (state 1 at 58 °C read
  back 83 %). 16 KiB pages: both OSes ran them on hardware. Unverified: LED
  count and index direction, fan trips, camera facts, I2C devices, watchdog.
- `devices/tools/gen-raze.py` renders the derived files; `--check` prints a
  diff and fails on drift, on rule breaks and when the kernel facts disagree
  with `gaia/kernel.toml` and `raze.config` (read only). The output is
  committed rather than generated at package build time
  (`47f8769`, `3f959d0`).

### Self-test

- `/usr/lib/board/selftest [--interactive] [--json]`: LED ring, fan,
  camera, I2C scan, watchdog and USB gadget, each ok/skip/fail with a message
  and data. `--json` prints a format-1 report and writes
  `/run/board/selftest.json`. Root only; restores the fan and the ring on
  exit or interrupt (`c4bbf6e`).
- Hardware access goes through `hw.sh`, replaceable by another backend
  (`BOARD_HW_BACKEND`). `raze-leds` uses it, applies the index offset, and has
  `pixel` and `refresh` (`c4bbf6e`).
- Atlas runs it over SSH after a flash or update and on request, keeps the
  last result per board serial and shows it on the device panel (`d685975`,
  `d06b5a3`).

### Optional layers

- `gaia/gpu-vulkan.toml`, after `gpu.toml`: Mesa's v3dv driver, the Vulkan
  loader and vulkan-tools (`e5f9087`).

### Smaller image

- No udev hwdb (systemd or eudev), about 13 MB.
- Kernel modules drop from 25 MB to 15 MB; sound stays, because DRM_VC4
  depends on it (`695d172`).

### Faster image builds

- **libcamera no longer pulls in libyuv, jpeg or bzip2.** The package's
  rpi/pisp selection carried them since its first commit, but libcamera
  0.7.2 uses libyuv only for its Android layer and virtual pipeline, libjpeg
  only for the `cam` app's optional JPEG output, and bzip2 not at all. An OS
  that needs any of them for its own software (for example libjpeg) must now
  select it (`BR2_PACKAGE_JPEG`, `BR2_PACKAGE_LIBYUV`, `BR2_PACKAGE_BZIP2`).
- **Kernel modules are no longer compressed** (`MODULE_COMPRESS` off): the
  EROFS root compresses them anyway, and xz-compressing ~1400 modules was
  most of the kernel's install time. Modules are plain `.ko` files.
- **No virtualization (KVM)** in the kernel.
- **A much smaller kernel build:** `raze.config` drops what a Raze never
  runs: NIC vendors without hardware here, netfilter beyond a basic nftables
  firewall (IPVS, ipset, ebtables, ALG helpers, exotic matches), most qdiscs,
  HAT sound codecs and cards, DRM panels and fbtft, TV/radio/test media,
  vendor HID drivers other than gamepads, unused filesystems and NLS tables,
  HAT sensor/IIO/hwmon/RTC/GPIO chips (lemnosd reads the board's sensors over
  i2c-dev), RAID/MTD/ATA extras, legacy gadget modules, KGDB and the ftrace
  family. Measured on 53679a5 at -j24 (Image, modules, dtbs): 458 s -> 260 s
  wall (6920 -> 3792 CPU-s), 1410 -> 312 modules, Image 24.4 -> 17.9 MB.
  Gamepads stay (xpad is new; joydev, Sony/PlayStation/Microsoft/Nintendo/
  Steam/Logitech HID), as do USB serial adapters, NVMe, device-mapper and
  zram. Wireless is intentionally out. Needs a boot test of the gadget,
  camera and display before release.
- **Two trims that never worked now do:** the TV/DVB/radio/SDR/test media
  options came back on because `MEDIA_SUPPORT_FILTER` was off (now on), and
  `DM_RAID` selected RAID (`BLK_DEV_MD`, `MD_RAID456`) back on (now off).
- **The build checks the kernel config:** `linux/check-config.sh` runs after
  the kernel is configured and fails, naming each option, when anything
  `raze.config` sets or turns off didn't take effect (on the old config it
  names exactly those 8). Test: `tests/kernel-config.sh`.
- libpisp no longer depends on Boost. It used Boost only for logging, and
  Buildroot's boost package has no Boost.Log unless an OS turns it on, so
  logging was always off and Boost was extracted, installed and copied into
  per-package trees for nothing. `-Dlogging=disabled` builds the same
  library.
- board-agent is a Gaia artifact instead of a Buildroot package (see Orion
  device agent above): no Rust toolchain (host-rustc) in the Buildroot tree,
  and a change to the agent no longer means a cold cargo build there.

### Runtime paths and settings

- **Clock:** the identity reports the board's `"time"`. Atlas shows how far
  off it is, and offers "Set clock from this computer" over SSH when it's 5 s
  or more off. The board has no RTC battery and no NTP over the USB link.
  OS images should keep timesyncd's clock file on /data (docs/ota.md).
- **SSH key fix** (found on hardware): `ssh-keys` installed nothing when the
  key file's last line had no newline. A POSIX `read` drops it, and Atlas's
  Flash tab wrote the file that way. The loop now reads an unterminated last
  line and CRLF files. It logs how many keys it installed, and warns when the
  file exists but holds no usable key. The Flash tab now always writes one
  key per line with a final newline. Test: `tests/ssh-keys.sh`.

- **SSH keys:** `ssh-keys` finds the boot partition from `root=` on the
  kernel command line, so an overlay root works. On the A/B layout that is
  p1, shared by both slots (`a6dec52`).
- **Overrides:** `*.env` settings are read from `/usr/lib/board`, then
  `/etc/board`, then `/data/board`. board-package.env is never read
  from `/data` (`695d172`).
- **Commit stamp:** `BOARD_PACKAGE_COMMIT` comes from the OS, in
  `/etc/board/board-package.env`, for git imports (`a6dec52`).
- **DHCP leases** for the USB gadget live in `/run` (`695d172`).

### Tests

- `devices/raze/tests/`:
  - `update.sh` uses a real sfdisk A/B image, compressed with xz and zstd,
    with an EROFS root slot when erofs-utils is new enough.
  - `usb-boot.sh` uses a fake mailbox.
  - `root-device.sh` checks finding the root device.
  - `gadget-address.sh` checks the gadget address vectors (shared with
    Atlas), the identity field and `usb-gadget-setup` with a fake configfs.
  - `selftest.sh` runs the self-test and the LED offset against a fake
    sysfs, /dev and configfs (`c4bbf6e`).
  - `manifest-lint.sh` runs `gen-raze.py --check` and checks the lint catches
    drift and rule breaks (`47f8769`, `3f959d0`).
  - `status.sh` covers the event log (sources, escaping, rotation, the /run
    fallback), the boot record, `status` with a fake sysfs and systemctl,
    drift, and `GET /status` / `GET /events` through identity-http, with
    injection attempts in the query (`019b215`).
- CI runs them on Ubuntu 24.04 when a change touches `devices/`, all at
  once through `tests/all.sh`, which runs every `tests/*.sh`.

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
