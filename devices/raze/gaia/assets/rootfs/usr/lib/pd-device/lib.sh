# shellcheck shell=sh
# Shared helpers for the pd-device scripts (device package contract 1).
# POSIX sh; also runs under busybox ash. Sourced, never executed.
#
# Every input path can be overridden through the environment, which is how the
# scripts are tested off-device:
#   PD_LIB_DIR      package files and defaults   (/usr/lib/pd-device)
#   PD_ETC_DIR      OS overrides                 (/etc/pd-device)
#   PD_DATA_DIR     user overrides, persistent   (/data/pd-device; the root
#                   may be read-only)
#   PD_RUN_DIR      generated runtime state      (/run/pd-device)
#   PD_DT_DIR       device tree                  (/proc/device-tree)
#   PD_NET_DIR      network interfaces           (/sys/class/net)
#   PD_OS_RELEASE   os-release file              (/etc/os-release, then /usr/lib/os-release)
#   PD_MACHINE_ID   machine id file              (/etc/machine-id)
#   PD_HOSTNAME_PROC  kernel hostname            (/proc/sys/kernel/hostname)

PD_LIB_DIR=${PD_LIB_DIR:-/usr/lib/pd-device}
PD_ETC_DIR=${PD_ETC_DIR:-/etc/pd-device}
PD_DATA_DIR=${PD_DATA_DIR:-/data/pd-device}
PD_RUN_DIR=${PD_RUN_DIR:-/run/pd-device}
PD_DT_DIR=${PD_DT_DIR:-/proc/device-tree}
PD_NET_DIR=${PD_NET_DIR:-/sys/class/net}
PD_OS_RELEASE=${PD_OS_RELEASE:-}
PD_MACHINE_ID=${PD_MACHINE_ID:-/etc/machine-id}
PD_HOSTNAME_PROC=${PD_HOSTNAME_PROC:-/proc/sys/kernel/hostname}

# The root filesystem's block device, e.g. /dev/mmcblk0p5. The kernel command
# line names it (resolving PARTUUID=, UUID=, LABEL=); /proc/mounts is only a
# fallback, since it says /dev/root on a Pi and overlay on an overlay root.
# Empty when there is no block device behind / (overlay on tmpfs, ramdisk).
#   PD_PROC_CMDLINE  kernel command line   (/proc/cmdline)
pd_root_device() {
	_pd_src=$(tr ' ' '\n' < "${PD_PROC_CMDLINE:-/proc/cmdline}" 2>/dev/null | sed -n 's/^root=//p' | tail -n 1)
	case "$_pd_src" in
	PARTUUID=* | UUID=* | LABEL=*)
		_pd_src=$(findfs "$_pd_src" 2>/dev/null || blkid -l -o device -t "$_pd_src" 2>/dev/null || true)
		;;
	esac
	case "$_pd_src" in
	/dev/root | /dev/ram* | '')
		_pd_src=$(awk '$2 == "/" { print $1 }' /proc/mounts 2>/dev/null | tail -n 1)
		;;
	esac
	case "$_pd_src" in
	/dev/*[0-9]) echo "$_pd_src" ;;
	esac
}

# Partition <n> of the disk the root device <dev> is on.
pd_sibling_partition() {
	case "$1" in
	*[0-9]p[0-9]*) echo "${1%p[0-9]*}p$2" ;; # mmcblk0p2, nvme0n1p2
	*[a-z][0-9]*) echo "${1%%[0-9]*}$2" ;;   # sda2
	*) return 1 ;;
	esac
}

pd_log() {
	printf '%s: %s\n' "${0##*/}" "$*" >&2
}

# Source <name> from the package defaults, then the OS overrides in
# /etc/pd-device, then the user's in /data/pd-device; the last value wins.
pd_load_env() {
	# /data/pd-device comes last, so a setting survives on a read-only root.
	# device-package.env describes the image itself and is never taken
	# from /data.
	_pd_data="$PD_DATA_DIR/$1"
	[ "$1" != device-package.env ] || _pd_data=''
	for _pd_env in "$PD_LIB_DIR/$1" "$PD_ETC_DIR/$1" $_pd_data; do
		if [ -r "$_pd_env" ]; then
			# shellcheck disable=SC1090
			. "$_pd_env"
		fi
	done
}

