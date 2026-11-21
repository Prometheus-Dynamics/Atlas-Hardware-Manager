#!/bin/bash
# Route A on Raze #2, steps 1-3 (route-A-steps.md v4). The commands are the rehearsal's (rehearse-route-a.next.sh),
# with gssh pointed at the board: Raze #2's 993 over the PC's usb0 link, host key pinned after a serial check.
# Usage: board-route-a.sh <env> <step1|step2|step3>
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
source "$1"; shift
export R2_HOST R2_KNOWN_HOSTS
: "${CTL:?}" "${SOCS:?}" "${BOARD_A1:?}" "${PAYLOAD:?}" "${FIRMWARE:?}" "${DATA:?}" "${SLOT:?}" "${SERIAL:?}" "${R2:?}" "${state:?}"
export TMPDIR
mkdir -p "$state"
ending=board
die() { echo "board-route-a FAIL: $*" >&2; exit 1; }
say() { echo "board-route-a $(date +%T) $*"; }
gssh() {  # <993> <command...>   paced: at least 2 s between connections
  shift
  local now last; now=$(date +%s%N); last=$(cat "$state/.gssh-last" 2>/dev/null || echo 0)
  if (( now - last < 2000000000 )); then sleep "$(( (2000000000 - (now - last)) / 1000000 ))e-3"; fi
  date +%s%N > "$state/.gssh-last"
  "$R2" "$@"
}
serial_check() {
  [[ $(gssh 993 'tr -d "\0" < /proc/device-tree/serial-number') == "$SERIAL" ]] || die "serial check: not $SERIAL"
}
step1() {
  say "step 1: relabel p2"
  serial_check 993
  gssh 993 'e2label /dev/mmcblk0p2 lemnos-data'
  [[ $(gssh 993 'blkid | grep -c "LABEL=\"lemnos-data\""') == 1 ]] || die "step 1: not exactly one lemnos-data label"
  gssh 993 cat /proc/cmdline | grep -q 'PARTUUID=5056525a-02' || die "step 1: 993's PARTUUID word is gone"
  gssh 993 blkid > "$state/step1-blkid.txt"
  say "step 1: done (exactly one LABEL=lemnos-data; the PARTUUID word is in 993's cmdline)"
}
step2() {
  say "step 2: the new slot's files on /data"
  serial_check 993
  (cd "$DATA" && find slots -type f -exec sha256sum {} +) > "$state/d.sha256"
  gssh 993 'for f in '"$(cd "$DATA" && find slots -type f | tr '\n' ' ')"'; do [ ! -e /data/$f ] || { echo "/data/$f exists"; exit 1; }; done' || die "step 2: a file of D is already on /data"
  tar -C "$DATA/slots" -cf - . | gssh 993 'tar -C /data/slots --no-same-owner --no-same-permissions -xf - && chown -R 0:0 /data/slots/bootfiles && sync'
  gssh 993 'cd /data && sha256sum -c -s' < "$state/d.sha256" || die "step 2: D's hashes on /data"
  gssh 993 'ls -ln /data/slots; df -k /data; tune2fs -l /dev/mmcblk0p2 | grep -E "Free inodes"' > "$state/step2-after.txt"
  say "step 2: done ($(wc -l < "$state/d.sha256") files, hashes match)"
}
step3() {
  local set=0 dev=/dev/mmcblk0p1
  say "step 3: --add-to set $set on a dump, verify, compare, write back, re-dump"
  serial_check 993
  gssh 993 'if mountpoint -q /run/atlas-p1ro; then umount /run/atlas-p1ro; fi'
  gssh 993 "dd if=$dev bs=1M status=none" > "$state/p1.before.img"
  [[ $(stat -c %s "$state/p1.before.img") == 82837504 ]] || die "step 3: the dump is not 82,837,504 B"
  sha256sum "$state/p1.before.img" > "$state/p1.before.sha256"
  rm -f "$state/p1.new.img" "$state/p1.changed" "$state/p1.undo"
  "$CTL" boot-partition --add-to "$state/p1.before.img" --layout-of "$BOARD_A1" --socs "$SOCS" --slot "$SLOT" \
    --payload "$PAYLOAD" --firmware "$FIRMWARE" --set "$set" --replace-trial \
    --out "$state/p1.new.img" --changed-out "$state/p1.changed" --undo-out "$state/p1.undo" || die "step 3: --add-to"
  "$CTL" boot-partition --verify "$state/p1.new.img" --layout-of "$BOARD_A1" --socs "$SOCS" > "$state/verify.txt" 2>&1 || true
  [[ $(grep -c 'BAD' "$state/verify.txt") == 2 ]] && grep -q '^.*default\.txt: record file is 55 bytes' "$state/verify.txt" \
    && grep -q 'config\.txt: record file is 405 bytes' "$state/verify.txt" || { cat "$state/verify.txt" >&2; die "step 3: the verify's BAD lines are not exactly the two expected"; }
  # 993's files byte-identical apart from the two replaced texts (trial.txt, tryboot.txt)
  mdir -/ -b -i "$state/p1.before.img" :: | sort > "$state/files.before"
  local f
  while read -r f; do
    case "${f#::/}" in trial.txt|tryboot.txt) continue ;; esac
    [[ $f == */ ]] && continue
    cmp <(mtype -i "$state/p1.before.img" "$f") <(mtype -i "$state/p1.new.img" "$f") || die "step 3: 993's $f changed"
  done < "$state/files.before"
  echo "$(wc -l < "$state/files.before") entries of 993 compared" > "$state/compare.txt"
  fsck.fat -n "$state/p1.new.img" > "$state/fsck.txt" || die "step 3: fsck.fat on the new image"
  [[ $(gssh 993 "sha256sum $dev" | cut -d' ' -f1) == $(cut -d' ' -f1 "$state/p1.before.sha256") ]] \
    || die "step 3: p1 changed since the dump"
  serial_check 993
  writeback "$state/p1.new.img" "$state/p1.changed" forward
  gssh 993 "dd if=$dev bs=1M status=none" > "$state/p1.after.img"
  cmp "$state/p1.after.img" "$state/p1.new.img" || die "step 3: the re-dump is not the prepared image"
  say "step 3: done (verify: the two expected BAD lines only; 993's files equal; $(grep -vc '^#' "$state/p1.changed") ranges written and read back; the whole re-dump equals p1.new.img)"
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
undo3() {  # step 3's way back: p1.restored.img's ranges in reverse class order, then the whole re-dump
  local dev=/dev/mmcblk0p1
  say "undo step 3: write back p1.restored.img's ranges (dirs, fat1, fat2, data), re-dump, compare with p1.before.img"
  serial_check 993
  cmp "$state/p1.restored.img" "$state/p1.before.img" || die "undo 3: p1.restored.img is not p1.before.img"
  [[ $(gssh 993 "sha256sum $dev" | cut -d' ' -f1) == $(sha256sum < "$state/p1.after.img" | cut -d' ' -f1) ]] || die "undo 3: p1 is not step 3's re-dump"
  writeback "$state/p1.restored.img" "$state/p1.changed" reverse
  gssh 993 "dd if=$dev bs=1M status=none" > "$state/p1.undone.img"
  cmp "$state/p1.undone.img" "$state/p1.before.img" || die "undo 3: the re-dump is not p1.before.img"
  say "undo step 3: done (31 ranges written and read back; the whole re-dump equals p1.before.img)"
}
p1dev() { echo /dev/mmcblk0p1; }
"$1"
