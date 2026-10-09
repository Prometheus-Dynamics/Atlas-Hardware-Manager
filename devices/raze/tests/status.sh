#!/bin/sh
# Off-device test of the board's awareness: the event log (lib.sh
# board_event, event), the boot record, `status` (fake sysfs and systemctl),
# drift, and the identity endpoint's GET /status and GET /events (one HTTP
# exchange on stdin, as board-http.socket runs it).
# Needs python3. Run: sh devices/raze/tests/status.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
lib=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-status-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

S=$T/sys
mkdir -p "$T/run" "$T/data" "$T/etc" "$T/bin" "$T/dt" "$T/net" "$T/ssh" "$T/boot/overlays" "$T/p1" \
	"$S/class/thermal/thermal_zone0" "$S/class/thermal/cooling_device0" "$S/class/hwmon/hwmon0"
printf 'ID=helios\nVERSION_ID=2026.3.0\n' > "$T/os-release"
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
echo cpu-thermal > "$S/class/thermal/thermal_zone0/type"
echo 54321 > "$S/class/thermal/thermal_zone0/temp"
echo pwm-fan > "$S/class/thermal/cooling_device0/type"
echo 4 > "$S/class/thermal/cooling_device0/max_state"
echo 2 > "$S/class/thermal/cooling_device0/cur_state"
echo pwmfan > "$S/class/hwmon/hwmon0/name"
echo 212 > "$S/class/hwmon/hwmon0/pwm1"
echo '1234.56 99.0' > "$T/uptime"
cat > "$T/bin/systemctl" <<'EOF'
#!/bin/sh
# --failed --plain --no-legend; $T/systemctl-fails: as for the identity
# endpoint's sandboxed user, which can't reach systemd.
[ "$1" = --failed ] || exit 0
[ ! -e "${0%/bin/systemctl}/systemctl-fails" ] || { echo "Failed to connect to bus" >&2; exit 1; }
printf 'foo.service loaded failed failed Foo\n'
printf '● bar.mount loaded failed failed Bar\n'
EOF
chmod +x "$T/bin/systemctl"

export BOARD_LIB_DIR=$lib BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run
export BOARD_DT_DIR=$T/dt BOARD_NET_DIR=$T/net BOARD_SYS_DIR=$S BOARD_DEV_DIR=$T/dev BOARD_OS_RELEASE=$T/os-release
export BOARD_BOOT_ID=boot-1 BOARD_UNAME_R=7.2.9-test UPDATE_CMDLINE_ROOT=5 BOARD_HW_BACKEND=sysfs
export BOARD_PROC_UPTIME=$T/uptime BOARD_SYSTEMCTL=$T/bin/systemctl BOARD_ROOT_RO=1 BOARD_SSHD_DIR=$T/ssh
export BOARD_DRIFT_BOOT_DIR=$T/boot BOARD_DRIFT_AB_DIR=$T/p1
unset BOARD_EVENT_SOURCE 2>/dev/null || true

# py <expression over d, the JSON on stdin>: prints its value.
py() { python3 -c 'import json, sys; d = json.load(sys.stdin); print(eval(sys.argv[1], {"d": d}))' "$1"; }
# is <expected> <expression> <what>: checks the JSON on stdin.
is() {
	got=$(py "$2") || fail "$3: not JSON"
	[ "$got" = "$1" ] || fail "$3: expected $1, got $got"
}
events() { cat "$T/data/events.jsonl"; }
event() { sh "$lib/event" "$@"; }

echo "board_event appends one JSON line, source local by default"
event test.kind 'a "quoted" message\with a backslash' key=value 'odd key!=x' novalue
events | tail -n 1 | is test.kind 'd["kind"]' kind
events | tail -n 1 | is local 'd["source"]' source
events | tail -n 1 | is boot-1 'd["boot_id"]' "boot id"
events | tail -n 1 | is 'a "quoted" message\with a backslash' 'd["message"]' message
events | tail -n 1 | is "{'key': 'value', 'oddkey': 'x'}" 'd["data"]' data
[ "$(events | wc -l)" -eq 1 ] || fail "one line per event"

echo "the source comes from BOARD_EVENT_SOURCE; unknown ones are local"
BOARD_EVENT_SOURCE=atlas event clock.set "clock set"
events | tail -n 1 | is atlas 'd["source"]' "atlas source"
BOARD_EVENT_SOURCE='orion"' event x "y"
events | tail -n 1 | is local 'd["source"]' "a bad source"
if event only-kind 2>/dev/null; then fail "an event needs a message"; fi
if event 2>/dev/null; then fail "an event needs a kind"; fi

