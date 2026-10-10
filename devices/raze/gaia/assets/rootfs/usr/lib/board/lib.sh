# shellcheck shell=sh
# Shared helpers for the board scripts (device package contract 1).
# POSIX sh; also runs under busybox ash. Sourced, never executed.
#
# Every input path can be overridden through the environment, which is how the
# scripts are tested off-device:
#   BOARD_LIB_DIR      package files and defaults   (/usr/lib/board)
#   BOARD_ETC_DIR      OS overrides                 (/etc/board)
#   BOARD_DATA_DIR     user overrides, persistent   (/data/board; the root
#                   may be read-only)
#   BOARD_RUN_DIR      generated runtime state      (/run/board)
#   BOARD_DT_DIR       device tree                  (/proc/device-tree)
#   BOARD_NET_DIR      network interfaces           (/sys/class/net)
#   BOARD_OS_RELEASE   os-release file              (/etc/os-release, then /usr/lib/os-release)
#   BOARD_MACHINE_ID   machine id file              (/etc/machine-id)
#   BOARD_HOSTNAME_PROC  kernel hostname            (/proc/sys/kernel/hostname)

BOARD_LIB_DIR=${BOARD_LIB_DIR:-/usr/lib/board}
BOARD_ETC_DIR=${BOARD_ETC_DIR:-/etc/board}
BOARD_DATA_DIR=${BOARD_DATA_DIR:-/data/board}
BOARD_RUN_DIR=${BOARD_RUN_DIR:-/run/board}
BOARD_DT_DIR=${BOARD_DT_DIR:-/proc/device-tree}
BOARD_NET_DIR=${BOARD_NET_DIR:-/sys/class/net}
BOARD_OS_RELEASE=${BOARD_OS_RELEASE:-}
BOARD_MACHINE_ID=${BOARD_MACHINE_ID:-/etc/machine-id}
BOARD_HOSTNAME_PROC=${BOARD_HOSTNAME_PROC:-/proc/sys/kernel/hostname}

# The root filesystem's block device, e.g. /dev/mmcblk0p5. The kernel command
# line names it (resolving PARTUUID=, UUID=, LABEL=); /proc/mounts is only a
# fallback, since it says /dev/root on a Pi and overlay on an overlay root.
# Empty when there is no block device behind / (overlay on tmpfs, ramdisk).
#   BOARD_PROC_CMDLINE  kernel command line   (/proc/cmdline)
board_root_device() {
	_board_src=$(tr ' ' '\n' < "${BOARD_PROC_CMDLINE:-/proc/cmdline}" 2>/dev/null | sed -n 's/^root=//p' | tail -n 1)
	case "$_board_src" in
	PARTUUID=* | UUID=* | LABEL=*)
		_board_src=$(findfs "$_board_src" 2>/dev/null || blkid -l -o device -t "$_board_src" 2>/dev/null || true)
		;;
	esac
	case "$_board_src" in
	/dev/root | /dev/ram* | '')
		_board_src=$(awk '$2 == "/" { print $1 }' /proc/mounts 2>/dev/null | tail -n 1)
		;;
	esac
	case "$_board_src" in
	/dev/*[0-9]) echo "$_board_src" ;;
	esac
}

# Partition <n> of the disk the root device <dev> is on.
board_sibling_partition() {
	case "$1" in
	*[0-9]p[0-9]*) echo "${1%p[0-9]*}p$2" ;; # mmcblk0p2, nvme0n1p2
	*[a-z][0-9]*) echo "${1%%[0-9]*}$2" ;;   # sda2
	*) return 1 ;;
	esac
}

board_log() {
	printf '%s: %s\n' "${0##*/}" "$*" >&2
}

# Source <name> from the package defaults, then the OS overrides in
# /etc/board, then the user's in /data/board; the last value wins.
board_load_env() {
	# /data/board comes last, so a setting survives on a read-only root.
	# board-package.env describes the image itself and is never taken
	# from /data.
	_board_data="$BOARD_DATA_DIR/$1"
	[ "$1" != board-package.env ] || _board_data=''
	for _board_env in "$BOARD_LIB_DIR/$1" "$BOARD_ETC_DIR/$1" $_board_data; do
		if [ -r "$_board_env" ]; then
			# shellcheck disable=SC1090
			. "$_board_env"
		fi
	done
}

# First non-empty, non-comment line of the first readable file given.
board_first_line() {
	for _board_f in "$@"; do
		if [ -r "$_board_f" ]; then
			sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e '/^#/d' -e '/^$/d' "$_board_f" | head -n 1
			return 0
		fi
	done
	return 0
}

