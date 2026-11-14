#!/usr/bin/env bash
# Checks every device description under devices/ with Gaia `next`.
#
#   scripts/check-devices.sh               check devices/, then prove the checker still catches
#                                          a broken file (the fixtures; the gate runs this)
#   scripts/check-devices.sh --root DIR    check another tree laid out like devices/ (no self-check)
#   scripts/check-devices.sh --write-locks regenerate every product's committed gaia.lock, then check
#                                          (with --root DIR, the same for that tree)
#   scripts/check-devices.sh --self-test   the verbose form: the good fixtures must pass, broken must
#                                          fail
#
# The Gaia commit is read from devices/gaia.lock. The gaia binary is $GAIA_BIN when set, else a build
# from that commit cached under ${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/<commit>/.
# Either way it must say, in `gaia --version`, that it was built from that commit (`gaia <version>
# (<commit>)`); a binary that says another commit, says none, or says `-dirty` is refused.
# The recipes come from the same Gaia checkout. Every family, board, component and product file is
# validated (products against the checkout's recipes/, so a bogus recipe name fails), and every
# product is planned.
#
# Each product's gaia.lock, committed beside its product.toml, is checked OFFLINE: `gaia lock` runs on a
# copy of it with git and curl replaced by programs that fail, so a lock that needs a request is a
# failure, not a silent fetch, and the plan uses the committed lock. `--write-locks` is the only mode
# that asks the network (each source's ref, a download, an image's digest): it starts from the committed
# lock, so a source the lock already holds for what its description names is not asked again (gaia lock's
# rule). What is new or changed is resolved. A host that is unreachable or rate limited there is a FAILURE,
# not a note. To refresh a source that did not change in its description, run `gaia lock --update NAME`.
#
# Products that take a bench SSH public key (`{ input = "bench-key" }` in a component) are planned with
# a throwaway public key the check makes in a temp dir for each run; no key is committed.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
self=$repo/scripts/check-devices.sh
root=$repo/devices
mode=check
root_given=0
write_locks=0

die() { echo "check-devices: $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case $1 in
    --root) [[ $# -ge 2 ]] || die "--root needs a directory"; root=$2; root_given=1; shift 2 ;;
    --self-test) mode=self-test; shift ;;
    --write-locks) write_locks=1; shift ;;
    -h|--help) sed -n '2,15p' "$self"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

# The broken fixture must fail, given its run's exit status ($2) and output ($1). Under gaia
# validate the output must also name the broken key, so the check shows it caught the bad file.
broken_failed_as_it_must() {
  local out=$1 rc=$2
  [[ $rc -ne 0 ]] || return 1
  if grep -q 'command: validate' "$out"; then
    grep -q 'target\.kernal: unknown key' "$out" || return 1
  fi
  return 0
}

