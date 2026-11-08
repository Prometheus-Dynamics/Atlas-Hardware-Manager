# shellcheck shell=sh
# The BCM2712 (Pi 5, CM5) boot chip's part of the board scripts, sourced by
# lib.sh after its defaults: what the Pi firmware says about a boot, and how
# slots tell it what to boot (`slots` has mounted the boot FAT as $BOOT).

# The USB device controller's module, and what a missing one usually means.
BOARD_UDC_MODULE=dwc2
BOARD_UDC_HINT='is dtoverlay=dwc2,dr_mode=peripheral set?'

# The reset register (PM_RSTS) as the bootloader saw it at this boot
# (chosen/bootloader/rsts), as a number; empty off a Pi.
board_rsts() {
	[ -r "$BOARD_DT_DIR/chosen/bootloader/rsts" ] || return 0
	od -An -tu1 -N4 "$BOARD_DT_DIR/chosen/bootloader/rsts" |
		awk 'NF == 4 { printf "%d", $1 * 16777216 + $2 * 65536 + $3 * 256 + $4 }'
}

# Whether the firmware booted this time as a tryboot (chosen/bootloader/
# tryboot): 1 or 0. Under a runner (QEMU) its atlas.tryboot=0|1 instead;
# empty when neither says.
board_tryboot() {
	if [ ! -r "$BOARD_DT_DIR/chosen/bootloader/tryboot" ]; then
		board_cmdline_arg atlas.tryboot
		case "$BOARD_ARG" in 0 | 1) echo "$BOARD_ARG" ;; esac
		return 0
	fi
	od -An -tu1 -N4 "$BOARD_DT_DIR/chosen/bootloader/tryboot" | awk 'NF == 4 { print ($4 || $3 || $2 || $1) ? 1 : 0 }'
}

# How this boot began: "power-on" (PM_RSTS bit 12: the board had lost power,
# which also forgets a pending tryboot: a bench brown-out), "reset" (bit 5:
# the watchdog, which a reboot and a panic restart use too), else empty.
board_reset_cause() {
	_board_rsts=$(board_rsts)
	[ -n "$_board_rsts" ] || return 0
	if [ $((_board_rsts & 4096)) -ne 0 ]; then
		echo power-on
	elif [ $((_board_rsts & 32)) -ne 0 ]; then
		echo reset
	fi
}

# Sets BOARD_BL_VERSION and BOARD_BL_TIMESTAMP (empty when unknown). Reads the device
# tree first (no VideoCore access needed), then `vcgencmd bootloader_version`.
board_bootloader() {
	BOARD_BL_VERSION=''
	BOARD_BL_TIMESTAMP=''
	_board_bl="$BOARD_DT_DIR/chosen/bootloader"
	if [ -r "$_board_bl/version" ]; then
		BOARD_BL_VERSION=$(tr -d '\000' < "$_board_bl/version" | tr -cd 'A-Za-z0-9._-')
	fi
	if [ -r "$_board_bl/build-timestamp" ]; then
		BOARD_BL_TIMESTAMP=$(od -An -tu1 -N4 "$_board_bl/build-timestamp" |
			awk 'NF == 4 { printf "%d", $1 * 16777216 + $2 * 65536 + $3 * 256 + $4 }')
	fi
	if [ -z "$BOARD_BL_VERSION" ] && command -v vcgencmd >/dev/null 2>&1; then
		_board_out=$(vcgencmd bootloader_version 2>/dev/null) || _board_out=''
		BOARD_BL_VERSION=$(printf '%s\n' "$_board_out" | sed -n 's/^version \([0-9A-Za-z]*\).*/\1/p' | head -n 1)
		if [ -z "$BOARD_BL_TIMESTAMP" ]; then
			BOARD_BL_TIMESTAMP=$(printf '%s\n' "$_board_out" | sed -n 's/^timestamp \([0-9]*\).*/\1/p' | head -n 1)
		fi
	fi
}

