# RAZE-DBUS-HELPER (working title: GAIA-EXECBIT) — evidence, not yet a fix

Status: NOT COMMITTED. Evidence only. Nothing in devices/ changed. The helper fix is on hold
(coordinator ruling): it will be `BR2_ROOTFS_POST_FAKEROOT_SCRIPT` once Gaia ships the fakeroot-stage root
and that rev is pinned in the same commit. No placeholder, no workaround.

Pinned sources (read by git form, no download): Buildroot 2026.08 at d5180309b1b66ef3b8eaccca70ad69be8e0729a1
(`git --git-dir /mnt/ssd-samsung/gaia/git/66fcad2eba12298ff9818d43.git show <rev>:<path>`); systemd 258.7 and
dbus 1.16.2 tarballs in the Gaia store-build download cache (read with `tar -xOz`, not extracted into the repo).

## 1. Buildroot order (file:line at d5180309)
- Permission table applied: fs/common.mk:85 writes PACKAGES_PERMISSIONS_TABLE into ROOTFS_FULL_DEVICES_TABLE;
  dbus.mk:32-34 DBUS_PERMISSIONS (unconditional) names usr/libexec/dbus-daemon-launch-helper; pkg-generic.mk:1227-1228.
- Users table: fs/common.mk:80 PACKAGES_USERS; dbus.mk:36-38 DBUS_USERS (group dbus), systemd.mk:700-712 SYSTEMD_USERS.
- Fakeroot script (per-filesystem copy, `$(FS_DIR)/<fs>/target`, fs/common.mk:110 and Makefile:470-473):
  rsync from output/target (172-173) → chown -R 0:0 (178) → mkusers (179, writes /etc/passwd, group in THAT copy)
  → makedevs -d table (180; makedevs.c:486 chdir(rootdir), group lookup from etc/group at 286 = the copy)
  → ROOTFS_PRE_CMD_HOOKS (181-182): ROOTFS_RM_HWDB_DATA (Makefile:842), SYSTEMD_LOCALE_PURGE_CATALOGS (systemd.mk:809),
  SYSTEMD_UPDATE_CATALOGS (823), SYSTEMD_RM_CATALOG_UPDATE_SERVICE (831), SYSTEMD_PRESET_ALL (836 → 834)
  → BR2_ROOTFS_POST_FAKEROOT_SCRIPT (183-185) → run/ and tmp/ emptied (189-190) → rootfs command (193)
  → `rm -rf $(TARGET_DIR)` (196). So nothing from the fakeroot stage is in output/target.
- post-build script: Makefile:827-832 (target-finalize, before the fakeroot stage; output/target).
- dbus is built with meson (dbus.mk:168): traditional_activation defaults true (dbus meson_options.txt:261-264),
  so dbus-daemon-launch-helper is built and installed only because use_traditional_activation is true
  (dbus meson.build:522, bus/meson.build:145-184). dbus.mk sets no -Dtraditional_activation.
- libdbus soname: dbus meson.build:63-80 gives lt_current 41, lt_age 38, revision 3 → version_info 3.38.3,
  soversion 3. usr/lib/libdbus-1.so.3.38.3 in post-build.sh's list is correct for 1.16.2.

## 2. Options for the helper (evidence only)
- (d) Do not install dbus: package/systemd/Config.in:31 `select BR2_PACKAGE_DBUS if !BR2_PACKAGE_DBUS_BROKER` is a
  Kconfig select; no fragment line can turn it off. systemd.mk has no dbus dependency; it sets -Ddbus=disabled
  (systemd.mk:69, 915). Dropping the select line needs a Buildroot patch. A product cannot carry one today: the
  product.toml [sources.buildroot] has only git and ref (devices/products/raze-base/product.toml:20-22), and no
  Atlas file patches Buildroot. Gaia 9efa797f support for a Buildroot patch: UNCLEAR, ask Gaia.
  Other packages that select or depend on BR2_PACKAGE_DBUS (full grep): at-spi2-core, avahi, basu, bluez, cog,
  connman, dtui, efl, firewalld, gvfs, iwd, network-manager, polkit, rauc, udisks, wpa_supplicant, and more. The
  ones this product enables: systemd only (fragment). dbus-broker (dbus-broker.mk:27,54) is the alternative
  selection: it has no DBUS_PERMISSIONS entry, but it is still a bus on the image.
- (e) dbus without the helper: `-Dtraditional_activation=false` would stop the build of the helper, but
  DBUS_PERMISSIONS (dbus.mk:32-34) is unconditional, so makedevs still fails ("regular file ... does not exist",
  makedevs.c:578-582). Clearing makedevs needs the entry removed too: a Buildroot patch (same problem as d).
