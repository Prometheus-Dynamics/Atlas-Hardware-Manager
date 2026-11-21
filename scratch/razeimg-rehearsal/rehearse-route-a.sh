#!/usr/bin/env bash
# Rehearsal of route (A) (route-A-steps.md v3) on the Raze #2 replica, in Gaia's emulated boot
# (Gaia-Image-Builder scripts/qemu-rpi-slots.sh, -M virt, no firmware), BEFORE any board. Nothing here touches a board.
#
#   rehearse-route-a.sh ENDING
#     ENDING: login     the new slot gives a login; its auto-confirm is refused by name ("not ours": route A cannot
#                       confirm, by design); the runner stops the dead-man; plain reboot back to 993
#             nologin   nobody logs in; the dead-man returns the board to 993 at N; the marker is gone after
#             stopped   marker present, unit stopped at N/2: nothing fires for > N; plain reboot
#             nomarker  no marker: nothing fires for > N; plain reboot
#             pid1      login, dead-man NOT stopped, `kill -STOP 1` (systemd frozen): the reboot still fires at N
#             plain     the new slot is left by a plain `reboot` from a login (the marker removed first)
#             handover  ONLY on the handover variant (its board file's one changed line points hw-watchdog at
#                       watchdog0, the emulated i6300esb; compare-roots.py proves that is the only difference):
#                       slot-init arms the watchdog, lemnosd takes it over and the guest stays up well past slot-init's
#                       last feed + the timeout; stopping lemnosd then resets the guest within the timeout + margin;
#                       the harness is started again on the same disk and the board is back on 993
#             handover-debug  the handover variant with QEMU's watchdog action "debug" (HARNESS_QEMU_ARGS="-action
#                       watchdog=debug"): a firing is logged ("watchdog: timer fired") instead of a reset, so one boot
#                       answers, in order: does lemnosd (as lemnos, real unit) open and feed the watchdog; does no firing
#                       follow for three timeouts once it feeds; does stopping lemnosd make it fire. Emulation only.
#             all       every ending above but handover, in that order, each from a fresh copy of the replica
#
# Each ending runs the step list exactly as the runner will (steps 0-4 and the return steps), then shows 993 boots,
# its state files are restored, and it can stage again (step 3's dump + --add-to + --verify succeed on the returned
# p1). Every check that fails stops the run with the step's name: this script is the list, executable.
#
# What the emulation cannot show (the board run is the first time for these): the BCM2835 hardware watchdog (the
# virt machine has an i6300esb; the hang case is not rehearsed), the boot count, every Pi device (RP1, CSI, the USB
# gadget: the login is over the virt NIC, not lemnos0 over USB), the Pi firmware itself (Gaia's harness does its
# config.txt/tryboot part), the eMMC (p1 writes go to a sparse file), and 993's own kernel (both slots boot Gaia's
# virt kernel, so neither slot's kernel-release check is the board's).
# Stand-in S2 (coordinator 2026-11-18): 993 boots the virt 7.3.5 4K kernel, not its own (7.2.9-v8-16k, 16K pages).
# REPLICA is work/replica-s2.img: the replica with ONE change, /data/slots/<993>.json "kernel" set to 7.3.5-v8 (p1 is
# byte-exact, sha256 of sectors 0-163839 equal to the replica's); the 993-state baseline (step 0) is taken on it.
# The virt kernel has no CONFIG_RUST: lemnos.ko is not loaded in the new slot (the gcc D1 build has none anyway).
# The new slot is reached on its serial console, not over SSH: a login over lemnos0 (Orion's SSH on the USB gadget) is
# NOT proven by emulation; the first real board run on Raze #2 proves it. "login" = console login, then the runner
# stops the dead-man.
#
# Inputs (environment; all required unless a default is given):
#   HARNESS   Gaia's qemu-rpi-slots.sh at the commit Gaia names (default: the 1c2b9dc9 copy beside this: it sets /chosen/bootloader/boot-mode and moves the disk node to the slot's os.src.rpi-boot-mode-N path, which Lemnos slot-init needs; 629ce98a did neither)
#   KERNEL    Gaia's virt kernel Image for rpi-7.3.y b005af66 (Gaia supplies it)
#   REPLICA   the replica disk (default replica/disk.img beside this; its RECORD.txt says what it is)
#   CTL       lemnos-ctl at the final Lemnos rev (with --add-to)
#   SOCS      that rev's socs/ directory
#   BOARD_A1  the A1 BENCH board file (grow_data = false), exactly the file the board run uses
#             (step 3's --layout-of only; on the handover variant too: its root's one changed line, watchdog0, is not
#             boot layout, and lemnos-ctl refuses that name for the board's tree, so the layout is the flashed file's)
#   PAYLOAD FIRMWARE   B's payload and firmware directories (Gaia's D1 build at that rev)
#   DATA      D's directory (slots/<id>.erofs, .json, .meta from `data-partition --install`)
#   SLOT      the new slot's id
#   N         the dead-man's seconds for these boots (default 120: small, so a run takes minutes, not 10)
#   WORK      scratch directory (default work/ beside this)
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HARNESS=${HARNESS:-$here/qemu-rpi-slots.6efa33ed.sh}  # Gaia 6efa33ed's runner (git show 6efa33ed:scripts/qemu-rpi-slots.sh): its own overlay-marker emulation (2c05550f), --no-watchdog (f4187cf9)
REPLICA=${REPLICA:-$here/work/replica-s2.img}
N=${N:-120}
WORK=${WORK:-$here/work}
SERIAL=${SERIAL:?set SERIAL in the scratch env (the board serial the harness puts in the device tree; the steps check it)}
GUEST=10.77.0.50
for v in KERNEL CTL SOCS BOARD_A1 PAYLOAD FIRMWARE DATA SLOT; do
  [[ -n ${!v:-} ]] || { echo "rehearse: $v is not set (see the header)" >&2; exit 2; }
done
ending=${1:-}
[[ $ending =~ ^(login|nologin|stopped|nomarker|pid1|plain|handover|handover-debug|all|probe)$ ]] || { sed -n '2,14p' "$0" >&2; exit 2; }

die() { echo "rehearse [$ending] FAIL: $*" >&2; exit 1; }
say() { echo "rehearse [$ending] $(date +%T) $*"; }