# True when this board can restart straight into USB boot (RPIBOOT) without
# its button: a BCM2712 (Pi 5/CM5) firmware takes a one-time boot order
# through the mailbox (set_reboot_order), and vcmailbox is installed.
#   BOARD_VCMAILBOX  the vcmailbox command   (vcmailbox)
board_usb_boot_supported() {
	tr '\000' '\n' 2>/dev/null < "$BOARD_DT_DIR/compatible" | grep -qx 'brcm,bcm2712' &&
		command -v "${BOARD_VCMAILBOX:-vcmailbox}" >/dev/null 2>&1
}

# board_boot_hashes <boot dir>: "<sha256>  <path>" lines for the files that
# decide how a slot boots: config.txt, cmdline.txt, kernel*.img, overlays/*.
board_boot_hashes() {
	(
		cd "$1" 2>/dev/null || exit 0
		for _board_f in config.txt cmdline.txt kernel*.img overlays/*; do
			[ -f "$_board_f" ] || continue
			printf '%s  %s\n' "$(sha256sum < "$_board_f" | cut -d' ' -f1)" "$_board_f"
		done
	)
}

# --- slots -------------------------------------------------------------------

# boot_set_default <id>: the firmware's next ordinary boot starts <id>.
boot_set_default() { write_line default.txt "os_prefix=slots/$1/"; }

# boot_set_trial <id>: the next trial restart starts <id>. A runner (QEMU)
# boots with atlas.tryboot=0|1 and never sees the reboot argument, so for it
# the "0 tryboot" is p1's tryboot.req too, which it deletes as it starts the
# one trial boot (slot-init leaves it the same way).
boot_set_trial() {
	write_line trial.txt "os_prefix=slots/$1/"
	board_cmdline_arg atlas.tryboot
	[ -z "$BOARD_ARG" ] || write_line tryboot.req ''
}

# boot_restart_trial: restart once into the trial slot, after the bench's lock.
boot_restart_trial() {
	if [ -n "${UPDATE_REBOOT:-}" ]; then
		eval "$UPDATE_REBOOT"
		return
	fi
	"$BOARD_LIB_DIR/reboot-locked" "$SYSTEMCTL" reboot --reboot-argument='0 tryboot'
}

# boot_image_default <FAT dir>: the slot a disk image boots by default.
boot_image_default() { sed -n 's|^os_prefix=slots/\(.*\)/$|\1|p' "$1/default.txt" 2>/dev/null | head -n 1; }

# boot_prepare_slot <slot boot dir>: the firmware reads overlays from os_prefix
# only when overlays/README exists; without it the slot gets the root overlays
# (the default slot's). Bundles from before slots.toml shipped one get it here.
boot_prepare_slot() {
	if [ -d "$1/overlays" ] && [ ! -e "$1/overlays/README" ]; then
		printf 'Overlays for this slot. The firmware reads them from os_prefix only when this file exists.\n' > "$1/overlays/README"
	fi
}

# missing_overlays <boot FAT> <slot's overlays dir>: the overlays the board's
# config (config.txt, the board's fragment) loads that the slot doesn't carry,
# one a line. The firmware reads a slot's overlays from its own overlays/ (it
# has a README), so a slot without one boots without that device (the shipped
# PhotonVision 2026 images: no camera).
missing_overlays() {
	board_load_env board-package.env
	for _cfg in "$1/config.txt" ${BOARD_DEVICE_CONFIG:+"$1/$BOARD_DEVICE_CONFIG"}; do
		[ -f "$_cfg" ] && tr -d '\r' < "$_cfg" | sed -n 's/^[[:space:]]*dtoverlay=\([^,[:space:]]*\).*/\1/p'
	done | sort -u | while IFS= read -r _o; do
		[ -n "$_o" ] || continue
		ls -1 "$2" 2>/dev/null | tr 'A-Z' 'a-z' | grep -qx "$(printf '%s' "$_o" | tr 'A-Z' 'a-z').dtbo" || echo "$_o"
	done
}
