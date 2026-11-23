#!/usr/bin/env bash
# The first-boot check: one named step that says, item by item, whether a freshly booted image does what its unit
# settings must still let it do. It is the same on the emulator boot of an image and on a board: everything goes
# through two ways in, a root console that runs one command line, and the board's Orion address as a PC reaches it.
#
#   scripts/first-boot-check.sh --console 'CMD' --address ADDR --orion 'CMD' --atlas 'CMD' [--only ITEM[,ITEM]] [--out DIR]
#
#   --console CMD   run as `CMD <seconds> '<one shell line>'`; prints what the board printed. Emulator: Gaia's
#                   console socket through a small reader; board: the serial console.
#   --address ADDR  the board's Orion address as this PC reaches it (orion+tcp://host:port).
#   --orion CMD     Orion's own command-line client for this PC, built from the pinned Orion (crates/client/ctl),
#                   with whatever must stand before it to reach ADDR (the emulator: nsenter into the runner's
#                   network namespace). `--address ADDR --key <a key file>` is added here.
#   --atlas CMD     atlas-next, with the same prefix and its state (`--state DIR` is not added: give the environment).
#   --only          run these items only.
#   --out DIR       keep each item's raw output here (default: a new directory under ${TMPDIR:-/tmp}).
#
# Each item ends in one line: `first-boot: PASS <item>: ...`, `FAIL <item>: ...`, or `NOT RUN <item>: why`. An item
# is NOT RUN only for a reason written here, never because it failed. Exit 1 when any item failed, 0 otherwise;
# the last line counts all three.
#
# The items (Orion's six unit-setting checks, and the image's own):
#   units            no unit failed; the journal is on the data partition (board-journal); no leftover peer file
#   files-fact       the hello document says `files` (`orion node show`)
#   files-push       a file pushed into each read-write root (roots/app/, roots/calibrations/) arrives with its bytes
#   files-fetch      a file under the read-only root is fetched with its bytes (roots/logs/; a root's files are
#                    named roots/<root>/<path>, Orion 2026-11-23)
#   files-dirs       the read-write roots are root:appdata 2770
#   files-two-users  NOT RUN: a second, non-root member of appdata must write beside root; the image has no tool to
#                    become one (owed with pin move 15)
#   shell            NOT RUN: a shell over the session; neither client has the verb yet (Atlas's Terminal tab)
#   ssh              NOT RUN: an SSH session through the comm core; the same
#   action           an action is run through the comm core (the slots device's `list`)
#   topic            node logic publishes a topic that this PC reads
#   workload         NOT RUN: put a workload; the image runs no executor for one
#   time             this PC's time is offered and taken, the board asks for it, and says `clock synced`
#   files-pull       NOT RUN unless --pull-url URL --pull-size N --pull-sha256 HEX: a file pulled by the board
#   mdns             the comm core says it announces itself
#   ways-in          the comm core says which listeners it keeps to an interface
#   ways-in-peer     NOT RUN: nothing on the board says which interfaces the Orion listener is kept to (asked of
#                    Orion)
set -uo pipefail

console="" address="" orion="" atlas="" only="" out="" pull_url="" pull_size="" pull_sha=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --console) console=$2; shift 2 ;;
    --address) address=$2; shift 2 ;;
    --orion) orion=$2; shift 2 ;;
    --atlas) atlas=$2; shift 2 ;;
    --only) only=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    --pull-url) pull_url=$2; shift 2 ;;
    --pull-size) pull_size=$2; shift 2 ;;
    --pull-sha256) pull_sha=$2; shift 2 ;;
    *) echo "first-boot-check: unknown option $1" >&2; exit 2 ;;
  esac
done
for need in console address orion atlas; do
  [[ -n ${!need} ]] || { echo "first-boot-check: --$need is needed" >&2; exit 2; }
done
out=${out:-$(mktemp -d "${TMPDIR:-/tmp}/first-boot.XXXXXX")}
mkdir -p "$out"
key=$out/orion-ctl.key