- (c) Placeholder (empty file, rejected by the coordinator). Would leave an empty root:dbus 4750 file in the image:
  a setuid-bit file, which the fragment's own comment (busybox entry) says Gaia strips unless listed; and it
  would fail the post-build leftover check unless exempted. The dbus group exists at makedevs time (mkusers runs
  first, fs/common.mk:179-180), so makedevs resolves it.
- Chosen (on hold): post-fakeroot removal once Gaia's fakeroot-stage root is pinned.
- Step 3 of post-build.sh fails at the leftover check (line 122 `find . -path '*dbus*'`) if the helper is kept,
  as Gaia reported. The dry run must reproduce that before any change; it is not yet run (see section 5).

## 3. Audit table (post-build.sh lines)
Verdict key: NOW = no effect today (script runs before fakeroot; output/target is what Gaia takes); AFTER = takes
effect once the fakeroot-stage root is the image.

| line(s) | what it assumes | verified how | verdict |
|---|---|---|---|
| 50 DBUS_REQUIRED usr/libexec/dbus-daemon-launch-helper | dbus installs the helper | dbus.mk:32-34; meson bus/meson.build:145-184 | removal blocked by makedevs; see §2 |
| 50-64 other required paths | dbus 1.16.2 file layout | dbus.mk, meson; not verified against an installed tree | NEEDS-THE-BUILD |
| 65-66 libdbus-1.so.3.38.3 | soname from version_info | dbus meson.build:63-80 → 3.38.3 | VERIFIED-BY-READING |
| 68-69 DBUS_OPTIONAL run/dbus, dbus-*-hostname1/timedate1 | units created by dbus-activation | systemd units dbus-org.*: NOT VERIFIED in 258.7 tarball | NEEDS-THE-BUILD |
| 88-93 OFF_SERVICES | fragment turns resolved/timesyncd/hostnamed/timedated off | fragment lines; systemd.mk resolve=false (907) | VERIFIED-BY-READING (config) |
| 118-124 removal + `rm -rf` | files present at target-finalize | Makefile:756-827 | NOW |
| 124-138 mask ln -sfn /dev/null wait-online | preset-all honours a mask | systemd 258.7 install.c not read for masks | NEEDS-THE-BUILD |
| 140-146 `find . -path '*dbus*'` leftover | fails on the helper (Gaia log) | Gaia log: step 3 fails with the helper kept | VERIFIED (Gaia log) |
| 148-160 ELF libdbus scan | readelf of target ELF | readelf present on host; TARGET_READELF not exported | NOW |
| 162-210 enabled-units checks | presets applied after the script | Makefile:827 vs fs/common.mk:181-182 | NOW (its model is the preset set; see §4) |

Wait-online mask (the one Gaia asked about): systemd-networkd.service [Install] has
`Also=systemd-networkd-wait-online.service` (systemd-258.7 units/systemd-networkd.service.in:70),
and 90-systemd.preset:  `enable systemd-networkd-wait-online.service` (a named rule). A lower-sorted preset
`disable systemd-networkd-wait-online.service` would be the first match, so the preset file can decide it; whether
the Also= link is then made anyway when networkd is enabled is NEEDS-THE-BUILD (install.c enable path not read).
The mask must stay until that is run.

## 4. preset-all, read from systemd 258.7 source
- query_presets (src/shared/install.c:3484-3520): when no rule matches, the action is PRESET_ENABLE
  ("Preset files don't specify rule for %s. Enabling."). So preset-all ENABLES EVERY INSTALLED UNIT that no
  rule names. This is VERIFIED-BY-READING. The post-build header's claim is the same.
- Presets on the image: 90-systemd.preset (systemd meson presets/meson.build, installed to system-preset),
  80-buildroot.preset (empty, systemd.mk:785-787), 80-orion.preset (Orion component, orion/component.toml:84-86,
  content at Orion 455a956e not read here: NOT VERIFIED).
- Units with an [Install] section in 258.7 units/: 69. Matched by 90-systemd.preset: 38 (enable/disable/ignore rules). Not named
  (UNMATCHED → enabled by preset-all): 31 units. The unmatched list (source: units/ in the tarball) — every one is
  enabled if its option is on in our build; many are absent because the fragment turns the option off. Which
  are present on the image: NEEDS-THE-BUILD (needs the built tree). Examples of the unmatched ones:
  serial-getty@.service (template; the getty@ rule does not match serial-getty), systemd-networkd.socket,
  systemd-networkd-varlink.socket, systemd-oomd.service/socket, systemd-sysupdate(.timer), systemd-nspawn@.service,
  systemd-pcrlock-* (5), systemd-udev-load-credentials.service, systemd-userdb-load-credentials.service,
  systemd-factory-reset.socket, systemd-homed-firstboot.service, systemd-journald@.socket,
  systemd-mountfsd.service, systemd-nsresourced.service, systemd-pcrextend.socket, systemd-sysext.socket,
  systemd-resolved-monitor/varlink.socket, systemd-userdbd.service, systemd-vmspawn@.service.
  Names with an option we turn off (resolved, timesyncd, pstore=false at systemd.mk:917, tpm2=disabled at 905,
  bootloader=disabled at 908): their units are not in the image (NEEDS-THE-BUILD to be sure).

