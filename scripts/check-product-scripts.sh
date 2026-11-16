#!/usr/bin/env bash
# Every shell script a product's fragments name (`BR2_ROOTFS_*SCRIPT="@here@/<file>"`, os/raze.fragment and the other
# products' os fragments) is checked two ways before any real build:
#   1. syntax: `sh -n` (or `bash -n` when the shebang says bash);
#   2. a dry run against a stand-in tree, where one exists. Where none exists the run prints, visibly,
#        dry run: NOT CHECKED (no stand-in tree for <script>)
#      and it never passes silently.
# The stand-in is scripts/fake-raze-root.sh (a labelled fake tree, not a built root). The raze-base dry runs follow
# Buildroot's order (see os/post-fakeroot.sh): post-build.sh; then the helper must exist for makedevs; then the presets'
# result (simulated); then post-fakeroot.sh. post-fakeroot.sh is STAGED (not yet named by a fragment) and is checked anyway.
#
#   scripts/check-product-scripts.sh               the device check (scripts/gate.sh runs it)
#   scripts/check-product-scripts.sh --self-test    a script with a syntax error is refused; a clean one passes
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
die() { echo "check-product-scripts: $*" >&2; exit 1; }

# The syntax check of one script: prints why, returns 1 when it does not parse.
syntax_ok() {
  local f=$1 first interp
  first=$(head -1 "$f" 2>/dev/null || true)
  case $first in
    *bash*) interp=bash ;;
    *) interp=sh ;;
  esac
  if ! "$interp" -n "$f" 2>"$f.syntax.err"; then
    echo "check-product-scripts: FAIL syntax ($interp -n) $f: $(head -3 "$f.syntax.err" | tr '\n' ' ')" >&2
    rm -f "$f.syntax.err"; return 1
  fi
  rm -f "$f.syntax.err"
  echo "check-product-scripts: ok syntax ($interp -n) $f"
}

# A stand-in tree for the raze-base dry runs: the fake root, with the HOSTNAMED-off removal the build makes.
raze_tree() {  # raze_tree DIR: makes DIR as the tree before the post-build script runs
  local d=$1
  "$repo/scripts/fake-raze-root.sh" "$d" >/dev/null 2>&1 || die "no fake root for the dry run"
  rm -f "$d/usr/lib/systemd/system/systemd-hostnamed.service" "$d/usr/lib/systemd/system/systemd-hostnamed.socket" \
    "$d/usr/lib/systemd/system/sockets.target.wants/systemd-hostnamed.socket" "$d/usr/lib/systemd/systemd-hostnamed" \
    "$d/usr/bin/hostnamectl"
}