# Board serial from the device tree, as lowercase hex. Empty when unreadable.
board_serial() {
	if [ -r "$BOARD_DT_DIR/serial-number" ]; then
		tr -d '\000' < "$BOARD_DT_DIR/serial-number" | tr 'A-F' 'a-f' | tr -cd '0-9a-f'
	fi
}

board_machine_id() {
	if [ -r "$BOARD_MACHINE_ID" ]; then
		tr -cd '0-9a-f' < "$BOARD_MACHINE_ID"
	fi
}

# Last 8 characters of the serial; first 8 of the machine id without a serial.
board_serial8() {
	_board_s=$(board_serial)
	if [ -z "$_board_s" ]; then
		_board_s=$(board_machine_id)
		printf '%s' "$_board_s" | cut -c1-8
		return 0
	fi
	if [ "${#_board_s}" -le 8 ]; then
		printf '%s' "$_board_s"
	else
		printf '%s' "${_board_s#"${_board_s%????????}"}"
	fi
}

# Seed for derived MAC addresses: the board serial, else the machine id. Never
# a constant: prints nothing when neither exists, and callers then leave the
# kernel's random addresses alone.
board_mac_seed() {
	_board_s=$(board_serial)
	if [ -n "$_board_s" ]; then
		printf '%s' "$_board_s"
		return 0
	fi
	_board_s=$(board_machine_id)
	if [ -n "$_board_s" ]; then
		printf 'machine-id:%s' "$_board_s"
	fi
}

# Base MAC: 02 (locally administered, unicast) followed by the first five bytes
# of sha256(seed). Same rule as the earlier gadget script, so existing devices
# keep their host-side addresses.
board_mac_base() {
	_board_seed=$(board_mac_seed)
	[ -n "$_board_seed" ] || return 1
	printf '%s' "$_board_seed" | sha256sum | cut -c1-10 |
		sed 's/\(..\)\(..\)\(..\)\(..\)\(..\)/02:\1:\2:\3:\4:\5/'
}

# board_mac_variant <mac> <mask>: XOR the last byte with mask (e.g. 0x10).
board_mac_variant() {
	_board_pre=${1%:*}
	_board_last=${1##*:}
	printf '%s:%02x\n' "$_board_pre" $((0x$_board_last ^ $2))
}

# USB gadget addressing, scheme serial-hash-v1 (manifest.json
# capabilities.gadget-net.addressing; Atlas computes the same in Rust).
#
# The gadget's USB serial, normalized to its last 8 hex digits in lowercase,
# picks one /USB_GADGET_ADDR_PREFIX subnet of USB_GADGET_ADDR_BASE: the first
# 4 bytes of sha256(those 8 ASCII characters), as a big-endian number, modulo
# the number of subnets outside USB_GADGET_ADDR_EXCLUDE, index the remaining
# subnets in ascending order. The device takes the subnet's first host
# address. A serial that isn't all hex or is shorter than 8 digits has no
# subnet, and the board keeps BOARD_GADGET_LEGACY_ADDRESS.
BOARD_GADGET_LEGACY_ADDRESS=172.31.250.1/24

# Dotted IPv4 address to an integer. Fails on anything else.
board_ip4_int() {
	printf '%s\n' "$1" | awk -F. '
		NF != 4 { exit 1 }
		{
			for (i = 1; i <= 4; i++) {
				if ($i !~ /^[0-9]+$/ || $i + 0 > 255) { exit 1 }
			}
			printf "%.0f\n", (($1 * 256 + $2) * 256 + $3) * 256 + $4
		}'
}

# Integer to dotted IPv4.
board_int_ip4() {
	printf '%d.%d.%d.%d\n' $(($1 >> 24 & 255)) $(($1 >> 16 & 255)) $(($1 >> 8 & 255)) $(($1 & 255))
}

# board_board_serial8 <serial>: Atlas's board-serial rule (normalize_board_serial):
# NULs and surrounding whitespace dropped, all hex, at least 8 digits; prints
# the last 8 in lowercase. Fails otherwise.
board_board_serial8() {
	_board_s=$(printf '%s' "$1" | tr -d '\000')
	while :; do
		case "$_board_s" in
		[[:space:]]*) _board_s=${_board_s#?} ;;
		*[[:space:]]) _board_s=${_board_s%?} ;;
		*) break ;;
		esac
	done
	case "$_board_s" in
	'' | *[!0-9A-Fa-f]*) return 1 ;;
	esac
	[ "${#_board_s}" -ge 8 ] || return 1
	printf '%s\n' "${_board_s#"${_board_s%????????}"}" | tr 'A-F' 'a-f'
}