# First non-empty, non-comment line of the first readable file given.
pd_first_line() {
	for _pd_f in "$@"; do
		if [ -r "$_pd_f" ]; then
			sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e '/^#/d' -e '/^$/d' "$_pd_f" | head -n 1
			return 0
		fi
	done
	return 0
}

# Board serial from the device tree, as lowercase hex. Empty when unreadable.
pd_serial() {
	if [ -r "$PD_DT_DIR/serial-number" ]; then
		tr -d '\000' < "$PD_DT_DIR/serial-number" | tr 'A-F' 'a-f' | tr -cd '0-9a-f'
	fi
}

pd_machine_id() {
	if [ -r "$PD_MACHINE_ID" ]; then
		tr -cd '0-9a-f' < "$PD_MACHINE_ID"
	fi
}

# Last 8 characters of the serial; first 8 of the machine id without a serial.
pd_serial8() {
	_pd_s=$(pd_serial)
	if [ -z "$_pd_s" ]; then
		_pd_s=$(pd_machine_id)
		printf '%s' "$_pd_s" | cut -c1-8
		return 0
	fi
	if [ "${#_pd_s}" -le 8 ]; then
		printf '%s' "$_pd_s"
	else
		printf '%s' "${_pd_s#"${_pd_s%????????}"}"
	fi
}

# Seed for derived MAC addresses: the board serial, else the machine id. Never
# a constant: prints nothing when neither exists, and callers then leave the
# kernel's random addresses alone.
pd_mac_seed() {
	_pd_s=$(pd_serial)
	if [ -n "$_pd_s" ]; then
		printf '%s' "$_pd_s"
		return 0
	fi
	_pd_s=$(pd_machine_id)
	if [ -n "$_pd_s" ]; then
		printf 'machine-id:%s' "$_pd_s"
	fi
}

# Base MAC: 02 (locally administered, unicast) followed by the first five bytes
# of sha256(seed). Same rule as the earlier gadget script, so existing devices
# keep their host-side addresses.
pd_mac_base() {
	_pd_seed=$(pd_mac_seed)
	[ -n "$_pd_seed" ] || return 1
	printf '%s' "$_pd_seed" | sha256sum | cut -c1-10 |
		sed 's/\(..\)\(..\)\(..\)\(..\)\(..\)/02:\1:\2:\3:\4:\5/'
}

# pd_mac_variant <mac> <mask>: XOR the last byte with mask (e.g. 0x10).
pd_mac_variant() {
	_pd_pre=${1%:*}
	_pd_last=${1##*:}
	printf '%s:%02x\n' "$_pd_pre" $((0x$_pd_last ^ $2))
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
# subnet, and the board keeps PD_GADGET_LEGACY_ADDRESS.
PD_GADGET_LEGACY_ADDRESS=172.31.250.1/24

# Dotted IPv4 address to an integer. Fails on anything else.
pd_ip4_int() {
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
pd_int_ip4() {
	printf '%d.%d.%d.%d\n' $(($1 >> 24 & 255)) $(($1 >> 16 & 255)) $(($1 >> 8 & 255)) $(($1 & 255))
}

# pd_board_serial8 <serial>: Atlas's board-serial rule (normalize_board_serial):
# NULs and surrounding whitespace dropped, all hex, at least 8 digits; prints
# the last 8 in lowercase. Fails otherwise.
pd_board_serial8() {
	_pd_s=$(printf '%s' "$1" | tr -d '\000')
	while :; do
		case "$_pd_s" in
		[[:space:]]*) _pd_s=${_pd_s#?} ;;
		*[[:space:]]) _pd_s=${_pd_s%?} ;;
		*) break ;;
		esac
	done
	case "$_pd_s" in
	'' | *[!0-9A-Fa-f]*) return 1 ;;
	esac
	[ "${#_pd_s}" -ge 8 ] || return 1
	printf '%s\n' "${_pd_s#"${_pd_s%????????}"}" | tr 'A-F' 'a-f'
}

