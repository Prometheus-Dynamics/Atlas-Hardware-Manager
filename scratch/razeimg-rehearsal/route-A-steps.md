# Route (A): new Raze slots booted beside dev slot 993 (DRAFT v4, Lemnos b78e5317; rehearsed in emulation, nothing run on a board)

Raze #2 first; Raze #1 untouched. Lemnos's runner runs every step as root over 993's OpenSSH (usbbr0), or over
Orion's SSH where it says "in the new slot". Atlas runs nothing on a board; it supplies the files and this list.
This round ends with two proven new slots that still boot only under 993's config.txt. A2 (the switch to
Lemnos's config.txt) is NOT in this round; it gets its own list later (see the end).

v2 changes (coordinator's rulings 2026-11-18, Lemnos's IMG-A1-trial-life.md and IMG-A1-steps-review.md at
Lemnos origin/next 423ae736): forms T and S are gone, one form with the bench dead-man; no step writes state.env;
the p1 supplies are everything Lemnos's builder emits but default.txt and config.txt; p1 is changed only through
`lemnos-ctl boot-partition --add-to` on a dump on the PC, then a sector write-back and a whole-partition re-dump
compare; the second slot goes into bay1 by the same tool; the restores after a boot cover what Lemnos writes; A2
is out.

Sources: RAZE1/image-facts.txt, boot-facts-raze1.txt, boot-facts-raze2.txt; notes/IMG-A1-code-answers.md,
IMG-A1-trial-life.md, IMG-A1-steps-review.md; dev's slot-init (atlas.data=PARTUUID=<disk>-02); Lemnos
contracts/slot-layout.toml v5. Lemnos file:line references are in those notes, not repeated here.

## STANDING PRECONDITION (coordinator 2026-11-21): blocks step 4

- The bay layout has booted under real Pi firmware at this Lemnos rev: **NO** (Lemnos b78e5317; run record: none.
  Lemnos testing/board-runs/I5i8.md P1-P10 and I5i6 point 10 are all "untested on board"). Step 4 does not run
  until this reads YES with the run record named here. The emulation cannot answer it: the harness loads the virt
  kernel and emulates the firmware, so "the firmware loads bay0" was never provable before a board.

## Gate before any step: the tools and the rehearsal

- G1. `lemnos-ctl boot-partition --add-to` (Lemnos pr/ADD-TO 1b8067a0) at the final Lemnos rev the coordinator names.
- G2. The dead-man (Atlas, components/bench-trial-deadman, bench image only) rehearsed in an emulated boot of the
  built image, both cases, before any board:
  (a) marker present, nothing stops the unit: the board restarts at N s (kmsg shows "a plain restart in N s",
      then "no stop in N s, restarting"), and the marker is gone after it;
  (b) marker absent: the unit is skipped by its condition and nothing restarts (wait past N);
  (c) marker present, `systemctl stop bench-trial-deadman` before N: nothing restarts, the marker is still there.
  The rehearsal method is OPEN (below): the emulated boot needs a machine the Raze kernel does not run on.
- G3. The coordinator has ruled on this list.

## The dead-man (what the image carries)

- /etc/systemd/system/bench-trial-deadman.service: DefaultDependencies=no, After=local-fs.target (slot-init moves
  /data into the new root before switch_root, so /data is mounted when systemd starts),
  ConditionPathExists=/data/board/trial-deadman, WantedBy=sysinit.target. Nothing orders it after lemnosd, Orion
  or the network. No Restart=.
- /usr/libexec/bench-trial-deadman: reads N from the marker (digits only; empty, 0 or anything else = 600), logs
  to /dev/kmsg, sleeps N (a plain process, so it fires even if pid 1 wedges after it started), removes the marker,
  then `systemctl reboot --force --force` (sync, then reboot(2); pid 1 is not asked).
