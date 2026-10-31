#!/bin/sh
# Apply the USB boot udev rules (Pi, Rockchip, K230) now, including to a board already plugged in.
udevadm control --reload-rules >/dev/null 2>&1 || true
for vendor in 0a5c 2207 29f1; do
	udevadm trigger --action=add --subsystem-match=usb --attr-match=idVendor=$vendor >/dev/null 2>&1 || true
done
exit 0
