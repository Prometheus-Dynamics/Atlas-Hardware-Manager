# shellcheck shell=sh
# Hardware layer for raze-leds and the selftest (device package contract 1).
# POSIX sh; also runs under busybox ash. Sourced after lib.sh, never executed.
#
# Every hardware access of those scripts goes through the hw_* functions.
# This file has what all backends share (I2C probes and chip ids, the camera,
# the watchdog, the USB gadget) and loads the backend for the LED ring and the
# fan:
#
#   lemnosd  hw-lemnosd.sh: through lemnosd, the board's hardware service
#            (lemnos-ctl). The ring and the fan are lemnosd's; the fan goes
#            back to the thermal governor through lemnosd's hand-back.
#   sysfs    hw-sysfs.sh: frames written to /dev/leds0, the pwm-fan cooling
#            device driven directly with its thermal zone paused. The
#            fallback for images without lemnosd.
#
# BOARD_HW_BACKEND (environment, else /etc/board/hw.env or /data/board/hw.env)
# names one; unset or "auto" means lemnosd when lemnos-ctl is installed, else
# sysfs. HW_BACKEND holds the one loaded. A BOARD_HW_BACKEND that is a path is
# a replacement backend file the callers source instead of this one (it may
# source this file and override functions; here it counts as "auto").
#
# Settings: leds.env and hardware.env, both generated from the package's
# manifest.json (devices/tools/gen-raze.py). Paths, overridable for tests:
#   BOARD_SYS_DIR    sysfs                 (/sys)
#   BOARD_DEV_DIR    device nodes          (/dev)
#   BOARD_CONFIGFS   configfs              (/sys/kernel/config)
#   BOARD_HW_MISSING tools to treat as not installed, space-separated (tests)

BOARD_SYS_DIR=${BOARD_SYS_DIR:-/sys}
BOARD_DEV_DIR=${BOARD_DEV_DIR:-/dev}
BOARD_CONFIGFS=${BOARD_CONFIGFS:-/sys/kernel/config}
board_load_env leds.env
board_load_env hardware.env
_hw_backend=${BOARD_HW_BACKEND:-}
board_load_env hw.env
[ -z "$_hw_backend" ] || BOARD_HW_BACKEND=$_hw_backend

# Whether a tool is installed.
hw_have() {
	case " ${BOARD_HW_MISSING:-} " in
	*" $1 "*) return 1 ;;
	esac
	command -v "$1" >/dev/null 2>&1
}

# A /dev path under BOARD_DEV_DIR; other paths unchanged.
hw_dev_path() {
	case "$1" in
	/dev/*) printf '%s/%s\n' "$BOARD_DEV_DIR" "${1#/dev/}" ;;
	*) printf '%s\n' "$1" ;;
	esac
}

hw_leds_count() {
	printf '%s\n' "${RAZE_LEDS_COUNT:-${HW_LEDS_COUNT:-16}}"
}

# Item <n> (from 1) of the remaining arguments.
_hw_nth() {
	_hw_n=$1
	[ "$_hw_n" -lt "$#" ] || return 0
	shift "$_hw_n"
	printf '%s' "${1:-}"
}

# ---------------------------------------------------------------------------
# I2C.

# The device-tree path of an I2C adapter's node (its own of_node, else its
# parent device's), as Lemnos's i2c: selectors see it. Empty without one.
_hw_i2c_of_node() {
	for _hw_cand in "$1/of_node" "$(dirname "$(readlink -f "$1")")/of_node"; do
		_hw_node=$(readlink -f "$_hw_cand" 2>/dev/null) || continue
		if [ -d "$_hw_node" ]; then
			printf '%s\n' "$_hw_node"
			return 0
		fi
	done
	return 1
}

# hw_i2c_select_matches <adapter dir> <selector>: the selector is Lemnos's
# (key=value[;key=value...], keys name, compatible, of, node); every key must
# match.
hw_i2c_select_matches() {
	_hw_adapter=$1
	_hw_rest=$2
	while [ -n "$_hw_rest" ]; do
		_hw_kv=${_hw_rest%%;*}
		case "$_hw_rest" in *";"*) _hw_rest=${_hw_rest#*;} ;; *) _hw_rest='' ;; esac
		_hw_k=${_hw_kv%%=*}
		_hw_v=${_hw_kv#*=}
		case "$_hw_k" in
		name)
			[ "$(cat "$_hw_adapter/name" 2>/dev/null)" = "$_hw_v" ] || return 1
			;;
		compatible)
			_hw_node=$(_hw_i2c_of_node "$_hw_adapter") || return 1
			tr '\000' '\n' < "$_hw_node/compatible" 2>/dev/null | grep -qxF -- "$_hw_v" || return 1
			;;
		of | node)
			_hw_node=$(_hw_i2c_of_node "$_hw_adapter") || return 1
			_hw_path=${_hw_node#*/firmware/devicetree/base}
			[ "$_hw_path" != "$_hw_node" ] || return 1
			if [ "$_hw_k" = of ]; then
				[ "$_hw_path" = "${_hw_v%/}" ] || return 1
			else
				[ "${_hw_path##*/}" = "$_hw_v" ] || return 1
			fi
			;;
		*) return 1 ;;
		esac
	done
}

