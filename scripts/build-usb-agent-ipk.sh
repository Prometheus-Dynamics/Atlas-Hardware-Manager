#!/usr/bin/env bash
# Builds the SystemCore package of atlas-usb-agent: a static aarch64 binary
# and its systemd unit, as an opkg .ipk Atlas installs on the controller
# (one click, as Phoenix Tuner installs its server on a roboRIO). Gaia can
# build the same from these inputs.
#
#   scripts/build-usb-agent-ipk.sh [out dir]    (default: the target dir)
#
# Prints the .ipk's path. Needs the aarch64-unknown-linux-musl target;
# links with rust-lld, so no cross C toolchain.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
target_dir=$(cd "$root" && cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
out=${1:-$target_dir/usb-agent-ipk}
version=$("$root/scripts/version.sh")
# opkg orders `~dev` below the release, as dpkg does.
package_version=${version/-dev/\~dev}

CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld \
  cargo build -q --manifest-path "$root/Cargo.toml" --profile device \
  --target aarch64-unknown-linux-musl -p atlas-usb-agent
binary=$target_dir/aarch64-unknown-linux-musl/device/atlas-usb-agent

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/data/usr/local/bin" "$work/data/etc/systemd/system" "$work/control"
install -m 0755 "$binary" "$work/data/usr/local/bin/atlas-usb-agent"
install -m 0644 "$root/crates/atlas-usb-agent/package/atlas-usb-agent.service" \
  "$work/data/etc/systemd/system/atlas-usb-agent.service"
cat > "$work/control/control" <<CONTROL
Package: atlas-usb-agent
Version: $package_version
Architecture: aarch64
Maintainer: Prometheus Dynamics
Section: utils
Priority: optional
Installed-Size: $(du -sk "$work/data" | cut -f1)
Description: Atlas's agent on this robot controller: lists boards in USB boot plugged in here and flashes them for Atlas.
CONTROL
cat > "$work/control/postinst" <<'SCRIPT'
#!/bin/sh
systemctl daemon-reload
systemctl enable atlas-usb-agent.service
systemctl restart atlas-usb-agent.service
exit 0
SCRIPT
cat > "$work/control/prerm" <<'SCRIPT'
#!/bin/sh
systemctl disable --now atlas-usb-agent.service 2>/dev/null
exit 0
SCRIPT
chmod 0755 "$work/control/postinst" "$work/control/prerm"
echo "2.0" > "$work/debian-binary"
# The package's own paths only: a `./` entry makes opkg try to remove `/`
# as an obsolete file on upgrade.
tar --owner=0 --group=0 --numeric-owner -C "$work/control" -czf "$work/control.tar.gz" \
  ./control ./postinst ./prerm
tar --owner=0 --group=0 --numeric-owner -C "$work/data" -czf "$work/data.tar.gz" ./usr ./etc
mkdir -p "$out"
ipk=$out/atlas-usb-agent_${package_version}_aarch64.ipk
rm -f "$ipk"
(cd "$work" && ar rc "$ipk" debian-binary control.tar.gz data.tar.gz)
echo "$ipk"