# The Lemnos rule (AGENTS.md: every Lemnos component in a product is locked to ONE rev). Every source of
# the Lemnos repository in a product's gaia.lock is one [source.*] block with a url and a commit: the
# commits of those blocks must be one. Prints the distinct revs, one per line.
lemnos_revs() {
  awk '
    /^\[/ { lem = 0; next }
    /^url = .*Prometheus-Dynamics\/Lemnos/ { lem = 1; next }
    /^commit = / && lem { c = $0; sub(/^commit = "/, "", c); sub(/"$/, "", c); print c }
  ' "$1" | sort -u
}

# Prints "REV1 and REV2" (plus a count of any more) and returns 1 when the lock pins two or more Lemnos revs.
lemnos_two_revs() {
  local revs=() r
  while IFS= read -r r; do
    [[ -n $r ]] && revs+=("$r")
  done < <(lemnos_revs "$1")
  [[ ${#revs[@]} -le 1 ]] && return 0
  local more=$(( ${#revs[@]} - 2 )) tail=""
  (( more > 0 )) && tail=" (and $more more)"
  echo "${revs[0]} and ${revs[1]}$tail"
  return 1
}

# Proves the rule catches two revs and passes one, on a lock written here (the fixture is not a product).
lemnos_rule_self_test() {
  local two one rc=0
  two=$(mktemp); one=$(mktemp)
  cat >"$two" <<'EOF'
[source."a.src"]
kind = "git"
url = "https://github.com/Prometheus-Dynamics/Lemnos"
ref = "1111111111111111111111111111111111111111"
commit = "1111111111111111111111111111111111111111"

[source."b.src"]
kind = "git"
url = "https://github.com/Prometheus-Dynamics/Lemnos"
ref = "2222222222222222222222222222222222222222"
commit = "2222222222222222222222222222222222222222"
EOF
  sed 's/2222222222222222222222222222222222222222/1111111111111111111111111111111111111111/g' "$two" >"$one"
  if lemnos_two_revs "$two" >/dev/null; then
    echo "check-devices: self-test: a lock with two Lemnos revs was not refused" >&2; rc=1
  elif [[ $(lemnos_two_revs "$two") != "1111111111111111111111111111111111111111 and 2222222222222222222222222222222222222222" ]]; then
    echo "check-devices: self-test: the two-rev refusal does not name both revs" >&2; rc=1
  fi
  lemnos_two_revs "$one" >/dev/null || { echo "check-devices: self-test: a lock with one Lemnos rev was refused" >&2; rc=1; }
  rm -f "$two" "$one"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the Lemnos one-rev rule refuses two revs and passes one"
  return $rc
}

# The Lemnos board file rule (PR B26). A product whose components include lemnosd runs a daemon that reads
# /etc/lemnos/board.toml, so the board it is built for must name its Lemnos board file under [descriptions]
# (`lemnos = "lemnos-board.toml"`, beside the board.toml), and that file must exist. Zero names is refused here,
# naming the product and the board. Gaia refuses, at plan time, an install that names a description the board
# does not state, and a description file that does not exist; a product that carries lemnosd and installs
# nothing from the description gets no such refusal, which is the case this rule covers. (A board names one
# file per description name, so one name is one file.)
# Prints the description's path, relative to the board's directory: the board's own [descriptions], else the
# family it extends. Prints nothing when neither names it.
lemnos_description() {
  local file=$1 found fam
  found=$(awk '
    /^[[:space:]]*\[/ { sec = $0; gsub(/[[:space:]]/, "", sec); next }
    sec == "[descriptions]" && /^[[:space:]]*lemnos[[:space:]]*=/ {
      v = $0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/".*$/, "", v); print v; exit
    }
  ' "$file")
  if [[ -z $found ]]; then
    fam=$({ grep -ho '^extends = "[^"]*"' "$file" || true; } | sed 's/^extends = "//; s/"$//' | head -1)
    if [[ -n $fam && -f $(dirname "$file")/$fam ]]; then
      found=$(lemnos_description "$(realpath -m "$(dirname "$file")/$fam")")
    fi
  fi
  echo "$found"
}

# Prints a line naming the product and its board and returns 1 when the product carries lemnosd and its board
# names no Lemnos board file, or names one that does not exist. Passes (prints nothing) otherwise.
lemnos_board_rule() {  # <product.toml>
  local product=$1 pdir pname board_rel board_file bname desc
  pdir=$(dirname "$product"); pname=$(basename "$pdir")
  board_rel=$({ grep -ho '"[^"]*board\.toml"' "$product" || true; } | tr -d '"' | head -1)
  board_file=$(realpath -m "$pdir/$board_rel")
  bname=$(basename "$(dirname "$board_file")")
  desc=$(lemnos_description "$board_file")
  if [[ -z $desc ]]; then
    echo "product $pname (board $bname): carries lemnosd, which reads /etc/lemnos/board.toml, but its board names no Lemnos board file under [descriptions] (lemnos = \"lemnos-board.toml\")"
    return 1
  fi
  if [[ ! -f $(dirname "$board_file")/$desc ]]; then
    echo "product $pname (board $bname): carries lemnosd, and its board names the Lemnos board file $desc, which is not beside the board"
    return 1
  fi
  return 0
}

# Proves the rule refuses a product whose board names no Lemnos board file, and one whose named file is missing,
# and passes the real Raze product, on a scratch copy of devices/.
lemnos_board_rule_self_test() {
  local tmp rc=0 out prod
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  prod=$repo/devices/products/raze-base/product.toml
  if ! out=$(lemnos_board_rule "$prod"); then
    echo "check-devices: self-test: the Raze product no longer passes the Lemnos board file rule: $out" >&2; rc=1
  fi
  sed -i '/^\[descriptions\]$/,$d' "$tmp/devices/boards/raze/board.toml"
  if out=$(lemnos_board_rule "$tmp/devices/products/raze-base/product.toml"); then
    echo "check-devices: self-test: a lemnosd product whose board names no Lemnos board file was not refused" >&2; rc=1
  elif [[ $out != *"product raze-base (board raze)"* || $out != *"no Lemnos board file"* ]]; then
    echo "check-devices: self-test: the refusal does not name the product and board: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/boards/raze/board.toml" "$tmp/devices/boards/raze/board.toml"
  rm -f "$tmp/devices/boards/raze/lemnos-board.toml"
  if out=$(lemnos_board_rule "$tmp/devices/products/raze-base/product.toml"); then
    echo "check-devices: self-test: a lemnosd product whose named Lemnos board file is missing was not refused" >&2; rc=1
  elif [[ $out != *"lemnos-board.toml, which is not beside the board"* ]]; then
    echo "check-devices: self-test: the missing-file refusal does not name the file: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the Lemnos board file rule refuses a lemnosd product with no board file name or a missing file, and passes Raze"
  return $rc
}

# The bring-up rule (PR B31). Every bring-up product (its name ends in -bringup) carries the watchdog and LED signs
# (components/bringup-alive), its own LED and link settings (components/bringup-alive-settings, which install
# /etc/board-alive.leds and /etc/board-alive.links), and a family whose kernel fragment builds in the watchdog driver
# and both LED triggers the signs use. A fifth bring-up product cannot miss them. Prints one line per problem and
# returns 1 when there is any. The watchdog symbol is named per family here: a new family is added by name.
bringup_rule() {  # <devices dir>
  local root=$1 p pdir pname sc board bdir fam frag sym dst s rc=0 out="" want_off dt dtd dtsf ext extf loads links l
  for p in "$root"/products/*-bringup/product.toml; do
    [[ -f $p ]] || continue
    pdir=$(dirname "$p"); pname=$(basename "$pdir")
    if ! grep -q '^bringup-alive = "[^"]*bringup-alive/component\.toml"$' "$p"; then
      out+="product $pname: no bringup-alive component in [components] (the watchdog and LED signs)"$'\n'; rc=1
    elif ! grep -q '^components = \[.*"bringup-alive"' "$p"; then
      out+="product $pname: bringup-alive is named in [components] but no image lists it"$'\n'; rc=1
    fi
    sc=$pdir/components/bringup-alive-settings/component.toml
    if ! grep -q '^bringup-alive-settings = ' "$p" || [[ ! -f $sc ]]; then
      out+="product $pname: no bringup-alive-settings component (the LED and link the signs use)"$'\n'; rc=1
    else
      for dst in /etc/board-alive.leds /etc/board-alive.links; do
        grep -q "^to = \"$dst\"" "$sc" || { out+="product $pname: bringup-alive-settings does not install $dst"$'\n'; rc=1; }
      done
    fi
    board=$(grep -o '^board = "[^"]*"' "$p" | head -1 | cut -d'"' -f2)
    bdir=$(dirname "$(realpath -m "$pdir/$board")")
    fam=$(grep -o '^extends = "[^"]*"' "$bdir/board.toml" | head -1 | cut -d'"' -f2)
    fam=$(dirname "$(realpath -m "$bdir/$fam")")
    sym=
    case $(basename "$fam") in
      rk35xx) sym=CONFIG_DW_WATCHDOG ;;
      sun60i) sym=CONFIG_SUNXI_WATCHDOG ;;
      *) out+="product $pname: family $(basename "$fam") has no watchdog symbol in the bring-up rule"$'\n'; rc=1 ;;
    esac
    frag=$fam/assets/linux.fragment
    for s in ${sym:+$sym} CONFIG_LEDS_TRIGGER_HEARTBEAT CONFIG_LEDS_TRIGGER_TIMER; do
      grep -qx "$s=y" "$frag" 2>/dev/null || { out+="product $pname: $(basename "$fam")'s fragment does not build in $s=y (the bring-up signs need it)"$'\n'; rc=1; }
    done
    # The Ethernet (B31): the bring-up's network is matched by type, Buildroot's name-based eth0.network is off, and
    # every link the LED signs name is one the image creates: `ether` (bringup-network) or usb0 (usb-gadget-network).
    if ! grep -q '^bringup-network = "[^"]*bringup-network/component\.toml"$' "$p"; then
      out+="product $pname: no bringup-network component in [components] (the Ethernet by type)"$'\n'; rc=1
    fi
    if ! grep -q '^# BR2_SYSTEM_DHCP is not set$' "$pdir/os.fragment" 2>/dev/null || grep -q '^BR2_SYSTEM_DHCP=' "$pdir/os.fragment" 2>/dev/null; then
      out+="product $pname: its os.fragment does not unset BR2_SYSTEM_DHCP (Buildroot's eth0.network matches Name=eth0, which the GMAC does not have)"$'\n'; rc=1
    fi
    # The loader (B31): an RK35xx bring-up board's device tree carries the reboot-mode node at the offset that
    # places U-Boot's boot-mode register inside the syscon it sits in, with the loader value; its SPL returns to the
    # BootROM for that value (families/rk35xx/assets/u-boot.fragment); and the image's extlinux loads that dtb.
    # The A733 is excused by name (no equivalent is established: docs/prs/B31.md, point 2).
    case $(basename "$fam") in
      rk35xx)
        case $(basename "$bdir") in
          radxa-zero-3e) want_off=0x200 ;;
          radxa-rock-5c) want_off=0x80 ;;
          radxa-rock-4d) want_off=0x40 ;;
          *) want_off=; out+="product $pname: board $(basename "$bdir") has no loader offset in the bring-up rule (add it with its U-Boot register)"$'\n'; rc=1 ;;
        esac
        dt=$(grep -o '^device_tree = "[^"]*"' "$bdir/board.toml" | head -1 | cut -d'"' -f2)
        dtd=$(grep -o '^device_tree_dir = "[^"]*"' "$bdir/board.toml" | head -1 | cut -d'"' -f2)
        dtsf=$bdir/${dtd:-}/$dt.dts
        if [[ -n ${want_off:-} ]]; then
          if [[ ! -f $dtsf ]] || ! grep -q "^\s*reboot-mode {" "$dtsf" || ! grep -q "mode-loader = <0xef08a53c>;" "$dtsf" || ! grep -q "offset = <$want_off>;" "$dtsf"; then
            out+="product $pname: its board's dts ($dtsf) lacks the reboot-mode node with mode-loader = <0xef08a53c> and offset = <$want_off>"$'\n'; rc=1
          fi
        fi
        grep -qx 'CONFIG_SPL_ROCKCHIP_BACK_TO_BROM=y' "$fam/assets/u-boot.fragment" 2>/dev/null || { out+="product $pname: $(basename "$fam")'s U-Boot fragment does not build in CONFIG_SPL_ROCKCHIP_BACK_TO_BROM=y (the SPL must return to the BootROM for the loader value)"$'\n'; rc=1; }
        ext=$(grep -o '^radxa-[a-z0-9-]*-extlinux = "[^"]*"' "$p" | head -1 | cut -d'"' -f2)
        if [[ -n $ext ]]; then
          extf=$(dirname "$(realpath -m "$pdir/$ext")")/extlinux.conf
          loads=$(grep -o 'devicetree /boot/[^ ]*' "$extf" 2>/dev/null | head -1 | sed 's#devicetree /boot/##')
          if [[ $loads != "${dt##*/}.dtb" ]]; then
            out+="product $pname: its extlinux loads /boot/${loads:-nothing}, but its board builds ${dt##*/}.dtb"$'\n'; rc=1
          fi
        fi
        ;;
      sun60i)
        out+="NOTE product $pname: its loader point is EXCUSED by name: the A733 chain does not build: three patch series are named and not applied (families/sun60i/family.toml). No reboot-to-loader equivalent is established either (UNVERIFIED; docs/prs/B31.md, point 2)"$'\n'
        ;;
    esac
    links=$pdir/components/bringup-alive-settings/files/board-alive.links
    if [[ -f $links ]]; then
      while IFS= read -r l; do
        case $l in
          ether) grep -q '^bringup-network = ' "$p" || { out+="product $pname: its LED link is ether, but it has no bringup-network component"$'\n'; rc=1; } ;;
          usb0) grep -q '^usb-gadget-network = ' "$p" || { out+="product $pname: its LED link is usb0, but it has no usb-gadget-network component (the one that creates usb0)"$'\n'; rc=1; } ;;
          *) out+="product $pname: its LED link '$l' is an interface the image does not create (only ether or usb0)"$'\n'; rc=1 ;;
        esac
      done < <(grep -v '^[[:space:]]*\(#\|$\)' "$links")
    fi
  done
  net=$root/components/bringup-network/files/10-bringup-ethernet.network
  if compgen -G "$root/products/*-bringup/product.toml" >/dev/null && { ! grep -qx 'Type=ether' "$net" 2>/dev/null || grep -v '^#' "$net" 2>/dev/null | grep -q 'Name=eth0'; }; then
    out+="components/bringup-network: its file must match Type=ether, and never a legacy name such as Name=eth0"$'\n'; rc=1
  fi
  printf '%s' "$out"
  return $rc
}

# Proves the bring-up rule passes the real tree, and refuses (on a scratch copy of devices/) a bring-up product
# with no watchdog component, one whose settings do not install the LED file, and a family missing a trigger.
bringup_rule_self_test() {
  local tmp rc=0 out
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  if ! out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: the real tree no longer passes the bring-up rule: $out" >&2; rc=1
  fi
  sed -i '/^bringup-alive = /d' "$tmp/devices/products/zero-3e-bringup/product.toml"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a bring-up product with no watchdog component was not refused" >&2; rc=1
  elif [[ $out != *"product zero-3e-bringup: no bringup-alive component"* ]]; then
    echo "check-devices: self-test: the refusal does not name the product: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/products/zero-3e-bringup/product.toml" "$tmp/devices/products/zero-3e-bringup/product.toml"
  sed -i '/^to = "\/etc\/board-alive.leds"$/d' "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/component.toml"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: settings that do not install the LED file were not refused" >&2; rc=1
  elif [[ $out != *"product rock-5c-bringup: bringup-alive-settings does not install /etc/board-alive.leds"* ]]; then
    echo "check-devices: self-test: the settings refusal does not name the file: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/products/rock-5c-bringup/components/bringup-alive-settings/component.toml" "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/component.toml"
  sed -i '/^CONFIG_LEDS_TRIGGER_TIMER=y$/d' "$tmp/devices/families/rk35xx/assets/linux.fragment"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a family without the timer trigger was not refused" >&2; rc=1
  elif [[ $out != *"rk35xx's fragment does not build in CONFIG_LEDS_TRIGGER_TIMER=y"* ]]; then
    echo "check-devices: self-test: the trigger refusal does not name the symbol: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/components/bringup-network/files/10-bringup-ethernet.network" "$tmp/devices/components/bringup-network/files/10-bringup-ethernet.network"
  echo eth0 > "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/files/board-alive.links"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: an LED link named eth0 (not created by the image) was not refused" >&2; rc=1
  elif [[ $out != *"product rock-5c-bringup: its LED link 'eth0' is an interface the image does not create"* ]]; then
    echo "check-devices: self-test: the link refusal does not name the product and link: $out" >&2; rc=1
  fi
  echo ether > "$tmp/devices/products/rock-5c-bringup/components/bringup-alive-settings/files/board-alive.links"
  sed -i '/^# BR2_SYSTEM_DHCP is not set$/d' "$tmp/devices/products/zero-3e-bringup/os.fragment"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a bring-up product that leaves BR2_SYSTEM_DHCP on was not refused" >&2; rc=1
  elif [[ $out != *"product zero-3e-bringup: its os.fragment does not unset BR2_SYSTEM_DHCP"* ]]; then
    echo "check-devices: self-test: the DHCP refusal does not name the product: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/products/zero-3e-bringup/os.fragment" "$tmp/devices/products/zero-3e-bringup/os.fragment"
  sed -i 's/^Type=ether$/Name=eth0/' "$tmp/devices/components/bringup-network/files/10-bringup-ethernet.network"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a bring-up network file matching eth0 by name was not refused" >&2; rc=1
  elif [[ $out != *"bringup-network: its file must match Type=ether"* ]]; then
    echo "check-devices: self-test: the network-file refusal does not name the file: $out" >&2; rc=1
  fi
  sed -i 's/offset = <0x40>;/offset = <0x44>;/' "$tmp/devices/boards/radxa-rock-4d/assets/dts/rockchip/rk3576-rock-4d-loader.dts"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: a reboot-mode node at the wrong offset was not refused" >&2; rc=1
  elif [[ $out != *"product rock-4d-bringup: its board's dts"* ]]; then
    echo "check-devices: self-test: the reboot-mode refusal does not name the product: $out" >&2; rc=1
  fi
  cp -a "$repo/devices/boards/radxa-rock-4d/assets/dts/rockchip/rk3576-rock-4d-loader.dts" "$tmp/devices/boards/radxa-rock-4d/assets/dts/rockchip/rk3576-rock-4d-loader.dts"
  sed -i 's#devicetree /boot/rk3588s-rock-5c-loader.dtb#devicetree /boot/rk3588s-rock-5c.dtb#' "$tmp/devices/components/radxa-rock-5c-extlinux/extlinux.conf"
  if out=$(bringup_rule "$tmp/devices"); then
    echo "check-devices: self-test: an extlinux that loads the wrong dtb was not refused" >&2; rc=1
  elif [[ $out != *"product rock-5c-bringup: its extlinux loads /boot/rk3588s-rock-5c.dtb"* ]]; then
    echo "check-devices: self-test: the extlinux refusal does not name the dtb: $out" >&2; rc=1
  fi
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "check-devices: self-test: the bring-up rule passes the real tree and refuses a bring-up product with no watchdog component, settings without the LED file, a family without a trigger, a LED link the image does not create, a product that leaves BR2_SYSTEM_DHCP on, or a network file matched by a legacy name, a reboot-mode node at the wrong offset, or an extlinux that loads the wrong dtb"
  return $rc
}

fixtures=$repo/devices/tests/fixtures

if [[ $mode == self-test ]]; then
  status=0
  for good in good good-f8 good-rust; do
    echo "== $good fixture: must pass"
    "$self" --root "$fixtures/$good" || { echo "check-devices: self-test: $good fixture did not pass" >&2; status=1; }
  done
  lemnos_rule_self_test || status=1
  lemnos_board_rule_self_test || status=1
  bringup_rule_self_test || status=1
  echo "== broken fixture: must fail"
  out=$(mktemp)
  rc=0
  "$self" --root "$fixtures/broken" >"$out" 2>&1 || rc=$?
  cat "$out"
  if broken_failed_as_it_must "$out" "$rc"; then
    echo "check-devices: self-test: broken fixture failed as it must"
  else
    echo "check-devices: self-test: broken fixture did not fail as it must (see above)" >&2; status=1
  fi
  rm -f "$out"
  exit $status
fi

# ---- the Gaia pin -------------------------------------------------------------

lock=$repo/devices/gaia.lock
[[ -f $lock ]] || die "missing $lock"
gaia_url=$(sed -n 's/^url = "\(.*\)"$/\1/p' "$lock")
gaia_commit=$(sed -n 's/^commit = "\(.*\)"$/\1/p' "$lock")
[[ -n $gaia_url ]] || die "$lock has no url"
[[ $gaia_commit =~ ^[0-9a-f]{40}$ ]] || die "$lock: commit must be a full 40-digit hex commit"

cache=${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}
dir=$cache/$gaia_commit

# Clones the locked commit into $dir/src unless that checkout is already there.
ensure_src() {
  if [[ -f $dir/src/.built-from && $(cat "$dir/src/.built-from") == "$gaia_commit" ]]; then
    return 0
  fi
  command -v git >/dev/null || die "git is needed to fetch Gaia"
  local work=$dir.partial
  rm -rf "$work"; mkdir -p "$work"
  echo "gaia: cloning $gaia_url" >&2
  git clone -q --no-checkout "$gaia_url" "$work/src" \
    || die "cannot clone $gaia_url (no network, or the repository is private and no token is set)"
  git -C "$work/src" checkout -q --detach "$gaia_commit" \
    || die "commit $gaia_commit is not in $gaia_url"
  echo "$gaia_commit" > "$work/src/.built-from"
  mkdir -p "$dir"
  rm -rf "$dir/src"; mv "$work/src" "$dir/src"; rm -rf "$work"
}

# Builds gaia from $dir/src into $dir/bin/gaia unless that build is already there.
ensure_bin() {
  if [[ -x $dir/bin/gaia && -f $dir/bin/.built-from && $(cat "$dir/bin/.built-from") == "$gaia_commit" ]]; then
    echo "gaia: cached build $dir/bin/gaia" >&2
    return 0
  fi
  command -v cargo >/dev/null || die "cargo is needed to build Gaia (install Rust with rustup)"
  echo "gaia: building gaia at $gaia_commit (slow the first time)" >&2
  (cd "$dir/src" && CARGO_TARGET_DIR="$dir/target" cargo build --release -p gaia --bin gaia) >&2 \
    || die "building gaia at $gaia_commit failed"
  mkdir -p "$dir/bin"
  cp "$dir/target/release/gaia" "$dir/bin/gaia"
  rm -rf "$dir/target"
  echo "$gaia_commit" > "$dir/bin/.built-from"
}

ensure_src
if [[ -n ${GAIA_BIN:-} ]]; then
  gaia=$GAIA_BIN
  [[ -x $gaia ]] || die "GAIA_BIN=$gaia is not an executable"
else
  ensure_bin
  gaia=$dir/bin/gaia
fi
# The binary must be a build of the pinned commit: it prints `gaia <version> (<commit>)`, and that
# commit must be devices/gaia.lock's. A binary that prints no commit (an old Gaia) is refused too.
gaia_line=$("$gaia" --version 2>&1 | head -1 || true)
if [[ ! $gaia_line =~ ^gaia\ [^\ ]+\ \(([^\)]*)\)$ ]]; then
  die "$gaia does not print its commit ('$gaia_line'), so it cannot be checked against $gaia_commit (devices/gaia.lock). Build that commit, or unset GAIA_BIN to let the check build it."
fi
gaia_built=${BASH_REMATCH[1]}
if [[ $gaia_built != "$gaia_commit" ]]; then
  die "$gaia was built from '$gaia_built', but devices/gaia.lock pins $gaia_commit: a stale binary. Rebuild Gaia at $gaia_commit, or unset GAIA_BIN."
fi
recipes=$dir/src/recipes

# python3 is the tool the python component builder names; gaia plan needs it given.
python3_path=$(command -v python3) || die "python3 is needed (the python builder's tool)"
tool_args=(--tool "python=$python3_path")
# The slot recipe names veritysetup, and gaia plan wants every named tool
# present even though planning never runs it (Gaia has accepted dropping that).
# Where it isn't installed, planning gets a stand-in so the description can
# still be checked; a build needs the real one. CI installs the real one.
if ! command -v veritysetup >/dev/null 2>&1; then
  stand_in_dir=$(mktemp -d)
  printf '#!/bin/sh\necho "veritysetup stand-in: planning only" >&2\nexit 1\n' > "$stand_in_dir/veritysetup"
  chmod +x "$stand_in_dir/veritysetup"
  tool_args+=(--tool "veritysetup=$stand_in_dir/veritysetup")
  echo "check-devices: NOTE veritysetup is not installed; planning uses a stand-in (a real build needs cryptsetup)"
fi
# The rust builder names bwrap, and gaia plan fails without it on PATH ("needs its input `bwrap`")
# even though planning never runs it. Where it isn't installed, planning gets a stand-in, as
# veritysetup does above; a build of a Rust component needs the real one (bubblewrap).
if ! command -v bwrap >/dev/null 2>&1; then
  stand_in_dir=${stand_in_dir:-$(mktemp -d)}
  printf '#!/bin/sh\necho "bwrap stand-in: planning only" >&2\nexit 1\n' > "$stand_in_dir/bwrap"
  chmod +x "$stand_in_dir/bwrap"
  tool_args+=(--tool "bwrap=$stand_in_dir/bwrap")
  echo "check-devices: NOTE bwrap is not installed; planning uses a stand-in (a build of a Rust component needs bubblewrap)"
fi

# ---- which commands: validate when gaia has it (and plan always), else plan ----

if "$gaia" --help 2>&1 | grep -Eq '^ +gaia validate '; then
  verb=validate
  checks="gaia validate, then gaia plan"
else
  verb=plan
  checks="gaia plan"
fi
echo "gaia: $("$gaia" --version 2>&1 | head -1), command: $verb, locked commit $gaia_commit" >&2

# ---- the slot root chain (PR B16) ------------------------------------------------
# A slot's root is opened by lemnos-slot-init before any module can be loaded from a root filesystem
# (Lemnos docs/slots.md: the initramfs loads dm modules only from /lib/modules, and Lemnos is removing
# that loader). So on every slot-booting family, the chain below is built in (=y) in the family's own
# kernel fragments, and no fragment of that family's boards may set one of them to m or "not set".
# The list lives here, once. "*" applies to every slot family; "FAMILY:" applies to that family only.
#   sha256: CRYPTO_SHA256 is dm-verity's hash (the verity table names sha256); the storage host is the
#   controller the root and data partitions sit on (the Raze: the CM5 eMMC host; the RK35xx: the DW MMC).
#   MMC_BLOCK: the block layer over MMC. Without it a host driver finds the card and no block device exists.
#   "*" is right while both slot families boot from MMC; an NVMe-booting family would name BLK_DEV_NVME instead.
#   The RK35xx boards have two storage hosts: the SD slot (DesignWare MMC) and the eMMC socket (DWCMSHC, the
#   rk3588 compatible; the RK3576's rk3576-dwcmshc binds the same driver through its fallback). Both are built in.
SLOT_CHAIN=(
  "*:BLK_DEV_DM" "*:DM_BUFIO" "*:DM_VERITY" "*:BLK_DEV_LOOP" "*:EROFS_FS" "*:EXT4_FS" "*:CRYPTO_SHA256"
  "*:MMC_BLOCK"
  "sun60i:MMC_SUNXI" "bcm2712:MMC_SDHCI_BRCMSTB"
  # the Pi firmware mailbox slot-init uses for RPIBOOT (kernel contract version 2, PR B29)
  "bcm2712:BRCM_CHAR_DRIVERS" "bcm2712:MAILBOX" "bcm2712:BCM2835_MBOX" "bcm2712:BCM_VCIO" "bcm2712:RASPBERRYPI_FIRMWARE"
  "rk35xx:MMC_DW_ROCKCHIP" "rk35xx:MMC_SDHCI_OF_DWCMSHC"
)
# A product whose recipe is one of these boots a slot (sd-image makes the slot file, slot is the slot recipe).
SLOT_RECIPES="sd-image|slot"

# Textual check: this reads the fragment files, not a resolved kernel config (the script has no kernel
# tree). The resolved-config proof is the table in docs/prs/B16.md, from a merge_config run per board.
# Usage: slot_chain_check <devices dir>. Prints one FAIL line per problem and returns 1 if any.
slot_chain_check() {
  local dev=$1 rc=0 fam sym want val f b p recipe fragfile
  local families=() boards=()
  # slot families: the family of every board that a slot-recipe product names
  while IFS= read -r p; do
    grep -qE "^recipe = \"($SLOT_RECIPES)\"" "$p" || continue
    local bpath
    bpath=$(grep -ho '"[^"]*board\.toml"' "$p" | tr -d '"' | head -1)
    [[ -n $bpath ]] || continue
    b=$(realpath -m "$(dirname "$p")/$bpath")
    fam=$(grep -ho '"[^"]*family\.toml"' "$b" | tr -d '"' | head -1)
    [[ -n $fam ]] || continue
    fam=$(basename "$(dirname "$(realpath -m "$(dirname "$b")/$fam")")")
    [[ " ${families[*]-} " == *" $fam "* ]] || families+=("$fam")
  done < <(find "$dev/products" -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort)
  for fam in ${families[@]+"${families[@]}"}; do
    local famfile=$dev/families/$fam/family.toml
    # the family's own kernel fragments, then every board's kernel fragments that extends it
    local frags=()
    while IFS= read -r f; do frags+=("$dev/families/$fam/$f"); done < <(
      grep -E '^fragments = ' "$famfile" | grep -ho '"[^"]*\.fragment"' | tr -d '"')
    local bfiles=()
    while IFS= read -r b; do
      [[ $(grep -c "families/$fam/family.toml" "$b") -gt 0 ]] || continue
      local bdir; bdir=$(dirname "$b")
      while IFS= read -r f; do bfiles+=("$bdir/$f"); done < <(
        grep -E '^fragments = ' "$b" | grep -ho '"[^"]*\.\(fragment\|config\)"' | tr -d '"')
    done < <(find "$dev/boards" -mindepth 2 -maxdepth 2 -name board.toml 2>/dev/null | sort)
    # required: each chain symbol that applies to this family must be =y in one of the family's fragments
    for entry in "${SLOT_CHAIN[@]}"; do
      [[ ${entry%%:*} == "*" || ${entry%%:*} == "$fam" ]] || continue
      sym=${entry#*:}
      local found=""
      for f in ${frags[@]+"${frags[@]}"}; do
        if [[ -f $f ]] && grep -qE "^CONFIG_${sym}=y\$" "$f"; then found=$f; fi
      done
      if [[ -z $found ]]; then
        echo "check-devices: FAIL families/$fam: CONFIG_$sym is not =y in the family's kernel fragments (found: none). The slot's root is opened before any module can load, so a slot of this family does not boot." >&2
        rc=1
      fi
    done
    # forbidden: no fragment of this family or of its boards may take a chain symbol to m or off
    for f in ${frags[@]+"${frags[@]}"} ${bfiles[@]+"${bfiles[@]}"}; do
      [[ -f $f ]] || continue
      for entry in "${SLOT_CHAIN[@]}"; do
        [[ ${entry%%:*} == "*" || ${entry%%:*} == "$fam" ]] || continue
        sym=${entry#*:}
        if grep -qE "^CONFIG_${sym}=m\$|^# CONFIG_${sym} is not set\$" "$f"; then
          val=$(grep -oE "^CONFIG_${sym}=m|^# CONFIG_${sym} is not set" "$f" | head -1)
          echo "check-devices: FAIL ${f#$dev/}: $val, but CONFIG_$sym must be =y on a slot family ($fam): the slot's root is opened before any module can load." >&2
          rc=1
        fi
      done
    done
  done
  return $rc
}

# Proves the slot root chain check catches (1) a board fragment that sets a chain symbol to m, and (2) a family
# fragment that lacks a required =y, both on a scratch copy of devices/; and passes on the real tree.
slot_chain_self_test() {
  local tmp rc=0
  tmp=$(mktemp -d)
  cp -a "$repo/devices" "$tmp/devices"
  echo "CONFIG_DM_VERITY=m" >> "$tmp/devices/boards/raze/kernel/raze.config"
  slot_chain_check "$tmp/devices" >/dev/null 2>&1 && rc=1
  cp -a "$repo/devices/." "$tmp/devices/"
  sed -i '/^CONFIG_EXT4_FS=y$/d' "$tmp/devices/families/rk35xx/assets/linux.fragment"
  slot_chain_check "$tmp/devices" >/dev/null 2>&1 && rc=1
  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && slot_chain_check "$repo/devices" >/dev/null 2>&1 || rc=1
  return $rc
}

# ---- the tree ------------------------------------------------------------------

[[ -d $root ]] || die "no such directory: $root"
root=$(cd "$root" && pwd)
cd "$root"
regen="scripts/check-devices.sh --write-locks"
[[ $root_given -eq 0 ]] || regen="$regen --root $root"
fail=0
bad() { echo "check-devices: FAIL $*" >&2; fail=1; }

# Each kind is one directory per item, holding its description file. A board directory that holds
# only app/ is recognition-only (a radio, a roboRIO) or a board whose description is not merged
# yet: it is skipped, not counted, and reported in one line. Every other bad entry is reported and
# the rest of the tree is still checked.
app_only=()
for kind in families:family.toml boards:board.toml components:component.toml products:product.toml; do
  dirname_=${kind%%:*}; desc=${kind#*:}
  [[ -d $dirname_ ]] || continue
  while IFS= read -r entry; do
    [[ $entry == .gitkeep ]] && continue
    if [[ ! -d $dirname_/$entry ]]; then
      bad "$dirname_/$entry: a file here; each $dirname_ item is its own directory (see devices/README.md)"
    elif [[ -f $dirname_/$entry/$desc ]]; then
      continue
    elif [[ $dirname_ == boards && $(ls -A "$dirname_/$entry") == app && -d $dirname_/$entry/app ]]; then
      app_only+=("$entry")
    else
      bad "$dirname_/$entry: no $desc in it (a board directory may hold only app/ when it has no $desc)"
    fi
  done < <(ls -A "$dirname_")
done
echo "check-devices: skipped ${#app_only[@]} app-only board director$([[ ${#app_only[@]} == 1 ]] && echo y || echo ies), not counted as boards${app_only[*]+: ${app_only[*]}}"

count() { [[ -d $1 ]] || { echo 0; return; }; find "$1" -mindepth 2 -maxdepth 2 -name "$2" | wc -l; }
nf=$(count families family.toml); nb=$(count boards board.toml)
nc=$(count components component.toml); np=$(count products product.toml)
echo "check-devices: checking $nf families, $nb boards, $nc components, $np products under $root with $checks"

# Gaia reads a file only through a product or a board's extends, so a family, board or
# component that nothing names would never be checked. Every one must be named.
used=$(mktemp)
errs=$(mktemp)
outs=$(mktemp)
for f in $(find boards products -mindepth 2 -maxdepth 2 \( -name board.toml -o -name product.toml \) 2>/dev/null | sort); do
  d=$(dirname "$f")
  { grep -ho '"[^"]*\.toml"' "$f" || true; } | tr -d '"' | while IFS= read -r ref; do
    realpath -m "$d/$ref"
  done
done >> "$used"
unbuilt=""
for f in $(find families boards components -mindepth 2 -maxdepth 2 -name '*.toml' 2>/dev/null | sort); do
  if ! grep -qxF "$(realpath -m "$f")" "$used"; then
    # gaia validate reads every file on its own, so a board no product names
    # yet (a record of a board whose sources aren't chosen) is still checked.
    # Without validate, gaia reads a file only through a product.
    if [ "$verb" = validate ]; then
      unbuilt="$unbuilt ${f%/*.toml}"
    else
      bad "$f: no product or board names it, so gaia never reads it"
    fi
  fi
done

# The slot root chain is enforced on every run (see slot_chain_check above).
slot_chain_check "$root" || fail=1
# The bring-up rule (PR B31): every bring-up product carries the watchdog and LED signs (tree-wide).
bringup_msg=$(bringup_rule "$root") || true
while IFS= read -r line; do
  case $line in
    NOTE\ *) echo "check-devices: $line" ;;
    "") ;;
    *) bad "$line" ;;
  esac
done <<<"$bringup_msg"

if [ -n "$unbuilt" ]; then
  echo "check-devices: NOTE in no product yet, validated but never planned:$unbuilt"
fi

# gaia validate stops at the first bad file and names its file and key, so each file gets its own
# run and every bad file is reported. The kind comes from the directory the file sits in.
validate_file() {  # <kind> <file>
  # A product's recipe names are checked against the locked Gaia's recipes/ (validate --recipes).
  local recipes_arg=()
  if [[ $1 == product && -d $recipes ]]; then recipes_arg=(--recipes "$recipes"); fi
  if "$gaia" validate "$2" --kind "$1" ${recipes_arg[@]+"${recipes_arg[@]}"} >"$outs" 2>&1; then
    cat "$outs"
  else
    bad "$2: gaia validate failed:"
    sed 's/^/  /' "$outs" >&2
  fi
}
if [[ $verb == validate ]]; then
  for f in $(find families -mindepth 2 -maxdepth 2 -name family.toml 2>/dev/null | sort); do
    validate_file family "$f"
  done
  for f in $(find boards -mindepth 2 -maxdepth 2 -name board.toml 2>/dev/null | sort); do
    validate_file board "$f"
  done
  for f in $(find components -mindepth 2 -maxdepth 2 -name component.toml 2>/dev/null | sort); do
    validate_file component "$f"
  done
fi

# Nothing unique per unit is in an image. Orion's identity, its trust file and its SSH host key, node
# logic's state, and a bring-up product's dropbear host key are made on the board at first boot, on the
# data partition. An image that carried one would give every board flashed from it the same identity,
# and Orion cannot see that from inside. So no description installs a file there: under the data
# partition (the one exception is the empty mount-point marker), under Orion's default state directory,
# or anything named like a host key or a node identity. This covers pending/ too.
while IFS= read -r hit; do
  bad "an image must not carry per-unit state or keys (made on the board at first boot): $hit"
done < <(grep -rnE '^[[:space:]]*to[[:space:]]*=[[:space:]]*"(/data/|/var/lib/orion|/etc/dropbear/|[^"]*(host_key|node-identity|\.ed25519)[^"]*")' \
           --include='*.toml' . 2>/dev/null | grep -vE 'to[[:space:]]*=[[:space:]]*"/data/\.mountpoint"' || true)

# Every product is validated (when gaia can), then its lock is checked, then it is planned.
#
# The plain check (no --write-locks) never asks the network. The committed gaia.lock is copied, and
# `gaia lock --check` must find it matching the descriptions; then `gaia lock` runs on a second copy
# with git and curl set to /bin/false, so any request it would make is an error. The regenerated copy
# must equal the committed lock byte for byte (this also re-hashes every directory source, which
# --check does not). Only then is the product planned, with the committed lock.
#
# --write-locks starts each product's lock from its committed lock and lets gaia lock resolve what that
# lock lacks (the network). A source whose host is unreachable or rate limited is retried a few times,
# then it is a FAILURE: the committed lock is left as it was.
offline_tool=$(command -v false)
locks=$(mktemp -d)
bench_dir=""
lock_tries="1 2 3"
trap 'rm -rf "$locks" "$used" "$errs" "$outs" ${stand_in_dir:+"$stand_in_dir"} ${bench_dir:+"$bench_dir"}' EXIT

# Writes the lock $2 for product $1 (the network for what it lacks). Retries a host that is down or
# limiting requests; any other error ends it at once. Returns 0 when the lock was written.
write_lock() {
  local product=$1 lockfile=$2 attempt rc network
  for attempt in $lock_tries; do
    rc=0
    timeout "${LOCK_TRY_SECONDS:-60}" "$gaia" lock "$product" --lock "$lockfile" >"$errs" 2>&1 || rc=$?
    [[ $rc -eq 0 ]] && return 0
    if [[ $rc -eq 124 ]]; then
      echo "gaia lock: timed out after ${LOCK_TRY_SECONDS:-60} s" >>"$errs"
    fi
    network=0
    if [[ $rc -eq 124 ]] || grep -qiE 'unable to access|could not resolve|failed to connect|could not connect|timed out|connection reset|temporary failure|HTTP 429|HTTP 50[0234]|too many requests' "$errs"; then
      network=1
    fi
    [[ $network -eq 1 && $attempt != "${lock_tries##* }" ]] || break
    sleep $((attempt * 5))
  done
  return 1
}

# A throwaway bench public key for the products that take one (see the header). Made once per run,
# in a temp dir; the private half is deleted at once, so only the public key exists.
bench_key_pub() {
  if [[ -z $bench_dir ]]; then
    command -v ssh-keygen >/dev/null || die "ssh-keygen is needed to make the throwaway bench key that bring-up products are planned with"
    bench_dir=$(mktemp -d)
    ssh-keygen -q -t ed25519 -N '' -C atlas-check-throwaway -f "$bench_dir/key" || die "cannot make the throwaway bench key"
    rm -f "$bench_dir/key"
  fi
  echo "$bench_dir/key.pub"
}
for product in $(find products -mindepth 2 -maxdepth 2 -name product.toml 2>/dev/null | sort); do
  pdir=$(dirname "$product")
  committed=$pdir/gaia.lock
  if [[ $verb == validate ]]; then
    validate_file product "$product"
  fi
  lockfile=$locks/$(echo "$product" | tr '/' '_').lock
  if [[ $write_locks -eq 1 ]]; then
    if [[ -f $committed ]]; then cp "$committed" "$lockfile"; else rm -f "$lockfile"; fi
    if ! write_lock "$product" "$lockfile"; then
      bad "$product: gaia lock could not resolve its sources (a host unreachable or rate limited, or a bad description); the committed lock was left as it was:"
      sed 's/^/  /' "$errs" >&2
      continue
    fi
    cp "$lockfile" "$committed"
    echo "check-devices: wrote $root/$committed"
  else
    if [[ ! -f $committed ]]; then
      bad "$product: no gaia.lock beside it; write it with: $regen"
      continue
    fi
    cp "$committed" "$lockfile"
    if ! "$gaia" lock "$product" --lock "$lockfile" --check >"$errs" 2>&1; then
      bad "$product: gaia.lock beside it does not match the descriptions; regenerate with: $regen"
      sed 's/^/  /' "$errs" >&2
      continue
    fi
    cp "$committed" "$lockfile.offline"
    if ! "$gaia" lock "$product" --lock "$lockfile.offline" --tool "git=$offline_tool" --tool "curl=$offline_tool" >"$errs" 2>&1; then
      bad "$product: gaia lock needs the network for this lock (a source is not locked for what its description says, or a directory changed to a source it did not hash); regenerate with: $regen"
      sed 's/^/  /' "$errs" >&2
      continue
    fi
    if ! cmp -s "$lockfile.offline" "$committed"; then
      bad "$product: gaia.lock beside it differs from what gaia lock writes; regenerate with: $regen"
      { diff "$committed" "$lockfile.offline" || true; } | sed 's/^/  /' >&2
      continue
    fi
  fi
  # The Lemnos rule: every Lemnos source in this product's lock is one rev.
  if ! lemnos_msg=$(lemnos_two_revs "$committed"); then
    bad "$product: Lemnos is locked to more than one rev in this product, $lemnos_msg: every Lemnos component of a product must be locked to one rev"
  fi
  # The Lemnos board file rule: a product that carries lemnosd names its board's Lemnos board file (PR B26).
  if grep -q '/lemnosd/component\.toml"' "$product"; then
    if ! lemnos_board_msg=$(lemnos_board_rule "$product"); then
      bad "$product: $lemnos_board_msg"
    fi
  fi
  # A product whose components take a named input (`{ input = NAME }`) is planned with the input given;
  # gaia refuses an input that nothing uses, so only those products get it.
  plan_args=()
  if grep -rqs 'input = "bench-key"' "$pdir"; then
    plan_args=(--input "bench-key=$(bench_key_pub)")
  fi
  if "$gaia" plan "$product" --lock "$committed" --recipes "$recipes" "${tool_args[@]}" ${plan_args[@]+"${plan_args[@]}"} >"$outs" 2>&1; then
    echo "check-devices: ok $product (gaia plan)"
  else
    bad "$product: gaia plan failed:"
    sed 's/^/  /' "$outs" >&2
  fi
done

if [[ $fail -ne 0 ]]; then
  echo "check-devices: FAILED" >&2
  exit 1
fi
echo "check-devices: passed ($nf families, $nb boards, $nc components, $np products)"

# The default run also proves the checker still catches a broken file: the good fixtures must
# pass and the broken one must fail. Not run for --root, which names a tree to check alone.
if [[ $root_given -eq 0 ]]; then
  out=$(mktemp)
  for good in good good-f8 good-rust; do
    if ! "$self" --root "$fixtures/$good" >"$out" 2>&1; then
      cat "$out" >&2; die "self-check: the $good fixture no longer passes"
    fi
  done
  rc=0
  "$self" --root "$fixtures/broken" >"$out" 2>&1 || rc=$?
  if ! broken_failed_as_it_must "$out" "$rc"; then
    cat "$out" >&2
    die "self-check: the broken fixture passed, or failed without naming its bad key, so the checker no longer catches a broken file"
  fi
  rm -f "$out"
  lemnos_rule_self_test || die "self-check: the Lemnos one-rev rule no longer refuses two revs in one product"
  lemnos_board_rule_self_test || die "self-check: the Lemnos board file rule no longer refuses a lemnosd product without a board file"
  slot_chain_self_test || die "self-check: the slot root chain check no longer refuses a module or a missing =y"
  bringup_rule_self_test || die "self-check: the bring-up rule no longer refuses a bring-up product without its signs"
  # The version gate: a binary that prints another commit, or none, is refused, and the refusal names the pin.
  fake=$(mktemp -d)
  vout=$(mktemp)
  other=1111111111111111111111111111111111111111
  printf '#!/bin/sh\necho "gaia 2.0.6-dev (%s)"\n' "$other" > "$fake/gaia-other"
  printf '#!/bin/sh\necho "gaia 2.0.6-dev"\n' > "$fake/gaia-none"
  chmod +x "$fake/gaia-other" "$fake/gaia-none"
  for fb in gaia-other gaia-none; do
    rc=0
    GAIA_BIN="$fake/$fb" "$self" --root "$fixtures/good" >"$vout" 2>&1 || rc=$?
    if [[ $rc -eq 0 ]] || ! grep -q "devices/gaia.lock" "$vout" || ! grep -q "$gaia_commit" "$vout"; then
      cat "$vout" >&2; rm -rf "$fake" "$vout"; die "self-check: a $fb binary was not refused naming the pinned commit"
    fi
  done
  rm -rf "$fake" "$vout"
  echo "check-devices: self-check passed (good fixtures pass, broken fixture fails, a wrong or unknown gaia commit is refused)"
fi
