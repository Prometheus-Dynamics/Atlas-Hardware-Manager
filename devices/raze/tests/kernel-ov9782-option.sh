#!/bin/sh
# What BR2_LINUX_KERNEL_EXT_OV9782 does to the kernel build: on, the kernel's
# ov9282 is patched and built; off, it's left unpatched and turned off, so an
# image's own driver is the only one claiming the sensor. The stock overlays
# are built either way. Run: sh devices/raze/tests/kernel-ov9782-option.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
tree=$here/../gaia/buildroot-external
command -v make >/dev/null 2>&1 || {
	echo "kernel-ov9782-option: skipped (no make)"
	exit 0
}
fail() {
	echo "kernel-ov9782-option: $*" >&2
	exit 1
}
hooks() {
	printf 'BR2_EXTERNAL_RAZE_DEVICE_PATH := %s\nBR2_LINUX_KERNEL := y\nBR2_LINUX_KERNEL_EXT_OV9782 := %s\ninclude %s/linux/linux-ext-ov9782.mk\ninclude %s/external.mk\nall:\n\t@echo $(LINUX_PRE_PATCH_HOOKS) $(LINUX_POST_CONFIGURE_HOOKS) $(LINUX_POST_BUILD_HOOKS) $(LINUX_POST_INSTALL_IMAGES_HOOKS)\n' \
		"$tree" "$1" "$tree" "$tree" | make -s -f - all
}
has() { case " $2 " in *" $1 "*) ;; *) fail "$3: $1 missing in: $2" ;; esac; }
lacks() { case " $2 " in *" $1 "*) fail "$3: $1 should be gone: $2" ;; esac; }

on=$(hooks y)
has OV9782_PREPARE_KERNEL "$on" on
has OV9782_ENABLE_CONFIG "$on" on
lacks OV9782_DISABLE_CONFIG "$on" on
has RAZE_BUILD_KERNEL_OVERLAYS "$on" on
has RAZE_INSTALL_KERNEL_OVERLAYS "$on" on

off=$(hooks '')
lacks OV9782_PREPARE_KERNEL "$off" off
lacks OV9782_ENABLE_CONFIG "$off" off
has OV9782_DISABLE_CONFIG "$off" off
has RAZE_BUILD_KERNEL_OVERLAYS "$off" off
has RAZE_INSTALL_KERNEL_OVERLAYS "$off" off
echo "kernel-ov9782-option: ok"