# hw_i2c_bus <number on the reference image>: "<bus number now> <how>". The
# bus's selector (HW_I2C_BUSES) finds it whatever probe order numbered it
# (how: selector); when no adapter matches (number), several do (ambiguous)
# or the bus has no selector (number), the number given.
hw_i2c_bus() {
	_hw_how=number
	_hw_sel=''
	for _hw_e in ${HW_I2C_BUSES:-}; do
		[ "${_hw_e%%:*}" = "$1" ] && _hw_sel=${_hw_e#*:}
	done
	if [ -n "$_hw_sel" ]; then
		_hw_found=''
		_hw_count=0
		for _hw_a in "$BOARD_SYS_DIR"/bus/i2c/devices/i2c-*; do
			[ -e "$_hw_a" ] || continue
			if hw_i2c_select_matches "$_hw_a" "$_hw_sel"; then
				_hw_found=${_hw_a##*/i2c-}
				_hw_count=$((_hw_count + 1))
			fi
		done
		if [ "$_hw_count" = 1 ]; then
			printf '%s selector\n' "$_hw_found"
			return 0
		fi
		[ "$_hw_count" = 0 ] || _hw_how=ambiguous
	fi
	printf '%s %s\n' "$1" "$_hw_how"
}

# hw_i2c_probe <bus> <address, 0xNN>: prints one of
#   bound     a kernel driver owns the address
#   present   the address answers
#   absent    nothing answers
#   no-bus    the bus doesn't exist
#   unknown   there is no way to probe (no i2cdetect)
hw_i2c_probe() {
	_hw_bus=$1
	_hw_addr=$(($2))
	if [ ! -e "$BOARD_SYS_DIR/bus/i2c/devices/i2c-$_hw_bus" ] && [ ! -e "$BOARD_DEV_DIR/i2c-$_hw_bus" ]; then
		echo no-bus
		return 0
	fi
	_hw_client="$BOARD_SYS_DIR/bus/i2c/devices/$_hw_bus-$(printf '%04x' "$_hw_addr")"
	if [ -e "$_hw_client/driver" ]; then
		echo bound
		return 0
	fi
	if ! hw_have i2cdetect; then
		echo unknown
		return 0
	fi
	# Read-byte probes (-r) of this one address; the row holds just its cell.
	_hw_cell=$(i2cdetect -y -r "$_hw_bus" "$_hw_addr" "$_hw_addr" 2>/dev/null |
		awk -v row="$(printf '%02x:' $((_hw_addr & 0xf0)))" '$1 == row { print $2 }')
	case "$_hw_cell" in
	UU) echo bound ;;
	--) echo absent ;;
	[0-9a-f][0-9a-f]) echo present ;;
	*) echo unknown ;;
	esac
}

