#!/bin/sh
# A stand-in for /usr/lib/board/update with the same commands, output and
# files ($BOARD_RUN_DIR/update.json, update.pid), for board-agent's tests.
# Its state is in $BOARD_RUN_DIR/fake.env; each call is logged to
# $BOARD_RUN_DIR/calls. Image URLs steer it: "corrupt" fails the checksum,
# "hold" waits until $BOARD_RUN_DIR/release exists, and image-<v>.img.xz
# stages version <v>. With $BOARD_RUN_DIR/slots-layout it is the slots
# writer: its status lists slots, and boot-once/set-default/remove/pin/unpin
# work (a slot named "running" can't be removed).
set -eu
RUN=$BOARD_RUN_DIR
STATE=idle SLOT_ACTIVE=A SLOT_STAGED='' VERSION_ACTIVE=1.0 VERSION_STAGED='' PROGRESS=0 ERROR='' PREVIOUS=''
[ ! -r "$RUN/fake.env" ] || . "$RUN/fake.env"

# Under a lock: a `status` running beside a stage must not write back the
# state it read before the stage's.
save() {
	(
		flock 9
		write_state
	) 9>>"$RUN/fake.lock"
}

write_state() {
	cat > "$RUN/fake.env.$$" <<EOF
STATE='$STATE' SLOT_ACTIVE='$SLOT_ACTIVE' SLOT_STAGED='$SLOT_STAGED' VERSION_ACTIVE='$VERSION_ACTIVE'
VERSION_STAGED='$VERSION_STAGED' PROGRESS='$PROGRESS' ERROR='$ERROR' PREVIOUS='$PREVIOUS'
EOF
	mv "$RUN/fake.env.$$" "$RUN/fake.env"
	slots=''
	# With $RUN/slots-many, 13 slots more: a list too long for one Orion
	# status value, as on a real board.
	more=''
	if [ -e "$RUN/slots-many" ]; then
		for n in 1 2 3 4 5 6 7 8 9 10 11 12 13; do
			more="$more,{\"id\":\"helios-2026.2.0-dev.$n-styx-0000000$n\",\"family\":\"helios\",\"version\":\"2026.2.0-dev.$n\",\"display_name\":\"HeliOS\",\"pinned\":false}"
		done
	fi
	[ ! -e "$RUN/slots-layout" ] ||
		slots=',"layout":"slots","slot_default":"running","slot_previous":null,"slot_trial":null,"free_bytes":5000000000,"reserve_bytes":536870912,"slots":[{"id":"running","pinned":false},{"id":"pv-b","pinned":false}'"$more"']'
	printf '{"state":"%s","slot_active":"%s","slot_staged":"%s","version_active":"%s","version_staged":"%s","progress":%s,"error":"%s"%s}\n' \
		"$STATE" "$SLOT_ACTIVE" "$SLOT_STAGED" "$VERSION_ACTIVE" "$VERSION_STAGED" "$PROGRESS" "$ERROR" "$slots" > "$RUN/update.json.$$"
	mv "$RUN/update.json.$$" "$RUN/update.json"
}

echo "$*" >> "$RUN/calls"
# The agent runs the writer as Orion, for the board's event log.
[ "${BOARD_EVENT_SOURCE:-}" = orion ] || echo "source-missing $*" >> "$RUN/calls"
case "${1:-}" in
status)
	(
		flock 9
		[ ! -r "$RUN/fake.env" ] || . "$RUN/fake.env"
		write_state
	) 9>>"$RUN/fake.lock"
	cat "$RUN/update.json"
	;;
stage-url)
	url=$2
	if [ "$STATE" = trying ]; then
		echo "update: the last update is still on trial; reboot or confirm it first" >&2
		exit 3
	fi
	echo $$ > "$RUN/update.pid"
	trap 'rm -f "$RUN/update.pid"' EXIT
	STATE=staging ERROR='' PROGRESS=0 SLOT_STAGED='' VERSION_STAGED=''
	save
	case "$url" in
	*corrupt*)
		# Long enough for the agent to see it staging, as a download is.
		sleep 0.2
		STATE=error ERROR="the downloaded image failed its SHA-256 check"
		save
		echo "update: $ERROR" >&2
		# Something it started keeps stderr open a while: a retry comes
		# while the agent still reads it.
		sleep 0.5 &
		exit 1
		;;
	esac
	for p in 250 500 750; do
		PROGRESS=$p
		save
		sleep 0.05
	done
	case "$url" in
	*hold*)
		until [ -e "$RUN/release" ]; do sleep 0.05; done
		;;
	esac
	version=${url##*/image-}
	STATE=staged PROGRESS=1000 SLOT_STAGED=B VERSION_STAGED=${version%.img.xz}
	save
	;;
apply)
	if [ -e "$RUN/refuse-apply" ]; then
		ERROR="the pre-reboot hook stopped the restart; the update is still staged"
		save
		echo "update: $ERROR" >&2
		exit 1
	fi
	STATE=rebooting
	save
	;;
cancel)
	if [ -s "$RUN/update.pid" ]; then
		pid=$(cat "$RUN/update.pid")
		kill "$pid" 2>/dev/null || true
		while kill -0 "$pid" 2>/dev/null; do sleep 0.05; done
		rm -f "$RUN/update.pid"
	elif [ "$STATE" != staged ]; then
		echo idle
		exit 0
	fi
	STATE=cancelled SLOT_STAGED='' VERSION_STAGED='' PROGRESS=0
	save
	echo cancelled
	;;
rollback)
	if [ -z "$PREVIOUS" ]; then
		echo "update: there is no previous confirmed slot to go back to" >&2
		exit 1
	fi
	STATE=rebooting VERSION_ACTIVE=$PREVIOUS PREVIOUS=''
	save
	;;
boot-once | set-default | remove | pin | unpin)
	[ -e "$RUN/slots-layout" ] || exit 2
	if [ "$1 $2" = "remove running" ]; then
		echo "slots: running is the running slot" >&2
		exit 3
	fi
	[ "$1" != boot-once ] || STATE=rebooting
	save
	;;
*)
	exit 2
	;;
esac
