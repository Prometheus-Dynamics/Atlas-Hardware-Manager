# shellcheck shell=sh
# Hardware backend for raze-leds and the selftest (device package contract 1).
# POSIX sh; also runs under busybox ash. Sourced after lib.sh, never executed.
#
# Every hardware access of those scripts goes through the hw_* functions here,
# implemented on sysfs and /dev. Another backend (a hardware daemon) can
# replace this file with the same functions, and the callers and their output
# stay as they are.
#
# Settings: leds.env and hardware.env, both generated from the package's
# manifest.json (devices/tools/gen-raze.py). Paths, overridable for tests:
#   PD_SYS_DIR    sysfs                 (/sys)
#   PD_DEV_DIR    device nodes          (/dev)
#   PD_CONFIGFS   configfs              (/sys/kernel/config)
#   PD_HW_MISSING tools to treat as not installed, space-separated (tests)

PD_SYS_DIR=${PD_SYS_DIR:-/sys}
PD_DEV_DIR=${PD_DEV_DIR:-/dev}
PD_CONFIGFS=${PD_CONFIGFS:-/sys/kernel/config}
pd_load_env leds.env
pd_load_env hardware.env

# Whether a tool is installed.
hw_have() {
	case " ${PD_HW_MISSING:-} " in
	*" $1 "*) return 1 ;;
	esac
	command -v "$1" >/dev/null 2>&1
}