# hw_i2c_chip_id <id> <bus> <address>: reads the chip id register the
# manifest names for device <id> (HW_I2C_IDS) and prints one of
#   ok <value>          it reads what the manifest says
#   mismatch <value>    it reads something else
#   suspended           it needs its power control bit set first, and the
#                       backend leaves that to the chip's owner (lemnosd)
#   failed              the read failed
#   none                the manifest gives no chip id for it
#   unknown             there is no way to read it (no i2cget/i2cset)
# Reads are SMBus byte or word reads of a register (an SMBus word comes back
# byte-swapped from a big-endian register; HW_I2C_IDS holds that form), with
# hw_i2c_get: i2cget here, lemnosd's brokered raw I2C in the lemnosd backend
# (HW_I2C_BROKERED=1), which needs no i2c-tools and never interleaves with
# lemnosd's own polling. A
# chip that boots suspended (the BMM150) gets its power control bit set for
# the read and the register put back as it was, unless a kernel driver owns
# it or HW_I2C_POWER_DANCE=0 (the lemnosd backend: lemnosd owns the chip).
# hw_i2c_get <bus> <address> <register> b|w: one register, printed as
# i2cget prints it. The lemnosd backend replaces it.
hw_i2c_get() {
	i2cget -y "$1" "$2" "$3" "$4" 2>/dev/null
}