# The dry run of one script (its path in the repo). Returns 1 on a failure; prints NOT CHECKED when there is no stand-in.
dry_run() {
  local f=$1 rel=${1#"$repo"/} scratch pb pf
  pb=$repo/devices/products/raze-base/os/post-build.sh
  pf=$repo/devices/products/raze-base/os/post-fakeroot.sh
  case $rel in
    devices/products/raze-base/os/post-build.sh)
      scratch=$(mktemp -d "${TMPDIR:-/tmp}/dry-XXXXXX")
      trap 'rm -rf "$scratch"' RETURN
      raze_tree "$scratch/t"
      if sh "$f" "$scratch/t" >"$scratch/run.log" 2>&1 \
          && [ "$(readlink "$scratch/t/etc/systemd/system/systemd-networkd-wait-online.service")" = /dev/null ] \
          && [ -e "$scratch/t/usr/libexec/dbus-daemon-launch-helper" ]; then
        echo "check-product-scripts: ok dry run: $rel (fake tree: masks wait-online; leaves the dbus paths for post-fakeroot)"
      else
        echo "check-product-scripts: FAIL dry run: $rel on the fake tree: $(tail -1 "$scratch/run.log")" >&2
        return 1
      fi
      ;;
    devices/products/raze-base/os/post-fakeroot.sh)
      scratch=$(mktemp -d "${TMPDIR:-/tmp}/dry-XXXXXX")
      trap 'rm -rf "$scratch"' RETURN
      # The build order, on the fake tree: post-build (the mask) ...
      raze_tree "$scratch/base"
      sh "$pb" "$scratch/base" >"$scratch/pb.log" 2>&1 || { echo "check-product-scripts: FAIL dry run: post-build before $rel: $(tail -1 "$scratch/pb.log")" >&2; return 1; }
      # ... then makedevs: the permission table names the helper, so the helper must be there (fs/common.mk:180).
      [ -e "$scratch/base/usr/libexec/dbus-daemon-launch-helper" ] \
        || { echo "check-product-scripts: FAIL dry run: makedevs would fail: the helper is absent before makedevs" >&2; return 1; }
      # ... then preset-all (fs/common.mk:181): variant A disables the dbus units (their links go), variant B leaves them.
      cp -a "$scratch/base" "$scratch/A"
      rm -f "$scratch/A/usr/lib/systemd/system/multi-user.target.wants/dbus.service" \
            "$scratch/A/usr/lib/systemd/system/sockets.target.wants/dbus.socket"
      cp -a "$scratch/base" "$scratch/B"
      for v in A B; do
        sh "$pf" "$scratch/$v" >"$scratch/$v.log" 2>&1 \
          || { echo "check-product-scripts: FAIL dry run: $rel ($v, after simulated preset-all): $(tail -1 "$scratch/$v.log")" >&2; return 1; }
        [ ! -e "$scratch/$v/usr/libexec/dbus-daemon-launch-helper" ] \
          || { echo "check-product-scripts: FAIL dry run: $rel ($v) left the helper" >&2; return 1; }
      done
      # Refusals: the helper absent when post-fakeroot runs; the mask missing; a dangling wants link to a dbus unit off the list.
      cp -a "$scratch/base" "$scratch/absent"; rm -f "$scratch/absent/usr/libexec/dbus-daemon-launch-helper"
      if sh "$pf" "$scratch/absent" >"$scratch/absent.log" 2>&1; then
        echo "check-product-scripts: FAIL dry run: $rel accepted a tree with no helper" >&2; return 1
      fi
      grep -q 'usr/libexec/dbus-daemon-launch-helper' "$scratch/absent.log" \
        || { echo "check-product-scripts: FAIL dry run: $rel refused the absent helper without naming it" >&2; return 1; }
      cp -a "$scratch/base" "$scratch/nomask"; rm -f "$scratch/nomask/etc/systemd/system/systemd-networkd-wait-online.service"
      if sh "$pf" "$scratch/nomask" >"$scratch/nomask.log" 2>&1; then
        echo "check-product-scripts: FAIL dry run: $rel accepted a tree with no wait-online mask" >&2; return 1
      fi
      cp -a "$scratch/base" "$scratch/extra"
      printf '[Unit]\nDescription=fake\n[Service]\nExecStart=/bin/false\n' > "$scratch/extra/usr/lib/systemd/system/dbus-fake-extra.service"
      ln -s ../dbus-fake-extra.service "$scratch/extra/usr/lib/systemd/system/multi-user.target.wants/dbus-fake-extra.service"
      if sh "$pf" "$scratch/extra" >"$scratch/extra.log" 2>&1; then
        echo "check-product-scripts: FAIL dry run: $rel accepted a dangling wants link to a dbus unit off the list" >&2; return 1
      fi
      grep -q 'dbus-fake-extra' "$scratch/extra.log" \
        || { echo "check-product-scripts: FAIL dry run: $rel did not name the dangling link" >&2; return 1; }
      echo "check-product-scripts: ok dry run: $rel (fake tree, Buildroot order: post-build, makedevs check, preset result A and B, post-fakeroot; refused: helper absent, mask missing, dangling dbus link)"
      ;;
    *)
      echo "check-product-scripts: dry run: NOT CHECKED (no stand-in tree for $rel)"
      ;;
  esac
}

# The scripts the fragments name: @here@/<file> values of the BR2_*SCRIPT options, resolved to their files.
named_scripts() {
  local frag
  while IFS= read -r frag; do
    local dir; dir=$(dirname "$frag")
    sed -n 's/^BR2_[A-Z_]*SCRIPT="@here@\/\([^"]*\)"/\1/p' "$frag" | while IFS= read -r name; do
      echo "$dir/$name"
    done
  done < <(find "$repo/devices/products" -name '*.fragment' | sort)
  # STAGED: not yet named by a fragment (wired in the commit that pins Gaia's fixed rev). Checked the same way.
  [ -f "$repo/devices/products/raze-base/os/post-fakeroot.sh" ] && echo "$repo/devices/products/raze-base/os/post-fakeroot.sh"
  return 0
}

if [ "${1:-}" = --self-test ]; then
  t=$(mktemp -d "${TMPDIR:-/tmp}/cps-XXXXXX")
  trap 'rm -rf "$t"' EXIT
  printf '#!/bin/sh\nif then\n' > "$t/bad.sh"
  printf '#!/bin/sh\nexit 0\n' > "$t/good.sh"
  syntax_ok "$t/bad.sh" >/dev/null 2>&1 && die "self-test: a script with a syntax error was accepted"
  syntax_ok "$t/good.sh" >/dev/null || die "self-test: a clean script was refused"
  # The fragment scan must find raze-base's post-build script (the one this rule exists for).
  named_scripts > "$t/names"
  grep -q 'raze-base/os/post-build.sh' "$t/names" || die "self-test: the fragment scan misses raze-base's post-build.sh"
  echo "check-product-scripts: self-test passed (a syntax error is refused; a clean script passes; the fragment scan finds post-build.sh)"
  exit 0
fi

fail=0
checked=0
while IFS= read -r f; do
  [ -f "$f" ] || die "a fragment names a file that is not there: $f"
  syntax_ok "$f" || fail=1
  checked=$((checked + 1))
  dry_run "$f" || fail=1
done < <(named_scripts | sort -u)
[ "$checked" -gt 0 ] || die "no fragment-named scripts found (the scan is broken, or the products have none)"
[ "$fail" -eq 0 ] || die "FAILED ($checked script(s))"
echo "check-product-scripts: passed ($checked script(s): syntax checked; dry runs as printed above)"
