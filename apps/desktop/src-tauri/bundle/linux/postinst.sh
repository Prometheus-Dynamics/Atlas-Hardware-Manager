#!/bin/sh
# Apply the Pi USB boot udev rule now, including to a board already plugged in.
udevadm control --reload-rules >/dev/null 2>&1 || true
udevadm trigger --action=add --subsystem-match=usb --attr-match=idVendor=0a5c >/dev/null 2>&1 || true
exit 0
