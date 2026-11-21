#!/bin/sh
# Checkpoint 7 (route-A-steps.md, coordinator 2026-11-20): a line every 60 s with the time, every zone's temperature
# (millidegrees) and the fan line's state, to /run/cp7.log and kmsg. The stop rule acts here as well: any zone at
# 70000 or more, or line 45 not held high, means the plain `systemctl reboot` to 993 at once.
# The pattern is "|the fan header" because the kernel's gpio debugfs cuts the consumer at 20 characters.
while :; do
  z=""; hot=""
  for d in /sys/class/thermal/thermal_zone*; do
    t=$(cat $d/temp); z="$z$(cat $d/type)=$t "
    [ "$t" -ge 70000 ] && hot="$hot $(cat $d/type)=$t"
  done
  g=$(grep -F "|the fan header" /sys/kernel/debug/gpio | tr -s ' ')
  l="$(date +%T) up=$(cut -d' ' -f1 /proc/uptime) $z$g"
  echo "$l" >> /run/cp7.log; echo "cp7: $l" > /dev/kmsg
  if [ -n "$hot" ]; then echo "cp7: STOP at 70 C:$hot" | tee -a /run/cp7.log > /dev/kmsg; systemctl reboot; exit 0; fi
  case "$g" in *"out hi"*) ;; *) echo "cp7: STOP line 45 not held high: [$g]" | tee -a /run/cp7.log > /dev/kmsg; systemctl reboot; exit 0 ;; esac
  sleep 60
done
