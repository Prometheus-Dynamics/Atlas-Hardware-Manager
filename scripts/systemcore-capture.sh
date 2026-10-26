#!/bin/bash
# Read-only capture of what a SystemCore serves without a login, so Atlas's
# SystemCore driver can be written against the real thing (its endpoints
# aren't documented). GETs only: no login, no POST, nothing changed on the
# unit. Writes ./systemcore-capture/ (share the folder, or a tar of it).
#   bash scripts/systemcore-capture.sh [host]   host: robot.local (default),
#                                             172.27.0.1 over USB, 10.TE.AM.2
set -u
host=${1:-robot.local}
out=systemcore-capture
mkdir -p "$out"
get() { # get <path> <file>: headers and body, 5 s at most
	curl -s -m 5 -D "$out/$2.headers" -o "$out/$2" "http://$host$1" || echo "no answer: $1" >> "$out/missing.txt"
}
get / root.html
get /configure configure.html
# Every script and stylesheet the pages load, then every API-like path in them.
for page in "$out/root.html" "$out/configure.html"; do
	[ -s "$page" ] || continue
	grep -oE '(src|href)="[^"]+\.(js|css)[^"]*"' "$page" | sed -E 's/^(src|href)="//; s/"$//'
done | sort -u | while IFS= read -r asset; do
	case "$asset" in http*) continue ;; esac
	name=$(printf '%s' "$asset" | tr '/?&=' '____')
	get "/${asset#/}" "asset$name"
done
cat "$out"/root.html "$out"/configure.html "$out"/asset* 2>/dev/null |
	grep -oE '["'"'"'`](/(api|v[0-9]|rest|cgi-bin|ws|status|system|info|packages|package|ota|team|config|release|version)[^"'"'"'` ]*)' |
	tr -d '"'"'"'`' | sort -u > "$out/paths.txt"
# The read-only ones that look like data, once each.
grep -vE 'install|remove|delete|upload|update|set|reboot|restart|flash|ota|post' "$out/paths.txt" | head -40 |
	while IFS= read -r path; do
		get "$path" "path$(printf '%s' "$path" | tr '/?&=' '____')"
	done
# Which of the usual FRC ports answer (connect only).
for port in 22 80 443 1110 1115 1130 1140 1150 1160 1735 1740 5800 5801 5810 5811 8080; do
	if timeout 2 bash -c "exec 3<>/dev/tcp/$host/$port" 2>/dev/null; then echo "$port open"; fi
done > "$out/ports.txt" 2>/dev/null
echo "wrote $out/ ($(ls "$out" | wc -l) files); paths found: $(wc -l < "$out/paths.txt")"