# board_gadget_subnet <usb serial>: this board's gadget address under
# serial-hash-v1, as <address>/<prefix> (for example 172.31.209.217/29).
# Fails when the serial gives no subnet or the settings are invalid.
#   USB_GADGET_ADDR_BASE     the block subnets come from  (172.31.0.0/16)
#   USB_GADGET_ADDR_PREFIX   each board's subnet length   (29)
#   USB_GADGET_ADDR_EXCLUDE  CIDRs left out, space-separated (172.31.250.0/24)
board_gadget_subnet() {
	_board_s8=$(board_board_serial8 "$1") || return 1
	_board_base=${USB_GADGET_ADDR_BASE:-172.31.0.0/16}
	_board_prefix=${USB_GADGET_ADDR_PREFIX:-29}
	_board_excl=${USB_GADGET_ADDR_EXCLUDE-172.31.250.0/24}
	_board_bpre=${_board_base#*/}
	case "$_board_bpre$_board_prefix" in
	'' | *[!0-9]*) return 1 ;;
	esac
	[ "$_board_bpre" -le "$_board_prefix" ] && [ "$_board_prefix" -le 30 ] || return 1
	_board_bnet=$(board_ip4_int "${_board_base%/*}") || return 1
	_board_size=$((1 << (32 - _board_prefix)))
	_board_slots=$((1 << (_board_prefix - _board_bpre)))
	_board_bnet=$((_board_bnet - _board_bnet % (_board_size * _board_slots)))
	# Excluded subnets as "first-slot count" lines, ascending.
	_board_ranges=''
	for _board_c in $_board_excl; do
		_board_epre=${_board_c#*/}
		case "$_board_epre" in
		'' | *[!0-9]*) return 1 ;;
		esac
		[ "$_board_epre" -ge "$_board_bpre" ] && [ "$_board_epre" -le "$_board_prefix" ] || return 1
		_board_e=$(board_ip4_int "${_board_c%/*}") || return 1
		_board_e=$((_board_e - _board_e % (1 << (32 - _board_epre)) - _board_bnet))
		[ "$_board_e" -ge 0 ] && [ "$_board_e" -lt $((_board_size * _board_slots)) ] || return 1
		_board_ranges="$_board_ranges$((_board_e / _board_size)) $((1 << (_board_prefix - _board_epre)))
"
	done
	_board_ranges=$(printf '%s' "$_board_ranges" | sort -n -k1,1 | uniq)
	_board_usable=$_board_slots
	while read -r _board_start _board_len; do
		[ -n "$_board_start" ] || continue
		_board_usable=$((_board_usable - _board_len))
	done <<EOF
$_board_ranges
EOF
	[ "$_board_usable" -gt 0 ] || return 1
	_board_h=$(printf '%s' "$_board_s8" | sha256sum | cut -c1-8)
	_board_slot=$((0x$_board_h % _board_usable))
	while read -r _board_start _board_len; do
		[ -n "$_board_start" ] || continue
		if [ "$_board_start" -le "$_board_slot" ]; then
			_board_slot=$((_board_slot + _board_len))
		fi
	done <<EOF
$_board_ranges
EOF
	printf '%s/%s\n' "$(board_int_ip4 $((_board_bnet + _board_slot * _board_size + 1)))" "$_board_prefix"
}

# The gadget's USB serial string: USB_GADGET_SERIAL, or with AUTO the board
# serial, else the first 16 hex digits of the machine id.
board_gadget_serial() {
	_board_gs=${USB_GADGET_SERIAL:-AUTO}
	if [ "$_board_gs" = AUTO ]; then
		_board_gs=$(board_serial)
		[ -n "$_board_gs" ] || _board_gs=$(board_machine_id | cut -c1-16)
	fi
	printf '%s\n' "$_board_gs"
}