echo "without a writable /data, events go to /run"
: > "$T/not-a-dir"
BOARD_DATA_DIR=$T/not-a-dir/board event fallback "to run"
grep -q '"kind":"fallback"' "$T/run/events.jsonl" || fail "the event should be in /run"

echo "the log keeps the last BOARD_EVENT_MAX events"
mkdir -p "$T/rot"
for i in $(seq 1 25); do BOARD_DATA_DIR=$T/rot BOARD_EVENT_MAX=20 event n "$i"; done
n=$(wc -l < "$T/rot/events.jsonl")
[ "$n" -ge 20 ] && [ "$n" -le 22 ] || fail "rotation kept $n lines"
tail -n 1 "$T/rot/events.jsonl" | is 25 'd["message"]' "the newest event stays"

echo "events as JSON: since, limit, both files, junk skipped"
printf '{"t":12,"boot_id"\ngarbage\n' >> "$T/data/events.jsonl"
event --json | is 4 'len(d["events"])' "all events (3 + the one in /run)"
event --json --limit 2 | is 2 'len(d["events"])' "the newest two"
event --json --since 9999999999 | is 0 'len(d["events"])' "since the future"
event --json | is boot-1 'd["boot_id"]' "the current boot"
if event --json --since abc 2>/dev/null; then fail "--since must be a number"; fi
if event --json --limit -1 2>/dev/null; then fail "--limit must be a number"; fi

echo "boot-record counts boots and tells a clean shutdown"
sh "$lib/boot-record"
is 1 'd["count"]' "first boot" < "$T/run/boot.json"
is None 'd["previous_clean"]' "nothing before the first boot" < "$T/run/boot.json"
is A 'd["slot"]' slot < "$T/run/boot.json"
is 7.2.9-test 'd["kernel"]' kernel < "$T/run/boot.json"
sh "$lib/boot-record"
is 1 'd["count"]' "a second run in the same boot doesn't count" < "$T/run/boot.json"
sh "$lib/boot-record" --shutdown
BOARD_BOOT_ID=boot-2 sh "$lib/boot-record"
is 2 'd["count"]' "second boot" < "$T/run/boot.json"
is True 'd["previous_clean"]' "after a clean shutdown" < "$T/run/boot.json"
BOARD_BOOT_ID=boot-3 sh "$lib/boot-record"
is False 'd["previous_clean"]' "after a power cut" < "$T/run/boot.json"
events | tail -n 1 | is "boot 3, slot A, kernel 7.2.9-test; the previous boot didn't shut down cleanly" 'd["message"]' "boot event"
events | tail -n 1 | is "{'count': '3', 'slot': 'A', 'kernel': '7.2.9-test', 'previous_clean': 'false'}" 'd["data"]' "boot data"
export BOARD_BOOT_ID=boot-3

echo "status: boot, failed units, temperatures, fan, update, clock"
printf '{"state":"staging","slot_active":"A","slot_staged":"B","version_active":"1.0","version_staged":"","progress":420,"error":"","version_previous":null,"started_by":"orion"}\n' > "$T/run/update.json"
printf 'BOARD_ACTIONS=locate\n' > "$T/etc/identity.env"
board_files() {
	printf 'kernel=kernel_2712.img\n' > "$T/boot/config.txt"
	printf 'console=tty1 root=/dev/mmcblk0p5 rootwait\n' > "$T/boot/cmdline.txt"
	printf 'Linux version 7.2.9-test\n' > "$T/boot/kernel_2712.img"
	printf 'dtbo' > "$T/boot/overlays/raze.dtbo"
}
board_files
. "$lib/lib.sh"
board_boot_hashes "$T/boot" > "$T/p1/board-boot.A.sha256"
sh "$lib/status" > "$T/status.json"
is 1 'd["version"]' version < "$T/status.json"
is "['foo.service', 'bar.mount']" 'd["failed_units"]' "failed units" < "$T/status.json"

