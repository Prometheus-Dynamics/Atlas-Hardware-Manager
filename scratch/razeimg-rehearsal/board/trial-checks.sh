#!/bin/sh
# Route A step 4 checks in the new slot (read-only), sections printed for the PC to save. Usage: trial-checks.sh <part>
case "$1" in
state)  # the boot's own state, read before anything is stopped or started
  echo "==serial $(tr -d '\0' < /proc/device-tree/serial-number)"; echo "==uname $(uname -r)"; echo "==uptime $(cat /proc/uptime)"
  echo "==cmdline"; cat /proc/cmdline
  echo "==is-system-running $(timeout 60 systemctl is-system-running --wait)"
  for u in lemnosd lemnos-orion orion-comm orion-node bench-trial-deadman; do echo "==unit $u $(systemctl show $u -p ActiveState -p SubState -p NRestarts --value | tr '\n' ' ')"; done
  echo "==failed"; systemctl --failed --no-legend --no-pager
  pid=$(systemctl show -p MainPID --value lemnosd); echo "==lemnosd pid $pid"; grep -E '^(Uid|Gid|Groups|CapEff):' /proc/$pid/status
  echo "==run-lemnos"; ls -ldn /run/lemnos; ls -ld /run/lemnos ;;
cp)  # the first-flash checkpoints 1-6
  echo "==list"; lemnos-ctl list
  echo "==lsmod"; lsmod | grep -i lemnos; echo "==dev-lemnos"; ls -l /dev/lemnos* 2>&1
  echo "==boot-mode"; for f in /proc/device-tree/chosen/bootloader/boot-mode /proc/device-tree/chosen/bootloader/partition; do [ -e $f ] && echo "$f $(od -An -tx1 $f)"; done; tr -d '\0' < /proc/device-tree/chosen/bootloader/tryboot 2>/dev/null | od -An -tx1; grep -o 'rpi-boot-mode[^ ]*' /proc/cmdline
  echo "==dt-marker"; find /proc/device-tree -name 'lemnos,*' 2>/dev/null | head -20; for f in $(find /proc/device-tree -name 'lemnos,overlay-sha256' -o -name 'lemnos,board' 2>/dev/null); do echo "$f: $(tr -d '\0' < $f)"; done
  echo "==ramoops"; ls /proc/device-tree/reserved-memory/ 2>/dev/null | grep -i ramoops
  echo "==watchdog"; ls -l /dev/watchdog*; for f in identity state timeout timeleft; do echo "watchdog0 $f: $(cat /sys/class/watchdog/watchdog0/$f 2>&1)"; done
  echo "==slotinit-kmsg"; dmesg | grep -E 'slot-init|watchdog|wdt' | head -40
  echo "==lemnosd-watchdog"; journalctl -u lemnosd -o short-monotonic --no-pager | grep -iE 'watchdog|running ok|retrying' | head -40
  echo "==gadget"; ls /sys/class/udc/ 2>&1; cat /sys/class/udc/*/state 2>&1
  echo "==getty"; systemctl status serial-getty@ttyGS0 --no-pager 2>&1 | head -4 ;;
slots)  # route A: lemnosd leaves the slots state alone (past 45 s of uptime)
  echo "==slots-list"; lemnos-ctl call slots list; echo; echo "==slots-info"; lemnos-ctl call slots info; echo
  echo "==confirm-lines"; journalctl -u lemnosd -o cat --no-pager | grep -ci confirm
  echo "==p1"; mkdir -p /run/a1p1; mountpoint -q /run/a1p1 || mount -o ro /dev/mmcblk0p1 /run/a1p1; sha256sum /run/a1p1/default.txt /run/a1p1/trial.txt; umount /run/a1p1
  echo "==board-slots"; ls -la /data/board/slots/; (cd /data/board/slots && sha256sum *)
  echo "==993-dirs"; ls -ldin /data/os/helios/helios/orion /data/os/helios/helios/state /data/log/helios ;;
esac