# The address the gadget bridge gets, after usb-gadget.env is loaded. Sets
# BOARD_GADGET_CIDR (<address>/<prefix>) and BOARD_GADGET_ADDRESSING:
#   serial-hash-v1  derived from the USB serial (USB_GADGET_ADDRESS=AUTO)
#   pinned          USB_GADGET_ADDRESS (a bare address means /24)
#   fallback        BOARD_GADGET_LEGACY_ADDRESS: the serial gives no subnet
board_gadget_address() {
	case "${USB_GADGET_ADDRESS:-AUTO}" in
	AUTO)
		if BOARD_GADGET_CIDR=$(board_gadget_subnet "$(board_gadget_serial)"); then
			BOARD_GADGET_ADDRESSING=serial-hash-v1
		else
			BOARD_GADGET_CIDR=$BOARD_GADGET_LEGACY_ADDRESS
			BOARD_GADGET_ADDRESSING=fallback
		fi
		;;
	*/*)
		BOARD_GADGET_CIDR=$USB_GADGET_ADDRESS
		BOARD_GADGET_ADDRESSING=pinned
		;;
	*)
		BOARD_GADGET_CIDR=$USB_GADGET_ADDRESS/24
		BOARD_GADGET_ADDRESSING=pinned
		;;
	esac
}

# board_gadget_dhcp_range <address>/<prefix>: dnsmasq's dhcp-range for the host
# end of the link: the addresses after the device's, up to 19 of them and the
# subnet's last host address, with the netmask and a 1h lease. Fails when the
# subnet has no room.
board_gadget_dhcp_range() {
	_board_pre=${1#*/}
	case "$_board_pre" in
	'' | *[!0-9]*) return 1 ;;
	esac
	[ "$_board_pre" -ge 8 ] && [ "$_board_pre" -le 30 ] || return 1
	_board_a=$(board_ip4_int "${1%/*}") || return 1
	_board_size=$((1 << (32 - _board_pre)))
	_board_last=$((_board_a - _board_a % _board_size + _board_size - 2))
	_board_end=$((_board_a + 19))
	[ "$_board_end" -le "$_board_last" ] || _board_end=$_board_last
	[ "$_board_a" -lt "$_board_end" ] || return 1
	printf '%s,%s,%s,1h\n' "$(board_int_ip4 $((_board_a + 1)))" "$(board_int_ip4 "$_board_end")" \
		"$(board_int_ip4 $((0xffffffff - _board_size + 1)))"
}

# Value of KEY in os-release, unquoted, with \" \\ \$ \` unescaped.
board_os_field() {
	_board_osr=$BOARD_OS_RELEASE
	if [ -z "$_board_osr" ]; then
		if [ -r /etc/os-release ]; then
			_board_osr=/etc/os-release
		else
			_board_osr=/usr/lib/os-release
		fi
	fi
	[ -r "$_board_osr" ] || return 0
	sed -n "s/^$1=//p" "$_board_osr" | head -n 1 |
		sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\\(.*\\)'\$/\\1/" -e 's/\\\(["\\$`]\)/\1/g'
}

board_hostname() {
	if [ -r "$BOARD_HOSTNAME_PROC" ]; then
		tr -d '\n' < "$BOARD_HOSTNAME_PROC"
	else
		uname -n
	fi
}

# Rev id: OS/user override file, then the package's rev-detect hook, then the
# package default. See manifest.json "rev_source".
board_rev() {
	_board_rev=$(board_first_line "$BOARD_ETC_DIR/rev")
	if [ -z "$_board_rev" ] && [ -x "$BOARD_LIB_DIR/rev-detect" ]; then
		_board_rev=$("$BOARD_LIB_DIR/rev-detect" 2>/dev/null | head -n 1)
	fi
	if [ -z "$_board_rev" ]; then
		_board_rev=${BOARD_DEVICE_REV_DEFAULT:-}
	fi
	printf '%s' "$_board_rev" | tr -cd 'A-Za-z0-9._-'
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

# JSON string literal for $1, or null when empty. Control characters are dropped.
board_json_str() {
	if [ -z "$1" ]; then
		printf 'null'
		return 0
	fi
	printf '"%s"' "$(printf '%s' "$1" | tr -d '\000-\037' | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')"
}

# Update method ids: "image-write" always, then one id per line from
# update-methods.d/* in the package and OS directories.
# True when this board can restart straight into USB boot (RPIBOOT) without
# its button: a BCM2712 (Pi 5/CM5) firmware takes a one-time boot order
# through the mailbox (set_reboot_order), and vcmailbox is installed.
#   BOARD_VCMAILBOX  the vcmailbox command   (vcmailbox)
board_usb_boot_supported() {
	tr '\000' '\n' 2>/dev/null < "$BOARD_DT_DIR/compatible" | grep -qx 'brcm,bcm2712' &&
		command -v "${BOARD_VCMAILBOX:-vcmailbox}" >/dev/null 2>&1
}

board_update_methods() {
	{
		printf '%s\n' image-write
		# board-update writes this once it has seen an A/B layout.
		if grep -q '"slot_active":"[AB]"' "$BOARD_RUN_DIR/update.json" 2>/dev/null; then
			printf '%s\n' ab-tryboot
		fi
		# Atlas can restart it into USB boot for a fresh install (usb-boot).
		if board_usb_boot_supported; then
			printf '%s\n' usb-boot-reboot
		fi
		for _board_d in "$BOARD_LIB_DIR/update-methods.d" "$BOARD_ETC_DIR/update-methods.d"; do
			[ -d "$_board_d" ] || continue
			for _board_f in "$_board_d"/*; do
				[ -r "$_board_f" ] || continue
				sed -e 's/#.*//' -e 's/[[:space:]]//g' "$_board_f"
			done
		done
	} | grep -E '^[a-z0-9][a-z0-9._-]*$' | awk '!seen[$0]++'
}

# "iface mac" lines for Ethernet-type interfaces (ARPHRD_ETHER), lo excluded.
board_macs() {
	for _board_if in "$BOARD_NET_DIR"/*; do
		[ -e "$_board_if/address" ] || continue
		_board_name=${_board_if##*/}
		[ "$_board_name" = lo ] && continue
		_board_type=$(cat "$_board_if/type" 2>/dev/null) || _board_type=''
		[ "$_board_type" = 1 ] || continue
		_board_mac=$(tr 'A-F' 'a-f' < "$_board_if/address" | tr -d '\n')
		case "$_board_mac" in
		00:00:00:00:00:00 | '') continue ;;
		esac
		printf '%s %s\n' "$_board_name" "$_board_mac"
	done
}

# The identity document (contract 1), one line of JSON.
# The label Atlas shows for an action id.
board_action_label() {
	case "$1" in
	locate) printf 'Find it' ;;
	*) printf '%s' "$1" ;;
	esac
}

