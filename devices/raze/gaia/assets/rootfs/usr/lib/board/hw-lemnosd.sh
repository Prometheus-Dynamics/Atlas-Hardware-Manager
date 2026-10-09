# shellcheck shell=sh
# The lemnosd backend of hw.sh: the LED ring, the fan and the sensors through
# lemnosd, the board's hardware service, with its command-line client
# lemnos-ctl. Sourced by hw.sh; see there. Same functions as hw-sysfs.sh.
#
# Device ids are board.toml's (HW_LEMNOS_* in hardware.env). Callers name
# their lemnosd client in HW_LEMNOS_CLIENT (default "board") and, for LED
# intents, a priority in HW_LEMNOS_PRIORITY (unset: lemnos-ctl's 50).
# HW_LEMNOS_TEST=1 (the self-test) puts LED intents on lemnosd's test layer,
# above every client's status, leased for HW_LEMNOS_TEST_SECONDS (60).
# lemnos-ctl keeps a client's LED intents after it exits, one per layer, until
# the same client replaces them or sends `led off`.

HW_LEMNOS_RING=${HW_LEMNOS_RING:-status-ring}
HW_LEMNOS_FAN=${HW_LEMNOS_FAN:-fan}
# lemnosd owns the sensors: never toggle a chip's power for an id read.
HW_I2C_POWER_DANCE=0

_hw_ctl() {
	lemnos-ctl --client "${HW_LEMNOS_CLIENT:-board}" "$@"
}

# hw_lemnos_status <device id>: the device's status in lemnosd (available,
# degraded, faulted, missing), "unknown-device" when lemnosd has no such
# device, "no-service" when lemnosd doesn't answer.
hw_lemnos_status() {
	if ! _hw_list=$(_hw_ctl list 2>/dev/null); then
		echo no-service
		return 0
	fi
	printf '%s\n' "$_hw_list" | awk -v id="$1" '$1 == id { print $4; found = 1; exit } END { if (!found) print "unknown-device" }'
}

# ---------------------------------------------------------------------------
# LED ring. Colours are "r,g,b,w", 0-255 each; lemnosd lights RGB, so W is
# dropped (as the Raze's 24-bit ring does). Indexes are logical: lemnosd
# applies the ring's offset and direction from board.toml.

hw_leds_name() {
	printf 'lemnosd %s\n' "$HW_LEMNOS_RING"
}

hw_leds_status() {
	hw_lemnos_status "$HW_LEMNOS_RING"
}

# RRGGBB for r,g,b[,w].
_hw_leds_hex() {
	_hw_r=${1%%,*}
	_hw_rest=${1#*,}
	_hw_g=${_hw_rest%%,*}
	_hw_rest=${_hw_rest#*,}
	printf '%02x%02x%02x' "$_hw_r" "$_hw_g" "${_hw_rest%%,*}"
}

_hw_led() {
	if [ "${HW_LEMNOS_TEST:-0}" = 1 ]; then
		# `led off --test` clears only the test layer.
		if [ "$1" = off ]; then
			set -- "$@" --test
		else
			set -- "$@" --test --seconds "${HW_LEMNOS_TEST_SECONDS:-60}"
		fi
	fi
	_hw_ctl ${HW_LEMNOS_PRIORITY:+--priority "$HW_LEMNOS_PRIORITY"} led "$@" --device "$HW_LEMNOS_RING" >/dev/null
}

# hw_leds_fill <r,g,b,w>: one colour on every LED, at once (no fade).
hw_leds_fill() {
	_hw_led color "$(_hw_leds_hex "$1")" --fade 0
}

# hw_leds_write_frame <colour>...: one frame; argument i (from 0) is logical
# LED i. LEDs without an argument are off.
hw_leds_write_frame() {
	_hw_frame=''
	_hw_i=0
	while [ "$_hw_i" -lt "$(hw_leds_count)" ]; do
		_hw_c=0,0,0,0
		if [ "$_hw_i" -lt "$#" ]; then
			eval "_hw_c=\${$((_hw_i + 1))}"
		fi
		_hw_frame="$_hw_frame${_hw_frame:+,}$(_hw_leds_hex "$_hw_c")"
		_hw_i=$((_hw_i + 1))
	done
	_hw_led frame "$_hw_frame" --fade 0
}

# Gives the ring back: drops this client's intents, so lemnosd shows the
# other clients' (raze-leds, applications, system states) again.
hw_leds_release() {
	_hw_led off 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Fan: lemnosd's hwmon fan. A duty a client sets moves the pwm-fan cooling
# state with it; `lemnos-ctl fan release <fan>` hands it back to the kernel
# governor while lemnosd keeps running (the zone re-evaluates at once). The
# next client write takes it back. Nothing is paused.

HW_FAN_STATUS=''
# The fan's lemnosd device id, when lemnosd has the fan.
hw_fan_cdev() {
	HW_FAN_STATUS=$(hw_lemnos_status "$HW_LEMNOS_FAN")
	[ "$HW_FAN_STATUS" = available ] || [ "$HW_FAN_STATUS" = degraded ] || return 1
	printf '%s\n' "$HW_LEMNOS_FAN"
}

hw_fan_missing() {
	_hw_s=${HW_FAN_STATUS:-$(hw_lemnos_status "$HW_LEMNOS_FAN")}
	case "$_hw_s" in
	no-service) echo "lemnosd doesn't answer (is lemnosd.service running?)" ;;
	unknown-device) echo "lemnosd has no device $HW_LEMNOS_FAN (check /etc/lemnos/board.toml)" ;;
	*) echo "lemnosd reports the fan $_hw_s: no ${HW_FAN_HWMON:-pwmfan} hwmon fan (the raze-fan overlay didn't load?)" ;;
	esac
}