# pd_gadget_subnet <usb serial>: this board's gadget address under
# serial-hash-v1, as <address>/<prefix> (for example 172.31.209.217/29).
# Fails when the serial gives no subnet or the settings are invalid.
#   USB_GADGET_ADDR_BASE     the block subnets come from  (172.31.0.0/16)
#   USB_GADGET_ADDR_PREFIX   each board's subnet length   (29)
#   USB_GADGET_ADDR_EXCLUDE  CIDRs left out, space-separated (172.31.250.0/24)
pd_gadget_subnet() {
	_pd_s8=$(pd_board_serial8 "$1") || return 1
	_pd_base=${USB_GADGET_ADDR_BASE:-172.31.0.0/16}
	_pd_prefix=${USB_GADGET_ADDR_PREFIX:-29}
	_pd_excl=${USB_GADGET_ADDR_EXCLUDE-172.31.250.0/24}
	_pd_bpre=${_pd_base#*/}
	case "$_pd_bpre$_pd_prefix" in
	'' | *[!0-9]*) return 1 ;;
	esac
	[ "$_pd_bpre" -le "$_pd_prefix" ] && [ "$_pd_prefix" -le 30 ] || return 1
	_pd_bnet=$(pd_ip4_int "${_pd_base%/*}") || return 1
	_pd_size=$((1 << (32 - _pd_prefix)))
	_pd_slots=$((1 << (_pd_prefix - _pd_bpre)))
	_pd_bnet=$((_pd_bnet - _pd_bnet % (_pd_size * _pd_slots)))
	# Excluded subnets as "first-slot count" lines, ascending.
	_pd_ranges=''
	for _pd_c in $_pd_excl; do
		_pd_epre=${_pd_c#*/}
		case "$_pd_epre" in
		'' | *[!0-9]*) return 1 ;;
		esac
		[ "$_pd_epre" -ge "$_pd_bpre" ] && [ "$_pd_epre" -le "$_pd_prefix" ] || return 1
		_pd_e=$(pd_ip4_int "${_pd_c%/*}") || return 1
		_pd_e=$((_pd_e - _pd_e % (1 << (32 - _pd_epre)) - _pd_bnet))
		[ "$_pd_e" -ge 0 ] && [ "$_pd_e" -lt $((_pd_size * _pd_slots)) ] || return 1
		_pd_ranges="$_pd_ranges$((_pd_e / _pd_size)) $((1 << (_pd_prefix - _pd_epre)))
"
	done
	_pd_ranges=$(printf '%s' "$_pd_ranges" | sort -n -k1,1 | uniq)
	_pd_usable=$_pd_slots
	while read -r _pd_start _pd_len; do
		[ -n "$_pd_start" ] || continue
		_pd_usable=$((_pd_usable - _pd_len))
	done <<EOF
$_pd_ranges
EOF
	[ "$_pd_usable" -gt 0 ] || return 1
	_pd_h=$(printf '%s' "$_pd_s8" | sha256sum | cut -c1-8)
	_pd_slot=$((0x$_pd_h % _pd_usable))
	while read -r _pd_start _pd_len; do
		[ -n "$_pd_start" ] || continue
		if [ "$_pd_start" -le "$_pd_slot" ]; then
			_pd_slot=$((_pd_slot + _pd_len))
		fi
	done <<EOF
$_pd_ranges
EOF
	printf '%s/%s\n' "$(pd_int_ip4 $((_pd_bnet + _pd_slot * _pd_size + 1)))" "$_pd_prefix"
}

# The gadget's USB serial string: USB_GADGET_SERIAL, or with AUTO the board
# serial, else the first 16 hex digits of the machine id.
pd_gadget_serial() {
	_pd_gs=${USB_GADGET_SERIAL:-AUTO}
	if [ "$_pd_gs" = AUTO ]; then
		_pd_gs=$(pd_serial)
		[ -n "$_pd_gs" ] || _pd_gs=$(pd_machine_id | cut -c1-16)
	fi
	printf '%s\n' "$_pd_gs"
}