# The identity's optional endpoints and the actions this device offers
# (BOARD_ACTIONS):
# ,"endpoints":{"status":"/status","events":"/events","stream":":5898/stream",
#  "actions":"/actions"},"actions":[{"id":..,"label":..}]
# status, events and stream are read-only; they are listed when the package
# has them. stream is on its own port of the same host (board-stream.socket).
#   BOARD_STREAM_BIN  board-stream (tests)
board_actions_json() {
	_board_eps=''
	if [ -x "$BOARD_LIB_DIR/status" ]; then
		_board_eps='"status":"/status","events":"/events"'
	fi
	if [ -x "${BOARD_STREAM_BIN:-/usr/bin/board-stream}" ]; then
		_board_eps="$_board_eps${_board_eps:+,}\"stream\":\":5898/stream\""
	fi
	if [ -n "${BOARD_ACTIONS:-}" ]; then
		_board_eps="$_board_eps${_board_eps:+,}\"actions\":\"/actions\""
	fi
	[ -z "$_board_eps" ] || printf ',"endpoints":{%s}' "$_board_eps"
	[ -n "${BOARD_ACTIONS:-}" ] || return 0
	printf ',"actions":['
	_board_sep=''
	for _board_a in $BOARD_ACTIONS; do
		printf '%s{"id":%s,"label":%s}' "$_board_sep" "$(board_json_str "$_board_a")" "$(board_json_str "$(board_action_label "$_board_a")")"
		_board_sep=','
	done
	printf ']'
}

# Whether <action> is one this device offers.
board_action_offered() {
	for _board_a in ${BOARD_ACTIONS:-}; do
		[ "$_board_a" = "$1" ] && return 0
	done
	return 1
}

