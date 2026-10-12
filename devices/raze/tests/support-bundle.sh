#!/bin/sh
# Off-device test of support-bundle: what goes in, the journals of the last
# boots, pstore, and that secrets come out. The board's own scripts and its
# tools (journalctl, dmesg, ip, nft, nmcli, systemctl) are faked.
# Run: sh devices/raze/tests/support-bundle.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
real=$here/../gaia/assets/rootfs/usr/lib/board
T=$(mktemp -d "${TMPDIR:-/tmp}/board-support-test.XXXXXX")
trap 'rm -rf "$T"' EXIT

fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# The package's lib.sh and env files, with identity, status and usb-route
# replaced by fakes that print secrets for the redaction to find.
lib=$T/lib
mkdir -p "$lib" "$T/dt" "$T/etc" "$T/data/pstore/b1" "$T/data/pstore/b2" "$T/run" "$T/bin" "$T/pstore" "$T/out"
cp "$real/lib.sh" "$real"/*.env "$lib/"
cp "$real/support-bundle" "$lib/"
printf '10000000a317bcbe\000' > "$T/dt/serial-number"
cat > "$lib/identity" <<'EOF'
#!/bin/sh
echo '{"contract":1,"model":"raze","orion":{"token":"tok-123 456"}}'
EOF
cat > "$lib/status" <<'EOF'
#!/bin/sh
echo '{"version":1,"update":{"url":"http://10.0.0.5:7700/images/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/raze.img"}}'
EOF
cat > "$lib/usb-route" <<'EOF'
#!/bin/sh
echo '{"enabled":true,"active":true,"subnets":["10.53.38.0/24"]}'
EOF
chmod +x "$lib/identity" "$lib/status" "$lib/usb-route"
export BOARD_LIB_DIR=$lib BOARD_DT_DIR=$T/dt BOARD_ETC_DIR=$T/etc BOARD_DATA_DIR=$T/data BOARD_RUN_DIR=$T/run
export BOARD_PSTORE_DIR=$T/pstore BOARD_SUPPORT_STAMP=20261012-120000 BOARD_SUPPORT_LINES=5

# journalctl: three boots, the oldest one long; -b -3 doesn't exist.
cat > "$T/bin/journalctl" <<'EOF'
#!/bin/sh
case "$*" in
*--list-boots*) printf -- ' -2 aaa Mon\n -1 bbb Tue\n  0 ccc Wed\n' ;;
*"-b 0 "*) echo "boot ccc: Authorization: Bearer abc.def-ghi" ; echo "wpa: psk=hunter2 ok" ;;
*"-b -1 "*) echo "boot bbb: reached shutdown, then nothing"; echo "sent Bearer qq.rr" ;;
*"-b -2 "*) i=0; while [ $i -lt 12 ]; do echo "boot aaa line $i"; i=$((i + 1)); done ;;
*) echo "No such boot" >&2; exit 1 ;;
esac
EOF
cat > "$T/bin/dmesg" <<'EOF'
#!/bin/sh
echo "[    0.000000] Booting Linux"
echo "-----BEGIN OPENSSH PRIVATE KEY-----"
echo "b3BlbnNzaC1rZXktdjEAAAAA"
echo "-----END OPENSSH PRIVATE KEY-----"
EOF
cat > "$T/bin/ip" <<'EOF'
#!/bin/sh
echo "ip $*: 2: eth0 inet 10.53.38.11/24"
EOF
cat > "$T/bin/nmcli" <<'EOF'
#!/bin/sh
echo "eth0:ethernet:connected:robot"
echo "url https://team:s3cret@example.org/x"
EOF
for tool in nft systemctl networkctl; do
	printf '#!/bin/sh\necho "%s $*"\n' "$tool" > "$T/bin/$tool"
done
chmod +x "$T/bin"/*
export PATH="$T/bin:$PATH"

echo "1" > "$T/run/update.json"
echo '{"overall":"ok"}' > "$T/run/selftest.json"
echo '{"t":1,"kind":"boot","msg":"ORION_TOKEN=abcd"}' > "$T/data/events.jsonl"
echo "panic in b1" > "$T/data/pstore/b1/dmesg-ramoops-0"
echo "console of b2" > "$T/data/pstore/b2/console-ramoops-0"
echo "still in RAM" > "$T/pstore/console-ramoops-0"
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl atlas" > "$T/data/events.jsonl.extra"
cat "$T/data/events.jsonl.extra" >> "$T/data/events.jsonl"

echo "writes a .tar.gz under /run/board/support and prints its path"
path=$(sh "$lib/support-bundle")
[ "$path" = "$T/run/support/raze-10000000a317bcbe-20261012-120000.tar.gz" ] || fail "path: $path"
tar -C "$T/out" -xzf "$path"
B=$T/out/raze-10000000a317bcbe-20261012-120000
for f in README.txt journal-boots.txt journal-boot0.txt journal-boot-1.txt journal-boot-2.txt dmesg.txt \
	versions.txt identity.json status.json update.json selftest.json hardware.json events.jsonl network.txt system.txt; do
	[ -f "$B/$f" ] || fail "missing $f: $(ls "$B")"
done

echo "the last three boots' journals, each capped, the cap said"
grep -q "reached shutdown" "$B/journal-boot-1.txt" || fail "boot -1: $(cat "$B/journal-boot-1.txt")"
grep -q "12 lines, the last 5" "$B/journal-boot-2.txt" || fail "cap note: $(head -n 1 "$B/journal-boot-2.txt")"
grep -q "line 11" "$B/journal-boot-2.txt" || fail "the newest lines should stay"
! grep -q "line 6$" "$B/journal-boot-2.txt" || fail "older lines should go"
[ ! -e "$B/journal-boot-3.txt" ] || fail "only three boots"

echo "pstore: kept records and what is still in RAM"
[ -f "$B/pstore/b1/dmesg-ramoops-0" ] && [ -f "$B/pstore/b2/console-ramoops-0" ] && [ -f "$B/pstore/live/console-ramoops-0" ] ||
	fail "pstore: $(find "$B/pstore")"
grep -q "/sys/fs/pstore\|still in RAM" "$B/pstore/live/console-ramoops-0" || fail "live pstore"

echo "secrets are out"
all=$(cat "$B"/*.txt "$B"/*.json "$B"/*.jsonl)
for secret in hunter2 qq.rr abc.def-ghi "tok-123" b3BlbnNzaC1rZXktdjEAAAAA s3cret abcd \
	AAAAC3NzaC1lZDI1NTE5 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef; do
	case "$all" in *"$secret"*) fail "left in: $secret" ;; esac
done
grep -q 'sent Bearer <redacted>' "$B/journal-boot-1.txt" || fail "bearer: $(cat "$B/journal-boot-1.txt")"
grep -q 'Authorization: <redacted>' "$B/journal-boot0.txt" || fail "authorization: $(cat "$B/journal-boot0.txt")"
grep -q 'psk=<redacted> ok' "$B/journal-boot0.txt" || fail "psk: $(cat "$B/journal-boot0.txt")"
grep -q '"token":"<redacted>"' "$B/identity.json" || fail "json: $(cat "$B/identity.json")"
grep -q 'BEGIN OPENSSH PRIVATE KEY' "$B/dmesg.txt" || fail "the PEM markers stay"
grep -q 'ssh-ed25519 <redacted> atlas' "$B/events.jsonl" || fail "ssh key: $(cat "$B/events.jsonl")"
grep -q '/images/<redacted>/raze.img' "$B/status.json" || fail "image token: $(cat "$B/status.json")"
grep -q 'https://<redacted>@example.org' "$B/network.txt" || fail "url creds"
grep -q 'Booting Linux' "$B/dmesg.txt" || fail "dmesg lost"
grep -q 'psk passwd\|passwd passphrase' "$B/README.txt" || fail "README lists the rules: $(cat "$B/README.txt")"

echo "a missing source says so"
grep -q "not there" "$B/hardware.json" || fail "hardware.json: $(cat "$B/hardware.json")"

echo "--stdout gives the same bundle"
sh "$lib/support-bundle" --stdout | tar -tzf - | grep -q "raze-10000000a317bcbe-20261012-120000/README.txt" || fail "stdout"

echo "only the newest two are kept"
sleep 1
BOARD_SUPPORT_STAMP=20261012-120001 sh "$lib/support-bundle" > /dev/null
sleep 1
BOARD_SUPPORT_STAMP=20261012-120002 sh "$lib/support-bundle" > /dev/null
[ "$(ls "$T/run/support" | wc -l | tr -d ' ')" = 2 ] || fail "kept: $(ls "$T/run/support")"
[ ! -e "$path" ] || fail "the oldest should be gone"

echo "support-bundle ok"
