#!/usr/bin/env bash
# Every shell script a product's fragments name (`BR2_ROOTFS_*SCRIPT="@here@/<file>"`, os/raze.fragment and the other
# products' os fragments) is checked two ways before any real build:
#   1. syntax: `sh -n` (or `bash -n` when the shebang says bash);
#   2. a dry run against a stand-in tree, where one exists. Where none exists the run prints, visibly,
#        dry run: NOT CHECKED (no stand-in tree for <script>)
#      and it never passes silently.
# The stand-in for raze-base's post-build.sh is scripts/fake-raze-root.sh (a labelled fake tree, not a built root).
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

# The dry run of one script (its path in the repo). Returns 1 on a failure; prints NOT CHECKED when there is no stand-in.
dry_run() {
  local f=$1 rel=${1#"$repo"/} scratch
  case $rel in
    devices/products/raze-base/os/post-build.sh)
      scratch=$(mktemp -d "${TMPDIR:-/tmp}/dry-XXXXXX")
      trap 'rm -rf "$scratch"' RETURN
      "$repo/scripts/fake-raze-root.sh" "$scratch/root" >/dev/null 2>&1 || die "no fake root for the dry run"
      cp -a "$scratch/root" "$scratch/copy"
      # The tree the HOSTNAMED-off build makes: hostnamed's units, link and binaries are not there (as post-build's own
      # self-test builds it). The raw fixture keeps them, to prove the refusal in that self-test.
      d=$scratch/copy
      rm -f "$d/usr/lib/systemd/system/systemd-hostnamed.service" "$d/usr/lib/systemd/system/systemd-hostnamed.socket" \
        "$d/usr/lib/systemd/system/sockets.target.wants/systemd-hostnamed.socket" "$d/usr/lib/systemd/systemd-hostnamed" \
        "$d/usr/bin/hostnamectl"
      if sh "$f" "$scratch/copy" >"$scratch/run.log" 2>&1; then
        echo "check-product-scripts: ok dry run: $rel on the fake tree (the stand-in; not a built root)"
      else
        echo "check-product-scripts: FAIL dry run: $rel on the fake tree: $(tail -1 "$scratch/run.log")" >&2
        return 1
      fi
      ;;
    devices/products/raze-base/os/post-fakeroot.sh)
      scratch=$(mktemp -d "${TMPDIR:-/tmp}/dry-XXXXXX")
      trap 'rm -rf "$scratch"' RETURN
      "$repo/scripts/fake-raze-root.sh" "$scratch/root" >/dev/null 2>&1 || die "no fake root for the dry run"
      # 1. The tree as the build leaves it before the fakeroot stage: post-build has run (the HOSTNAMED-off tree, then the
      #   post-build script), so the helper is there; the post-fakeroot script removes it and passes.
      cp -a "$scratch/root" "$scratch/ok"
      d=$scratch/ok
      rm -f "$d/usr/lib/systemd/system/systemd-hostnamed.service" "$d/usr/lib/systemd/system/systemd-hostnamed.socket" \
        "$d/usr/lib/systemd/system/sockets.target.wants/systemd-hostnamed.socket" "$d/usr/lib/systemd/systemd-hostnamed" \
        "$d/usr/bin/hostnamectl"
      sh "$repo/devices/products/raze-base/os/post-build.sh" "$d" >"$scratch/pb.log" 2>&1 \
        || { echo "check-product-scripts: FAIL dry run: post-build.sh before $rel: $(tail -1 "$scratch/pb.log")" >&2; return 1; }
      cp -a "$d" "$scratch/pre"
      sh "$f" "$d" >"$scratch/ok.log" 2>&1 || { echo "check-product-scripts: FAIL dry run: $rel: $(tail -1 "$scratch/ok.log")" >&2; return 1; }
      [ ! -e "$d/usr/libexec/dbus-daemon-launch-helper" ] || { echo "check-product-scripts: FAIL dry run: $rel left the helper" >&2; return 1; }
      # 2. The helper absent: refused, and the message says so (Buildroot's table changed).
      cp -a "$scratch/pre" "$scratch/absent"; rm -f "$scratch/absent/usr/libexec/dbus-daemon-launch-helper"
      if sh "$f" "$scratch/absent" >"$scratch/absent.log" 2>&1; then
        echo "check-product-scripts: FAIL dry run: $rel accepted a tree with no helper to remove" >&2; return 1
      fi
      grep -q 'absent before the removal' "$scratch/absent.log" || { echo "check-product-scripts: FAIL dry run: $rel refused the absent helper without saying why" >&2; return 1; }
      # 3. Another dbus path survives: refused, and it is named.
      cp -a "$scratch/pre" "$scratch/left"; echo "FAKE leftover" > "$scratch/left/usr/bin/dbus-send"
      if sh "$f" "$scratch/left" >"$scratch/left.log" 2>&1; then
        echo "check-product-scripts: FAIL dry run: $rel accepted a tree with a dbus path left" >&2; return 1
      fi
      grep -q 'usr/bin/dbus-send' "$scratch/left.log" || { echo "check-product-scripts: FAIL dry run: $rel did not name the leftover dbus path" >&2; return 1; }
      echo "check-product-scripts: ok dry run: $rel (fake tree: removes the helper; refused when the helper is absent; refused when a dbus path is left)"
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
  named_scripts | grep -q 'raze-base/os/post-build.sh' || die "self-test: the fragment scan misses raze-base's post-build.sh"
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