# The number of cooling states, from the pwm-fan cooling device (read only),
# else from the package's levels.
hw_fan_max_state() {
	for _hw_d in "$BOARD_SYS_DIR"/class/thermal/cooling_device*; do
		if [ "$(cat "$_hw_d/type" 2>/dev/null)" = "${HW_FAN_COOLING_TYPE:-pwm-fan}" ]; then
			cat "$_hw_d/max_state"
			return
		fi
	done
	set -- ${HW_FAN_LEVELS:-}
	echo $(($# - 1))
}

# "<pwm> <rpm>" from lemnosd's reading: duty (0..1) as 0-255, speed in rpm;
# "-" for what the fan doesn't report.
hw_fan_read() {
	_hw_ctl read "$HW_LEMNOS_FAN" 2>/dev/null | awk '
		{
			pwm = "-"; rpm = "-"
			for (i = 1; i <= NF; i++) {
				split($i, kv, "=")
				v = kv[2]; sub(/[^0-9.-]+$/, "", v)
				if (kv[1] == "duty" && v != "" && v != "-") pwm = int(v * 255 + 0.5)
				if (kv[1] == "speed" && v != "" && v != "-") rpm = int(v + 0.5)
			}
		}
		END { print (pwm == "" ? "-" : pwm) " " (rpm == "" ? "-" : rpm) }'
}

# The cooling state whose level is the current duty; empty when none is.
hw_fan_get_state() {
	_hw_pwm=$(hw_fan_read)
	_hw_pwm=${_hw_pwm%% *}
	_hw_s=0
	for _hw_level in ${HW_FAN_LEVELS:-}; do
		if [ "$_hw_level" = "$_hw_pwm" ]; then
			echo "$_hw_s"
			return 0
		fi
		_hw_s=$((_hw_s + 1))
	done
}

# Sets the duty of cooling state <s> (its level in HW_FAN_LEVELS).
hw_fan_set_state() {
	_hw_level=$(_hw_nth $(($1 + 1)) ${HW_FAN_LEVELS:-})
	[ -n "$_hw_level" ] || return 1
	_hw_ctl set "$HW_LEMNOS_FAN" duty "$(awk -v l="$_hw_level" 'BEGIN { printf "%.4f", l / 255 }')" >/dev/null
}

HW_FAN_HELD=''
hw_fan_hold() {
	HW_FAN_HELD=lemnosd
}

# Hands the fan back to the governor through lemnosd's hand-back; the state
# argument of the sysfs backend isn't needed.
hw_fan_release() {
	[ -n "$HW_FAN_HELD" ] || return 0
	HW_FAN_HELD=''
	if ! _hw_out=$(_hw_ctl fan release "$HW_LEMNOS_FAN" 2>&1); then
		board_log "lemnos-ctl fan release: $_hw_out"
		return 1
	fi
}

# ---------------------------------------------------------------------------
# I2C devices.

# hw_i2c_service <manifest device id>: lemnosd's status for the board device
# that drives it (available means its driver bound and checked the chip id),
# "none" when board.toml has none.
hw_i2c_service() {
	for _hw_e in ${HW_LEMNOS_DEVICES:-}; do
		if [ "${_hw_e%%:*}" = "$1" ]; then
			hw_lemnos_status "${_hw_e#*:}"
			return 0
		fi
	done
	echo none
}