# The gadget network's address, as an optional identity field, when the
# package runs it (USB_GADGET_NET=bridge):
# ,"gadget":{"address":..,"prefix":..,"addressing":..}
board_gadget_json() {
	board_load_env usb-gadget.env
	[ "${USB_GADGET_ENABLED:-1}" = 1 ] && [ "${USB_GADGET_NET:-bridge}" = bridge ] || return 0
	board_gadget_address
	_board_gpre=${BOARD_GADGET_CIDR#*/}
	case "$_board_gpre" in
	'' | *[!0-9]*) return 0 ;;
	esac
	printf ',"gadget":{"address":%s,"prefix":%s,"addressing":%s}' \
		"$(board_json_str "${BOARD_GADGET_CIDR%/*}")" "$_board_gpre" "$(board_json_str "$BOARD_GADGET_ADDRESSING")"
}

# The camera views, as the image writes them (PhotonVision:
# /usr/lib/photonvision-os/camera-streams, at boot and on every settings
# change): ,"camera_streams":[{"name":..,"url":":1182/stream.mjpg"},..].
# Left out unless the file holds a JSON array.
board_camera_streams_json() {
	_board_cs="$BOARD_RUN_DIR/camera-streams.json"
	[ -s "$_board_cs" ] || return 0
	_board_cs=$(tr -d '\n\r' <"$_board_cs")
	case "$_board_cs" in
	'['*']') printf ',"camera_streams":%s' "$_board_cs" ;;
	esac
}

board_identity_json() {
	board_load_env board-package.env
	board_load_env identity.env
	board_bootloader

	_board_os_name=$(board_os_field ID)
	_board_os_ver=$(board_os_field VERSION_ID)
	[ -n "$_board_os_ver" ] || _board_os_ver=$(board_os_field VERSION)

	printf '{"contract":1'
	printf ',"model":%s' "$(board_json_str "${BOARD_DEVICE_MODEL:-}")"
	printf ',"rev":%s' "$(board_json_str "$(board_rev)")"
	printf ',"serial":%s' "$(board_json_str "$(board_serial)")"
	printf ',"hostname":%s' "$(board_json_str "$(board_hostname)")"
	printf ',"os":{"name":%s,"version":%s}' "$(board_json_str "$_board_os_name")" "$(board_json_str "$_board_os_ver")"
	printf ',"device_package":{"version":%s,"commit":%s}' \
		"$(board_json_str "${BOARD_PACKAGE_VERSION:-}")" "$(board_json_str "${BOARD_PACKAGE_COMMIT:-}")"
	if [ -n "$BOARD_BL_VERSION" ] || [ -n "$BOARD_BL_TIMESTAMP" ]; then
		printf ',"bootloader":{"version":%s' "$(board_json_str "$BOARD_BL_VERSION")"
		if [ -n "$BOARD_BL_TIMESTAMP" ]; then
			printf ',"timestamp":%s' "$BOARD_BL_TIMESTAMP"
		fi
		printf '}'
	fi
	printf ',"update_methods":['
	_board_sep=''
	for _board_m in $(board_update_methods); do
		printf '%s"%s"' "$_board_sep" "$_board_m"
		_board_sep=','
	done
	printf ']'
	# Root-only diagnostics, run over SSH: the self-test, and PhotonVision's
	# nt-server (point its NetworkTables server elsewhere and back).
	_board_diag=''
	if [ -x "$BOARD_LIB_DIR/selftest" ]; then
		_board_diag='"selftest"'
	fi
	if [ -x "${BOARD_NT_SERVER_BIN:-/usr/lib/photonvision-os/nt-server}" ]; then
		_board_diag="$_board_diag${_board_diag:+,}\"nt-server\""
	fi
	[ -z "$_board_diag" ] || printf ',"diagnostics":[%s]' "$_board_diag"
	# The A/B updater's state, as board-update last wrote it.
	if [ -s "$BOARD_RUN_DIR/update.json" ]; then
		printf ',"update":%s' "$(head -n 1 "$BOARD_RUN_DIR/update.json")"
	fi
	printf ',"manage_url":%s' "$(board_json_str "$(board_first_line "$BOARD_ETC_DIR/manage-url" "$BOARD_LIB_DIR/manage-url")")"
	board_camera_streams_json
	# The board's clock, so a host can tell it is wrong: a Raze has no RTC
	# battery and often no NTP over the USB link.
	printf ',"time":%s' "$(date +%s)"
	board_actions_json
	board_gadget_json
	printf ',"macs":{'
	_board_sep=''
	board_macs | while read -r _board_name _board_mac; do
		printf '%s%s:"%s"' "$_board_sep" "$(board_json_str "$_board_name")" "$_board_mac"
		_board_sep=','
	done
	printf '}}\n'
}

# ---------------------------------------------------------------------------
# Event log: what happened to the board, whoever did it (docs/ota.md, "Board
# awareness"). One JSON object per line:
#   {"t":<unix s>,"boot_id":..,"seq":<n>,"uptime_s":<s>,"kind":..,
#    "source":..,"message":..,"data":{..}}
# t is the board's clock, which can be far off or step back (no RTC); seq
# numbers the events in the order they were written, across boots, so a
# reader catches up by seq (/events?after_seq=) whatever the clock did.
# in $BOARD_DATA_DIR/events.jsonl (persistent), or $BOARD_RUN_DIR/events.jsonl
# when /data isn't writable. The last BOARD_EVENT_MAX lines are kept.
#   BOARD_EVENT_SOURCE  who asked: atlas (Atlas over SSH), orion (board-agent),
#                       local (the default: someone on the board)
#   BOARD_BOOT_ID       this boot's id (/proc/sys/kernel/random/boot_id)
#   BOARD_EVENT_DEFER   1: a writer that runs before /data is mounted (data
#                       setup) queues its events in
#                       $BOARD_RUN_DIR/events.pending instead of the /run log,
#                       and the first event written to /data afterwards logs
#                       them first, so they persist with the boot's others
BOARD_EVENT_MAX=${BOARD_EVENT_MAX:-2000}

# The kernel's id of this boot; empty when unknown.
board_boot_id() {
	_board_bid=${BOARD_BOOT_ID:-${UPDATE_BOOT_ID:-}}
	if [ -z "$_board_bid" ]; then
		_board_bid=$(tr -cd '0-9a-f-' < /proc/sys/kernel/random/boot_id 2>/dev/null) || _board_bid=''
	fi
	printf '%s\n' "$_board_bid"
}

# The file events are appended to: /data when it is writable, else /run.
board_events_file() {
	if mkdir -p "$BOARD_DATA_DIR" 2>/dev/null && [ -w "$BOARD_DATA_DIR" ]; then
		printf '%s\n' "$BOARD_DATA_DIR/events.jsonl"
	else
		printf '%s\n' "$BOARD_RUN_DIR/events.jsonl"
	fi
}

# The event source as given, else "local"; anything unknown counts as local.
board_event_source() {
	case "${BOARD_EVENT_SOURCE:-}" in
	atlas | orion | local) printf '%s\n' "$BOARD_EVENT_SOURCE" ;;
	*) echo local ;;
	esac
}

# The deferred events (BOARD_EVENT_DEFER), one a line: the arguments of
# board_event, separated by the unit separator (\037), which is taken out of
# them (as are newlines).
board_event_defer() {
	mkdir -p "$BOARD_RUN_DIR" 2>/dev/null || return 0
	_board_us=$(printf '\037')
	_board_line=''
	for _board_arg in "$@"; do
		_board_arg=$(printf '%s' "$_board_arg" | tr -d '\037\n')
		_board_line="$_board_line${_board_line:+$_board_us}$_board_arg"
	done
	printf '%s\n' "$_board_line" >> "$BOARD_RUN_DIR/events.pending" 2>/dev/null || true
}

# Logs the deferred events, once /data takes events. Taken away first, so a
# second writer (or the events below) doesn't log them again.
board_events_replay() {
	[ -s "$BOARD_RUN_DIR/events.pending" ] || return 0
	mv -f "$BOARD_RUN_DIR/events.pending" "$BOARD_RUN_DIR/events.replay" 2>/dev/null || return 0
	_board_us=$(printf '\037')
	while IFS= read -r _board_line || [ -n "$_board_line" ]; do
		[ -n "$_board_line" ] || continue
		_board_ifs=$IFS
		IFS=$_board_us
		set -f
		# shellcheck disable=SC2086
		set -- $_board_line
		set +f
		IFS=$_board_ifs
		BOARD_EVENT_DEFER=0 board_event "$@"
	done < "$BOARD_RUN_DIR/events.replay"
	rm -f "$BOARD_RUN_DIR/events.replay"
}

# board_event <kind> <message> [key=value ...]: appends one event. Never
# fails the caller: an event that can't be written is only lost.
board_event() {
	case "$(board_events_file)" in
	"$BOARD_DATA_DIR"/*) board_events_replay ;;
	*)
		if [ "${BOARD_EVENT_DEFER:-0}" = 1 ]; then
			board_event_defer "$@"
			return 0
		fi
		;;
	esac
	_board_ek=$(printf '%s' "${1:-}" | tr -cd 'A-Za-z0-9._-')
	[ -n "$_board_ek" ] || return 0
	_board_em=${2:-}
	if [ $# -ge 2 ]; then shift 2; else shift $#; fi
	_board_ed=''
	for _board_kv in "$@"; do
		case "$_board_kv" in
		*=*) ;;
		*) continue ;;
		esac
		_board_dk=$(printf '%s' "${_board_kv%%=*}" | tr -cd 'A-Za-z0-9._-')
		[ -n "$_board_dk" ] || continue
		_board_dv=$(board_json_str "${_board_kv#*=}")
		[ "$_board_dv" != null ] || _board_dv='""'
		_board_ed="$_board_ed${_board_ed:+,}\"$_board_dk\":$_board_dv"
	done
	_board_ef=$(board_events_file)
	mkdir -p "${_board_ef%/*}" 2>/dev/null || return 0
	_board_up=$(cut -d. -f1 /proc/uptime 2>/dev/null) || _board_up=''
	case "$_board_up" in '' | *[!0-9]*) _board_up=null ;; esac
	# The seq and the append under one lock (the rotation's), so lines are
	# in seq order.
	(
		if command -v flock >/dev/null 2>&1; then
			flock -w 2 7 || exit 1
		fi
		_board_seq=$(($(board_events_last_seq "$_board_ef") + 1))
		printf '{"t":%s,"boot_id":%s,"seq":%s,"uptime_s":%s,"kind":"%s","source":"%s","message":%s,"data":{%s}}\n' \
			"$(date +%s)" "$(board_json_str "$(board_boot_id)")" "$_board_seq" "$_board_up" "$_board_ek" \
			"$(board_event_source)" "$(board_json_str "$_board_em")" "$_board_ed" >> "$_board_ef" &&
			chmod 0644 "$_board_ef" &&
			printf '%s\n' "$_board_seq" > "${_board_ef%/*}/event-seq.tmp" &&
			mv -f "${_board_ef%/*}/event-seq.tmp" "${_board_ef%/*}/event-seq"
	) 7>> "$_board_ef.lock" 2>/dev/null || return 0
	board_events_rotate "$_board_ef"
	return 0
}

# The newest seq written: the larger of event-seq next to the log and the
# highest seq in the log, so neither a lost or torn event-seq nor a torn
# last line (a power cut mid-write leaves NULs) starts the count over; 0 for
# none.
board_events_last_seq() {
	_board_c=$(cat "${1%/*}/event-seq" 2>/dev/null) || _board_c=''
	case "$_board_c" in '' | *[!0-9]*) _board_c=0 ;; esac
	_board_l=$(tr -d '\000' < "$1" 2>/dev/null |
		sed -n 's/^{"t":[0-9]*,"boot_id":[^,]*,"seq":\([0-9][0-9]*\),.*/\1/p' | sort -n | tail -n 1)
	case "$_board_l" in '' | *[!0-9]*) _board_l=0 ;; esac
	[ "$_board_l" -gt "$_board_c" ] && _board_c=$_board_l
	printf '%s\n' "$_board_c"
}