echo "failed units: root's kept list for the sandboxed endpoint; null, never [], when unknown"
touch "$T/systemctl-fails"
sh "$lib/status" | is None 'd["failed_units"]' "systemctl unreachable and nothing kept: unknown"
rm "$T/systemctl-fails"
sh "$lib/status" --refresh-failed --quiet || fail "--refresh-failed"
is "['foo.service', 'bar.mount']" 'd["units"]' "the kept list" < "$T/run/failed.json"
touch "$T/systemctl-fails"
sh "$lib/status" | is "['foo.service', 'bar.mount']" 'd["failed_units"]' "a fresh kept list is used"
BOARD_FAILED_MAX_AGE=0 sh "$lib/status" | is None 'd["failed_units"]' "a stale one isn't"
if sh "$lib/status" --refresh-failed --quiet 2>/dev/null; then fail "--refresh-failed must fail without systemctl"; fi
rm "$T/systemctl-fails" "$T/run/failed.json"
is "[{'id': 'cpu-thermal', 'celsius': 54.3}]" 'd["temperatures"]' temperatures < "$T/status.json"
is "{'state': 2, 'max_state': 4, 'pwm': 212, 'rpm': None}" 'd["fan"]' fan < "$T/status.json"
is "3 A 1234 False" '" ".join(str(d["boot"][k]) for k in ("count", "slot", "uptime_s", "previous_clean"))' boot < "$T/status.json"
is "staging 420 orion" '" ".join(str(d["update"][k]) for k in ("state", "progress", "started_by"))' update < "$T/status.json"
is True 'isinstance(d["clock"]["time"], int)' clock < "$T/status.json"

echo "drift: the stage's hashes match; overrides are listed"
is "stage 0 True []" '" ".join(str(x) for x in (d["drift"]["baseline"], d["drift"]["count"], d["drift"]["root_read_only"], d["drift"]["flags"]))' \
	"no drift" < "$T/status.json"
is "[('etc', 'present')]" '[(i["area"], i["change"]) for i in d["drift"]["items"]]' "/etc/board files are listed" < "$T/status.json"

echo "drift: a changed cmdline, a removed kernel, a new overlay, a /data override"
printf 'console=tty1 root=/dev/mmcblk0p5 rootwait init=/bin/sh\n' > "$T/boot/cmdline.txt"
rm "$T/boot/kernel_2712.img"
printf 'new' > "$T/boot/overlays/extra.dtbo"
printf 'UPDATE_CONFIRM_DELAY=5\n' > "$T/data/update.env"
sh "$lib/status" --refresh-drift --quiet
d=$(cat "$T/run/drift.json")
printf '%s' "$d" | is 4 'd["count"]' "drift count"
printf '%s' "$d" | is "['cmdline', 'update_env']" 'd["flags"]' flags
printf '%s' "$d" | is "[('boot', 'cmdline.txt', 'changed'), ('boot', 'kernel_2712.img', 'missing'), ('boot', 'overlays/extra.dtbo', 'added'), ('data', 'update.env', 'present')]" \
	'sorted((i["area"], i["path"].rsplit("/data/", 1)[-1], i["change"]) for i in d["items"] if i["area"] != "etc")' items
printf '%s' "$d" | grep -q 'events.jsonl\|boot-count' && fail "the board's own state isn't an override"

echo "the kept result is reused while it is fresh"
board_files
rm "$T/boot/overlays/extra.dtbo" "$T/data/update.env"
sh "$lib/status" | is 4 'd["drift"]["count"]' "a fresh result isn't redone"
BOARD_DRIFT_MAX_AGE=0 sh "$lib/status" | is 0 'd["drift"]["count"]' "a stale one is"

echo "without the stage's hashes, what the board first saw is the baseline"
rm "$T/p1/board-boot.A.sha256"
sh "$lib/status" --refresh-drift --quiet
is "first-seen 0" '" ".join(str(x) for x in (d["baseline"], d["count"]))' "first seen" < "$T/run/drift.json"
[ -s "$T/data/drift/boot.A.sha256" ] || fail "the first-seen hashes are kept in /data"
printf 'kernel=kernel_2712.img\ndtparam=audio=on\n' > "$T/boot/config.txt"
sh "$lib/status" --refresh-drift --quiet
is "['config']" 'd["flags"]' "config.txt changed" < "$T/run/drift.json"