# A /dev path under PD_DEV_DIR; other paths unchanged.
hw_dev_path() {
	case "$1" in
	/dev/*) printf '%s/%s\n' "$PD_DEV_DIR" "${1#/dev/}" ;;
	*) printf '%s\n' "$1" ;;
	esac
}

# ---------------------------------------------------------------------------
# LED ring. Colours are "r,g,b,w", 0-255 each. Logical LED i sits in driver
# slot (RAZE_LEDS_OFFSET + RAZE_LEDS_DIRECTION * i) mod RAZE_LEDS_COUNT, and
# every slot takes 4 bytes in RAZE_LEDS_ORDER (the driver drops W on a 24-bit
# ring).

hw_leds_device() {
	hw_dev_path "${RAZE_LEDS_DEVICE:-/dev/leds0}"
}

hw_leds_count() {
	printf '%s\n' "${RAZE_LEDS_COUNT:-16}"
}

# The driver bytes for one colour, as printf octal escapes.
_hw_leds_pixel() {
	_hw_r=${1%%,*}
	_hw_rest=${1#*,}
	_hw_g=${_hw_rest%%,*}
	_hw_rest=${_hw_rest#*,}
	_hw_b=${_hw_rest%%,*}
	case "$_hw_rest" in
	*,*) _hw_w=${_hw_rest#*,} ;;
	*) _hw_w=0 ;;
	esac
	_hw_px=''
	for _hw_ch in $(printf '%s' "${RAZE_LEDS_ORDER:-rgbw}" | sed 's/./& /g'); do
		case "$_hw_ch" in
		r) _hw_v=$_hw_r ;;
		g) _hw_v=$_hw_g ;;
		b) _hw_v=$_hw_b ;;
		w) _hw_v=$_hw_w ;;
		*) continue ;;
		esac
		_hw_px="$_hw_px\\$(printf '%03o' "$_hw_v")"
	done
	printf '%s' "$_hw_px"
}

_hw_leds_writable() {
	_hw_dev=$(hw_leds_device)
	if [ ! -w "$_hw_dev" ]; then
		pd_log "$_hw_dev is missing or not writable (is the LED ring driver loaded?)"
		return 1
	fi
}

# hw_leds_fill <r,g,b,w>: one colour on every LED.
hw_leds_fill() {
	_hw_leds_writable || return 1
	_hw_px=$(_hw_leds_pixel "$1")
	_hw_all=''
	_hw_i=0
	while [ "$_hw_i" -lt "${RAZE_LEDS_COUNT:-16}" ]; do
		_hw_all="$_hw_all$_hw_px"
		_hw_i=$((_hw_i + 1))
	done
	# shellcheck disable=SC2059
	printf "$_hw_all" > "$_hw_dev"
}

# hw_leds_write_frame <colour>...: one frame; argument i (from 0) is logical
# LED i. LEDs without an argument are off.
hw_leds_write_frame() {
	_hw_leds_writable || return 1
	_hw_n=${RAZE_LEDS_COUNT:-16}
	_hw_off=${RAZE_LEDS_OFFSET:-0}
	_hw_dir=${RAZE_LEDS_DIRECTION:-1}
	_hw_all=''
	_hw_last=''
	_hw_lastpx=''
	_hw_slot=0
	while [ "$_hw_slot" -lt "$_hw_n" ]; do
		# The direction is +1 or -1, so it is its own inverse.
		_hw_l=$(((((_hw_slot - _hw_off) * _hw_dir) % _hw_n + _hw_n) % _hw_n))
		_hw_c=0,0,0,0
		if [ "$_hw_l" -lt "$#" ]; then
			eval "_hw_c=\${$((_hw_l + 1))}"
		fi
		if [ "$_hw_c" != "$_hw_last" ]; then
			_hw_last=$_hw_c
			_hw_lastpx=$(_hw_leds_pixel "$_hw_c")
		fi
		_hw_all="$_hw_all$_hw_lastpx"
		_hw_slot=$((_hw_slot + 1))
	done
	# shellcheck disable=SC2059
	printf "$_hw_all" > "$_hw_dev"
}

# Whether the ring device can be opened for writing, without writing to it.
hw_leds_open() {
	_hw_dev=$(hw_leds_device)
	[ -e "$_hw_dev" ] && (: >> "$_hw_dev") 2>/dev/null
}

# ---------------------------------------------------------------------------
# Fan: the pwm-fan cooling device of the thermal framework, and its hwmon.

# The fan's cooling device directory.
hw_fan_cdev() {
	for _hw_d in "$PD_SYS_DIR"/class/thermal/cooling_device*; do
		[ -r "$_hw_d/type" ] || continue
		if [ "$(cat "$_hw_d/type")" = "${HW_FAN_COOLING_TYPE:-pwm-fan}" ]; then
			printf '%s\n' "$_hw_d"
			return 0
		fi
	done
	return 1
}

hw_fan_max_state() {
	cat "$(hw_fan_cdev)/max_state"
}

hw_fan_get_state() {
	cat "$(hw_fan_cdev)/cur_state"
}

hw_fan_set_state() {
	printf '%s\n' "$1" > "$(hw_fan_cdev)/cur_state"
}

# "<pwm> <rpm>": the duty (0-255) and the tachometer, "-" for what the fan
# doesn't report.
hw_fan_read() {
	_hw_pwm=-
	_hw_rpm=-
	for _hw_h in "$PD_SYS_DIR"/class/hwmon/hwmon*; do
		[ "$(cat "$_hw_h/name" 2>/dev/null)" = "${HW_FAN_HWMON:-pwmfan}" ] || continue
		[ -r "$_hw_h/pwm1" ] && _hw_pwm=$(cat "$_hw_h/pwm1")
		[ -r "$_hw_h/fan1_input" ] && _hw_rpm=$(cat "$_hw_h/fan1_input")
		break
	done
	printf '%s %s\n' "$_hw_pwm" "$_hw_rpm"
}

# Pause the thermal governor on the zones that drive the fan, so a state set
# by hand stays while it is read back. hw_fan_release undoes it.
HW_FAN_HELD=''
hw_fan_hold() {
	_hw_cdev=$(hw_fan_cdev) || return 1
	_hw_target=$(readlink -f "$_hw_cdev")
	for _hw_z in "$PD_SYS_DIR"/class/thermal/thermal_zone*; do
		[ -w "$_hw_z/mode" ] || continue
		[ "$(cat "$_hw_z/mode" 2>/dev/null)" = enabled ] || continue
		for _hw_link in "$_hw_z"/cdev[0-9]*; do
			case "$_hw_link" in *_*) continue ;; esac
			if [ "$(readlink -f "$_hw_link")" = "$_hw_target" ]; then
				echo disabled > "$_hw_z/mode" && HW_FAN_HELD="$HW_FAN_HELD $_hw_z"
				break
			fi
		done
	done
}

hw_fan_release() {
	for _hw_z in $HW_FAN_HELD; do
		echo enabled > "$_hw_z/mode" 2>/dev/null || true
	done
	HW_FAN_HELD=''
}

# ---------------------------------------------------------------------------
# I2C.

# hw_i2c_probe <bus> <address, 0xNN>: prints one of
#   bound     a kernel driver owns the address
#   present   the address answers
#   absent    nothing answers
#   no-bus    the bus doesn't exist
#   unknown   there is no way to probe (no i2cdetect)
hw_i2c_probe() {
	_hw_bus=$1
	_hw_addr=$(($2))
	if [ ! -e "$PD_SYS_DIR/bus/i2c/devices/i2c-$_hw_bus" ] && [ ! -e "$PD_DEV_DIR/i2c-$_hw_bus" ]; then
		echo no-bus
		return 0
	fi
	_hw_client="$PD_SYS_DIR/bus/i2c/devices/$_hw_bus-$(printf '%04x' "$_hw_addr")"
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

# ---------------------------------------------------------------------------
# Camera. hw_camera_list prints one line per thing that sees a camera:
#   i2c <client name> <bus-address> <driver or "unbound">
#   media <media node> <graph driver> <entity name, spaces as _>
#   video <video node> <name, spaces as _>
#   libcamera <cam -l line, spaces as _>
# hw_camera_list <address 0xNN>
hw_camera_list() {
	_hw_addr=$(printf '%04x' $(($1)))
	for _hw_c in "$PD_SYS_DIR"/bus/i2c/devices/*-"$_hw_addr"; do
		[ -e "$_hw_c" ] || continue
		_hw_drv=unbound
		[ -e "$_hw_c/driver" ] && _hw_drv=$(basename "$(readlink -f "$_hw_c/driver")")
		printf 'i2c %s %s %s\n' "$(cat "$_hw_c/name" 2>/dev/null || echo '?')" "${_hw_c##*/}" "$_hw_drv"
	done
	if hw_have media-ctl; then
		for _hw_m in "$PD_DEV_DIR"/media*; do
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
	for _hw_v in "$PD_SYS_DIR"/class/video4linux/video*; do
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
	cat "$PD_SYS_DIR/class/watchdog/${_hw_wd##*/}/identity" 2>/dev/null || true
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
	cat "$PD_CONFIGFS/usb_gadget/$1/UDC" 2>/dev/null || true
}

# Whether interface <name> exists and is administratively up.
hw_net_up() {
	[ -r "$PD_NET_DIR/$1/flags" ] || return 1
	[ $(($(cat "$PD_NET_DIR/$1/flags") & 1)) = 1 ]
}

hw_net_operstate() {
	cat "$PD_NET_DIR/$1/operstate" 2>/dev/null || true
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
