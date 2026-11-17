#!/usr/bin/env bash
# The built-root rule ("accounts and enablement"), for a built root (not the fake tree). For each built root:
#   A. the ENABLED units EQUAL the product's expected set. The expected set is ONE derivation, printed below:
#        expected = the product's preset `enable` lines  (what preset-all enables: os/overlay/.../*.preset)
#                 ∪ every unit named in [[services]] with enable = true, of the components the product's images use
#                   (what Gaia's services layer links: gaia.tree.services, Gaia crates/nodes/tree/src/services.rs:25-103)
#      The enabled set is the names of the links in the tree's *.wants / *.requires directories, MINUS units the tree
#      masks (a unit masked in etc/systemd/system is not enabled: systemd refuses its link).
#   B. two claims are refused (the derivation itself, before any tree is looked at):
#        - a unit in the preset's enable lines AND in a component's [[services]]: two owners of one link path;
#        - a unit masked in the tree AND named by a component's [[services]] or by the preset: the mask wins, the link is lost.
#   C. every User= and Group= of every enabled unit exists in the tree's etc/passwd and etc/group, and so does the orion
#      account when orion-comm.service is enabled (its unit's User= is the account it runs as).
# With no built tree the check prints, visibly, "built-root check: NOT CHECKED (no built tree given)".
#
#   scripts/check-built-root.sh [--product DIR] [--built NAME=DIR]...   (ATLAS_BUILT_TREES="NAME=DIR ..." too: the same
#                                                                        names as scripts/check-watchdog-owner.sh)
#   scripts/check-built-root.sh --self-test                             fake products and fake built trees
#
# OPEN (NEEDS-GAIA, asked): whether a [[services]] link survives `systemctl preset-all` with `disable *`. The answer that
# decides it: Gaia's services layer is a later step (gaia.tree.services), never written into Buildroot's target, so
# preset-all does not see it. Until the built root is checked this is an expectation, not a verified fact.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
die() { echo "check-built-root: $*" >&2; exit 1; }

# derive PRODUCT_DIR: prints "preset<TAB>UNIT<TAB>SOURCE" and "services<TAB>UNIT<TAB>SOURCE" lines (the one derivation).
derive() {
  python3 - "$1" <<'PY'
import glob, os, sys, tomllib
prod = sys.argv[1]
doc = tomllib.load(open(os.path.join(prod, "product.toml"), "rb"))
comps = doc.get("components", {})
names = set()
for im in doc.get("images", []):
    names |= set(im.get("components", []))
for n in sorted(names):
    path = comps.get(n)
    if path is None:
        print(f"ERR\tmissing component {n}\t{n}")
        continue
    c = tomllib.load(open(os.path.normpath(os.path.join(prod, path)), "rb"))
    for s in c.get("services", []):
        if s.get("enable"):
            print(f"services\t{s['unit']}\tcomponent {n}")
for p in sorted(glob.glob(os.path.join(prod, "**", "*.preset"), recursive=True)):
    for line in open(p):
        words = line.split("#")[0].split()
        if len(words) == 2 and words[0] == "enable":
            print(f"preset\t{words[1]}\t{os.path.relpath(p, prod)}")
PY
}

# masked_units DIR: the units the tree masks (a link to /dev/null in etc/systemd/system).
masked_units() {
  find "$1/etc/systemd/system" -maxdepth 1 -type l -lname /dev/null 2>/dev/null | while IFS= read -r l; do basename "$l"; done | sort -u
}

# enabled_units DIR: the unit names linked into any .wants/.requires directory, minus the masked ones.
enabled_units() {
  local d=$1 masked
  masked=$(masked_units "$d")
  find "$d/etc/systemd/system" "$d/usr/lib/systemd/system" -type d \( -name '*.wants' -o -name '*.requires' \) 2>/dev/null \
    | while IFS= read -r w; do ls -1A "$w"; done | sort -u | while IFS= read -r u; do
        case " $(echo $masked | tr '\n' ' ') " in *" $u "*) ;; *) echo "$u" ;; esac
      done
}

# unit_file DIR UNIT: the unit file in the tree (etc first, as systemd reads it), or nothing.
unit_file() {
  for dir in etc/systemd/system usr/lib/systemd/system; do
    [ -f "$1/$dir/$2" ] && { echo "$1/$dir/$2"; return 0; }
  done
  return 0
}

