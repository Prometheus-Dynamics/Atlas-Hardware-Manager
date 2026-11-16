#!/usr/bin/env bash
# The built-root rule ("accounts and enablement"), for a built root (not the fake tree). For each built root:
#   A. the set of ENABLED units (names of the links in any *.wants / *.requires directory of the tree) EQUALS the product's
#      stated set (--expect FILE: one unit per line, the product's preset/intent list). No more, no less.
#   B. every User= and Group= of every enabled unit exists in the tree's etc/passwd and etc/group, and so does the orion
#      account when orion-comm.service is enabled (its unit's User= is the account it runs as).
# With no built tree the check prints, visibly, "built-root check: NOT CHECKED (no built tree given)" and passes nothing.
#
#   scripts/check-built-root.sh --built NAME=DIR [--expect FILE] [--built ...]   a built root (or ATLAS_BUILT_TREES="NAME=DIR ...")
#   scripts/check-built-root.sh --self-test                                       fake built trees: refusals and a pass
set -euo pipefail

die() { echo "check-built-root: $*" >&2; exit 1; }

# enabled_units DIR: the unit names linked into any .wants/.requires directory (the files a preset-all run makes).
enabled_units() {
  local d=$1
  find "$d/etc/systemd/system" "$d/usr/lib/systemd/system" -type d \( -name '*.wants' -o -name '*.requires' \) 2>/dev/null \
    | while IFS= read -r w; do ls -1A "$w"; done | sort -u
}

# unit_file DIR UNIT: the unit file in the tree (etc first, as systemd reads it), or nothing.
unit_file() {
  for dir in etc/systemd/system usr/lib/systemd/system; do
    [ -f "$1/$dir/$2" ] && { echo "$1/$dir/$2"; return 0; }
  done
  return 0
}