Per-unit delta table (the built image is NEEDS-THE-BUILD; "wanted?" is from the product's own statements):

| unit | rule | today (output/target) | after fix | wanted? | fix if not wanted |
|---|---|---|---|---|---|
| systemd-networkd.service | 90 enable | enabled by Gaia [[services]] (raze-base-files:44) | enabled | yes (lemnos0 DHCP) | — |
| systemd-networkd-wait-online.service | 90 enable + Also= | not enabled | masked by post-build (line 124-138) | no | a `disable` line in a lower file; mask only if the Also= link survives (NEEDS-THE-BUILD) |
| systemd-network-generator.service | 90 enable | not enabled | enabled | unclear (kernel cmdline ip=) | `disable` in product preset |
| systemd-pstore.service | 90 enable | not enabled | enabled (pstore=false at systemd.mk:917 may remove it) | ramoops is kernel-side; pstore unit reads /sys/fs/pstore | NEEDS-THE-BUILD |
| systemd-journald-audit.socket | 90 enable | not enabled | enabled | no (no audit) | `disable` |
| systemd-userdbd.socket / homed / homed-activate | 90 enable | not enabled | enabled if the option is on | no | `disable` (option check first) |
| systemd-mountfsd / nsresourced sockets | 90 enable | not enabled | enabled if present | no | `disable` |
| systemd-sysext / confext / boot-clear-sysfail / boot-update | 90 enable | not enabled | enabled if present | no | `disable` (bootloader off; check presence) |
| remote-fs / remote-*-target | 90 enable | not enabled | enabled (targets, harmless) | no need | `disable` (keep or drop) |
| machines.target | 90 enable | not enabled | enabled if machined on | no | `disable` |
| serial-getty@.service, getty@.service | UNMATCHED / 90 enable | not enabled (serial-getty@ttyGS0 is Gaia's) | template: instances per [Install] | ttyGS0 yes; others no | `disable serial-getty@*` plus the explicit ttyGS0 link (Gaia [[services]] already names it) |
| systemd-oomd.service/socket | UNMATCHED | not enabled | enabled if oomd on | no | `disable` (option check) |
| systemd-sysupdate*, pcrlock*, factory-reset, vmspawn, nspawn@ | UNMATCHED | not enabled | enabled if present | no | `disable` (options off → absent) |
| systemd-udev-load-credentials, userdb-load-credentials | UNMATCHED | not enabled | enabled | unclear | NEEDS-THE-BUILD |
| lemnosd, lemnos-orion, orion-comm, orion-node, lemnos0-address | none (Lemnos/Orion units) | enabled by Gaia [[services]] | enabled | yes | — |

The preset file that carries these `disable` lines is a product file (for example devices/products/raze-base/os/
presets/…preset, installed by a fragment, numbered before 90-systemd). It must be written and gated; not done here.

## 5. What could not be checked here
- No built tree, so the presence of every unit above is NEEDS-THE-BUILD.
- The fake-tree dry run of post-build.sh (must fail at step 3 with the helper kept, as Gaia's log says) is not run;
  the self-test needs a fixture root, which is not built.
- User/group table (2b) and /var (2d): from reading only:
  - systemd-networkd runs as User=systemd-network (systemd-258.7 units/systemd-networkd.service.in:54). The user
    is defined in SYSTEMD_NETWORKD_USER (systemd.mk:572), added to PACKAGES_USERS via SYSTEMD_USERS (systemd.mk:709),
    which is applied in mkusers in the fakeroot stage (fs/common.mk:179). It is NOT in output/target, so networkd's
    User= cannot be resolved today (consistent with Gaia's finding that no package users are in the image).
  - Units or tmpfiles that name lemnos, lemnos-orion, orion: created by component [[users]] (lemnos/component.toml:65-71,
    orion/component.toml:95-98), which Gaia applies outside the fakeroot stage: VERIFIED-BY-READING of the components.
  - systemd-network.conf tmpfiles (systemd-258.7 tmpfiles.d/systemd-network.conf) owns /run/systemd/netif as
    systemd-network: after the fix, it works only if the user exists at boot.
  - Permissions table: SYSTEMD_JOURNALD_PERMISSIONS (systemd.mk:119) makes /var/log/journal owned by systemd-journal
    (group from SYSTEMD_USERS, systemd.mk:704). After the fix, /var is read-only (see below), so this directory
    cannot be written at runtime.
- /var:
  - skeleton-init-systemd.mk:23/29: /etc/fstab gets only `/dev/root / auto ro 0 1` (output/target, not fakeroot).
  - skeleton-init-systemd.mk:32-60 (with BR2_INIT_SYSTEMD_VAR_FACTORY, system/Config.in:214): moves /var to
    /usr/share/factory/var, writes usr/lib/tmpfiles.d/00-buildroot-var.conf (C!/L+! lines) and installs
    factory/var.mount. That hook is ROOTFS_PRE_CMD_HOOKS → fakeroot stage only (line 60). Today /var in output/target is
    a real directory with build content.
  - var.mount (factory/var.mount, package/skeleton-init-systemd/factory/var.mount) has no [Install]; nothing in
    the components pulls it: a read-only root keeps /var on the EROFS root. So after the fix /var is an empty
    read-only directory, and the C! copies cannot be written. NEEDS-THE-BUILD.
  - Orion's comments say /var is "RAM on the bring-up layout and inside the slot root on a managed board"
    (orion/files/orion.conf:11), which contradicts a read-only slot root. Orion moved its state to /data.
  - The journal: raze-base-files/files/60-board-journald.conf:2-3 says the OS keeps /var/log/journal on /data.
    No component installs a /var/log/journal link or a /data mount (only dev-bringup-hostkey data.mount, a DEV-only
    component). That comment is unimplemented: the journal would stay on /run unless systemd falls back itself.
    NEEDS-THE-BUILD.
  - Conflicts: SYSTEMD_JOURNALD_PERMISSIONS (`/var/log/journal d 2755`) makes that directory in the fakeroot copy;
    the var factory moves /var, so the directory is created under /var only if /var is still writable at tmpfiles
    time. Flag for the coordinator; NEEDS-THE-BUILD.
- Journal catalog (systemd.mk:813-823): fakeroot stage; builds /usr/share/factory/var/lib/systemd/catalog/database
  and adds a tmpfiles L line to usr/lib/tmpfiles.d/var.conf (line 820-821). Those files come only with the fakeroot root.
- hwdb.d removal: Makefile:839-842, fakeroot stage.

## 6. Exec bits (Gaia 9efa797f)
Gaia 9efa797f stores `post-build.sh` at mode 644 (confirmed: a stored tree object under
`$GAIA_STORE/objects/tree/e6/e6c52cce…/post-build.sh` is `644`). The exec-bit check (to add) must fail on that build.
The check is not written yet.

## 7. Status
- Not done: the helper fix (on hold), the sh -n + dry-run rule, the exec-bit check, the built-tree rule, the gate run,
  the pin, and the PR.

## 8. What this PR changes (after the evidence commit)
- `devices/products/raze-base/os/post-build.sh`: the helper leaves the removal list (`DBUS_HELPER`, step 1 still requires
  it, step 3 exempts exactly that path). Verified on the fake tree: with the helper kept out of the exemption, the script
  fails at step 3 with Gaia's message (`dbus paths left after the removal ...: usr/libexec/dbus-daemon-launch-helper`).
- `devices/products/raze-base/os/post-fakeroot.sh` (0755): removes the helper, refuses when it is absent, refuses any
  dbus path left. STAGED: not named by `raze.fragment` (a comment at the POST_BUILD line says so). It is wired in the
  commit that pins the Gaia rev whose rootfs is the fakeroot-stage result. Under the current pin that copy is discarded
  (`fs/common.mk:196`), so a removal there would not reach the image.
- `devices/products/raze-base/os/overlay/usr/lib/systemd/system-preset/10-raze-base.preset`: the image's stated unit set
  (`enable` lines with a reason each) and a final `disable *`. Inert until the pin (see its header). Its `UNSURE:` lines are
  for the coordinator's ruling.
- `scripts/fake-raze-root.sh`: a labelled FAKE tree, not a built root (placeholders; the paths come from post-build.sh).
- `scripts/check-product-scripts.sh`: `sh -n` (or `bash -n` on a bash shebang) for every fragment-named script; a dry run where
  a stand-in tree exists, otherwise a visible `dry run: NOT CHECKED (no stand-in tree for <script>)`. Self-test: a syntax
  error is refused. The dry run of post-build runs it after the HOSTNAMED-off removal (as its own self-test builds it).
- `scripts/check-fragment-modes.sh`: `--self-test` (a stored 0644 copy is refused, a 0755 copy passes, no stored copy is
  refused) runs in the gate. `--real` asks Gaia for the stored trees (`gaia build … --until gaia.source.input`, the settings
  from `scripts/version-rule.sh`) and fails for raze-base post-build.sh at 9efa797f: the output was
  `FAIL …/post-build.sh: stored 644 … (executable in the tree): Gaia drops the exec bit of a fragment-named script`.
  The real run is NOT in the gate in this commit; it joins with the Gaia pin that fixes the exec bit.
- `scripts/check-built-root.sh`: `--built NAME=DIR` (or `ATLAS_BUILT_TREES`) and `--expect FILE`: the enabled units must EQUAL
  the expected set, and every User=/Group= of an enabled unit (plus `orion` when `orion-comm.service` is enabled) must exist
  in the tree's etc/passwd or etc/group. With no tree it prints `NOT CHECKED`. Self-tests on fake trees: a missing
  `systemd-network` account is refused; an extra enabled unit is refused; a missing enabled unit is refused; an equal,
  complete tree passes. The gate runs the self-test and the NOT CHECKED line.

## 9. Preset semantics (VERIFIED-BY-READING, systemd 258.7 `src/shared/install.c`)
- presets_find_config: `/etc/systemd/system-preset` and `/usr/lib/systemd/system-preset`, merged by file name (conf_files_list
  semantics; the /etc file of the same name shadows /usr/lib). Lines are read in name order.
- query_presets: first matching rule wins; no match → PRESET_ENABLE (:3504).
- `disable`: unit_file_preset → install_info_discover on the `minus` side (:3636-3637): the unit's links are removed.
- Gaia's own [[services]] links: whether they are made before or after the root step is NOT VERIFIED here; it decides
  whether `disable *` removes them. Gaia states its own link-making order (the coordinator to confirm).
- Wait-online: the mask stays. NEEDS-THE-BUILD whether `disable *` covers it despite networkd's `Also=`
  (units/systemd-networkd.service.in:70).

## 10. /var (open: to be answered per slot product when Gaia's first fixed root exists)
Buildroot, quoted (skeleton-init-systemd.mk:32-36 and :37-60):
> On a R/O rootfs, /var is a tmpfs filesystem. So, at build time, we redirect /var to the "factory settings" location.
> Just before the filesystem gets created, the /var symlink will be replaced with a real (but empty) directory, and the
> "factory files" will be copied back there by the tmpfiles.d mechanism.
The hook is `SKELETON_INIT_SYSTEMD_ROOTFS_PRE_CMD_HOOKS` (line 60): it runs in the fakeroot stage, only with
`BR2_INIT_SYSTEMD_VAR_FACTORY` (system/Config.in:214). It writes `usr/lib/tmpfiles.d/00-buildroot-var.conf` (line 56)
and installs `factory/var.mount` (lines 57-58):
```
[Unit]
Description=Buildroot /var tmpfs
DefaultDependencies=no
Conflicts=umount.target
Before=local-fs.target umount.target
After=swap.target
[Mount]
What=tmpfs
Where=/var
Type=tmpfs
Options=nosuid,nodev,size=50%%,nr_inodes=1m
```
The unit has no `[Install]` section. Whether anything pulls `var.mount` in: NEEDS-THE-BUILD (systemd's implicit
dependencies on a mount unit for a path under /var come from RequiresMountsFor= and from units ordering on it; none of our
units names /var today, see below).
What our products do with /var today (read by reading):
- `/etc/fstab` gets `/dev/root / auto ro 0 1` only (skeleton-init-systemd.mk:29); no /var line.
- orion's state moved off /var to /data (orion/component.toml:12, 54-55; orion/files/orion.conf:11-19, `state.dir = /data/orion`),
  with a comment that /var is "RAM on the bring-up layout and inside the slot root on a managed board" (orion.conf:11).
- The data partition: boards/raze/board.toml:66-71 (`name = "data"`, role data, the LAST partition). Nothing in
  devices/components mounts it on raze-base: the only `data.mount` is in dev-bringup-hostkey (DEV ONLY).
- Journal: `raze-base-files/files/60-board-journald.conf` (full text quoted in §11) sets `Storage=persistent`, and its
  comment says "The OS keeps /var/log/journal on /data". No component installs a /var/log/journal link or a /data mount.

## 11. Journal and /data facts (for the coordinator's ruling; no change made)
- 60-board-journald.conf (full text, raze-base-files/files/60-board-journald.conf, installed at
  /usr/lib/systemd/journald.conf.d/60-board.conf by raze-base-files/component.toml:7-10): the comment block (lines 1-6)
  says the journal is kept across boots on /data (`/var/log/journal` on /data), bounded; then
  `Storage=persistent`, `SystemMaxUse=16M`, `SystemMaxFileSize=4M`, `SystemKeepFree=256M`, `RuntimeMaxUse=4M`,
  `RuntimeMaxFileSize=1M`.
- What the image has today: the file is in output/target. /var is a real directory on the read-only root; nothing mounts
  /var or /data on raze-base, so `/var/log/journal` cannot be created on a read-only root. What journald does then (volatile
  in /run, or a failure): NEEDS-THE-BUILD (journald's fallback to /run/log/journal is systemd's documented behaviour,
  not verified in the 258.7 source here).
- pstore/ramoops: configured. boards/raze/board.toml:2-4 (keeps pstore: required), board.toml:37-42 (overlays
  `ramoops-pi5`, `vc4-kms-v3d-pi5`), boot/raze-device.txt:9-12 (`dtoverlay=ramoops-pi5,total-size=0x40000,record-size=0x10000,
  console-size=0x20000`: 256 KiB at 0x0b000000, 128 KiB of it the console), raze-trim.config:25 (no wired console; pstore is
  the log). Kernel config PSTORE, PSTORE_RAM, PSTORE_CONSOLE built in (board.toml:4).
  What it captures: the kernel console of an early boot and the previous boot's log, in RAM. The Lemnos crash-log reader
  (docs/prs/B29.md, B20.md mention it): not read in this session.
- /data on raze-base: a data partition (board.toml:69-72, the last partition, type 0x83, ext4 per the dev fixture
  `data.mount`, which is DEV ONLY). What already writes there: Orion's state (`/data/orion`, orion/files/orion.conf:13-19),
  and the orion comm/node drop-ins that create `/data/orion` (orion/files/orion-comm-state.conf:9, orion-node-state.conf:11).
  Lemnos's slot-init moves /data into the new root (dev-bringup-hostkey/component.toml:6-7 quoting Lemnos docs/slots.md):
  a managed-board mount of /data is Lemnos's, not in this repo. Its size: the partition has no size here (board.toml:66-67).
- To implement "bounded persistent journal on the data partition": (a) a `var-log-journal.mount` or a symlink
  `/var/log/journal -> /data/journal` created on the read-only root is impossible without a mount, so a unit that mounts
  /data (Lemnos owns the managed-board mount) plus a `ConditionPathIsMountPoint=`-gated journald setting, or (b)
  `Storage=persistent` with `/var/log/journal` bind-mounted from /data by a unit ordered before systemd-journald. Size: the
  caps already exist (60-board-journald.conf); the mount is the work. Small if Lemnos's /data mount exists; otherwise not
  small. Ruling needed.

## 12. Built-tree checks still NOT CHECKED here
- No built root exists in this session: the enablement and account rule has no tree to run on (NOT CHECKED, printed).
- The 31 unnamed `[Install]` units (258.7): under `10-raze-base.preset`'s final `disable *` they are disabled unless named;
  which of them are present in the image: NEEDS-THE-BUILD (not chased one by one, per the ruling).
- Dropping dbus stays: a later size item, measured on a built root.

## 13. The split (after the Gaia log: the helper trips three places in post-build)
Gaia's log: post-build stopped at the ELF scan (old line 134) on the helper. So the helper tripped the removal list, the
leftover check and the ELF scan. Ruling: split by stage, not exemptions.
- `post-build.sh` (output/target, before mkusers/makedevs/preset-all, Makefile:827-832): ONLY the wait-online mask. It no
  longer removes D-Bus and has no dbus check. Its self-test: the mask is placed, the helper and dbus.service are still there.
- `post-fakeroot.sh` (per-filesystem copy, fs/common.mk:183-185, after makedevs:180 and preset-all:181): the WHOLE D-Bus
  removal (the fixed list, the helper included), then the checks: no dbus path left, no ELF needing libdbus-1, no
  Type=dbus enabled unit, no socket starting a turned-off daemon or naming a missing unit, no dangling wants link, the mask
  still present. Its self-test: the refusals listed in the script.

Order check (is the split right?):
- makedevs (fs/common.mk:180) needs the helper present (DBUS_PERMISSIONS, dbus.mk:32-34; makedevs.c:578-582). Post-build
  does not remove it, so makedevs passes. Nothing else in the permission table names a dbus path. OK.
- preset-all (fs/common.mk:181, systemd.mk:834) runs BEFORE post-fakeroot. A dbus unit a preset ENABLES is linked by then;
  post-fakeroot removes it, and the dangling-link check fails if its link survived. Our preset ends `disable *`, so
  preset-all disables dbus.service and dbus.socket (their links go). The dry run models both outcomes (A: links removed,
  B: links kept); both pass, and the post-fakeroot list tolerates an absent wants link for that reason.
- The mask: preset-all honours a mask (systemd.mk:834 runs after post-build's mask, Makefile:827 before fs/common.mk:181).
  OK.
- Anything between target-finalize and post-fakeroot that needs dbus present: NOT FOUND in fs/common.mk:179-182 (mkusers,
  makedevs, the pre-cmd hooks). The skeleton var factory, if on, moves /var before the removal: NOT on for raze-base
  (raze-base fragment does not set BR2_INIT_SYSTEMD_VAR_FACTORY; rock-4d/rock-5c/zero-3e fragments do, os.fragment:103).
- The dbus USERS (`dbus`, DBUS_USERS dbus.mk:36-38) stay in /etc/passwd and /etc/group after the removal: a user with no
  files. Harmless; the built-root rule checks enabled units' accounts only.

What this means for the image, plainly: between now and the Gaia pin, `raze.fragment` names only post-build.sh (now the
mask only), so the image KEEPS D-Bus (the helper included, the libdbus ELF too). The build no longer fails on dbus. That is
what Gaia's timing copy does too. The helper is setuid (4750) and ships until the pin; the pin commit wires post-fakeroot.

Dry run order (scripts/check-product-scripts.sh, fake tree): post-build.sh; the simulated makedevs check (helper present);
the simulated preset-all (A: dbus links removed; B: kept); post-fakeroot.sh on each; refusals: helper absent, mask missing,
dangling link to a dbus unit off the list. The fake tree is a placeholder tree (scripts/fake-raze-root.sh), not a built root:
every step of post-fakeroot runs to the end on it, including the ELF scan on a real small ELF (fixture-user-elf, no NEEDED
libdbus). The ELF scan decides by `readelf -d` for `Shared library: [libdbus-1.so…]` on every file whose first four bytes
are the ELF magic.

Gaia's own packed rootfs.tar (its current run's fakeroot-stage result): requested from Gaia; not in this session.

## 14. CORRECTION to §10 and §11: who mounts /data, and when (Lemnos at the pinned rev 31e0f736, read by git form)
- My earlier wording "nothing on raze-base mounts /data" was about devices/components only. It is WRONG as a statement about
  the board: Lemnos's slot-init mounts it. Read: service/slot-init/src/boot.rs:1268-1284 — the data partition (ext4,
  `noatime,errors=remount-ro`) is mounted at `self.layout.data` (`/data` by default, drivers/system-slots/src/lib.rs:231,
  named-files lib.rs:79), in the INITRAMFS, before the root switch. `data_mounted` is set (boot.rs:1284).
- Then boot.rs:1392-1412: the slot's EROFS root is mounted at `newroot`, and the data mount is MOVED into it:
  `move_mount(&self.layout.data, &newroot/data)` (boot.rs:1405). The move fails with "no /data in the image?" when the
  image has no `/data` directory (boot.rs:1405-1407). So the raze image MUST contain an empty `/data` directory; nothing in
  devices/components/raze-base-files creates it (only the DEV-ONLY dev-bringup-hostkey does, data-mountpoint). OPEN, to add
  in the same commit as the journal, or Lemnos's requirement stated by Lemnos.
- The driver (system-slots) does NOT mount /data: it reads the mount table to find the device mounted at `data`
  (drivers/system-slots/src/lib.rs:1198-1210, /proc/self/mountinfo). So: slot-init mounts, lemnosd reads.
- Timing: /data is mounted BEFORE systemd starts (the initramfs), and it is inside the slot root from switch_root on. A
  journal path on /data is therefore available to every unit of the slot, from the first unit, with no ordering needed.
  Slots live there too: `<data>/slots/<id>.erofs` (Lemnos docs/slots.md:15-17), so the journal shares the partition with the
  slot images (see §16).
- Correction to §11: "Orion's state is already on /data": true (orion/files/orion.conf:13-19). The reason it is on /data is the
  one above. §11's "nothing mounts /data" is replaced by this section.

## 15. Built-root rule: the one derivation, the Also= closure, the open questions
- expected enabled set = the product's preset `enable` lines ∪ every `[[services]] enable = true` of the components the
  product's images use (Gaia's answer: [[services]] links are a LATER Gaia step, gaia.tree.services, Gaia
  crates/nodes/tree/src/services.rs:25-103, a layer of symlinks in etc/systemd/system/*.wants merged over the OS root).
- Two claims refused: a unit in the preset AND in a component's [[services]] (two owners of one link path); a unit masked in
  the tree AND expected (its [[services]] or a preset line): the mask wins, the link is lost. Both have self-tests.
- Masked units are not counted as enabled (systemd refuses a masked unit's link).
- Preset (10-raze-base.preset), now: `enable` for the Also= units of systemd-networkd (below); `disable` for wait-online
  (masked, not counted); `disable *` for the rest. The lemnosd/orion/networkd/getty units are NOT in the preset: their
  components own them (`[[services]]`).
- Also= closure, from the units of systemd 258.7 (the only Also= in the closure of the units we enable):
  - systemd-networkd.service.in:59 `Also=systemd-networkd.socket systemd-networkd-varlink.socket` → both listed `enable`.
  - systemd-networkd.service.in:64 `Also=systemd-network-generator.service` → listed `enable`.
  - systemd-networkd.service.in:70 `Also=systemd-networkd-wait-online.service` → listed `disable` (masked by post-build.sh).
  - The Also= targets have no further Also= (checked: networkd.socket, varlink.socket, network-generator.service.in,
    wait-online.service.in, wait-online@.service.in). serial-getty@.service.in:54 and getty@.service.in:64 have WantedBy only.
  - NOT CHECKED: Lemnos's and Orion's own units (their [Install] Also=): their unit files are not read here; Lemnos's
    packaging/systemd is at its pinned rev, not read. Needs reading before the pin.
- wait-online outcome the rule expects: masked ⇒ no wants link ⇒ not counted. Whether the Also= link is still made when the
  mask exists: NEEDS-THE-BUILD (the enable path in install.c was not read). If a link appears, the rule reports it as
  "enabled but in neither" (the expected set does not list it), which is the detection we want.
- OPEN (NEEDS-GAIA, asked as the coordinator relayed): none left on the services question; Gaia's answer is recorded above.

## 16. Journal on the data partition: what is decided, and what is NOT (prepared, not wired)
Not wired: no unit, tmpfiles line or conf is added to any fragment in this PR. The coordinator said: prepare, or describe the
exact files. These are the conditions, each with what I can establish now:
1. Hard cap and rate limit. Data partition size: boards/raze/board.toml:66-72 gives the data partition the LAST place and NO
   size; its size is the rest of the disk. The smallest data partition we ship is therefore NOT FOUND here (the bench boards
   on their branches are not read). So the cap cannot be chosen against a number yet. What is known: the slot images live on
   the same partition (Lemnos docs/slots.md:15-17; a raze root image is 18,178,048 bytes at lzma, docs of §PR B20 / product
   comment: devices/products/raze-base/product.toml:61-62), so the journal must leave room for at least the slots, the
   slot store (the trial, previous and default slots: slots.md:23) and the state. Proposal, to be set by the coordinator:
   SystemMaxUse=16M (the 60-board-journald.conf value) and SystemKeepFree = slot store headroom (NEEDS the slot store's
   peak size, Lemnos slots.md). Rate limit: RateLimitIntervalSec=30s, RateLimitBurst=200 (journald.conf(5) defaults are
   30s/10000, which is too loose for a small partition; the value is a proposal, not verified against journald here).
2. Flash wear. SyncIntervalSec is a journald.conf setting (default 5m); journald fsyncs immediately on CRIT/ALERT/EMERG
   messages (journald.conf(5); NOT verified in the 258.7 source here). The steady-state write rate on a quiet robot is an
   ESTIMATE that needs a board: what our units log per minute is not measured. Method: count the lines our units produce on
   a quiet board for an hour (journalctl --disk-usage before/after), then multiply. Mark: ESTIMATE, NEEDS-THE-BOARD.
3. Early boot. /data is mounted in the initramfs (§14), so the journal can write to /var/log/journal from the first unit IF
   the directory exists at that point. Before that (the initramfs), the journal does not run. So there is no flush unit
   needed for the boot path; systemd-journal-flush.service is only needed when the journal started volatile (/run) and is
   moved later. Our layout does not need it. How /var/log/journal gets onto the data partition: a symlink in the image,
   /var/log/journal -> /data/journal, created at build time (the root is read-only, /var is a real directory in the image:
   Buildroot's var factory is off for raze-base, see the doc §10), plus a tmpfiles line `d /data/journal 2755 root systemd-journal`
   so the target exists. Owner: the tmpfiles line and the symlink are image content (overlay or a component), not a unit.
   The mount unit: none needed (the mount is slot-init's, §14). NEEDS-BUILD: whether journald follows the symlink when its
   target is created by tmpfiles at boot (it should, as a directory path, unverified).
4. Slot switch and rollback. Nothing in the slot switch path was read to wipe /data: the slot-init move keeps the data mount
   (boot.rs:1405-1411). The journal is on data, not in the slot, so a slot change does not remove it. Journal files written by
   slot A are readable after booting slot B only if the machine-id is the same, or with `journalctl -D /data/journal/<id>` /
   `--merge`. The machine-id: Buildroot writes an EMPTY /etc/machine-id (systemd.mk:673-675, `touch`), in a read-only root.
   What systemd does with an empty machine-id on a read-only root: NEEDS-THE-BUILD (it may generate one per boot, in which case
   each boot writes a new /data/journal/<id>/ directory; the old ones stay readable with --merge). The comment in
   60-board-journald.conf refers to "board-machine-id.service" to keep one directory: NO such unit exists in devices/ (grep:
   the comment is the only mention). So the stable-id promise in that comment is UNIMPLEMENTED: it would need that unit
   (the coordinator's call: a unit that writes /etc/machine-id once onto /data, or a fixed id in the image).
Found, not fixed here: (a) 60-board-journald.conf's /data claim is unimplemented (no symlink, no tmpfiles line, no
board-machine-id.service); (b) the image needs an empty /data directory for the slot-init move (boot.rs:1405-1407), which
raze-base-files does not create; (c) machine-id is an empty file from Buildroot's hook (systemd.mk:673-675).
The journal direction stays as the coordinator confirmed in principle; conditions 1 (the sizes) and 4 (the machine-id) are the
two that can change the direction, and both need the build or the bench boards.