hw_i2c_chip_id() {
	_hw_spec=''
	for _hw_e in ${HW_I2C_IDS:-}; do
		[ "${_hw_e%%:*}" = "$1" ] && _hw_spec=${_hw_e#*:}
	done
	if [ -z "$_hw_spec" ]; then
		echo none
		return 0
	fi
	if [ "${HW_I2C_BROKERED:-0}" != 1 ] && ! hw_have i2cget; then
		echo unknown
		return 0
	fi
	_hw_reg=${_hw_spec%%:*}
	_hw_rest=${_hw_spec#*:}
	_hw_mode=${_hw_rest%%:*}
	_hw_rest=${_hw_rest#*:}
	_hw_want=${_hw_rest%%:*}
	_hw_power=''
	case "$_hw_rest" in *:*) _hw_power=${_hw_rest#*:} ;; esac
	_hw_restore=''
	if [ -n "$_hw_power" ]; then
		_hw_preg=${_hw_power%.*}
		_hw_bit=$((1 << ${_hw_power#*.}))
		_hw_pval=$(hw_i2c_get "$2" "$3" "$_hw_preg" b) || {
			echo failed
			return 0
		}
		if [ $((_hw_pval & _hw_bit)) = 0 ]; then
			if [ "${HW_I2C_POWER_DANCE:-1}" != 1 ] || ! hw_have i2cset ||
				[ -e "$BOARD_SYS_DIR/bus/i2c/devices/$2-$(printf '%04x' $(($3)))/driver" ]; then
				echo suspended
				return 0
			fi
			i2cset -y "$2" "$3" "$_hw_preg" $((_hw_pval | _hw_bit)) b 2>/dev/null || {
				echo failed
				return 0
			}
			_hw_restore=$_hw_pval
			sleep 0.01 2>/dev/null || sleep 1
		fi
	fi
	_hw_got=$(hw_i2c_get "$2" "$3" "$_hw_reg" "$_hw_mode") || _hw_got=''
	[ -z "$_hw_restore" ] || i2cset -y "$2" "$3" "$_hw_preg" "$_hw_restore" b 2>/dev/null || true
	if [ -z "$_hw_got" ]; then
		echo failed
	elif [ $((_hw_got)) = $((_hw_want)) ]; then
		echo "ok $_hw_got"
	else
		echo "mismatch $_hw_got"
	fi
}

# ---------------------------------------------------------------------------
# Camera. hw_camera_list prints one line per thing that sees a camera:
#   i2c <client name> <bus-address> <driver or "unbound">
#   media <media node> <graph driver> <entity name, spaces as _>
#   video <video node> <name, spaces as _>
#   libcamera <cam -l line, spaces as _>
# hw_camera_list <address 0xNN>
hw_camera_list() {
	_hw_addr=$(printf '%04x' $(($1)))
	for _hw_c in "$BOARD_SYS_DIR"/bus/i2c/devices/*-"$_hw_addr"; do
		[ -e "$_hw_c" ] || continue
		_hw_drv=unbound
		[ -e "$_hw_c/driver" ] && _hw_drv=$(basename "$(readlink -f "$_hw_c/driver")")
		printf 'i2c %s %s %s\n' "$(cat "$_hw_c/name" 2>/dev/null || echo '?')" "${_hw_c##*/}" "$_hw_drv"
	done
	if hw_have media-ctl; then
		for _hw_m in "$BOARD_DEV_DIR"/media*; do
			[ -e "$_hw_m" ] || continue
			media-ctl -d "$_hw_m" -p 2>/dev/null | awk -v dev="$_hw_m" '
				$1 == "driver" { drv = $2 }
				/^- entity [0-9]+:/ {
					name = $0
					sub(/^- entity [0-9]+: */, "", name)
					sub(/ *\(.*$/, "", name)
					gsub(/ /, "_", name)
					print "media " dev " " drv " " name
				}'
		done
	fi
	for _hw_v in "$BOARD_SYS_DIR"/class/video4linux/video*; do
		[ -r "$_hw_v/name" ] || continue
		printf 'video %s %s\n' "${_hw_v##*/}" "$(tr ' ' '_' < "$_hw_v/name")"
	done
	if hw_have cam; then
		cam -l 2>/dev/null | sed -n 's/^ *[0-9][0-9]*: *//p' | tr ' ' '_' | sed 's/^/libcamera /'
	fi
}

# ---------------------------------------------------------------------------
# Watchdog.

# The watchdog device node when it exists.
hw_watchdog_device() {
	_hw_wd=$(hw_dev_path "${HW_WATCHDOG_DEVICE:-/dev/watchdog0}")
	[ -e "$_hw_wd" ] && printf '%s\n' "$_hw_wd"
}

# The kernel driver's name for it, when sysfs says.
hw_watchdog_identity() {
	_hw_wd=${HW_WATCHDOG_DEVICE:-/dev/watchdog0}
	cat "$BOARD_SYS_DIR/class/watchdog/${_hw_wd##*/}/identity" 2>/dev/null || true
}

# systemd's RuntimeWatchdogUSec ("15s", "0", "infinity"); empty without systemctl.
hw_watchdog_runtime() {
	hw_have systemctl || return 0
	systemctl show -p RuntimeWatchdogUSec --value 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# USB gadget and its network.

# The controller the configfs gadget <name> is bound to; empty when unbound.
hw_gadget_udc() {
	cat "$BOARD_CONFIGFS/usb_gadget/$1/UDC" 2>/dev/null || true
}

# Whether interface <name> exists and is administratively up.
hw_net_up() {
	[ -r "$BOARD_NET_DIR/$1/flags" ] || return 1
	[ $(($(cat "$BOARD_NET_DIR/$1/flags") & 1)) = 1 ]
}

hw_net_operstate() {
	cat "$BOARD_NET_DIR/$1/operstate" 2>/dev/null || true
}

# IPv4 addresses (a.b.c.d/n) of interface <name>, one per line. Prints "?"
# when there is no way to tell (no ip tool).
hw_net_ipv4() {
	if ! hw_have ip; then
		echo '?'
		return 0
	fi
	ip -4 -o addr show dev "$1" 2>/dev/null | awk '{ for (i = 1; i < NF; i++) if ($i == "inet") print $(i + 1) }'
}

# ---------------------------------------------------------------------------
# The backend for the LED ring and the fan.

case "${BOARD_HW_BACKEND:-auto}" in
lemnosd | sysfs) HW_BACKEND=$BOARD_HW_BACKEND ;;
auto | */*)
	if hw_have lemnos-ctl; then HW_BACKEND=lemnosd; else HW_BACKEND=sysfs; fi
	;;
*)
	board_log "BOARD_HW_BACKEND=$BOARD_HW_BACKEND is not lemnosd, sysfs or auto; using sysfs"
	HW_BACKEND=sysfs
	;;
esac
# shellcheck source=hw-sysfs.sh
. "$BOARD_LIB_DIR/hw-$HW_BACKEND.sh"
