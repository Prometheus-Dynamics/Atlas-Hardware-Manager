# shellcheck shell=sh
# The sysfs backend of hw.sh: the LED ring and the fan driven directly, for
# images without lemnosd. Sourced by hw.sh; see there. The lemnosd backend
# (hw-lemnosd.sh) has the same functions.

# ---------------------------------------------------------------------------
# LED ring. Colours are "r,g,b,w", 0-255 each. Logical LED i sits in driver
# slot (RAZE_LEDS_OFFSET + RAZE_LEDS_DIRECTION * i) mod RAZE_LEDS_COUNT, and
# every slot takes 4 bytes in RAZE_LEDS_ORDER (the driver drops W on a 24-bit
# ring).

hw_leds_device() {
	hw_dev_path "${RAZE_LEDS_DEVICE:-/dev/leds0}"
}

# What the report calls the ring.
hw_leds_name() {
	printf '%s\n' "${RAZE_LEDS_DEVICE:-/dev/leds0}"
}

# The ring's state: available (it can be opened for writing, nothing is
# written), missing or unwritable.
hw_leds_status() {
	_hw_dev=$(hw_leds_device)
	if [ ! -e "$_hw_dev" ]; then
		echo missing
	elif (: >> "$_hw_dev") 2>/dev/null; then
		echo available
	else
		echo unwritable
	fi
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
		board_log "$_hw_dev is missing or not writable (is the LED ring driver loaded?)"
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

# Gives the ring back after frames of our own: raze-leds draws its saved
# state again, or the ring goes dark.
hw_leds_release() {
	if [ -r "$BOARD_RUN_DIR/leds.state" ] && [ -x "$BOARD_LIB_DIR/raze-leds" ]; then
		"$BOARD_LIB_DIR/raze-leds" refresh 2>/dev/null && return 0
	fi
	hw_leds_fill 0,0,0,0 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Fan: the pwm-fan cooling device of the thermal framework, and its hwmon.

# The fan's cooling device directory.
hw_fan_cdev() {
	for _hw_d in "$BOARD_SYS_DIR"/class/thermal/cooling_device*; do
		[ -r "$_hw_d/type" ] || continue
		if [ "$(cat "$_hw_d/type")" = "${HW_FAN_COOLING_TYPE:-pwm-fan}" ]; then
			printf '%s\n' "$_hw_d"
			return 0
		fi
	done
	return 1
}

# Why hw_fan_cdev found nothing.
hw_fan_missing() {
	echo "no ${HW_FAN_COOLING_TYPE:-pwm-fan} cooling device: the raze-fan overlay didn't load"
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
	for _hw_h in "$BOARD_SYS_DIR"/class/hwmon/hwmon*; do
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
	for _hw_z in "$BOARD_SYS_DIR"/class/thermal/thermal_zone*; do
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

# Hands the fan back to the thermal governor. <state> (optional) is the
# cooling state to put back first.
hw_fan_release() {
	[ -z "${1:-}" ] || hw_fan_set_state "$1" 2>/dev/null || true
	for _hw_z in $HW_FAN_HELD; do
		echo enabled > "$_hw_z/mode" 2>/dev/null || true
		# The Raze's step_wise zone doesn't poll: it only re-evaluates on a
		# trip crossing, so the fan would stay at the last stepped level.
		# Writing the zone's own policy back makes the governor run now
		# (verified on a Raze, 2026-10-07).
		if [ -w "$_hw_z/policy" ]; then
			_hw_policy=$(cat "$_hw_z/policy" 2>/dev/null) &&
				echo "$_hw_policy" > "$_hw_z/policy" 2>/dev/null || true
		fi
	done
	HW_FAN_HELD=''
}

# ---------------------------------------------------------------------------
# I2C devices: no service owns them here, so the chip id reads may wake a
# suspended chip for the read (hw_i2c_chip_id).

# hw_i2c_service <id>: what a hardware service says about device <id>;
# "none" here.
hw_i2c_service() {
	echo none
}
