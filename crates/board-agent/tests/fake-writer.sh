#!/bin/sh
# A stand-in for /usr/lib/board/update with the same commands, output and
# files ($BOARD_RUN_DIR/update.json, update.pid), for board-agent's tests.
# Its state is in $BOARD_RUN_DIR/fake.env; each call is logged to
# $BOARD_RUN_DIR/calls. Image URLs steer it: "corrupt" fails the checksum,
# "hold" waits until $BOARD_RUN_DIR/release exists, and image-<v>.img.xz
# stages version <v>.
set -eu
RUN=$BOARD_RUN_DIR
STATE=idle SLOT_ACTIVE=A SLOT_STAGED='' VERSION_ACTIVE=1.0 VERSION_STAGED='' PROGRESS=0 ERROR='' PREVIOUS=''
[ ! -r "$RUN/fake.env" ] || . "$RUN/fake.env"

save() {
	cat > "$RUN/fake.env.tmp" <<EOF
STATE='$STATE' SLOT_ACTIVE='$SLOT_ACTIVE' SLOT_STAGED='$SLOT_STAGED' VERSION_ACTIVE='$VERSION_ACTIVE'
VERSION_STAGED='$VERSION_STAGED' PROGRESS='$PROGRESS' ERROR='$ERROR' PREVIOUS='$PREVIOUS'
EOF
	mv "$RUN/fake.env.tmp" "$RUN/fake.env"
	printf '{"state":"%s","slot_active":"%s","slot_staged":"%s","version_active":"%s","version_staged":"%s","progress":%s,"error":"%s"}\n' \
		"$STATE" "$SLOT_ACTIVE" "$SLOT_STAGED" "$VERSION_ACTIVE" "$VERSION_STAGED" "$PROGRESS" "$ERROR" > "$RUN/update.json.tmp"
	mv "$RUN/update.json.tmp" "$RUN/update.json"
}

echo "$*" >> "$RUN/calls"
case "${1:-}" in
status)
	save
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
		STATE=error ERROR="the downloaded image failed its SHA-256 check"
		save
		echo "update: $ERROR" >&2
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
*)
	exit 2
	;;
esac