# The address the gadget bridge gets, after usb-gadget.env is loaded. Sets
# PD_GADGET_CIDR (<address>/<prefix>) and PD_GADGET_ADDRESSING:
#   serial-hash-v1  derived from the USB serial (USB_GADGET_ADDRESS=AUTO)
#   pinned          USB_GADGET_ADDRESS (a bare address means /24)
#   fallback        PD_GADGET_LEGACY_ADDRESS: the serial gives no subnet
pd_gadget_address() {
	case "${USB_GADGET_ADDRESS:-AUTO}" in
	AUTO)
		if PD_GADGET_CIDR=$(pd_gadget_subnet "$(pd_gadget_serial)"); then
			PD_GADGET_ADDRESSING=serial-hash-v1
		else
			PD_GADGET_CIDR=$PD_GADGET_LEGACY_ADDRESS
			PD_GADGET_ADDRESSING=fallback
		fi
		;;
	*/*)
		PD_GADGET_CIDR=$USB_GADGET_ADDRESS
		PD_GADGET_ADDRESSING=pinned
		;;
	*)
		PD_GADGET_CIDR=$USB_GADGET_ADDRESS/24
		PD_GADGET_ADDRESSING=pinned
		;;
	esac
}

# pd_gadget_dhcp_range <address>/<prefix>: dnsmasq's dhcp-range for the host
# end of the link: the addresses after the device's, up to 19 of them and the
# subnet's last host address, with the netmask and a 1h lease. Fails when the
# subnet has no room.
pd_gadget_dhcp_range() {
	_pd_pre=${1#*/}
	case "$_pd_pre" in
	'' | *[!0-9]*) return 1 ;;
	esac
	[ "$_pd_pre" -ge 8 ] && [ "$_pd_pre" -le 30 ] || return 1
	_pd_a=$(pd_ip4_int "${1%/*}") || return 1
	_pd_size=$((1 << (32 - _pd_pre)))
	_pd_last=$((_pd_a - _pd_a % _pd_size + _pd_size - 2))
	_pd_end=$((_pd_a + 19))
	[ "$_pd_end" -le "$_pd_last" ] || _pd_end=$_pd_last
	[ "$_pd_a" -lt "$_pd_end" ] || return 1
	printf '%s,%s,%s,1h\n' "$(pd_int_ip4 $((_pd_a + 1)))" "$(pd_int_ip4 "$_pd_end")" \
		"$(pd_int_ip4 $((0xffffffff - _pd_size + 1)))"
}