state=
hpid=
# The guest is reached inside the harness's network namespace; known_hosts are per slot and per run (993's OpenSSH and
# the new slot's Orion SSH have different host keys on one address), never ~/.ssh/known_hosts.
gssh() {  # <993|new> <command...>
  local who=$1; shift
  # Paced: at least 2 s between connections (993's sshd refused the source after a burst of ~10, probe 2026-11-19).
  # The last connection's time is in a file: gssh often runs in a $(...) subshell, where a variable would not last.
  local now last; now=$(date +%s%N); last=$(cat "$state/.gssh-last" 2>/dev/null || echo 0)
  if (( now - last < 2000000000 )); then sleep "$(( (2000000000 - (now - last)) / 1000000 ))e-3"; fi
  date +%s%N > "$state/.gssh-last"
  # ConnectTimeout covers the banner exchange: 993 emulated (TCG, nice 10) at ~100 s of uptime with its confirm running
  # took longer than 5 s to send its banner (build 13 pid1, 2026-11-20: "timed out during banner exchange"). 30 s.
  nsenter -t "$(cat "$state/netns.pid")" -U -n ssh -F /dev/null -i "$HOME/.ssh/id_ed25519" \
    -o UserKnownHostsFile="$state/known_hosts.$who" -o StrictHostKeyChecking=accept-new -o ConnectTimeout=30 \
    -o BatchMode=yes "root@$GUEST" "$@"
}
# The new slot is reached on its serial console (the harness's console.sock): its SSH is Orion's on lemnos0, the USB
# gadget, which the virt machine does not have, and it configures no eth0. root logs in there with no password (the
# image's console rule, raze.fragment). con.py logs in when it sees a prompt and runs one command between markers.
gnew() {  # <command...>   (CT: seconds, default 60)
  python3 -I "$here/con.py" "$state/console.sock" "${CT:-60}" "$*"
}
gscp_from() {  # <993|new> <remote command> > local   (a stream, e.g. dd of p1)
  gssh "$@"
}
boots() { local n; n=$(grep -c '^== boot ' "$state/harness.log" 2>/dev/null) || true; echo "${n:-0}"; }
last_boot_tryboot() { grep '^== boot ' "$state/harness.log" | tail -1 | grep -q '(tryboot 1)'; }
# Waits until a login works on <who>, at most <s> seconds; returns 1 on timeout.
wait_login() {  # <993|new> <s>
  local t=0
  if [[ $1 == new ]]; then
    until CT=20 gnew true >/dev/null 2>&1; do sleep 3; t=$((t + 23)); [[ $t -lt $2 ]] || return 1; done
    return 0
  fi
  until gssh "$1" true 2>/dev/null; do sleep 3; t=$((t + 3)); [[ $t -lt $2 ]] || return 1; done
}
# Waits until the harness logs boot number <n>; returns 1 after <s> seconds.
wait_boot() {  # <n> <s>
  local t=0
  until [[ $(boots) -ge $1 ]]; do sleep 2; t=$((t + 2)); [[ $t -lt $2 ]] || return 1; done
}
serial_check() {  # <993|new>
  local got
  if [[ $1 == new ]]; then got=$(gnew "tr -d '\\000' < /proc/device-tree/serial-number"); else got=$(gssh "$1" cat /proc/device-tree/serial-number | tr -d '\0'); fi
  [[ ${got//$'\r'/} == "$SERIAL" ]] || die "serial check on $1 ('$got')"
}
on_993() {  # asserts the running slot is 993 (no tryboot this boot, and 993's slot words on its command line)
  last_boot_tryboot && die "expected a plain boot into 993, the harness booted a tryboot"
  gssh 993 cat /proc/cmdline | grep -q 'dev\.993' || die "the running slot is not 993"
}

start_harness() {
  state=$WORK/$ending
  rm -rf "$state"; mkdir -p "$state"
  # --ssh-key none: the harness would otherwise rewrite board/authorized_keys on the FAT (p1 must stay the board's
  # bytes); the replica's key is this PC's id_ed25519.pub already (checked 2026-11-18).
  # --append net.ifnames=0: the virtio NIC stays eth0, the name a Pi's Ethernet has and 993's network config matches
  # (enp0s1 otherwise: no address, no ssh). Gaia 629ce98a's runner added it itself; 1c2b9dc9's passes the line as is.
  # HARNESS_ARGS: extra runner options, e.g. Gaia's leave-the-watchdog-out option for the flashed build's endings
  # (coordinator 2026-11-20); empty for the handover variant, which needs the emulated watchdog.
  "$HARNESS" --state "$state" --fresh --kernel "$KERNEL" --net netns --serial "$SERIAL" --ssh-key none --append net.ifnames=0 ${HARNESS_ARGS:-} "$REPLICA" ${HARNESS_QEMU_ARGS:+-- $HARNESS_QEMU_ARGS} \
    > "$state/harness.log" 2>&1 < /dev/null &
  hpid=$!
  wait_boot 1 60 || die "the harness did not start a boot (log: $state/harness.log)"
  wait_login 993 300 || die "no login on 993's first boot"
  on_993
}
# After a guest reset (QEMU exits under -no-reboot and the runner says "== stopped"): the runner again on the SAME
# disk (no --fresh), its log appended so the boot count goes on.
restart_harness() {
  local t=0
  while kill -0 "$hpid" 2>/dev/null && [[ $t -lt 30 ]]; do sleep 1; t=$((t + 1)); done
  kill -0 "$hpid" 2>/dev/null && die "the runner did not end after the guest reset"
  "$HARNESS" --state "$state" --kernel "$KERNEL" --net netns --serial "$SERIAL" --ssh-key none --append net.ifnames=0 ${HARNESS_ARGS:-} "$REPLICA" ${HARNESS_QEMU_ARGS:+-- $HARNESS_QEMU_ARGS} \
    >> "$state/harness.log" 2>&1 < /dev/null &
  hpid=$!
}
stop_harness() {
  [[ -n $hpid ]] || return 0
  # Bounded: 993 powers off over ssh; if ssh is refused (or the new slot is up) the harness is stopped by its pid after
  # 90 s (the first probe hung here forever when sshd refused the poweroff).
  timeout 20 nsenter -t "$(cat "$state/netns.pid")" -U -n true 2>/dev/null && timeout 20 gssh 993 poweroff 2>/dev/null || true
  local t=0
  while kill -0 "$hpid" 2>/dev/null && [[ $t -lt 90 ]]; do sleep 2; t=$((t + 2)); done
  # unshare outlives a SIGTERM (the second probe): QEMU is asked to quit over its own QMP socket, then the harness
  # ends with it (--kill-child).
  if kill -0 "$hpid" 2>/dev/null; then
    echo "rehearse: the harness did not stop in 90 s: QMP quit" >&2
    python3 -I "$here/qmp-quit.py" "$state/qmp.sock" 2>/dev/null || true
    t=0; while kill -0 "$hpid" 2>/dev/null && [[ $t -lt 30 ]]; do sleep 1; t=$((t + 1)); done
    kill -0 "$hpid" 2>/dev/null && die "the harness (pid $hpid) did not stop after QMP quit"
  fi
  wait "$hpid" 2>/dev/null || true; hpid=
}
trap stop_harness EXIT

# --- the steps (route-A-steps.md v3) ---------------------------------------------------------------------------
step0() {
  say "step 0: record and save 993's state"
  serial_check 993
  # p1 is not mounted on 993 (its console log: only vda2 and the dm root): it is read from a dump on the host with
  # mtools, nothing mounted on the board. The p1 record stays in the state directory (scratch, never a repository).
  local dev f; dev=$(p1dev)
  gssh 993 "dd if=$dev bs=1M status=none" > "$state/p1.step0.img"
  mkdir -p "$state/step0-p1"
  for f in config.txt default.txt trial.txt tryboot.txt; do
    mcopy -n -i "$state/p1.step0.img" "::$f" "$state/step0-p1/$f" 2>/dev/null || echo "$f absent" >> "$state/step0-p1/ABSENT"
  done
  [[ -f $state/step0-p1/config.txt && -f $state/step0-p1/default.txt ]] || die "step 0: 993's p1 has no config.txt or default.txt"
  mdir -/ -i "$state/p1.step0.img" ::slots > "$state/step0-p1/slots.listing" 2>&1 || true
  sha256sum "$state/p1.step0.img" > "$state/step0-p1/p1.sha256"
  capture_slots gssh993 "step0-at-login"
  wait_993_confirm "step 0"
  capture_slots gssh993 "step0-after-confirm"
  gssh 993 'set -e; mkdir -p /data/a1-backup/step0; cd /data/a1-backup/step0
    mkdir -p slots
    cp -a /data/board/slots/. slots/
    ls -la /data/board/slots /data/board > listing.txt
    df /data > df.txt; blkid > blkid.txt; cat /proc/cmdline > cmdline.txt; uname -r > uname.txt
    find . -type f ! -name all.sha256 -exec sha256sum {} + > all.sha256; sha256sum -c -s all.sha256'
}
step1() {
  say "step 1: relabel p2"
  serial_check 993
  gssh 993 'e2label /dev/mmcblk0p2 lemnos-data 2>/dev/null || e2label /dev/vda2 lemnos-data'
  [[ $(gssh 993 'blkid | grep -c "LABEL=\"lemnos-data\""') == 1 ]] || die "step 1: not exactly one lemnos-data label"
  gssh 993 cat /proc/cmdline | grep -q 'PARTUUID=5056525a-02' || die "step 1: 993's PARTUUID word is gone"
}
step2() {
  say "step 2: the new slot's files on /data"
  serial_check 993
  tar -C "$DATA" -cf - slots | gssh 993 'tar -C /data -xf -'
  (cd "$DATA" && find slots -type f -exec sha256sum {} +) > "$state/d.sha256"
  gssh 993 'cd /data && sha256sum -c -s' < "$state/d.sha256" || die "step 2: D's hashes on /data"
}
p1dev() { gssh 993 'test -b /dev/mmcblk0p1 && echo /dev/mmcblk0p1 || echo /dev/vda1'; }
step3() {  # [set]
  local set=${1:-0} dev
  say "step 3: --add-to set $set on a dump, verify, compare, write back, re-dump"
  serial_check 993
  dev=$(p1dev)
  gssh 993 "dd if=$dev bs=1M status=none" > "$state/p1.before.img"
  rm -f "$state/p1.new.img" "$state/p1.changed" "$state/p1.undo"
  "$CTL" boot-partition --add-to "$state/p1.before.img" --layout-of "$BOARD_A1" --socs "$SOCS" --slot "$SLOT" \
    --payload "$PAYLOAD" --firmware "$FIRMWARE" --set "$set" --replace-trial \
    --out "$state/p1.new.img" --changed-out "$state/p1.changed" --undo-out "$state/p1.undo" || die "step 3: --add-to"
  "$CTL" boot-partition --verify "$state/p1.new.img" --layout-of "$BOARD_A1" --socs "$SOCS" > "$state/verify.txt" 2>&1 || true
  [[ $(grep -c 'BAD' "$state/verify.txt") == 2 ]] && grep -q '^.*default\.txt: record file is 55 bytes' "$state/verify.txt" \
    && grep -q 'config\.txt: record file is 405 bytes' "$state/verify.txt" || { cat "$state/verify.txt" >&2; die "step 3: the verify's BAD lines are not exactly the two expected"; }
  for f in config.txt default.txt; do
    cmp <(mtype -i "$state/p1.before.img" "::$f") <(mtype -i "$state/p1.new.img" "::$f") || die "step 3: 993's $f changed"
  done
  fsck.fat -n "$state/p1.new.img" > /dev/null || die "step 3: fsck.fat on the new image"
  [[ $(gssh 993 "sha256sum $dev" | cut -d' ' -f1) == $(sha256sum < "$state/p1.before.img" | cut -d' ' -f1) ]] \
    || die "step 3: p1 changed since the dump"
  writeback "$state/p1.new.img" "$state/p1.changed" forward
  gssh 993 "dd if=$dev bs=1M status=none" > "$state/p1.after.img"
  cmp "$state/p1.after.img" "$state/p1.new.img" || die "step 3: the re-dump is not the prepared image"
}
# Writes each listed range of IMAGE to p1 at the same sectors, reading each back. The list is Lemnos's --changed-out
# (c215f575 service/ctl/src/boot_add.rs:63-78): one "#" comment line, then
#   <first-sector> <sector-count> <class> <sha256-before> <sha256-after>   (512-byte sectors from the partition start)
# Classes (drivers/slot-store/src/fatadd.rs:446-473): new-file, new-dir, own-file, fat1, fat2, dir, root-dir. Order
# forward: data (new-file, new-dir, own-file), fat2, fat1, dirs (dir, root-dir); reverse for an undo. Any other class
# word, or a fatN other than 1 or 2, is refused before anything is written.
writeback() {  # <image> <list> <forward|reverse>
  local img=$1 list=$2 order=$3 dev first count cls before after want group
  local -a groups=(data fat2 fat1 dirs)
  [[ $order == reverse ]] && groups=(dirs fat1 fat2 data)
  group_of() { case $1 in new-file|new-dir|own-file) echo data ;; fat1) echo fat1 ;; fat2) echo fat2 ;; dir|root-dir) echo dirs ;; *) return 1 ;; esac; }
  while read -r first count cls before after; do
    [[ -z $first || $first == \#* ]] && continue
    group_of "$cls" >/dev/null || die "write-back: class '$cls' in $list is not one of new-file new-dir own-file fat1 fat2 dir root-dir"
    [[ $first =~ ^[0-9]+$ && $count =~ ^[0-9]+$ && ${#before} -eq 64 && ${#after} -eq 64 ]] || die "write-back: a malformed line in $list: $first $count $cls"
  done < "$list"
  dev=$(p1dev < /dev/null)
  # Three ssh sessions in all, not two per range (a tight loop of connections gets 993's sshd to refuse the source):
  # the ranges' bytes and an ordered manifest go over as one tar into /tmp (tmpfs); one session writes them in order
  # and prints each range's read-back sha256; the host checks every one.
  local wb=$state/wb.$order n=0 rel
  rm -rf "$wb"; mkdir -p "$wb/r"; : > "$wb/manifest"; : > "$wb/want"
  for group in "${groups[@]}"; do
    while read -r first count cls before after; do
      [[ -z $first || $first == \#* ]] && continue
      [[ $(group_of "$cls") == "$group" ]] || continue
      want=$after; [[ $order == reverse ]] && want=$before
      n=$((n + 1)); rel=$(printf 'r/%05d' "$n")
      dd if="$img" bs=512 skip="$first" count="$count" status=none of="$wb/$rel"
      [[ $(sha256sum < "$wb/$rel" | cut -d' ' -f1) == "$want" ]] || die "write-back: range $cls $first+$count of $img is not the listed bytes"
      echo "$rel $first $count" >> "$wb/manifest"; echo "$rel $first $count $cls $want" >> "$wb/want"
    done < "$list"
  done
  tar -C "$wb" -cf - manifest r | gssh 993 'rm -rf /tmp/a1-wb && mkdir -p /tmp/a1-wb && tar -C /tmp/a1-wb -xf -' \
    || die "write-back: sending the ranges"
  gssh 993 "set -e; cd /tmp/a1-wb; while read -r f first count; do
      dd if=\$f of=$dev bs=512 seek=\$first count=\$count conv=notrunc,fsync status=none
      echo \"\$f \$(dd if=$dev bs=512 skip=\$first count=\$count status=none | sha256sum | cut -d' ' -f1)\"
    done < manifest; rm -rf /tmp/a1-wb" < /dev/null > "$wb/readback" || die "write-back: writing the ranges"
  while read -r rel first count cls want; do
    grep -qx "$rel $want" "$wb/readback" || die "write-back: range $cls $first+$count reads back wrong"
  done < "$wb/want"
  [[ $(wc -l < "$wb/readback") -eq $n ]] || die "write-back: $n ranges sent, $(wc -l < "$wb/readback") read back"
  gssh 993 sync < /dev/null
}
step4() {  # <marker seconds or "none">
  say "step 4: marker ($1), reboot \"0 tryboot\""
  serial_check 993
  if [[ $1 == none ]]; then gssh 993 'rm -f /data/board/trial-deadman; sync'
  else gssh 993 "printf '%s\n' $1 > /data/board/trial-deadman; sync"; fi
  local before; before=$(boots)
  gssh 993 'reboot "0 tryboot"' || true
  wait_boot $((before + 1)) 120 || die "step 4: no new boot after the tryboot reboot"
  last_boot_tryboot || die "step 4: the harness did not boot a tryboot"
}
# D-Bus as shipped (ruling C): the launch helper's mode and owner, dbus.service, the system bus answering.
dbusfacts() {  # <gnew|gssh993>
  "$1" 'h=/usr/libexec/dbus-daemon-launch-helper; if [ -e $h ]; then set -- $(ls -ln $h); echo "helper $1 $3:$4"; else echo "helper absent"; fi; echo "dbus.service $(systemctl is-active dbus)"; dbus-send --system --print-reply --dest=org.freedesktop.DBus / org.freedesktop.DBus.GetId >/dev/null 2>&1 && echo "system bus answers" || echo "system bus SILENT"' < /dev/null | tr -d '\r'
}
gssh993() { gssh 993 "$@"; }
# 993's board-update-confirm.service (Type=exec, After=multi-user.target, /usr/lib/board/update confirm -> slots
# cmd_confirm, 993's /usr/lib/board/slots:868-975) rewrites /data/board/slots/ on EVERY boot, minutes after login:
# <run>.meta GOOD and LAST_BOOT, fallbacks, then save_state (state.env and state.json, each via a .tmp and mv). A copy
# or a check that overlaps it sees a half-written folder (rehearsal build 13, pid1: step 0's copy caught state.env.tmp).
# So step 0 and the return steps first wait, read-only, until that unit has run and exited in this boot: its
# ExecMainExitTimestampMonotonic is set and it is neither active nor activating. Bounded at 600 s.
# Read-only capture of /data/board/slots as found (coordinator 2026-11-20, ROUTE-A-WRITES): every file's size, mtime and
# sha256, the full text of the small state files, and 993's own view (its status file in /run, board-update-confirm's
# state). Nothing is written to /data. <gfn> is gnew (the trial slot) or gssh993; <tag> names the moment.
capture_slots() {  # <gfn> <tag>
  "$1" 'cd /data/board/slots || exit 0; date -u +%FT%TZ; cut -d" " -f1 /proc/uptime; ls -la --full-time . 2>/dev/null || ls -la .; echo ==sha; sha256sum * 2>/dev/null; for f in state.env state.json booted fallbacks settings.env trial-init trial-started trial.env; do [ -f "$f" ] && { echo "==text $f"; cat "$f"; }; done; echo ==confirm-unit; SYSTEMD_PAGER= systemctl show --no-pager -p ActiveState -p Result -p ExecMainExitTimestampMonotonic board-update-confirm.service 2>&1; echo ==status; cat /run/board/update.json 2>&1; echo; echo ==end' \
    < /dev/null | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' > "$state/slots-$2.txt" || true
}
wait_993_confirm() {  # <what>
  gssh 993 'i=0; while :; do a=$(systemctl show -p ActiveState --value board-update-confirm.service); x=$(systemctl show -p ExecMainExitTimestampMonotonic --value board-update-confirm.service); case "$a" in active|activating|deactivating) ;; *) [ "${x:-0}" != 0 ] && break ;; esac; i=$((i+1)); [ $i -lt 300 ] || { echo "board-update-confirm still $a after 600 s"; exit 1; }; sleep 2; done; echo "board-update-confirm done: $a, $(systemctl show -p Result --value board-update-confirm.service), exit at $x"' \
    < /dev/null | tr -d '\r' | tee -a "$state/confirm-wait.txt" || die "$1: 993's board-update-confirm did not finish"
}
# ROUTE-A-WRITES (Lemnos D1 rev, coordinator 2026-11-20): route A writes NOTHING under /data/board/slots. The whole
# folder must be byte-identical to step 0's copy (/data/a1-backup/step0/slots, taken after 993's step-0 confirm), apart
# from 993's own LAST_BOOT line in its .meta (no earlier than step 0's) and board-update-confirm's equal-value rewrites
# (equal bytes, so the sha compare holds). Checked on the trial slot before its reboot, and on 993 at its login BEFORE
# the confirm wait and any restore, and again after the confirm. BusyBox on the trial slot: no sort (ls sorts).
untouched_check() {  # <gfn> <tag>
  "$1" 'cd /data/a1-backup/step0/slots || exit 1; echo "==listing"; ls -1 /data/board/slots; echo "==step0"; ls -1; echo "==sha"; sha256sum $(ls -1 | grep -v "\.meta$") | (cd /data/board/slots && sha256sum -c) 2>&1; echo "sha-rc $?"' \
    < /dev/null | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' > "$state/untouched-$2.txt" || true
  python3 -I - "$state/untouched-$2.txt" <<'UPY' || die "$2: /data/board/slots is not step 0's (route A must write nothing there): see $state/untouched-$2.txt"
import sys
t = open(sys.argv[1]).read().splitlines()
sec, cur = {}, None
for l in t:
    if l.startswith("=="): cur = l[2:]; sec[cur] = []
    elif cur: sec[cur].append(l)
live, step0 = sec.get("listing", []), sec.get("step0", [])
sha = sec.get("sha", [])
bad = []
if live != step0: bad.append(f"the listing differs: {sorted(set(live) ^ set(step0))}")
if "sha-rc 0" not in sha: bad.append("a file's bytes differ: " + "; ".join(x for x in sha if not x.endswith(": OK")))
if bad: sys.exit("; ".join(bad))
print("untouched: listing and bytes equal to step 0's (.meta checked apart)")
UPY
  meta_check "$1" "$2"
}
meta_check() {  # <gfn> <tag>
  "$1" 'cd /data/a1-backup/step0/slots; for f in *.meta; do [ -e "$f" ] || continue; echo "@@ $f step0"; cat "$f"; echo "@@ $f live"; cat "/data/board/slots/$f"; done; echo "@@ end"' \
    < /dev/null | tr -d '\r' > "$state/meta-$2.txt" || die "$2: could not read the .meta files"
  python3 -I - "$state/meta-$2.txt" <<'MPY' || die "$2: 993's .meta is not step 0's (LAST_BOOT aside): see $state/meta-$2.txt"
import sys
parts, cur = {}, None
for line in open(sys.argv[1]).read().splitlines():
    if line.startswith("@@ "):
        cur = tuple(line[3:].rsplit(" ", 1)) if line != "@@ end" else None
        if cur: parts[cur] = []
    elif cur: parts[cur].append(line)
names = sorted({n for n, _ in parts})
if not names: sys.exit("no .meta in step 0's copy")
for n in names:
    a, b = parts.get((n, "step0"), []), parts.get((n, "live"), [])
    rest = lambda ls: [l for l in ls if not l.startswith("LAST_BOOT=")]
    lb = lambda ls: next((l.split("=", 1)[1] for l in ls if l.startswith("LAST_BOOT=")), "")
    if rest(a) != rest(b): sys.exit(f"{n}: lines other than LAST_BOOT differ: {rest(a)} vs {rest(b)}")
    if lb(a) and lb(b) < lb(a): sys.exit(f"{n}: LAST_BOOT {lb(b)} is earlier than step 0's {lb(a)}")
    print(f"{n}: other lines equal; LAST_BOOT {lb(a) or '-'} -> {lb(b) or '-'}")
MPY
}
# Back in 993 after any new-slot boot: marker gone, booted/state.env/state.json restored, Lemnos's added files moved.
return_steps() {  # <n>
  say "return steps (boot $1)"
  wait_login 993 300 || die "return: no login on 993"
  on_993; serial_check 993
  capture_slots gssh993 "return-$1-at-login"
  untouched_check gssh993 "return-$1-at-login"
  wait_993_confirm "return"
  capture_slots gssh993 "return-$1-after-confirm"
  untouched_check gssh993 "return-$1-after-confirm"
  # The route-A path from slot-init's own line, in the trial boot's console log (Lemnos D1 rev, slot-init boot.rs
  # STATE_UNTOUCHED): the boot that started the new slot says nothing is written under board/slots.
  local tl; tl=$(grep -al "slot-init: starting slot $SLOT " "$state"/boot-*.log | head -1)
  [[ -n $tl ]] || die "return: no console log shows slot-init starting $SLOT"
  grep -aq 'the firmware started a trial, and the trial records can.t be read: .*the image is not ours; .*state dir left untouched: nothing is written under board/slots this boot' "$tl" \
    || die "return: slot-init did not say the records are not ours and the state dir was left untouched in $tl"
  # Belt (coordinator 2026-11-20): the backup and restore stay; nothing is expected to need it.
  gssh 993 "set -e; rm -f /data/board/trial-deadman; mkdir -p /data/a1-backup/boot-$1; cd /data/board/slots
    for f in booted state.env state.json; do cp -p /data/a1-backup/step0/slots/\$f \$f; done
    for f in trial-init trial-started trial.env settings.env boots last-failure.log *.devices; do [ -e \"\$f\" ] && mv \"\$f\" /data/a1-backup/boot-$1/ || true; done
    for f in *.meta; do [ -e /data/a1-backup/step0/slots/\$f ] || mv \"\$f\" /data/a1-backup/boot-$1/; done
    cd /data/a1-backup/step0/slots && find . -type f ! -name '*.meta' -exec sha256sum {} + | (cd /data/board/slots && sha256sum -c -s)
    [ \"\$(ls /data/board/slots | sort)\" = \"\$(ls /data/a1-backup/step0/slots | sort)\" ]
    [ ! -e /data/board/trial-deadman ]" || die "return: 993's state is not step 0's"
  meta_check gssh993 "return-$1"
  if [[ -f $state/dbus-new.txt ]]; then
    dbusfacts gssh993 > "$state/dbus-993.txt"
    # Compared on both (coordinator 2026-11-20): dbus.service active and the system bus answering. The helper is a
    # deliberate difference (the strip): 993's setuid helper and its gid are recorded here, not compared.
    for f in "$state/dbus-new.txt" "$state/dbus-993.txt"; do
      grep -qx 'dbus.service active' "$f" || die "return: dbus.service is not active in $(basename "$f")"
      grep -qx 'system bus answers' "$f" || die "return: the system bus does not answer in $(basename "$f")"
    done
    echo "both: dbus.service active, system bus answers; 993's helper (recorded): $(grep '^helper ' "$state/dbus-993.txt")" > "$state/dbus-compare.txt"
  fi
}
can_stage_again() {
  say "993 can stage again: a fresh dump, --add-to and the verify pass on it"
  local s=$state; state=$state/again; mkdir -p "$state"; cp "$s/netns.pid" "$s"/known_hosts.* "$s/harness.log" "$state/" 2>/dev/null || true
  local dev; dev=$(p1dev)
  gssh 993 "dd if=$dev bs=1M status=none" > "$state/p1.before.img"
  "$CTL" boot-partition --add-to "$state/p1.before.img" --layout-of "$BOARD_A1" --socs "$SOCS" --slot "$SLOT" \
    --payload "$PAYLOAD" --firmware "$FIRMWARE" --set 1 --replace-trial \
    --out "$state/p1.new.img" --changed-out "$state/p1.changed" --undo-out "$state/p1.undo" > "$state/add-to.txt" 2>&1 || {
    # The same slot again: lemnos-ctl refuses by name and writes nothing (the trial's slot stays in bay0 after the
    # return, by design). That refusal, and the fresh dump verifying as step 3's did, is the answer for this slot.
    grep -q "the slot $SLOT is already in bay0: nothing was written" "$state/add-to.txt" || { cat "$state/add-to.txt" >&2; die "stage again: --add-to set 1"; }
    "$CTL" boot-partition --verify "$state/p1.before.img" --layout-of "$BOARD_A1" --socs "$SOCS" > "$state/verify.txt" 2>&1 || true
    [[ $(grep -c '^BAD ' "$state/verify.txt") -eq 2 ]] && grep -q 'default\.txt: record file is 55 bytes' "$state/verify.txt" \
      && grep -q 'config\.txt: record file is 405 bytes' "$state/verify.txt" || { cat "$state/verify.txt" >&2; die "stage again: the returned p1 does not verify as step 3's did"; }
    say "stage again: the same slot is refused by name (already in bay0, nothing written); the returned p1 verifies as step 3's"
  }
  state=$s
}
uptime_s() { gnew 'cut -d. -f1 /proc/uptime'; }

# --- the endings ------------------------------------------------------------------------------------------------
# --- 993's state on /data (coordinator 2026-11-20) -------------------------------------------------------------------
# What 993 keeps on /data for its daemons, from 993's own boot on the replica (probe, data-owners.993.txt) and its units:
# lemnosd keeps nothing there (StateDirectory=lemnos is /var/lib/lemnos on 993's /var tmpfs); Orion's node keeps its
# identity and snapshots in /var/lib/helios/orion, bound by helios-data-setup from p2's os/helios/helios (StateDirectory=
# helios/orion, mode 0750 orion:orion); HeliOS keeps /var/lib/helios/state (root:orion 0755) and /var/log/helios
# (root:orion 0755) there. The paths below are p2's own, so they read the same on 993 and on the new slot (which binds
# nothing over /data). Each gets a marker owned like its dir, mode 0640, readable by orion as owner or group.
SEEDED="/data/os/helios/helios/orion:orion:orion /data/os/helios/helios/state:root:orion /data/log/helios:root:orion"
seed_markers() {
  say "seed: a marker in each of 993's state dirs on /data"
  local e p o g
  for e in $SEEDED; do
    IFS=: read -r p o g <<< "$e"
    gssh 993 "set -e; [ -d $p ]; echo 'rehearsal marker $ending $p' > $p/.rehearsal-marker; chown $o:$g $p/.rehearsal-marker; chmod 0640 $p/.rehearsal-marker"
  done
  seed_listing 993 > "$state/seed.993.txt"
  grep -q 'MISSING' "$state/seed.993.txt" && die "seed: a seeded entry is missing on 993: see $state/seed.993.txt"
  return 0
}
# One line per seeded dir and marker: inode, mode, numeric owner, and the marker's sha256. Equal lines = the same
# inodes, never re-created, re-owned or re-moded, the marker's bytes intact. (Not the dirs' entries: 993's own Orion
# writes its snapshots there while it runs, before the trial and after the return.)
seed_listing() {  # <993|new>
  local cmd='for e in '"$SEEDED"'; do p=${e%%:*}; m=$p/.rehearsal-marker
      if [ -d "$p" ]; then set -- $(ls -ldin "$p"); echo "dir $p ino=$1 mode=$2 uid=$4 gid=$5"; else echo "MISSING $p"; fi
      if [ -f "$m" ]; then set -- $(ls -lin "$m"); echo "marker $m ino=$1 mode=$2 uid=$4 gid=$5 sha=$(sha256sum < "$m" | cut -c1-64)"; else echo "MISSING $m"; fi
    done'
  if [[ $1 == new ]]; then gnew "$cmd" | tr -d '\r'; else gssh 993 "$cmd"; fi
}
seed_check() {  # <993|new> <label>
  seed_listing "$1" > "$state/seed.$2.txt"
  diff "$state/seed.993.txt" "$state/seed.$2.txt" > "$state/seed.$2.diff" \
    || die "seed ($2): 993's state dirs or markers changed: see $state/seed.$2.diff"
  # The owner reads each marker as itself: orion is the owner or the group of all three.
  if [[ $1 == new ]]; then
    local e p
    for e in $SEEDED; do
      p=${e%%:*}
      gnew "systemd-run --wait --pipe --quiet -p User=orion cat $p/.rehearsal-marker" | tr -d '\r' | grep -qx "rehearsal marker $ending $p" \
        || die "seed ($2): orion cannot read $p/.rehearsal-marker on the new slot"
    done
  fi
  say "seed ($2): 993's state dirs and markers unchanged, the markers readable by orion"
}
run_ending() {
  local e=$1 b0
  ending=$e
  start_harness; step0; seed_markers; step1; step2; step3 0
  case $e in
    nomarker) step4 none ;;
    *)        step4 "$N" ;;
  esac
  b0=$(boots)
  case $e in
    login|plain)
      wait_login new 300 || die "no login on the new slot"
      serial_check new
      # The boot's own state, read before the script stops or starts anything (coordinator 2026-11-20): wait for the
      # boot to finish (a read), then the four declared daemons by name must be active with NRestarts=0, and no unit
      # may have failed at all. Nothing is excused: the ttyGS0 getty is not started in emulation (no gadget), not failed.
      CT=180 gnew 'systemctl is-system-running --wait; for u in lemnosd lemnos-orion orion-comm orion-node; do echo "unit $u $(systemctl show -p ActiveState --value $u.service) $(systemctl show -p NRestarts --value $u.service)"; done; echo "failed: [$(SYSTEMD_PAGER= systemctl --failed --no-legend --plain --no-pager | tr "\n" " " | sed "s/ *$//")]"' \
        | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' > "$state/daemons-boot.txt" || true
      for u in lemnosd lemnos-orion orion-comm orion-node; do
        grep -qx "unit $u active 0" "$state/daemons-boot.txt" || die "$u is not active with NRestarts=0 at the end of the boot: $(grep "^unit $u " "$state/daemons-boot.txt")"
      done
      grep -qx 'failed: \[\]' "$state/daemons-boot.txt" || die "failed units at the end of the boot: $(grep '^failed:' "$state/daemons-boot.txt")"
      gnew 'systemctl stop bench-trial-deadman; rm -f /data/board/trial-deadman'
      gnew 'systemctl is-active bench-trial-deadman' && die "the dead-man is still active after the stop"
      gnew 'uname -r; lemnos-ctl list' > "$state/new-slot.txt" || true
      # Evidence first, before any assertion can stop the run: the daemons' status and journals of this boot.
      # No unit probe here any more: unitprobe.sh (Fault 1's diagnostic, fixed in Lemnos b210934d) stops lemnosd and
      # lemnos-orion, so the facts below would not be the boot's own. It stays in this folder for a by-hand run.
      CT=90 gnew 'SYSTEMD_PAGER= systemctl status --no-pager -l lemnosd lemnos-orion orion-comm orion-node; journalctl -b --no-pager -o short-monotonic -u lemnosd -u lemnos-orion -u orion-comm -u orion-node | tail -n 120; lsmod; ls -l /dev/lemnos* /dev/watchdog* 2>&1; echo ==show; SYSTEMD_PAGER= systemctl show lemnosd -p CapabilityBoundingSet -p AmbientCapabilities -p SystemCallFilter -p ExecStart -p Environment -p User -p NoNewPrivileges; echo ==env; cat /etc/lemnos/lemnosd.env 2>&1; ls -ld /run/lemnos 2>&1; echo ==probe; systemd-run --wait --pipe --quiet -p CapabilityBoundingSet="CAP_CHOWN CAP_SETGID CAP_SETUID CAP_SETPCAP CAP_SYS_RAWIO" -p CapabilityBoundingSet=CAP_SYS_ADMIN sh -c "grep Cap /proc/self/status" 2>&1' \
        | tr -d '\r' > "$state/daemons.txt" || true
      # Root with systemctl and journalctl (coordinator 2026-11-18), and D-Bus as 993 ships it (ruling C, 2026-11-19:
      # D1 ships dbus; the no-dbus version of this check is rehearse-route-a.dbus-trim.sh, parked with save/D1-dbus-trim).
      # The dbus facts are read on the new slot AND on 993 and must be equal: helper mode/owner, dbus.service state,
      # the system bus answering. Plus: no file on the new slot's root carries a set-id bit (Gaia strips both).
      # One line each: the guest's own exit status is printed by the guest (gnew's status is the console's), output
      # joined onto the line; SYSTEMD_PAGER and --no-pager keep systemctl and journalctl off the console's pager.
      { echo "systemctl is-system-running: $(gnew 'SYSTEMD_PAGER= systemctl is-system-running; echo "(exit $?)"' | tr -d '\r' | tr '\n' ' ')"
        echo "journalctl -n1: $(gnew 'journalctl -n1 --no-pager >/dev/null; echo "(exit $?)"' | tr -d '\r' | tr '\n' ' ')"
        echo "failed units: $(gnew 'SYSTEMD_PAGER= systemctl --failed --no-legend --plain --no-pager' | tr -d '\r' | tr '\n' ' ')"
        echo "set-id files on /: [$(gnew 'find / -xdev -type f -perm /6000 2>/dev/null' | tr -d '\r' | tr '\n' ' ')]"
      } > "$state/root-checks.txt"
      dbusfacts gnew > "$state/dbus-new.txt"
      # The helper by name (coordinator 2026-11-20): the bus daemon's own group on this image, resolved from its
      # system.conf <user> and that user's primary group in /etc/passwd and /etc/group (not a hard-coded name).
      gnew 'h=/usr/libexec/dbus-daemon-launch-helper; u=$(sed -n "s:.*<user>\(.*\)</user>.*:\1:p" /usr/share/dbus-1/system.conf /etc/dbus-1/system.conf 2>/dev/null | head -1); g=$(grep "^$u:" /etc/passwd | cut -d: -f4); gn=$(grep "^[^:]*:[^:]*:$g:" /etc/group | cut -d: -f1); echo "busgroup $gn"; set -- $(ls -l $h); echo "helpername $1 $3:$4"' \
        | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' >> "$state/dbus-new.txt" || true
      bg=$(sed -n 's/^busgroup //p' "$state/dbus-new.txt")
      [[ -n $bg ]] || die "the bus daemon's group could not be resolved on the new slot: see $state/dbus-new.txt"
      grep -qx "helpername -rwxr-x--- root:$bg" "$state/dbus-new.txt" || die "the launch helper is not 0750 root:$bg (the bus daemon's group) with no set-id bit: see $state/dbus-new.txt"
      grep -q 'journalctl -n1: .*(exit 0)' "$state/root-checks.txt" || die "journalctl does not work as root"
      grep -Eq 'is-system-running: (running|degraded)' "$state/root-checks.txt" || die "systemctl does not answer as root: $(head -1 "$state/root-checks.txt")"
      grep -q '^set-id files on /: \[\]$' "$state/root-checks.txt" || die "set-id files on the new slot's root: $(grep '^set-id' "$state/root-checks.txt")"
      grep -Eqx 'helper -rwxr-x--- 0:[0-9]+' "$state/dbus-new.txt" || die "the dbus launch helper is not 0750 owned by root on the new slot: $(head -1 "$state/dbus-new.txt")"
      grep -qx 'dbus.service active' "$state/dbus-new.txt" || die "dbus.service is not active on the new slot"
      grep -qx 'system bus answers' "$state/dbus-new.txt" || die "the system bus does not answer on the new slot"
      # The state on 993's /data is owned by the new slot's users BY NAME (coordinator 2026-11-20: route A keeps 993's
      # /data, so lemnos 900, orion 901 and lemnos-orion 902 must be the ids the slot ships; devices/products/raze-base/
      # os/data-owners). Orion's dirs as its drop-ins make them (orion-comm: root:orion /data/orion; orion-node:
      # orion:orion /data/orion/node), and every entry on /data owned by one of the pinned ids resolves to its name.
      # BusyBox only on the slot (no stat, no find -uid): `ls -ld` gives names, `ls -ldn` the ids. One line per entry:
      # "OWNER:GROUP UID:GID PATH" for the two Orion dirs and every entry on /data owned by a pinned user or group.
      gnew 'for p in /data/orion /data/orion/node; do set -- $(ls -ld "$p"); a="$3:$4"; set -- $(ls -ldn "$p"); echo "$a $3:$4 $p"; done; echo ==pinned
            for u in 900 901 902; do find /data -xdev -user $u; find /data -xdev -group $u; done | while read -r p; do set -- $(ls -ld "$p"); a="$3:$4"; set -- $(ls -ldn "$p"); echo "$a $3:$4 $p"; done' \
        | tr -d '\r' > "$state/data-owners.txt" || true
      grep -qx 'root:orion 0:901 /data/orion' "$state/data-owners.txt" || die "/data/orion is not root:orion by name on the new slot: see $state/data-owners.txt"
      grep -qx 'orion:orion 901:901 /data/orion/node' "$state/data-owners.txt" || die "/data/orion/node is not orion:orion by name on the new slot: see $state/data-owners.txt"
      sed -n '/^==pinned$/,$p' "$state/data-owners.txt" | grep -Eq '^([0-9]+:|[^ ]*:[0-9]+ )' && die "an entry on /data has an owner that does not resolve to a name: see $state/data-owners.txt"
      # 993's state dirs, made by 993 before the trial: the new slot's daemons came up with them there, untouched.
      [[ $(gnew 'systemctl is-active lemnosd orion-node' | tr -d '\r' | grep -cx active) == 2 ]] || die "lemnosd or orion-node is not active on the new slot"
      seed_check new new-slot
      # lemnosd as user lemnos, /run/lemnos 0750 lemnos:lemnos (coordinator 2026-11-20, Lemnos b210934d RUNDIR fix):
      # lemnosd starts privileged and drops to lemnos; its running process's real, effective, saved and fs uids and
      # gids are all 900, and that id is lemnos in passwd. /run/lemnos by name and by id (BusyBox ls only).
      gnew 'pid=$(systemctl show -p MainPID --value lemnosd); echo "pid $pid"; grep -E "^(Uid|Gid):" /proc/$pid/status; grep "^[^:]*:[^:]*:900:" /etc/passwd | cut -d: -f1; set -- $(ls -ld /run/lemnos); a="$1 $3:$4"; set -- $(ls -ldn /run/lemnos); echo "rundir $a $3:$4"' \
        | tr -d '\r' > "$state/lemnosd-user.txt" || true
      grep -Eqx 'Uid:[[:space:]]+900[[:space:]]+900[[:space:]]+900[[:space:]]+900' "$state/lemnosd-user.txt" || die "lemnosd does not run as uid 900: see $state/lemnosd-user.txt"
      grep -Eqx 'Gid:[[:space:]]+900[[:space:]]+900[[:space:]]+900[[:space:]]+900' "$state/lemnosd-user.txt" || die "lemnosd does not run as gid 900: see $state/lemnosd-user.txt"
      grep -qx 'lemnos' "$state/lemnosd-user.txt" || die "uid 900 is not lemnos on the new slot: see $state/lemnosd-user.txt"
      grep -qx 'rundir drwxr-x--- lemnos:lemnos 900:900' "$state/lemnosd-user.txt" || die "/run/lemnos is not 0750 lemnos:lemnos: see $state/lemnosd-user.txt"
      # orion's home (coordinator 2026-11-20): Gaia makes no homes, so /var/lib/orion is not in the image; the units'
      # StateDirectory= makes it at boot on the writable /var. It must exist, belong to orion, and be passwd's home.
      gnew 'grep "^orion:" /etc/passwd | cut -d: -f6; ls -ldn /var/lib/orion; grep -E " /var(/lib)? " /proc/mounts' \
        | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' > "$state/orion-home.txt" || true  # BusyBox ls colours on the console
      grep -qx '/var/lib/orion' "$state/orion-home.txt" || die "orion's home is not /var/lib/orion: see $state/orion-home.txt"
      grep -Eq '^d[rwx-]{9} +[0-9]+ +901 +901 .*/var/lib/orion$|^d[rwx-]{9} +[0-9]+ +0 +901 .*/var/lib/orion$|^d[rwx-]{9} +[0-9]+ +0 +0 .*/var/lib/orion$' "$state/orion-home.txt" \
        || die "/var/lib/orion was not made at boot: see $state/orion-home.txt"
      # Route A, the Lemnos D1 rev (ROUTE-A-WRITES): lemnosd sees the boot records as not ours. `slots list` and `info`
      # carry "foreign", health is degraded with the same text, nothing is written, and the trial is not in its state
      # (993's state.env, TRIAL=''). slot-init no longer writes `booted`, so slot_active is 993's: which slot runs is
      # taken from slot-init's console line (return steps) and /proc/cmdline (recorded). Past 45 s: list and info have
      # foreign "foreign: the boot records are not ours; the slots state here is another system's and is left untouched: default.txt…", trial null, slot_default 993's; `lemnos-ctl list` shows the foreign health
      # text; no confirm line in lemnosd's journal; 993's default.txt (step 0) and trial.txt (step 3) byte-unchanged.
      # How lemnosd (dropped to lemnos) could reach the boot disk (coordinator 2026-11-20): its open fds that are block
      # devices or anything under /dev, its uid/gid/supplementary groups and capability sets, and the nodes' modes.
      gnew 'pid=$(systemctl show -p MainPID --value lemnosd); echo "pid $pid"; grep -E "^(Uid|Gid|Groups|CapInh|CapPrm|CapEff|CapBnd|CapAmb|NoNewPrivs):" /proc/$pid/status; echo ==fds; ls -l /proc/$pid/fd | grep -E -- "-> /dev/" || echo "(no fd under /dev)"; echo ==fd-count $(ls /proc/$pid/fd | wc -l); echo ==nodes; ls -l /dev/vda /dev/vda1 /dev/mmcblk0 /dev/mmcblk0p1 2>&1; echo ==mounts; grep -E "vda1|mmcblk0p1| /boot" /proc/mounts || echo "(the boot partition is not mounted)"; echo "==lemnosd mount namespace (/, /data, /sys, /sys/kernel/config, /dev)"; grep -E "^[0-9]+ [0-9]+ [^ ]+ [^ ]+ (/|/data|/sys|/sys/kernel/config|/dev) " /proc/$pid/mountinfo | cut -d" " -f5,6' \
        | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' > "$state/lemnosd-reach.txt" || true
      gnew 'u=$(cut -d. -f1 /proc/uptime); [ "$u" -ge 45 ] || sleep $((45 - u)); echo "uptime $(cut -d" " -f1 /proc/uptime)"; echo "cmdline $(cat /proc/cmdline)"; echo ==list; lemnos-ctl call slots list; echo; echo ==info; lemnos-ctl call slots info; echo; echo ==ctl-list; lemnos-ctl list; echo ==journal; journalctl -u lemnosd --no-pager -o cat | grep -iE "confirm" ; echo ==p1; d=/dev/mmcblk0p1; [ -b $d ] || d=/dev/vda1; mkdir -p /run/p1chk; mount -o ro $d /run/p1chk && { cd /run/p1chk; sha256sum default.txt trial.txt tryboot.txt 2>&1; cd /; umount /run/p1chk; }' \
        | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' > "$state/confirm-held.txt" || true
      sed -n '/^==list$/{n;p}' "$state/confirm-held.txt" > "$state/slots-list.json"
      sed -n '/^==info$/{n;p}' "$state/confirm-held.txt" > "$state/slots-info.json"
      python3 -I - "$state/slots-list.json" "$state/slots-info.json" <<'A2PY' || die "slots list/info are not route A's (foreign, no trial, 993 default): see $state/confirm-held.txt"
import json, sys
F = "foreign-unknown: the boot records can't be read to say whose they are; the slots state is left untouched until they can: /dev/"
v = json.load(open(sys.argv[1])); i = json.load(open(sys.argv[2]))
pd = lambda x: str(x.get("foreign", "")).startswith(F) and str(x.get("foreign", "")).endswith(": Permission denied (os error 13)")
checks = [(pd(v), "list: foreign-unknown (Permission denied)"), (pd(i), "info: foreign-unknown (Permission denied)"),
          (v.get("trial") is None, "trial is null"), (v.get("slot_trial") == "", "slot_trial empty"),
          (str(v.get("slot_default", "")).startswith("helios-2026.2.0-dev.993-"), "slot_default is 993's")]
bad = [t for ok, t in checks if not ok]
print("A2: " + ("ok: " + "; ".join(t for _, t in checks) if not bad else "NOT: " + "; ".join(bad)))
sys.exit(1 if bad else 0)
A2PY
      # Provisional ruling (a), coordinator 2026-11-20: lemnosd, dropped to lemnos, cannot open the boot partition's node
      # (root:disk 0660), so its ownership probe says foreign-unknown (EACCES), not foreign; it still writes nothing.
      # slot-init (root) names "not ours" and the untouched state dir on the console (checked in the return steps).
      sed -n '/^==ctl-list$/,/^==journal$/p' "$state/confirm-held.txt" | grep -q "slots .*(foreign-unknown: the boot records can't be read to say whose they are; the slots state is left untouched until they can)" || die "lemnos-ctl list does not show the slots device's foreign-unknown health text: see $state/confirm-held.txt"
      sed -n '/^==journal$/,/^==p1$/p' "$state/confirm-held.txt" | grep -v '^==' | grep -q . && die "lemnosd's journal has a confirm line: see $state/confirm-held.txt"
      grep -qx "$(sha256sum < "$state/step0-p1/default.txt" | cut -d' ' -f1)  default.txt" "$state/confirm-held.txt" || die "993's default.txt on p1 is not step 0's: see $state/confirm-held.txt"
      grep -qx "$(mtype -i "$state/p1.after.img" ::trial.txt | sha256sum | cut -d' ' -f1)  trial.txt" "$state/confirm-held.txt" || die "trial.txt on p1 is not as step 3 wrote it: see $state/confirm-held.txt"
      # A3, emulation only, plain only (coordinator 2026-11-20): one explicit confirm, after A2 so it cannot disturb it,
      # must be refused (FAILED_PRECONDITION; lemnos-ctl: "failed precondition") with /data/board/slots and p1's records
      # byte-identical before and after. Never a board step.
      if [[ $ending == plain ]]; then
        gnew 'd=/dev/mmcblk0p1; [ -b $d ] || d=/dev/vda1; snap() { echo "==$1"; (cd /data/board/slots && ls -la --full-time . 2>/dev/null || ls -la .; sha256sum *); mkdir -p /run/p1chk; mount -o ro $d /run/p1chk && { (cd /run/p1chk && sha256sum default.txt trial.txt tryboot.txt config.txt); umount /run/p1chk; }; }; snap before; echo ==call; lemnos-ctl call slots confirm; echo "rc $?"; snap after' \
          | tr -d '\r' | sed 's/\x1b\[[0-9;]*m//g' > "$state/a3-confirm.txt" || true
        # `lemnos-ctl call` prints the status as its code, "refused (-4102): <message>"; -4102 is FAILED_PRECONDITION
        # (b78e5317 engine/client/src/error.rs:289). The named form "(failed precondition: ...)" is lemnos-ctl's other
        # commands' (service/ctl/src/lib.rs:1270). Either is the refusal A3 asks for; nothing else is.
        grep -qE '^refused \(-4102\): |failed precondition' "$state/a3-confirm.txt" || die "A3: the confirm was not refused with failed precondition (-4102): see $state/a3-confirm.txt"
        grep -qx 'rc 0' "$state/a3-confirm.txt" && die "A3: the confirm call returned 0: see $state/a3-confirm.txt"
        cmp -s <(sed -n '/^==before$/,/^==call$/p' "$state/a3-confirm.txt" | sed 1d | sed '$d') <(sed -n '/^==after$/,$p' "$state/a3-confirm.txt" | sed 1d) \
          || die "A3: /data/board/slots or p1 changed across the refused confirm: see $state/a3-confirm.txt"
        say "A3: confirm refused (FAILED_PRECONDITION, -4102); the state dir and p1 byte-identical across it"
      fi
      capture_slots gnew "trial-before-reboot"
      untouched_check gnew "trial-before-reboot"
      gnew 'systemctl reboot' || true ;;
    nologin)
      # Nobody logs in. The dead-man fires at N: the next boot is a plain one, between N and N + the boot time.
      wait_boot $((b0 + 1)) $((N + 300)) || die "the dead-man did not return the board within N + 300 s"
      grep -aq 'bench-trial-deadman: no stop in' "$state/boot-$b0.log" 2>/dev/null \
        || grep -aq 'bench-trial-deadman: no stop in' "$state"/boot-*.log || die "no dead-man line in the new slot's console log" ;;
    stopped)
      wait_login new 300 || die "no login on the new slot"
      sleep $((N / 2)); gnew 'systemctl stop bench-trial-deadman'
      sleep $((N + 30)); [[ $(boots) -eq $b0 ]] || die "a reboot happened after the dead-man was stopped"
      gnew 'test -e /data/board/trial-deadman' || die "the marker went though the dead-man never fired"
      capture_slots gnew "trial-before-reboot"
      untouched_check gnew "trial-before-reboot"
      gnew 'rm -f /data/board/trial-deadman; systemctl reboot' || true ;;
    nomarker)
      wait_login new 300 || die "no login on the new slot"
      gnew 'systemctl is-active bench-trial-deadman' && die "the dead-man ran with no marker"
      sleep $((N + 30)); [[ $(boots) -eq $b0 ]] || die "a reboot happened with no marker"
      capture_slots gnew "trial-before-reboot"
      untouched_check gnew "trial-before-reboot"
      gnew 'systemctl reboot' || true ;;
    handover)
      # The slot-init -> lemnosd watchdog handover, on the variant whose board file names watchdog0 (the i6300esb).
      wait_login new 300 || die "no login on the new slot"
      gnew 'systemctl stop bench-trial-deadman; rm -f /data/board/trial-deadman' || true
      grep -aq 'slot-init: watchdog armed' "$state/boot-$b0.log" || die "slot-init did not arm the watchdog (no 'watchdog armed' line)"
      # slot-init's last feed is no later than its 'starting slot' line; the timeout is 15 s. Up 45 s past that line
      # (three timeouts) with no reset means something else fed it.
      local t0 t1 up
      t0=$(grep -a 'slot-init: starting slot' "$state/boot-$b0.log" | sed -n 's/^\[ *\([0-9]*\)\..*/\1/p' | head -1)
      [[ -n $t0 ]] || die "no 'starting slot' line in the new slot's console log"
      up=$(gnew 'cut -d. -f1 /proc/uptime' | tr -dc 0-9)
      [[ -n $up ]] || die "the new slot's uptime cannot be read"
      t1=$((t0 + 45)); (( up >= t1 )) || sleep $((t1 - up))
      [[ $(boots) -eq $b0 ]] || die "the guest reset before slot-init's last feed + 45 s: lemnosd did not take the watchdog"
      { echo "uptime: $(gnew 'cat /proc/uptime' | tr -d '\r')"
        echo "lemnosd: $(gnew 'systemctl is-active lemnosd' | tr -d '\r')"
        gnew 'lemnos-ctl list; for f in identity state timeout timeleft; do echo "watchdog0 $f: $(cat /sys/class/watchdog/watchdog0/$f 2>&1)"; done' | tr -d '\r'
      } > "$state/handover.txt"
      grep -qx 'lemnosd: active' "$state/handover.txt" || die "lemnosd is not active on the new slot: see $state/handover.txt"
      grep -q 'watchdog0 state: active' "$state/handover.txt" || die "the watchdog is not active: see $state/handover.txt"
      say "handover: up $(grep -o '^uptime: [0-9]*' "$state/handover.txt" | cut -d' ' -f2) s, slot-init's last feed by ${t0} s: lemnosd feeds it"
      # Stopping lemnosd ends the feeding (it never writes the magic close): the guest resets within the timeout.
      gnew 'systemctl stop lemnosd' || true
      local t=0
      until grep -q '^== stopped' "$state/harness.log"; do sleep 1; t=$((t + 1)); [[ $t -lt 45 ]] || die "no reset within 45 s of stopping lemnosd"; done
      grep -aqE 'reboot: Restarting system|Rebooting in' "$state/boot-$b0.log" && die "the guest restarted in order, not by the watchdog"
      say "handover: the guest reset ${t} s after lemnosd stopped (the watchdog, not an orderly restart)"
      restart_harness ;;
    handover-debug)
      # Coordinator 2026-11-20, emulation-only diagnosis (not a flash candidate): slot-init's 15 s is compiled
      # (Lemnos b78e5317 service/slot-init/src/dog.rs:48 DOG_SECONDS) and governs the gap from switch_root to lemnosd's
      # first feed, so no board-file line can lengthen it; QEMU's "debug" action logs each firing instead of resetting.
      [[ ${HARNESS_QEMU_ARGS:-} == *watchdog=debug* ]] || die "handover-debug needs HARNESS_QEMU_ARGS=\"-action watchdog=debug\""
      fires() { sed -n "/^== boot $b0 /,\$p" "$state/harness.log" | grep -c 'watchdog: timer fired' || true; }
      wait_login new 300 || die "no login on the new slot"
      gnew 'systemctl stop bench-trial-deadman; rm -f /data/board/trial-deadman' || true
      grep -aq 'slot-init: watchdog armed' "$state/boot-$b0.log" || die "slot-init did not arm the watchdog (no 'watchdog armed' line)"
      local t0 f_login f1 f2 t
      t0=$(grep -a 'slot-init: starting slot' "$state/boot-$b0.log" | sed -n 's/^\[ *\([0-9]*\)\..*/\1/p' | head -1)
      f_login=$(fires)
      # (1) does lemnosd, as user lemnos under the real unit, open the watchdog and feed it
      { echo "uptime: $(gnew 'cat /proc/uptime' | tr -d '\r')"; echo "firings since the trial boot began: $f_login"
        gnew 'pid=$(systemctl show -p MainPID --value lemnosd); echo "lemnosd: $(systemctl is-active lemnosd) pid $pid"; grep -E "^(Uid|Gid|Groups|CapEff):" /proc/$pid/status; echo ==row; lemnos-ctl list | grep -E "^watchdog "; echo ==fds; ls -l /proc/$pid/fd | grep -E -- "-> /dev/" || echo "(no fd under /dev)"; echo ==nodes; ls -l /dev/watchdog*; echo ==sysfs; for f in identity state timeout timeleft status bootstatus; do echo "watchdog0 $f: $(cat /sys/class/watchdog/watchdog0/$f 2>&1)"; done; echo ==journal; journalctl -u lemnosd -o short-monotonic --no-pager | grep -i watchdog | head -20' | tr -d '\r'
        # G7: the unit's sandbox as systemd reads it, and every /sys mount in lemnosd's namespace
        # (kept short: the serial console cuts a long typed line, which loses con.py's end marker)
        gnew 'echo ==G7-show; systemctl show --no-pager lemnosd -p ProtectKernelTunables -p ProtectSystem -p ReadWritePaths -p ReadOnlyPaths -p InaccessiblePaths' | tr -d '\r'
        gnew 'echo ==G7-unit; systemctl cat --no-pager lemnosd | grep -E "^(# /|Protect|ReadWrite|ReadOnly|Private|User|Group|Capability)"' | tr -d '\r'
        echo "==G7-sys-mounts: raw /proc/<lemnosd pid>/mountinfo lines. Fields: 1 mount id, 2 parent id, 3 major:minor, 4 root, 5 MOUNT POINT, 6 PER-MOUNT OPTIONS (a read-only bind shows ro here), optional fields, -, fs type, source, SUPERBLOCK OPTIONS (last)"
        gnew 'p=$(systemctl show -p MainPID --value lemnosd); grep -E " /sys(/[^ ]*)? " /proc/$p/mountinfo' | tr -d '\r'
        # The bind order and times (coordinator 2026-11-20): lemnosd binds in board-file order and waits on each, so
        # every line of its journal with the monotonic clock, up to and including the watchdog's "running ok".
        echo "==journal-full (lemnosd, -o short-monotonic, first 80 lines)"
        gnew 'journalctl -u lemnosd -o short-monotonic --no-pager | head -80' | tr -d '\r'
        gnew 'echo ==systemd-start; journalctl -o short-monotonic --no-pager | grep -E "Starting Lemnos hardware|Started Lemnos hardware|slot-init: starting slot" | head -5' | tr -d '\r'
        # G7 write test (coordinator 2026-11-20): an append open that writes no bytes, on a sysfs file and on a debugfs file
        # (the known read-only control). The slot has no nsenter, setpriv, su or chroot, so the stand-in is a transient
        # systemd-run unit with lemnosd's own sandbox settings (as root, then as lemnos 900:900), plus a control unit
        # with no sandbox. The shell's message names the errno.
        echo "==G7-write: transient systemd-run units (stand-in: the slot has no nsenter); root and lemnos with lemnosd's Protect*/ProtectSystem/PrivateTmp settings, none = no sandbox (control); ': >> FILE' opens for append and writes nothing"
        gnew 'P="-p ProtectSystem=strict -p ProtectHome=yes -p PrivateTmp=yes -p ProtectKernelTunables=yes -p ProtectKernelModules=yes -p ProtectKernelLogs=yes -p ProtectControlGroups=yes -p ProtectClock=yes -p ProtectHostname=yes"; echo "$P" > /run/g7p' >/dev/null || true
        for who in root lemnos none; do
          gnew "P=\$(cat /run/g7p); [ $who = none ] && P=; [ $who = lemnos ] && P=\"\$P -p User=lemnos -p Group=lemnos\"; for f in /sys/class/watchdog/watchdog0/uevent /sys/kernel/debug/gpio; do r=\$(systemd-run -q --wait --pipe \$P sh -c \": >> \$f && echo ok\" 2>&1); echo \"$who \$f: \$r\"; done" | tr -d '\r'
        done
      } > "$state/handover.txt"
      if sed -n '/^==row/,/^==sysfs/p' "$state/handover.txt" | grep -qi 'permission denied'; then
        die "STOP (coordinator's rule): lemnosd reports permission denied on the watchdog: the row and the node's ls -l are in $state/handover.txt"
      fi
      grep -q '^lemnosd: active' "$state/handover.txt" || die "lemnosd is not active on the new slot: see $state/handover.txt"
      grep -qE -- '-> /dev/watchdog' "$state/handover.txt" || die "(1) lemnosd holds no /dev/watchdog fd: see $state/handover.txt"
      grep -q 'watchdog0 state: active' "$state/handover.txt" || die "(1) the watchdog is not active: see $state/handover.txt"
      say "handover-debug (1): lemnosd as lemnos holds $(grep -oE -- '-> /dev/watchdog[0-9]*' "$state/handover.txt" | head -1 | cut -c4-), sysfs state active, $f_login firing(s) before login (slot-init's last feed by ${t0} s)"
      # (2) once lemnosd feeds, no firing for three timeouts (45 s)
      f1=$(fires); sleep 45; f2=$(fires)
      [[ $(boots) -eq $b0 ]] || die "(2) the guest restarted during the 45 s watch"
      [[ $f2 -eq $f1 ]] || die "(2) the watchdog fired $((f2 - f1)) time(s) in 45 s while lemnosd was running: it does not feed"
      echo "timeleft after 45 s: $(gnew 'cat /sys/class/watchdog/watchdog0/timeleft' | tr -d '\r')" >> "$state/handover.txt" || true
      say "handover-debug (2): no firing in 45 s (three timeouts) with lemnosd feeding"
      # (3) stopping lemnosd (no magic close) makes it fire within the timeout
      gnew 'systemctl stop lemnosd' || true
      t=0; until [[ $(fires) -gt $f2 ]]; do sleep 1; t=$((t + 1)); [[ $t -lt 45 ]] || die "(3) no firing within 45 s of stopping lemnosd"; done
      say "handover-debug (3): the watchdog fired ${t} s after lemnosd stopped (logged by QEMU's debug action instead of a reset)"
      capture_slots gnew "trial-before-reboot"
      untouched_check gnew "trial-before-reboot"
      gnew 'systemctl reboot' || true ;;
    pid1)
      wait_login new 300 || die "no login on the new slot"
      gnew 'systemctl is-active bench-trial-deadman' || die "the dead-man is not running"
      gnew 'kill -STOP 1' || true
      wait_boot $((b0 + 1)) $((N + 120)) || die "with pid 1 stopped the dead-man did not reboot within N + 120 s" ;;
  esac
  wait_boot $((b0 + 1)) 300 || die "no boot back to 993"
  return_steps "$b0"
  seed_check 993 returned
  can_stage_again
  say "PASSED"
  stop_harness
}