passed=0 failed=0 skipped=0
pass() { echo "first-boot: PASS $1: $2"; passed=$((passed + 1)); }
fail() { echo "first-boot: FAIL $1: $2"; failed=$((failed + 1)); }
not_run() { echo "first-boot: NOT RUN $1: $2"; skipped=$((skipped + 1)); }
wanted() { [[ -z $only ]] || [[ ",$only," == *",$1,"* ]]; }
# What the board printed for one command line, without the console's own echo of it, colours or carriage returns.
board() {  # <seconds> <shell line>
  # shellcheck disable=SC2086
  $console "$1" "$2" 2>&1 | tr -d '\r' | sed 's/\x1b\[[0-9;]*[mK]//g'
}
ctl() {  # orion's client against the board
  # shellcheck disable=SC2086
  $orion --address "$address" --key "$key" --timeout 20000 "$@"
}
first() { head -n "${2:-1}" "$1" | tr '\n' ' ' | cut -c1-300; }

if wanted units; then
  board 60 'systemctl --no-pager --failed --no-legend --plain | cut -d" " -f1; echo "state=$(systemctl is-system-running)"; echo "journal=$(systemctl is-active board-journal.service)"; grep -c " /var/log/journal " /proc/self/mountinfo; ls /data/journal | head -n 3; [ -e /run/lemnos0/orion-peer.env ] && echo PEERFILE' > "$out/units.txt"
  why=""
  grep -q '\.service\|\.mount\|\.socket' <(grep -v '^state=\|^journal=' "$out/units.txt" | grep '\.[a-z]*$') && why="failed units: $(grep '\.\(service\|mount\|socket\|target\)$' "$out/units.txt" | tr '\n' ' ')"
  grep -q '^journal=active$' "$out/units.txt" || why="$why board-journal.service is not active ($(grep '^journal=' "$out/units.txt"));"
  grep -qx '1' "$out/units.txt" || why="$why /var/log/journal is not a mount;"
  grep -q PEERFILE "$out/units.txt" && why="$why /run/lemnos0/orion-peer.env exists;"
  if [[ -z $why ]]; then pass units "no failed unit, $(grep '^state=' "$out/units.txt"), the journal is bound onto /data/journal"; else fail units "$why"; fi
fi

if wanted files-fact; then
  ctl node show > "$out/node-show.txt" 2>&1
  if grep -q '^files ' "$out/node-show.txt"; then pass files-fact "$(grep '^files ' "$out/node-show.txt")"
  else fail files-fact "\`orion node show\` has no \`files\` line (facts: $(sed -n 's/^\([a-z_.-]*\) .*/\1/p' "$out/node-show.txt" | tr '\n' ' '))"; fi
fi

stamp="first-boot-check $(date -u +%Y%m%dT%H%M%SZ) $$"
if wanted files-push; then
  echo "$stamp" > "$out/pushed.txt"
  why="" told=""
  for root in app calibrations; do
    if ctl file push "roots/$root/first-boot-check.txt" "$out/pushed.txt" > "$out/push-$root.txt" 2>&1; then
      got=$(board 30 "cat /data/$root/first-boot-check.txt" | grep -F "first-boot-check" | tail -n 1)
      [[ $got == "$stamp" ]] && told="$told $root/" || why="$why $root/: pushed, and /data/$root/first-boot-check.txt holds '$got';"
    else
      why="$why $root/: $(first "$out/push-$root.txt" 3);"
    fi
  done
  if [[ -z $why ]]; then pass files-push "arrived with its bytes in$told"; else fail files-push "$why"; fi
fi

if wanted files-fetch; then
  board 30 "echo '$stamp' > /var/log/first-boot-check.txt; sync" > /dev/null
  rm -f "$out/fetched.txt"
  if ctl file fetch roots/logs/first-boot-check.txt "$out/fetched.txt" --give-up 30000 > "$out/fetch.txt" 2>&1 && [[ $(cat "$out/fetched.txt" 2>/dev/null) == "$stamp" ]]; then
    pass files-fetch "roots/logs/first-boot-check.txt came back with its bytes"
  else
    fail files-fetch "$(first "$out/fetch.txt" 3)"
  fi
fi

if wanted files-dirs; then
  board 30 'ls -ldn /data/app /data/calibrations; grep "^appdata:" /etc/group' > "$out/dirs.txt"
  gid=$(sed -n 's/^appdata:[^:]*:\([0-9]*\):.*/\1/p' "$out/dirs.txt")
  bad=$(grep '^d' "$out/dirs.txt" | awk -v g="$gid" '$1 != "drwxrws---" || $3 != 0 || $4 != g { print $NF " is " $1 " " $3 ":" $4 }' | tr '\n' ';')
  if [[ -n $gid && -z $bad && $(grep -c '^d' "$out/dirs.txt") -eq 2 ]]; then pass files-dirs "/data/app and /data/calibrations are 0:$gid (root:appdata) drwxrws---"
  else fail files-dirs "${bad:-not both listed: $(first "$out/dirs.txt" 3)}"; fi