# check_tree PRODUCT NAME DIR: prints each finding; returns 1 on any refusal.
check_tree() {
  local product=$1 name=$2 d=$3 bad=0 u f owner acct kind rest name_acct by
  [ -d "$d" ] || die "no built tree for $name at $d"

  local lines preset_units services_units masked expected enabled
  lines=$(derive "$product") || die "could not derive the expected set from $product"
  if grep -q '^ERR' <<<"$lines"; then echo "$lines" | grep '^ERR' | cut -f2 | sed 's/^/check-built-root: FAIL /'; return 1; fi
  preset_units=$(awk -F'\t' '$1=="preset"{print $2}' <<<"$lines" | sort -u)
  services_units=$(awk -F'\t' '$1=="services"{print $2}' <<<"$lines" | sort -u)
  masked=$(masked_units "$d")

  # B. the double claims, refused before the tree is compared.
  for u in $(comm -12 <(printf '%s\n' "$preset_units") <(printf '%s\n' "$services_units")); do
    echo "check-built-root: FAIL $name: $u has two owners: the preset ($(awk -F'\t' -v u="$u" '$1=="preset"&&$2==u{print $3}' <<<"$lines")) and a component's [[services]] ($(awk -F'\t' -v u="$u" '$1=="services"&&$2==u{print $3}' <<<"$lines")). List it in one place only."
    bad=1
  done
  for u in $(comm -12 <(printf '%s\n' "$masked") <(printf '%s\n' "$services_units" "$preset_units" | sort -u)); do
    echo "check-built-root: FAIL $name: $u is masked in the tree and also expected enabled (its [[services]] or preset line): the mask wins, its link is lost"
    bad=1
  done

  # A. the expected set, from the one derivation, against the enabled links.
  expected=$(printf '%s\n%s\n' "$preset_units" "$services_units" | sed '/^$/d' | sort -u)
  enabled=$(enabled_units "$d" | sed '/^$/d' | sort -u)
  echo "check-built-root: expected for $name: $(printf '%s\n' "$preset_units" | sed '/^$/d' | wc -l) from the preset enable lines, $(printf '%s\n' "$services_units" | sed '/^$/d' | wc -l) from components' [[services]], $(printf '%s\n' "$expected" | sed '/^$/d' | wc -l) in all; $(printf '%s\n' "$masked" | sed '/^$/d' | wc -l) masked (not counted)"
  for u in $(comm -23 <(printf '%s\n' "$enabled") <(printf '%s\n' "$expected") | sed '/^$/d'); do
    echo "check-built-root: FAIL $name: enabled but in neither the preset nor a [[services]]: $u"; bad=1
  done
  for u in $(comm -13 <(printf '%s\n' "$enabled") <(printf '%s\n' "$expected") | sed '/^$/d'); do
    echo "check-built-root: FAIL $name: expected but not enabled: $u"; bad=1
  done

  # C. accounts: User=/Group= of every enabled unit, plus orion when orion-comm.service is enabled.
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
  [ "$bad" -eq 0 ] && echo "check-built-root: ok $name: enablement (the derived set) and accounts checked"
  return "$bad"
}

# make_product DIR: a fake product (product.toml, one component with two [[services]], a preset) for the self-test.
make_product() {
  local p=$1
  mkdir -p "$p/comp" "$p/os/overlay/usr/lib/systemd/system-preset"
  cat > "$p/product.toml" <<'TOML'
[components]
comp = "comp/component.toml"

[[images]]
components = ["comp"]
TOML
  cat > "$p/comp/component.toml" <<'TOML'
[[services]]
unit = "svc-a.service"
enable = true

[[services]]
unit = "svc-b.socket"
enable = true
TOML
  printf 'enable svc-c.service\ndisable *\n' > "$p/os/overlay/usr/lib/systemd/system-preset/10-test.preset"
}

# make_tree DIR [units...]: a fake built tree linking the given units (into the wants directory), with no accounts.
make_tree() {
  local d=$1; shift
  mkdir -p "$d/etc" "$d/usr/lib/systemd/system/multi-user.target.wants" "$d/usr/lib/systemd/system/sockets.target.wants"
  printf 'root:x:0:0:root:/root:/bin/sh\n' > "$d/etc/passwd"
  printf 'root:x:0:\n' > "$d/etc/group"
  local u
  for u in "$@"; do
    case $u in
      *.socket) ln -sfn "../$u" "$d/usr/lib/systemd/system/sockets.target.wants/$u" ;;
      *) ln -sfn "../$u" "$d/usr/lib/systemd/system/multi-user.target.wants/$u" ;;
    esac
  done
}