- N: 300 for the first board boot of this image (checkpoint 7; 600 is only the unit's default); the runner writes a shorter number into the marker for later sessions.
- Covered by it and not otherwise: lemnosd ready and healthy but no login through the gadget or Orion; pid 1
  wedged while lemnosd keeps feeding the hardware watchdog. Still on the watchdog: a kernel hang before the
  unit starts (kernel feeds for 30 s from probe, then slot-init).
- The marker is removed in three ways, so no later boot is cut short: by the dead-man when it fires; by the
  runner in the new slot right after a real login (`systemctl stop bench-trial-deadman` then
  `rm /data/board/trial-deadman`); and checked absent (and removed if present) from 993 after every return.
- 993 never reads the marker (it is a new path under /data/board/; 993's stack has no unit for it).

## Which board, every time

- A board is named by its serial only: Raze #2 = a317bcbee5226d57, Raze #1 = 0708c973b00a37bc. Before EVERY
  writing step the runner reads /proc/device-tree/serial-number on the board it is logged in to and stops if it
  is not the serial the step names. Disk id 0x5056525a, PARTUUIDs 5056525a-01/-02, FAT id 0F2E-5DBC and the ext4
  UUID are the same on both boards (they come from the image): never used to tell boards apart.
- Nothing assumes a disk size or an inode count: step 0 reads p1 free space, p2's size and `df -i /data` and
  records them (Raze #1: 14.56 GiB, 9,280 inodes; Raze #2: 29.12 GiB, 18,640 inodes).
- Recorded for later (a product question, not blocking): a new image should give each board its own disk id,
  FAT volume id and ext4 UUID at install, so ids from an image never collide across boards.

## What Atlas supplies (from Gaia's D1 slot-disk build at the named Lemnos rev; not built yet)

- B: the boot-partition image `lemnos-ctl boot-partition` builds for the Raze board file, complete (bay0 filled
  and committed). Every file it holds goes to p1 EXCEPT default.txt and config.txt: slots/bay0..2/ (five files,
  overlays/README, each firmware overlay's bay file, bayinfo.rec, in every set), trial.txt, previous.txt,
  trial.rec, reason.rec, episodes.rec, SECURE.REC, `COUNT   REC`, slot-layout.toml, firmware-overlays.txt
  (without it the bay layout is read without the overlays and the sets do not match the board), tryboot.txt
  (exactly the generated bytes: kernel_watchdog_timeout=15, include trial.txt, the board's config_lines).
- D: the slot's data files from `lemnos-ctl data-partition --install BUNDLE --out DIR`: slots/<id>.erofs,
  slots/<id>.json, slots/<id>.meta. NOT its board/slots/state.env.
- The manifest's `kernel` equals the kernel release the slot runs (7.3.5-v8 at this pin; slot-init refuses a
  mismatch). The cmdline comes from `lemnos-ctl tree --slot --cmdline-slot`.
- M: a manifest of every file of B and D with its size and sha256.
- The A1 BENCH board file (its own file, named so it cannot pass for the product's; the rehearsal uses exactly it):
  the product board file plus grow_data = false (p2 was not made by Lemnos's contract and already ends at
  the disk's end) and [boot] config_lines / firmware_overlays carrying 993's two raze-device.txt lines
  (dtoverlay=vc4-kms-v3d-pi5, dtoverlay=ramoops-pi5,total-size=0x40000,record-size=0x10000,console-size=0x20000).
- The bench image carries the dead-man (above). New files on /data stay few: one slot is three files.
- For the second slot: a second bundle (B2 for set 1 is produced by --add-to on the dump, not built separately),
  D2, M2.

## Step 0: record and save 993's state (nothing changed)

- FIRST, read-only (coordinator 2026-11-20): list /data's owners before anything else. 993 has no `stat`; BusyBox
  `ls`: `find /data -xdev -maxdepth 3 \( -path /data/lost+found -prune \) -o -exec ls -ldn {} +`, then the same to
  depth 6 under /data/os and /data/log with `ls -ld` (names), and `grep -E ' /data| /var' /proc/self/mountinfo`.
  Saved to the PC. Why: what 993 keeps on /data is known from its image and its own boot on the replica (below), not
  from Raze #2: the board has run 993 since 2026-11-13 and may hold more (blobs, plugins, other owners). Stop and
  report if any entry's owner is not root, orion (901) or systemd-journal, or not in the list below.
  What 993 keeps for its daemons (its units, and its first boot on the replica 2026-11-20): lemnosd NOTHING
  (StateDirectory=lemnos is /var/lib/lemnos on the /var tmpfs); orion-node its identity (node-identity.ed25519) and
  snapshots in /var/lib/helios/orion = p2's os/helios/helios/orion (orion:orion 0750, bound at /data/helios by
  helios-data-setup); HeliOS os/helios/helios/state (root:orion 0755) and log/helios (root:orion 0755). The new slot
  does not use them: its Orion keeps /data/orion (root:orion 0750) and /data/orion/node (orion:orion 0750), so it
  starts with a new node identity; 993's dirs stay as they are and 993 finds them again on return.
- Serial check. Read and record: p1 free space, p2 size, `df -i /data`, blkid, /proc/cmdline, `uname -r`.
- First wait (read-only) until 993's board-update-confirm.service has run and exited in this boot
  (ExecMainExitTimestampMonotonic set, not active/activating; bounded at 600 s): it rewrites /data/board/slots/ minutes
  after login on every boot (see step 4's return steps), and a copy that overlaps it is half-written.
  If the unit has not finished inside the 600 s bound, step 0 STOPS with nothing written (nothing is copied, no
  later step runs) and the unit's state is reported.
- Copy to /data/a1-backup/step0/ with a sha256 list: p1's config.txt, default.txt, trial.txt, tryboot.txt, and
  the sha256 of every file under p1's slots/993/; /data/board/slots/{booted,fallbacks,state.env,state.json,*.meta}
  and a listing of /data/board/slots/ and /data/board/.
- Lemnos b78e5317 (ROUTE-A-WRITES) writes NOTHING under /data/board/slots on route A: slot-init writes no trial-init,
  trial-started, booted, boot-unverified mark, meta, state or last-failure.log, and lemnosd's system-slots writes
  nothing at start or on any tick and refuses every writing call (FAILED_PRECONDITION). An unplanned return (power
  cut in the new slot, the dead-man, any fallback) therefore finds 993's own files as step 0 left them. The
  "before" for this (rehearsal on b210934d, 2026-11-20): slot-init wrote booted, trial-init and trial-started;
  lemnosd wrote nothing; 993 booted on its own state.env, its initramfs put booted back and its confirm removed
  the two markers.
- Note: 993's <slot>.meta LAST_BOOT may move during step 0 itself (993 rewrites it minutes after every boot,
  slots:963): the copy keeps whichever value it read; the return check leaves that line out (step 4's return steps).
- Check: backup hashes match. Back: nothing to undo.

## Step 1: relabel p2

- Serial check. `e2label /dev/mmcblk0p2 lemnos-data`. Lemnos's slot-init accepts only that label; 993 finds
  /data by atlas.data=PARTUUID=5056525a-02 (the label "data" is only a fallback behind that word).
- Check: blkid shows exactly one LABEL="lemnos-data"; 993's /proc/cmdline still has the PARTUUID word.
- Back: `e2label /dev/mmcblk0p2 data`.

## Step 2: the new slot's files on /data

- Serial check. Copy D's three files into /data/slots/ (beside 993's; Lemnos lists 993's slot and never deletes
  it).
- Check: sizes and hashes from M; `df -i /data`.
- Back: remove exactly those three files.

## Step 3: the new files on p1 (all on a dump on the PC, then a sector write-back)

Tool: `lemnos-ctl boot-partition --add-to` (Lemnos pr/ADD-TO 1b8067a0, docs/slots.md "Adding to an existing boot
partition"; the rev is the coordinator's final Lemnos rev). It never writes DUMP, refuses a block device, and its three
outputs must be new files.

1. Serial check. Dump p1 to the PC: `ssh root@<993 address> dd if=/dev/mmcblk0p1 bs=1M > p1.before.img`
   (82,837,504 B); `sha256sum p1.before.img` recorded.
2. On the PC:
   `lemnos-ctl boot-partition --add-to p1.before.img --layout-of <A1 bench board file> --socs <socs> --slot <new id>
    --payload <B's payload dir> --firmware <B's firmware dir> --set 0 --replace-trial
    --out p1.new.img --changed-out p1.changed --undo-out p1.undo`
   (--replace-trial: trial.txt in Lemnos's form naming set 0, so the tryboot reads the new set; 993's state says no
   trial, so its trial.txt is idle. default.txt is never created or touched; previous.txt is added naming bay0.)
3. On the PC: `lemnos-ctl boot-partition --verify p1.new.img --layout-of <A1 bench board file> --socs <socs>`. Go on
   only if there are EXACTLY these two BAD lines and no other:
   - `default.txt: record file is 55 bytes, expected 1024: the image is not ours`
   - `config.txt: record file is 405 bytes, expected M: ...` (993's config.txt is 405 bytes; M the fixed text's)
   (Without --replace-trial there would be four: trial.txt 55 and tryboot.txt 318 as well.) The verify does not read
   SECURE.REC or COUNT REC. `data-partition --verify --boot-image` cannot be used on this mixed partition.
4. On the PC, read-only (mtools on each image): 993's 44 files (config.txt, default.txt, trial.txt's old bytes are
   in p1.undo, tryboot.txt, slots/<993 id>/...) byte-identical between p1.before.img and p1.new.img except the two
   replaced texts (trial.txt, tryboot.txt), with the same directory entries and cluster chains; `fsck.fat -n
   p1.new.img` clean; the free space is still one run.
5. Serial check. On the board, before writing: `sha256sum /dev/mmcblk0p1` equals p1.before.img's (nothing changed
   since the dump; else stop and start again at 1).
6. Write back the ranges of p1.changed from p1.new.img to /dev/mmcblk0p1 at the same sectors (512-byte sectors from
   the partition start), in THIS order, so every intermediate state still reads as before for 993:
   (a) new file and new directory data; (b) the second FAT copy; (c) the first FAT copy; (d) directory and
   root-directory sectors. Each range: `dd if=p1.new.img of=/dev/mmcblk0p1 bs=512 skip=S seek=S count=N
   conv=notrunc,fsync`, then read it back and compare with its listed sha256-after. `sync` at the end.
7. Re-dump p1 (p1.after.img) and compare the WHOLE partition to p1.new.img: byte-identical, or stop and go back.
- Back (way back of step 3): `lemnos-ctl boot-partition --undo p1.undo --of p1.after.img --out p1.restored.img`
  (on the PC), check p1.restored.img's sha256 equals p1.before.img's, then write back the same ranges from
  p1.restored.img in the reverse class order (d, c, b, a), read back each, re-dump and compare the whole partition
  to p1.before.img.
- Limit of the undo (Lemnos's): only sectors whose bytes changed are listed. A new file's sector that was zero before
  and after the add is not listed; if the new slot later writes there, those bytes stay behind, in clusters that are
  free again after the undo. 993's files and all FAT metadata are restored exactly; the whole-partition compare after
  an undo is therefore against p1.before.img only if no new-slot boot wrote into p1 (it cannot in A1: no
  boot-partition write is possible with 993's default.txt in place); otherwise the compare is of 993's files and the
  metadata, and the leftover sectors are named.
- Sizes on the real dump (Lemnos): 33,267 free clusters in one run before, 14,668 after three bay sets, still one
  run; one set about 12.7 MB at the reference bay sizes.

## Step 4: the first boot of the new slot

- Serial check. TRIAL stays empty; no step writes state.env. From 993:
  `printf '300\n' > /data/board/trial-deadman; sync; reboot "0 tryboot"` (300 s: checkpoint 7).
- Success: from the PC, `ssh root@<lemnos0 address>` through Orion's SSH. First act: checkpoint 7's
  temperature loop. Then `systemctl stop bench-trial-deadman; rm /data/board/trial-deadman; systemd-run --on-active=600 systemctl reboot`
  (coordinator 2026-11-20: MAXIMUM SESSION on the trial slot 10 minutes from that stop, whatever the temperature. Once
  the dead-man is stopped the temperature loop is the only other bound, so the session arms its own reboot timer,
  as step 5 does; the operator also reboots to 993 by hand at 10 minutes if it has not fired, and earlier when the
  checkpoints are done). Then `cat /proc/device-tree/serial-number`
  (the step's serial), `uname -r`, `lemnos-ctl list`.
  Then the step-0 listing again for os/helios/helios/orion, os/helios/helios/state and log/helios (`ls -ldin`):
  same inodes, owners and modes as step 0's (nothing re-created or re-owned), lemnosd and orion-node active. End with a plain `systemctl reboot` (to 993).
- Before the reboot, the boot's own state, read before anything is stopped or started (coordinator 2026-11-20):
  `systemctl is-system-running --wait`; lemnosd, lemnos-orion, orion-comm and orion-node active with NRestarts=0;
  `systemctl --failed` empty. lemnosd's process runs as lemnos (Uid/Gid 900 in /proc/<MainPID>/status), and
  /run/lemnos is `drwxr-x--- lemnos:lemnos 900:900`.
- Route A, Lemnos b78e5317 (ruling A2, 2026-11-20): lemnosd sees the boot records as not ours and leaves the slots
  state alone; the trial is not in its state (993's state.env, TRIAL=''). Past 45 s of uptime, check:
  `lemnos-ctl call slots list` and `lemnos-ctl call slots info` each carry `"foreign":"foreign-unknown: the boot records can't be read to say whose they are; the slots state is left untouched until they can: /dev/mmcblk0p1: Permission denied (os error 13)"` (ruling (a), final: lemnosd as lemnos cannot open the node; slot-init, as root, names "the image is not ours");
  list has `"trial":null` and `slot_default` 993's slot; `lemnos-ctl list` shows the same foreign-unknown text as the
  slots device's degraded health; no confirm line in `journalctl -u lemnosd -o cat`; on p1 (mounted read-only),
  default.txt byte-identical to step 0's copy and trial.txt as step 3 wrote it. slot_active now shows 993's slot
  (slot-init no longer writes `booted`): which slot runs is read from slot-init's console line, `slot-init: starting
  slot <new> (…, on trial)`, whose line for the records ends "state dir left untouched: nothing is written under board/slots this boot", and from /proc/cmdline.
  The whole of /data/board/slots/ is byte-identical to step 0's copy (listing and hashes; the .meta apart from 993's
  LAST_BOOT). Then the plain `systemctl reboot` (to 993) is the expected end.
  THIS IMAGE IS FOR THE ROUTE-A TRIAL ONLY (coordinator 2026-11-20): at Lemnos b78e5317, slot management through
  lemnosd (install, set_default, trial, confirm, remove, pin) does not work under the real unit on ANY disk: after its
  drop to lemnos it cannot open the boot partition's node (root:disk 0660) and /data is read-only to it
  (ProtectSystem=strict; the image ships no ReadWritePaths under /data for it: its drop-ins are 90-a1-bench-board.conf
  and gadget.conf, whose only ReadWritePaths is -/sys/kernel/config/usb_gadget). On route A it reports
  `foreign-unknown … Permission denied (os error 13)` and writes nothing. Do NOT use this image for a full-image
  install; that waits for Lemnos OWNER-PROBE-ACCESS.
  Fan (coordinator 2026-11-20, build 15): this image has NO PWM fan. At Lemnos b78e5317 lemnosd's PWM fan never
  binds under the real unit (ProtectKernelTunables=yes, the sysfs pwm export gets EROFS) and there is no kernel fan
  fallback, so the board files declare no fan, no fan-control and no PWM1: nothing muxes GPIO45, the firmware's
  `gpio=45=op,dh` drives it high from power-on and the board overlay's kernel gpio-hog (lemnos-hog-45, output-high)
  holds it for the whole boot. High is full speed (993's raze-fan overlay: normal polarity). The fan runs at full
  speed by construction; LED control through lemnosd stays unproven under the real unit at this rev (the ring needs
  the module's PIO route).
  HARD RULE: do not run `lemnos-ctl confirm` (or `lemnos-ctl call slots confirm`), from any tool or UI (lemnos-ctl,
  an Orion peer through lemnos-orion, Atlas, HeliOS), while on the trial slot. No step calls confirm on route A: a
  confirm on route A is refused by name at b78e5317 (FAILED_PRECONDITION, nothing written), and wrote into 993's /data
  before it (b210934d resolve.rs:372-450): no step depends on which. Nor `previous` or `set_default`. In emulation
  ONLY, the plain ending makes one confirm call as a negative proof (refused, the state dir and p1 byte-identical). A confirmable install needs a boot
  partition laid out by our image builder (a full-image install, a different route).
- FIRST-FLASH CHECKPOINTS (coordinator 2026-11-20).
  Only the board can show (the rehearsal cannot, by its emulation):
  1. The USB gadget binds: the gadget's device (`gadget`, usb-gadget on udc 1000480000.usb) is running ok in
     `lemnos-ctl list`.
  2. lemnos.ko loads (the harness's virt kernel has no Rust symbols, so it never loads there): in the console log,
     `lsmod`, and lemnosd's device list showing the devices that need /dev/lemnos (the PiSP blocks).
  3. The ttyGS0 login (the getty is real only with the gadget): a login prompt on ttyGS0 from the PC.
  4. Boot mode and the mmc node from the firmware: the boot source `rpi-boot-mode-1=/soc@107c000000/mmc@fff000` as the
     firmware gives it, and lemnosd's slots device running ok with it (emulation stands in for both).
  5. The overlay marker applied by the firmware (the ramoops-pi5 and board overlays from firmware-overlays.txt):
     present in /proc/device-tree as the slot's record states.
  6. The watchdog handover on bcm2835-wdt at /soc@107c000000/watchdog@7d200000: the console log shows lemnosd took
     the watchdog, and the board stays up past the reset window. NOT PROVEN IN EMULATION (build 15v, 2026-11-20): on the emulated
     i6300esb (variant root differing in that one board-file line) slot-init armed 15 s and fed it until switch_root
     (last feed about 4 s), lemnosd only started about 14 s under QEMU TCG, and the guest was reset by the watchdog
     before lemnosd took it (serial log ends mid-boot, no login). So the board is the first place the handover can
     show: the gap from switch_root to lemnosd's first feed must stay under 15 s there. If the board resets within
     about 20 s of the trial boot, that is this gap; the board then returns to 993 by itself (the reset is a plain
     restart, TRIAL empty). The flashed root's endings run with the emulated watchdog left out, --no-watchdog.
  7. Temperature and fan (coordinator 2026-11-20; order and loop ruled 2026-11-20). FIRST after the login, before
     the dead-man stop and every other checkpoint, start this loop and leave it running for the whole session (a
     line every 60 s: the time, every zone's temperature in millidegrees, the fan line's state):
     `while :; do echo "$(date +%T) $(for z in /sys/class/thermal/thermal_zone*; do printf '%s=%s ' $(cat $z/type) $(cat $z/temp); done)$(grep -F "|the fan header's line" /sys/kernel/debug/gpio | tr -s ' ')"; sleep 60; done &`
     (the image has no vcgencmd; read-only throughout). Note at the first line whether the fan is spinning: there is
     no tach on this fan (dev's raze-fan ran `notach`), so by ear at the bench, plus `lemnos-ctl list`'s cpu-thermal
     row (build 15 declares no fan device).
     LIMIT: any reading of 70 °C (70000) or more means the plain `systemctl reboot` to 993 at once, no further check;
     report the readings. 70 °C is 10-15 °C under the CM5's throttle points (80-85 °C) and is where 993's own curve
     already runs the fan near full (pwm-fan trips 50, 60, 67.5, 75 °C at duty 179, 212, 245, 255 of 255).
     DEAD-MAN for the board run: 300 s (the marker's N in step 4). It bounds the boot with no login, the only stretch
     no reading covers. What an idle CM5 with a stopped fan reaches in 300 s: UNKNOWN (no figure on any of our
     boards). So the window is held at the dead-man's minimum: bench-trial-deadman has no floor of its own (any
     positive N; empty, 0 or non-numeric means 600), and the least N that still leaves room for a login is the
     board's boot-to-SSH time, which no board has measured for this image (emulation: about 3 min under the
     harness, and the rehearsal's own marker is 120 s). 300 s is that minimum with margin; a slower boot only means
     the dead-man returns the board to 993, a failed trial and no harm.
     EXPECTED (build 15): the fan at full speed for the whole trial, by ear, and in the first loop line's evidence
     the loop's gpio field: in /sys/kernel/debug/gpio, pinctrl-rp1's line 45 held by the hog, consumer "the fan
     header's line: held high, the fan at full speed (no " (the overlay's lemnos-hog-45 line-name), out, hi: the hog exists only if the firmware applied the board overlay, so this is also the
     overlay's proof. A STOPPED FAN IS A STOP, like the temperature limit: the plain `systemctl reboot` to 993 at
     once, and report it. Line 45 not hogged high is a finding to report (the overlay not applied, or the pin
     taken), even with the fan running on the firmware's level alone.
     POLARITY (993's overlay, which demonstrably cools this board): pin high = full speed, low = stopped.
     raze-fan.dtbo on 993's p1 (dev raze-fan-overlay.dts:56-58) sets `pwms = <&rp1_pwm1 3 41566 0>`, normal
     polarity, overriding the CM5 base tree's inverted flag; its cooling levels 179, 212, 245, 255, 255 rise with the
     trips. Dev's comment records the inverted setting leaving a Raze Gen 1's fan "basically not moving".
     Build 14 had the PWM declared (the pin muxed to PWM1 with channel 3 never enabled: a stopped fan expected);
     build 15 removes it (commit 591f51e8).
  DON'T: no slot management through lemnosd on this image (install, set_default, trial, confirm, remove, pin), and
  no full-image install with it: at Lemnos b78e5317 it does not work under the real unit on any disk (see step 4).
  DON'T: no `off` call (nor power.reset) on usb-c-power or usb-a-power during the trial, from any tool or UI
  (coordinator 2026-11-20): only an explicit off lowers a line (pinctrl keeps the level on failure, release or
  daemon stop), and which Raze connector the gadget's controller (udc:1000480000.usb, our only route in) is wired
  to is not recorded (dev raze-device.txt: "not recorded yet").
  DON'T: no confirm on the trial slot, from any tool or UI (lemnos-ctl, an Orion peer, Atlas, HeliOS): a confirm
  with no trial writes into 993's /data (see step 4's hard rule). Nothing on the image calls it on its own (build 13
  root, read 2026-11-20).
  By design, not checked on this route: route A never sets a trial in lemnosd's own state, so health:gadget and
  auto-confirm are unexercised here; they belong to a full-image install.
  Expected, not a fault: peers will see a new Orion node while the trial slot runs (its Orion keeps /data/orion,
  not 993's os/helios/helios/orion, which it leaves untouched).
  Emulation stand-ins used by the rehearsal: the virt kernel (7.3.5-v8, no Rust) instead of the slot's kernel, the
  i6300esb instead of bcm2835-wdt, no UDC, seccomp off, the boot-mode and mmc facts given by the harness.
- No login: the board returns to 993 by itself: the dead-man at 300 s; slot-init's plain restart on a failure
  before switch_root (rescue: 30 s, then the same); lemnos-good-boot's reboot-force if lemnosd is never ready
  (300 s); the hardware watchdog on a hang. Nothing of Lemnos sets tryboot again with TRIAL empty.
- Back in 993 after ANY new-slot boot (the return steps):
  - Serial check. Check /data/board/trial-deadman is absent; remove it if present.
  - At 993's login, BEFORE the confirm wait and any restore: /data/board/slots/ byte-identical to step 0's copy
    (listing and hashes; the .meta as below). Nothing Lemnos-named is there; anything found is a failure. Again after
    993's board-update-confirm (its rewrites are equal bytes).
  - Belt (kept, nothing expected): restore booted, state.env and state.json from /data/a1-backup/step0/; move any
    file not in step 0's listing into /data/a1-backup/boot-<n>/ as evidence.
  - Check after the belt: /data/board/slots/ byte-identical to step 0 (listing and hashes), except <slot>.meta's
    LAST_BOOT line; 993's `slots status` as in step 0;
  - <slot>.meta (coordinator 2026-11-20): 993 rewrites its own LAST_BOOT line on EVERY boot, minutes after it
    (/usr/lib/board/slots:963, `meta_set "$_run" LAST_BOOT`), whether or not a trial ran, so a byte compare depends on
    which side of that rewrite it lands (the rehearsal hit both). Read each .meta ONCE (step 0's copy and the live
    file in one session), then: every line but LAST_BOOT byte-identical to step 0's, and LAST_BOOT no earlier than
    step 0's. LAST_BOOT is 993's own running record: no restore puts the old value back.
    `fallbacks` and /data/slots untouched (Lemnos does not write them).
  - Before the restore and the check, wait (read-only) until 993's board-update-confirm.service has run and exited
    in this boot (ExecMainExitTimestampMonotonic set, not active/activating; bounded at 600 s). It runs minutes after
    login on EVERY boot (Type=exec, After=multi-user.target; /usr/lib/board/slots:868-975 cmd_confirm) and writes
    <run>.meta GOOD and LAST_BOOT, fallbacks, then state.env and state.json via .tmp + mv (slots:149-155), with the
    same values. A check that overlaps it sees a half-written folder (rehearsal build 13, pid1: state.env.tmp).
    If it has not finished inside the bound, the return steps stop before the restore and report it.

## Step 5: board-check sessions, only after step 4 gave a login

- Serial check. The same as step 4 with a shorter N in the marker: `printf '<session limit>\n' >
  /data/board/trial-deadman; sync; reboot "0 tryboot"`. In the new slot, after the login: stop the unit, remove
  the marker, arm the runner's own `systemd-run --on-active=<limit> systemctl reboot` (the same idea, under
  systemd), run the checks, plain reboot. The return steps of step 4 after each session.

## The second slot, in bay1 (before anything like A2)

- Step 2 for D2. Step 3 with `--add-to` installing set 1 from the second bundle; the same verify (the same two BAD
  lines), the same hash compares of 993's files AND of bay0's set, the same write-back and re-dump.
- Its boot: `--add-to` on a fresh dump of the board's p1 (which then holds set 0) with `--set 1 --replace-trial`:
  set 1 is added (set 0 byte-identical, Lemnos checked) and trial.txt names bay1; the same verify, compares,
  write-back order and re-dump. TRIAL stays empty. Then step 4
  (marker, `reboot "0 tryboot"`), the login, the return steps. Lemnos's reader needs bay1's five files, overlays and
  a committed bayinfo.rec matching them, and /data's manifest matching too.
- After the second slot is proven: trial.txt back to the bay that the next session names, or to step 0's bytes
  when the round ends (its old text named 993's set).

## Finding: the first board run, Raze #2, 2026-11-20 (build 15, Lemnos b78e5317)

- Steps 0-3 ran on Raze #2 and passed every check (evidence: board/run-steps.log, board/run/, board/step0/).
- Step 4: after `reboot "0 tryboot"` (PC 23:27:20) the board never enumerated on USB. The user saw the Pi bootloader's
  error blink (a count of flashes, a pause, repeated for ever) with the fan running: the firmware read tryboot.txt
  (gpio=45 applied) and HALTED before Linux. No return path covers a firmware halt (the dead-man, slot-init, the
  watchdog all need Linux; REBOOT_ON_FATAL_ERROR is not set), so the user power-cycled it; it came back on 993.
  Nothing was written by the trial (p1 = step 3's re-dump, /data/board/slots = step 0's, the marker still 300,
  /data/orion never created).
- Leading cause (accepted, coordinator 2026-11-21): bay0's kernel8.img is a gzip stream (6,912,556 B) zero-padded to
  its 10 MiB bay file, so the gzip trailer's ISIZE (the file's last 4 bytes) reads 0. 993's kernel on the same p1 is
  gzip too and boots, stored at its exact size with its real trailer. Other untested differences from 993's slot
  directory: no file under the firmware's default dtb name (device_tree=board.dtb only; the os_prefix viability
  rule), cmdline.txt space-padded to 4 KiB, zero padding after the dtb and each dtbo, no overlay_map.dtb/bcm2712d0.dtbo.
- Ruling: Lemnos designs a layout in which no file the firmware parses carries padding it could misread. Until it has
  booted on a board (the precondition above), step 4 is blocked.
- D at this rev is 12 files (.erofs, .json, .boot and 9 under slots/bootfiles/), not the three of the text above;
  step 2 copies them without the PC's owners (the first copy left 1000:1001 on them and on /data/slots, set back to 0:0).

## Abandoning at any point

- From 993: the return steps of step 4; then step 3's way back (sector write-back from p1.before.img, whole re-dump
  compare); step 2's (remove D's three files); step 1's (label "data"). Then 993's view equals step 0's: p1
  byte-identical to p1.before.img, /data/board/slots/ as in step 0, label "data", no marker.

## Not in this round: A2 (its own list later)

- To be written separately: the order (previous.txt and state.env before default.txt); a rename through a mounted
  vfat can move config.txt's clusters, while Lemnos's verify wants it exact and contiguous; after A2 there is no
  fallback to 993; the count rung (`[boot_count>3]`) has never run on a board. Until then both new slots boot only
  under 993's config.txt.

## Open

- O1 (G1): --add-to is in Lemnos pr/ADD-TO 1b8067a0; step 3 is written to its usage. Open only: the final rev.
- O2 (G2): the emulated boot for the dead-man rehearsal. The Raze's bcm2712 kernel does not run under QEMU (no Pi 5
  machine). Proposal: qemu-system-aarch64 -M virt with the built image's root (erofs) and a /data disk with or
  without the marker, on a generic arm64 kernel built from the same tree (the bcm2712 tree builds a virt-capable
  defconfig) or Gaia's; the unit, the script and systemd are the image's own. To be agreed with the coordinator
  and Gaia (which kernel, who builds it); downloads only on the coordinator's OK.
- O3 (step 0): what 993 does with Lemnos's rewritten files on an unplanned return (read-only read on the board).