# Value of KEY in os-release, unquoted, with \" \\ \$ \` unescaped.
pd_os_field() {
	_pd_osr=$PD_OS_RELEASE
	if [ -z "$_pd_osr" ]; then
		if [ -r /etc/os-release ]; then
			_pd_osr=/etc/os-release
		else
			_pd_osr=/usr/lib/os-release
		fi
	fi
	[ -r "$_pd_osr" ] || return 0
	sed -n "s/^$1=//p" "$_pd_osr" | head -n 1 |
		sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\\(.*\\)'\$/\\1/" -e 's/\\\(["\\$`]\)/\1/g'
}

pd_hostname() {
	if [ -r "$PD_HOSTNAME_PROC" ]; then
		tr -d '\n' < "$PD_HOSTNAME_PROC"
	else
		uname -n
	fi
}

# Rev id: OS/user override file, then the package's rev-detect hook, then the
# package default. See manifest.json "rev_source".
pd_rev() {
	_pd_rev=$(pd_first_line "$PD_ETC_DIR/rev")
	if [ -z "$_pd_rev" ] && [ -x "$PD_LIB_DIR/rev-detect" ]; then
		_pd_rev=$("$PD_LIB_DIR/rev-detect" 2>/dev/null | head -n 1)
	fi
	if [ -z "$_pd_rev" ]; then
		_pd_rev=${PD_DEVICE_REV_DEFAULT:-}
	fi
	printf '%s' "$_pd_rev" | tr -cd 'A-Za-z0-9._-'
}

# Sets PD_BL_VERSION and PD_BL_TIMESTAMP (empty when unknown). Reads the device
# tree first (no VideoCore access needed), then `vcgencmd bootloader_version`.
pd_bootloader() {
	PD_BL_VERSION=''
	PD_BL_TIMESTAMP=''
	_pd_bl="$PD_DT_DIR/chosen/bootloader"
	if [ -r "$_pd_bl/version" ]; then
		PD_BL_VERSION=$(tr -d '\000' < "$_pd_bl/version" | tr -cd 'A-Za-z0-9._-')
	fi
	if [ -r "$_pd_bl/build-timestamp" ]; then
		PD_BL_TIMESTAMP=$(od -An -tu1 -N4 "$_pd_bl/build-timestamp" |
			awk 'NF == 4 { printf "%d", $1 * 16777216 + $2 * 65536 + $3 * 256 + $4 }')
	fi
	if [ -z "$PD_BL_VERSION" ] && command -v vcgencmd >/dev/null 2>&1; then
		_pd_out=$(vcgencmd bootloader_version 2>/dev/null) || _pd_out=''
		PD_BL_VERSION=$(printf '%s\n' "$_pd_out" | sed -n 's/^version \([0-9A-Za-z]*\).*/\1/p' | head -n 1)
		if [ -z "$PD_BL_TIMESTAMP" ]; then
			PD_BL_TIMESTAMP=$(printf '%s\n' "$_pd_out" | sed -n 's/^timestamp \([0-9]*\).*/\1/p' | head -n 1)
		fi
	fi
}

# JSON string literal for $1, or null when empty. Control characters are dropped.
pd_json_str() {
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
#   PD_VCMAILBOX  the vcmailbox command   (vcmailbox)
pd_usb_boot_supported() {
	tr '\000' '\n' 2>/dev/null < "$PD_DT_DIR/compatible" | grep -qx 'brcm,bcm2712' &&
		command -v "${PD_VCMAILBOX:-vcmailbox}" >/dev/null 2>&1
}

pd_update_methods() {
	{
		printf '%s\n' image-write
		# pd-device-update writes this once it has seen an A/B layout.
		if grep -q '"slot_active":"[AB]"' "$PD_RUN_DIR/update.json" 2>/dev/null; then
			printf '%s\n' ab-tryboot
		fi
		# Atlas can restart it into USB boot for a fresh install (usb-boot).
		if pd_usb_boot_supported; then
			printf '%s\n' usb-boot-reboot
		fi
		for _pd_d in "$PD_LIB_DIR/update-methods.d" "$PD_ETC_DIR/update-methods.d"; do
			[ -d "$_pd_d" ] || continue
			for _pd_f in "$_pd_d"/*; do
				[ -r "$_pd_f" ] || continue
				sed -e 's/#.*//' -e 's/[[:space:]]//g' "$_pd_f"
			done
		done
	} | grep -E '^[a-z0-9][a-z0-9._-]*$' | awk '!seen[$0]++'
}

# "iface mac" lines for Ethernet-type interfaces (ARPHRD_ETHER), lo excluded.
pd_macs() {
	for _pd_if in "$PD_NET_DIR"/*; do
		[ -e "$_pd_if/address" ] || continue
		_pd_name=${_pd_if##*/}
		[ "$_pd_name" = lo ] && continue
		_pd_type=$(cat "$_pd_if/type" 2>/dev/null) || _pd_type=''
		[ "$_pd_type" = 1 ] || continue
		_pd_mac=$(tr 'A-F' 'a-f' < "$_pd_if/address" | tr -d '\n')
		case "$_pd_mac" in
		00:00:00:00:00:00 | '') continue ;;
		esac
		printf '%s %s\n' "$_pd_name" "$_pd_mac"
	done
}

# The identity document (contract 1), one line of JSON.
# The label Atlas shows for an action id.
pd_action_label() {
	case "$1" in
	locate) printf 'Find it' ;;
	*) printf '%s' "$1" ;;
	esac
}

# Actions this device offers (PD_ACTIONS), as optional identity fields:
# ,"endpoints":{"actions":"/actions"},"actions":[{"id":..,"label":..}]
pd_actions_json() {
	[ -n "${PD_ACTIONS:-}" ] || return 0
	printf ',"endpoints":{"actions":"/actions"},"actions":['
	_pd_sep=''
	for _pd_a in $PD_ACTIONS; do
		printf '%s{"id":%s,"label":%s}' "$_pd_sep" "$(pd_json_str "$_pd_a")" "$(pd_json_str "$(pd_action_label "$_pd_a")")"
		_pd_sep=','
	done
	printf ']'
}