# check_tree NAME DIR [EXPECT_FILE]: prints each finding; returns 1 on any refusal.
check_tree() {
  local name=$1 d=$2 expect=${3:-} bad=0 enabled u f owner acct
  [ -d "$d" ] || die "no built tree for $name at $d"
  enabled=$(enabled_units "$d")

  if [ -n "$expect" ]; then
    [ -f "$expect" ] || die "no expected set at $expect"
    local want; want=$(grep -v '^#' "$expect" | sed '/^$/d' | sort -u)
    extra=$(comm -23 <(printf '%s\n' "$enabled") <(printf '%s\n' "$want") | sed '/^$/d' || true)
    missing=$(comm -13 <(printf '%s\n' "$enabled") <(printf '%s\n' "$want") | sed '/^$/d' || true)
    for u in $extra; do echo "check-built-root: FAIL $name: enabled but not in the stated set: $u"; bad=1; done
    for u in $missing; do echo "check-built-root: FAIL $name: in the stated set but not enabled: $u"; bad=1; done
  else
    echo "check-built-root: NOT CHECKED $name: enablement (no --expect set given)"
  fi

  # The accounts: User=/Group= of every enabled unit, plus orion when orion-comm.service is enabled.
  local accounts=""
  for u in $enabled; do
    f=$(unit_file "$d" "$u")
    [ -n "$f" ] || continue
    for key in User Group; do
      owner=$(sed -n "s/^$key=//p" "$f" | tail -1)
      [ -n "$owner" ] && accounts="$accounts $key:$owner:$u"
    done
  done
  if printf '%s\n' "$enabled" | grep -qx 'orion-comm.service'; then accounts="$accounts User:orion:orion-comm.service"; fi
  for acct in $accounts; do
    kind=${acct%%:*}; rest=${acct#*:}; name_acct=${rest%%:*}; by=${rest#*:}
    db=$d/etc/passwd; [ "$kind" = Group ] && db=$d/etc/group
    if ! grep -q "^$name_acct:" "$db" 2>/dev/null; then
      echo "check-built-root: FAIL $name: $kind=$name_acct of $by has no entry in ${db#"$d"/}"
      bad=1
    fi
  done
  [ "$bad" -eq 0 ] && echo "check-built-root: ok $name: enablement and accounts checked"
  return "$bad"
}

if [ "${1:-}" = --self-test ]; then
  t=$(mktemp -d "${TMPDIR:-/tmp}/cbr-XXXXXX"); trap 'rm -rf "$t"' EXIT
  mk() {  # mk DIR [with-account]: a fake built tree with systemd-networkd enabled and its User=systemd-network
    local d=$1 acct=${2:-yes}
    mkdir -p "$d/etc" "$d/usr/lib/systemd/system/multi-user.target.wants"
    printf 'root:x:0:0:root:/root:/bin/sh\n' > "$d/etc/passwd"
    printf 'root:x:0:\n' > "$d/etc/group"
    [ "$acct" = yes ] && printf 'systemd-network:x:990:990::/:/bin/false\n' >> "$d/etc/passwd"
    printf '[Service]\nType=notify-reload\nUser=systemd-network\nExecStart=/bin/false\n' > "$d/usr/lib/systemd/system/systemd-networkd.service"
    ln -sfn ../systemd-networkd.service "$d/usr/lib/systemd/system/multi-user.target.wants/systemd-networkd.service"
  }
  printf 'systemd-networkd.service\n' > "$t/expect"
  mk "$t/ok"
  check_tree ok "$t/ok" "$t/expect" >/dev/null || die "self-test: the equal, account-complete tree did not pass"
  mk "$t/noacct" no
  if check_tree noacct "$t/noacct" "$t/expect" >"$t/noacct.log" 2>&1; then die "self-test: a missing User= account was accepted"; fi
  grep -q 'User=systemd-network' "$t/noacct.log" || die "self-test: the refusal does not name systemd-network"
  mk "$t/extra"
  ln -sfn ../systemd-networkd.service "$t/extra/usr/lib/systemd/system/multi-user.target.wants/extra.service"
  if check_tree extra "$t/extra" "$t/expect" >"$t/extra.log" 2>&1; then die "self-test: one extra enabled unit was accepted"; fi
  grep -q 'enabled but not in the stated set: extra.service' "$t/extra.log" || die "self-test: the extra unit is not named"
  mk "$t/missing"; printf 'systemd-networkd.service\nlemnosd.service\n' > "$t/expect2"
  if check_tree missing "$t/missing" "$t/expect2" >"$t/missing.log" 2>&1; then die "self-test: one missing enabled unit was accepted"; fi
  grep -q 'in the stated set but not enabled: lemnosd.service' "$t/missing.log" || die "self-test: the missing unit is not named"
  echo "check-built-root: self-test passed (a missing User= account, an extra enabled unit and a missing one are refused; an equal tree passes)"
  exit 0
fi

# Real use: the built trees named on the command line or in ATLAS_BUILT_TREES, one NAME=DIR per entry.
expect=""; trees=()
[ -n "${ATLAS_BUILT_TREES:-}" ] && read -r -a trees <<<"$ATLAS_BUILT_TREES"
while [ $# -gt 0 ]; do
  case $1 in
    --built) trees+=("$2"); shift 2 ;;
    --expect) expect=$2; shift 2 ;;
    *) die "usage: check-built-root.sh --built NAME=DIR [--expect FILE] | --self-test" ;;
  esac
done
if [ ${#trees[@]} -eq 0 ]; then
  echo "check-built-root: NOT CHECKED (no built tree given: pass --built NAME=DIR or ATLAS_BUILT_TREES)"
  exit 0
fi
fail=0
for t in "${trees[@]}"; do
  check_tree "${t%%=*}" "${t#*=}" "$expect" || fail=1
done
[ "$fail" -eq 0 ] || die "FAILED"
echo "check-built-root: passed (${#trees[@]} built tree(s))"