if [ "${1:-}" = --self-test ]; then
  t=$(mktemp -d "${TMPDIR:-/tmp}/cbr-XXXXXX"); trap 'rm -rf "$t"' EXIT
  make_product "$t/p"
  # 1. The equal tree passes: the derived set is svc-a, svc-b.socket (components) and svc-c (preset).
  make_tree "$t/ok" svc-a.service svc-b.socket svc-c.service
  check_tree "$t/p" ok "$t/ok" >/dev/null || die "self-test: the tree equal to the derived set did not pass"
  # 2. One extra enabled unit: refused, and named.
  make_tree "$t/extra" svc-a.service svc-b.socket svc-c.service extra.service
  if check_tree "$t/p" extra "$t/extra" >"$t/extra.log" 2>&1; then die "self-test: one extra enabled unit was accepted"; fi
  grep -q 'enabled but in neither' "$t/extra.log" && grep -q 'extra.service' "$t/extra.log" || die "self-test: the extra unit is not named"
  # 3. One missing enabled unit: refused, and named.
  make_tree "$t/missing" svc-a.service svc-c.service
  if check_tree "$t/p" missing "$t/missing" >"$t/missing.log" 2>&1; then die "self-test: one missing enabled unit was accepted"; fi
  grep -q 'expected but not enabled: svc-b.socket' "$t/missing.log" || die "self-test: the missing unit is not named"
  # 4. A missing User= account: refused, and named.
  make_tree "$t/noacct" svc-a.service svc-b.socket svc-c.service
  printf '[Service]\nUser=systemd-network\nExecStart=/bin/false\n' > "$t/noacct/usr/lib/systemd/system/svc-a.service"
  if check_tree "$t/p" noacct "$t/noacct" >"$t/noacct.log" 2>&1; then die "self-test: a missing User= account was accepted"; fi
  grep -q 'User=systemd-network' "$t/noacct.log" || die "self-test: the refusal does not name systemd-network"
  # 5. The two claims, from the derivation: a unit in the preset AND in a component's [[services]].
  make_product "$t/dup"
  printf 'enable svc-a.service\ndisable *\n' > "$t/dup/os/overlay/usr/lib/systemd/system-preset/10-test.preset"
  make_tree "$t/duptree" svc-a.service svc-b.socket
  if check_tree "$t/dup" duptree "$t/duptree" >"$t/dup.log" 2>&1; then die "self-test: a unit with two owners (preset and [[services]]) was accepted"; fi
  grep -q 'svc-a.service has two owners' "$t/dup.log" && grep -q 'component comp' "$t/dup.log" && grep -q 'preset' "$t/dup.log" \
    || die "self-test: the double claim does not name both sources"
  # 6. A unit masked in the tree AND named by [[services]] (svc-a): refused, the mask wins.
  make_tree "$t/masktree" svc-a.service svc-b.socket svc-c.service
  mkdir -p "$t/masktree/etc/systemd/system"; ln -sfn /dev/null "$t/masktree/etc/systemd/system/svc-a.service"
  if check_tree "$t/p" masktree "$t/masktree" >"$t/mask.log" 2>&1; then die "self-test: a masked unit named by [[services]] was accepted"; fi
  grep -q 'svc-a.service is masked in the tree' "$t/mask.log" || die "self-test: the masked-and-named refusal does not name the unit"
  # 7. A mask on a unit nobody expects (the wait-online mask) is not an enablement and is not refused.
  make_tree "$t/maskother" svc-a.service svc-b.socket svc-c.service
  mkdir -p "$t/maskother/etc/systemd/system"; ln -sfn /dev/null "$t/maskother/etc/systemd/system/systemd-networkd-wait-online.service"
  check_tree "$t/p" maskother "$t/maskother" >/dev/null || die "self-test: a mask on an unexpected unit was refused"
  # 8. No tree: NOT CHECKED, visibly.
  out=$("$0" --product "$t/p" 2>&1); [[ $out == *"NOT CHECKED"* ]] || die "self-test: no tree did not print NOT CHECKED"
  # 9. The real raze-base product derives a non-empty set, with its preset and its components' services.
  real=$(derive "$repo/devices/products/raze-base" | awk -F'\t' '{print $1}' | sort | uniq -c | tr -s ' ' | tr '\n' ';')
  [[ $real == *preset* && $real == *services* ]] || die "self-test: the real raze-base product did not derive both sources: $real"
  echo "check-built-root: self-test passed (the derived set is checked both ways; an extra or missing enabled unit, a missing User= account, a unit with two owners and a masked unit named by [[services]] are refused; a masked unit is not counted; no tree prints NOT CHECKED)"
  exit 0
fi

# Real use: the product and the built trees (--built NAME=DIR, or ATLAS_BUILT_TREES, the same names as check-watchdog-owner.sh).
product=$repo/devices/products/raze-base
trees=()
[ -n "${ATLAS_BUILT_TREES:-}" ] && read -r -a trees <<<"$ATLAS_BUILT_TREES"
while [ $# -gt 0 ]; do
  case $1 in
    --built) trees+=("$2"); shift 2 ;;
    --product) product=$2; shift 2 ;;
    *) die "usage: check-built-root.sh [--product DIR] [--built NAME=DIR]... | --self-test" ;;
  esac
done
if [ ${#trees[@]} -eq 0 ]; then
  echo "check-built-root: NOT CHECKED (no built tree given: pass --built NAME=DIR or ATLAS_BUILT_TREES)"
  exit 0
fi
fail=0
for t in "${trees[@]}"; do
  check_tree "$product" "${t%%=*}" "${t#*=}" || fail=1
done
[ "$fail" -eq 0 ] || die "FAILED"
echo "check-built-root: passed (${#trees[@]} built tree(s))"
