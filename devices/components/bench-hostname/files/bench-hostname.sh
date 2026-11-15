#!/bin/sh
# Names the bench from its serial: <prefix>-<last six hex digits>, the prefix from /etc/bench-hostname.prefix (each product's own). The serial is the device tree's serial-number, the same
# source usb-gadget-network's script hashes its addresses from, so the name and the USB addresses come from one value.
# Runs before networkd and resolved, so both announce the name. Writes the kernel's hostname (the root is read-only, so
# /etc/hostname cannot change). No serial: the base's static name stays.
set -eu
serial=$(tr -d '\000' < /proc/device-tree/serial-number 2>/dev/null || true)
hex=$(printf '%s' "$serial" | tr 'A-F' 'a-f')
case "$hex" in
  *[!0-9a-f]* | "") echo "bench-hostname: no hex serial (kept the base name)" >&2; exit 0 ;;
esac
prefix=$(tr -d '\n' < /etc/bench-hostname.prefix 2>/dev/null || true)
if [ -z "$prefix" ]; then
  echo "bench-hostname: no /etc/bench-hostname.prefix (kept the base name)" >&2
  exit 0
fi
if [ "${#hex}" -lt 6 ]; then
  echo "bench-hostname: serial too short (kept the base name)" >&2
  exit 0
fi
name="$prefix-$(printf '%s' "$hex" | tail -c 6)"
echo "$name" > /proc/sys/kernel/hostname
echo "bench-hostname: $name"