echo "a writable root: sshd_config against what was first seen; EROFS is skipped"
board_files
printf 'PermitRootLogin prohibit-password\n' > "$T/ssh/sshd_config"
BOARD_ROOT_RO=0 sh "$lib/status" --refresh-drift --quiet
is "False 0" '" ".join(str(x) for x in (d["root_read_only"], d["count"]))' "first look at the root" < "$T/run/drift.json"
printf 'PermitRootLogin yes\nPasswordAuthentication yes\n' > "$T/ssh/sshd_config"
BOARD_ROOT_RO=0 sh "$lib/status" --refresh-drift --quiet
is "['sshd_config']" 'd["flags"]' "sshd_config changed" < "$T/run/drift.json"
sh "$lib/status" --refresh-drift --quiet
is "True []" '" ".join(str(x) for x in (d["root_read_only"], d["flags"]))' "a read-only root can't drift" < "$T/run/drift.json"

echo "an unprivileged caller reads the kept result and checks nothing"
if [ "$(id -u)" != 0 ]; then
	at=$(py 'd["checked_at"]' < "$T/run/drift.json")
	env -u BOARD_DRIFT_BOOT_DIR -u BOARD_DRIFT_AB_DIR BOARD_DRIFT_MAX_AGE=0 sh "$lib/status" |
		is "$at" 'd["drift"]["checked_at"]' "the kept result"
	if env -u BOARD_DRIFT_BOOT_DIR -u BOARD_DRIFT_AB_DIR sh "$lib/status" --refresh-drift 2>/dev/null; then
		fail "--refresh-drift needs root"
	fi
fi

echo "HTTP: GET /status and /events, read-only"
# request <request line>: the whole response.
request() { printf '%s\r\nHost: board\r\n\r\n' "$1" | sh "$lib/identity-http"; }
body() { sed '1,/^\r$/d'; }
status_line() { head -n 1 | tr -d '\r'; }
[ "$(request 'GET /status HTTP/1.0' | status_line)" = "HTTP/1.0 200 OK" ] || fail "GET /status"
request 'GET /status HTTP/1.0' | body | is "1 ['foo.service', 'bar.mount']" '" ".join(str(x) for x in (d["version"], d["failed_units"]))' "the status body"
[ -z "$(request 'HEAD /status HTTP/1.0' | body)" ] || fail "HEAD has no body"
r=$(request 'POST /status HTTP/1.0')
printf '%s' "$r" | status_line | grep -q '^HTTP/1.0 405' || fail "POST /status: $(printf '%s' "$r" | status_line)"
printf '%s' "$r" | grep -q 'Allow: GET, HEAD' || fail "405 names the methods"
[ "$(request 'GET /events HTTP/1.0' | status_line)" = "HTTP/1.0 200 OK" ] || fail "GET /events"
request 'GET /events?limit=2 HTTP/1.0' | body | is 2 'len(d["events"])' "limit"
request 'GET /events?since=9999999999&limit=5 HTTP/1.0' | body | is 0 'len(d["events"])' "since"
request 'GET /events?foo=bar&limit=1 HTTP/1.0' | body | is 1 'len(d["events"])' "other keys are ignored"
request 'GET /events?limit=0099 HTTP/1.0' | body | is True 'len(d["events"]) > 2' "leading zeros are decimal"
request 'GET /events?limit=999999999 HTTP/1.0' | status_line | grep -q ' 200 ' || fail "a large limit is capped"
for bad in 'since=1;touch%20pwned' 'since=$(touch%20pwned)' 'limit=*' 'limit=`id`' 'since=' 'since=-1' 'limit=1e3' 'since=1234567890123'; do
	r=$(cd "$T" && request "GET /events?$bad HTTP/1.0")
	printf '%s' "$r" | status_line | grep -q '^HTTP/1.0 400' || fail "$bad: $(printf '%s' "$r" | status_line)"
done
[ ! -e "$T/pwned" ] || fail "a query ran a command"
request 'DELETE /events HTTP/1.0' | status_line | grep -q '^HTTP/1.0 405' || fail "DELETE /events"
request 'GET /status/x HTTP/1.0' | status_line | grep -q '^HTTP/1.0 404' || fail "GET /status/x"

echo "the identity lists the endpoints"
request 'GET /.well-known/pd-device HTTP/1.0' | body |
	is "{'status': '/status', 'events': '/events', 'actions': '/actions'}" 'd["endpoints"]' endpoints
printf 'BOARD_ACTIONS=\n' > "$T/etc/identity.env"
request 'GET /.well-known/pd-device HTTP/1.0' | body | is "{'status': '/status', 'events': '/events'} False" \
	'str(d["endpoints"]) + " " + str("actions" in d)' "endpoints without actions"

echo "ok"