# Whether <action> is one this device offers.
pd_action_offered() {
	for _pd_a in ${PD_ACTIONS:-}; do
		[ "$_pd_a" = "$1" ] && return 0
	done
	return 1
}

# The gadget network's address, as an optional identity field, when the
# package runs it (USB_GADGET_NET=bridge):
# ,"gadget":{"address":..,"prefix":..,"addressing":..}
pd_gadget_json() {
	pd_load_env usb-gadget.env
	[ "${USB_GADGET_ENABLED:-1}" = 1 ] && [ "${USB_GADGET_NET:-bridge}" = bridge ] || return 0
	pd_gadget_address
	_pd_gpre=${PD_GADGET_CIDR#*/}
	case "$_pd_gpre" in
	'' | *[!0-9]*) return 0 ;;
	esac
	printf ',"gadget":{"address":%s,"prefix":%s,"addressing":%s}' \
		"$(pd_json_str "${PD_GADGET_CIDR%/*}")" "$_pd_gpre" "$(pd_json_str "$PD_GADGET_ADDRESSING")"
}

pd_identity_json() {
	pd_load_env device-package.env
	pd_load_env identity.env
	pd_bootloader

	_pd_os_name=$(pd_os_field ID)
	_pd_os_ver=$(pd_os_field VERSION_ID)
	[ -n "$_pd_os_ver" ] || _pd_os_ver=$(pd_os_field VERSION)

	printf '{"contract":1'
	printf ',"model":%s' "$(pd_json_str "${PD_DEVICE_MODEL:-}")"
	printf ',"rev":%s' "$(pd_json_str "$(pd_rev)")"
	printf ',"serial":%s' "$(pd_json_str "$(pd_serial)")"
	printf ',"hostname":%s' "$(pd_json_str "$(pd_hostname)")"
	printf ',"os":{"name":%s,"version":%s}' "$(pd_json_str "$_pd_os_name")" "$(pd_json_str "$_pd_os_ver")"
	printf ',"device_package":{"version":%s,"commit":%s}' \
		"$(pd_json_str "${PD_DEVICE_PACKAGE_VERSION:-}")" "$(pd_json_str "${PD_DEVICE_PACKAGE_COMMIT:-}")"
	if [ -n "$PD_BL_VERSION" ] || [ -n "$PD_BL_TIMESTAMP" ]; then
		printf ',"bootloader":{"version":%s' "$(pd_json_str "$PD_BL_VERSION")"
		if [ -n "$PD_BL_TIMESTAMP" ]; then
			printf ',"timestamp":%s' "$PD_BL_TIMESTAMP"
		fi
		printf '}'
	fi
	printf ',"update_methods":['
	_pd_sep=''
	for _pd_m in $(pd_update_methods); do
		printf '%s"%s"' "$_pd_sep" "$_pd_m"
		_pd_sep=','
	done
	printf ']'
	# The A/B updater's state, as pd-device-update last wrote it.
	if [ -s "$PD_RUN_DIR/update.json" ]; then
		printf ',"update":%s' "$(head -n 1 "$PD_RUN_DIR/update.json")"
	fi
	printf ',"manage_url":%s' "$(pd_json_str "$(pd_first_line "$PD_ETC_DIR/manage-url" "$PD_LIB_DIR/manage-url")")"
	# The board's clock, so a host can tell it is wrong: a Raze has no RTC
	# battery and often no NTP over the USB link.
	printf ',"time":%s' "$(date +%s)"
	pd_actions_json
	pd_gadget_json
	printf ',"macs":{'
	_pd_sep=''
	pd_macs | while read -r _pd_name _pd_mac; do
		printf '%s%s:"%s"' "$_pd_sep" "$(pd_json_str "$_pd_name")" "$_pd_mac"
		_pd_sep=','
	done
	printf '}}\n'
}