# probe: boots 993 and tries, read-only, each command form the steps run on 993 (BusyBox options), then powers off.
# Nothing is written to p1, p2 or /data: the only write is a scratch file under /tmp (tmpfs).
probe() {
  start_harness
  say "probe: 993's command forms"
  # ONE ssh session for the whole battery (a tight loop of connections gets 993's sshd to refuse the source, as the
  # first probe did after ~10): the guest's sh reads the lines and prints each one's result.
  gssh 993 'while IFS= read -r t; do [ -n "$t" ] || continue; if sh -c "$t" >/dev/null 2>&1 </dev/null; then echo "OK    $t"; else echo "FAIL  $t"; fi; done' \
    > "$state/probe.txt" <<'T'
df /data
df -i /data
stat -f -c '%c %d' /data
sha256sum /proc/cmdline > /tmp/p.sha && sha256sum -c -s /tmp/p.sha
sha256sum /proc/cmdline > /tmp/p.sha && sha256sum -c -s /tmp/p.sha
dd if=/dev/vda1 bs=1M count=1 status=none of=/dev/null
dd if=/dev/vda1 bs=512 skip=8 count=8 status=none of=/dev/null
cp -a /proc/version /tmp/p.v && cp -p /proc/version /tmp/p.v2
find /data/board/slots -type f -exec sha256sum {} + > /dev/null
find / -xdev -type f -perm /6000
ls -la /data/board/slots /data/board
blkid
command -v e2label
uname -r
cut -d. -f1 /proc/uptime
tar --help
stat -c "helper %a %U:%G" /usr/libexec/dbus-daemon-launch-helper
systemctl is-active dbus
dbus-send --system --print-reply --dest=org.freedesktop.DBus / org.freedesktop.DBus.GetId
command -v mcopy
T
  cat "$state/probe.txt"
  # What 993 keeps on /data, by owner (the first read-only step on Raze #2 too): every dir and file to depth 3, numeric
  # and named owners, modes; plus the mounts that put /data under /var.
  gssh 993 'find /data -xdev -maxdepth 3 \( -path /data/lost+found -prune \) -o -exec ls -ldn {} +; echo ---; find /data -xdev -maxdepth 6 \( -path "/data/os/*" -o -path "/data/helios*" \) -exec ls -ld {} +; echo ---; grep -E " /data| /var" /proc/self/mountinfo' > "$state/data-owners.993.txt"
  cat "$state/data-owners.993.txt"
  stop_harness
}

if [[ $ending == probe ]]; then
  ending=probe; probe
elif [[ $ending == all ]]; then
  for e in login nologin stopped nomarker pid1 plain; do run_ending "$e"; done
  echo "rehearse: all endings PASSED"
else
  run_ending "$ending"
fi