fi

wanted files-two-users && not_run files-two-users "the image has no tool to run a command as a second, non-root member of appdata (owed with pin move 15)"
wanted shell && not_run shell "neither atlas-next nor Orion's command-line client opens a shell over the session yet (Atlas's Terminal tab)"
wanted ssh && not_run ssh "no client here opens an SSH session through the comm core yet"

if wanted action; then
  # shellcheck disable=SC2086
  if $atlas slots "$address" list > "$out/action.txt" 2>&1; then pass action "the slots device's list ran through the comm core: $(first "$out/action.txt" 2)"
  else fail action "$(first "$out/action.txt" 3)"; fi
fi

if wanted topic; then
  ctl resource list > "$out/resources.txt" 2>&1
  # Each Lemnos device publishes `<its resource name>.status` (the bridge is the node logic): the slots device's
  # when the board has one, else the first device's.
  device=$(awk '$2 == "lemnos.device" { print $1 }' "$out/resources.txt" | grep -x 'lemnos\.slots' || awk '$2 == "lemnos.device" { print $1; exit }' "$out/resources.txt")
  topic=${device:+$device.status}
  if [[ -z $topic ]]; then fail topic "no Lemnos device among the board's resources: $(first "$out/resources.txt" 3)"
  # A sample that arrives, with a value: `topic read` answers before the first one and says "no value yet".
  elif ctl topic watch "$topic" --count 1 --for 20000 > "$out/topic.txt" 2>&1 && grep -q ' = ' "$out/topic.txt"; then pass topic "$topic gave a sample: $(first "$out/topic.txt")"
  else fail topic "$topic gave no sample with a value in 20 s: $(first "$out/topic.txt" 3)"; fi
fi

wanted workload && not_run workload "the image runs no executor that takes a workload"

if wanted time; then
  # shellcheck disable=SC2086
  if $atlas time "$address" 150 > "$out/time.txt" 2>&1; then pass time "$(grep -E '^(before|after)|takes this computer' "$out/time.txt" | tr '\n' ' ' | cut -c1-400)"
  else fail time "$(tail -n 3 "$out/time.txt" | tr '\n' ' ' | cut -c1-400)"; fi
fi

if wanted files-pull; then
  if [[ -z $pull_url || -z $pull_size || -z $pull_sha ]]; then not_run files-pull "no --pull-url, --pull-size and --pull-sha256: nothing this board can reach was named"
  elif ctl file pull app/first-boot-pulled.bin "$pull_url" --size "$pull_size" --sha256 "$pull_sha" --for 120000 > "$out/pull.txt" 2>&1; then pass files-pull "$(first "$out/pull.txt")"
  else fail files-pull "$(first "$out/pull.txt" 3)"; fi
fi

if wanted mdns || wanted ways-in; then
  board 60 'journalctl -b --no-pager -o cat -u "orion*" | grep -i -E "mdns|announc|listen|interface|bound|kept to|open on|serving" | head -n 40' > "$out/comm-journal.txt"
fi
if wanted mdns; then
  if grep -q -i 'mdns: announcing ' "$out/comm-journal.txt"; then pass mdns "$(grep -i -m1 'mdns: announcing ' "$out/comm-journal.txt" | cut -c1-300)"
  else fail mdns "the comm core's journal has no line saying it announces itself: $(first "$out/comm-journal.txt" 3)"; fi
fi
if wanted ways-in; then
  # What the comm core says of a listener kept to an interface: `<service>: <interface>: open on <address>`, and
  # `serving the clock over SNTP on <interface>, at <address>`.
  kept=': [a-z0-9]+: open on |serving the clock over SNTP on [a-z0-9]+,'
  if grep -q -E "$kept" "$out/comm-journal.txt"; then pass ways-in "$(grep -E "$kept" "$out/comm-journal.txt" | sed 's/^INFO *//' | tr '\n' ';' | cut -c1-400)"
  else fail ways-in "the comm core's journal names no listener kept to an interface: $(first "$out/comm-journal.txt" 3)"; fi
fi
wanted ways-in-peer && not_run ways-in-peer "the comm core does not say which interfaces its Orion listener is kept to, and the image has no tool that lists listeners: asked of Orion"

echo "first-boot: $passed passed, $failed failed, $skipped not run (raw output in $out)"
[[ $failed -eq 0 ]]