# Keeps the last BOARD_EVENT_MAX lines once the file is 10 % over, under a
# lock when flock is there (appends are single short writes, so they don't
# interleave with each other).
board_events_rotate() {
	_board_n=$(wc -l < "$1" 2>/dev/null | tr -d ' ') || return 0
	[ "${_board_n:-0}" -gt $((BOARD_EVENT_MAX + BOARD_EVENT_MAX / 10)) ] || return 0
	(
		if command -v flock >/dev/null 2>&1; then
			flock -w 2 8 || exit 0
		fi
		tail -n "$BOARD_EVENT_MAX" "$1" > "$1.tmp.$$" && chmod 0644 "$1.tmp.$$" && mv -f "$1.tmp.$$" "$1"
	) 8>> "$1.lock" 2>/dev/null || rm -f "$1.tmp.$$" 2>/dev/null
	return 0
}

# board_events_json <since> <limit> [<after_seq>]:
#   {"time":..,"boot_id":..,"seq":<newest>|null,"events":[..]}
# from both event files. With <after_seq>, the oldest <limit> events whose
# seq is above it, in seq order (a reader pages forward with the last seq it
# got); else the newest <limit> whose t >= <since>, oldest first. Arguments
# must be decimal numbers (callers check them). A line that isn't a whole
# event (a write cut off by power loss) is skipped.
board_events_json() {
	_board_all=$(cat "$BOARD_RUN_DIR/events.jsonl" "$BOARD_DATA_DIR/events.jsonl" 2>/dev/null |
		awk '/^\{"t":[0-9]+,"boot_id":.*\}$/ {
			t = substr($0, 6); sub(/,.*/, "", t)
			s = -1
			if (match($0, /^\{"t":[0-9]+,"boot_id":[^,]*,"seq":[0-9]+,/)) {
				s = substr($0, 1, RLENGTH - 1); sub(/.*"seq":/, "", s)
			}
			print t "\t" s "\t" $0
		}')
	_board_newest=$(printf '%s\n' "$_board_all" | cut -f2 | sort -n | tail -n 1)
	case "$_board_newest" in '' | -1) _board_newest=null ;; esac
	printf '{"time":%s,"boot_id":%s,"seq":%s,"events":[' "$(date +%s)" "$(board_json_str "$(board_boot_id)")" "$_board_newest"
	if [ -n "${3:-}" ]; then
		printf '%s\n' "$_board_all" | awk -F '\t' -v after="$3" '$2 + 0 > after + 0' |
			sort -s -n -k2,2 | head -n "$2"
	else
		printf '%s\n' "$_board_all" | awk -F '\t' -v since="$1" 'NF >= 3 && $1 + 0 >= since + 0' |
			sort -s -n -k1,1 | tail -n "$2"
	fi | cut -f3- | awk 'NF { if (n++) printf ","; printf "%s", $0 }'
	printf ']}\n'
}

# The slot the root filesystem is in (A or B) by update.env's partition
# numbers; "unknown" off the A/B layout.
#   UPDATE_CMDLINE_ROOT  the root partition number (tests)
board_active_slot() {
	(
		board_load_env update.env
		_board_root=${UPDATE_CMDLINE_ROOT:-}
		if [ -z "$_board_root" ]; then
			_board_root=$(board_root_device)
			_board_root=${_board_root##*[!0-9]}
		fi
		# shellcheck disable=SC2086
		set -- ${UPDATE_SLOT_A:-2 5} ${UPDATE_SLOT_B:-3 6}
		case "$_board_root" in
		'') echo unknown ;;
		"$2") echo A ;;
		"$4") echo B ;;
		*) echo unknown ;;
		esac
	)
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
