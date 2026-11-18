#!/usr/bin/env bash
# The watchdog owner rule (PR WDOG-OWNER). Two questions per product:
#
#   1. OWNERS: exactly one thing feeds the hardware watchdog. Never both, never neither.
#        Lemnos   the board file the board names ([descriptions] lemnos) has a [[devices]] block with
#                 driver = "hw-watchdog" and keepalive = true, AND a [[hardware]] block with controller = "watchdog",
#                 AND the product lists lemnosd in its [components] (the daemon that feeds it). A keepalive with no
#                 lemnosd is declared, not fed: it is not an owner.
#        systemd  the effective RuntimeWatchdogSec is armed (not empty, 0, 0s, off, no, false, infinity), or the
#                 kernel command line says systemd.watchdog-sec=N with N armed.
#        feeder   any other file the product installs that writes /dev/watchdog* on a line that is not a comment.
#   2. ARMING: does something in the boot chain ARM the watchdog? If it does and no owner feeds it, the board resets
#      in a loop: a FINDING for any product. Only explicit lines in the tree count (coordinator's ruling 2026-11-16):
#      a U-Boot config line CONFIG_WATCHDOG_AUTOSTART=y, a `wdt` command in a boot script, the initramfs's Lemnos slot
#      init. Where the answer depends on resolved U-Boot defaults (not in the tree; Gaia --config-only does not resolve
#      U-Boot's .config) the output is "bootloader arming: NOT CHECKED (resolved U-Boot config needs a build)".
#
# Layers:
#   INPUTS (always): the files a product installs (its components' [[install]] blocks; the overlays its fragments name
#   by BR2_ROOTFS_OVERLAY) and the board's Lemnos board file.
#   BUILT (optional): ATLAS-side root of the image's systemd config = Gaia's <image>.system merge tree (what D1 must
#   pass). Its effective systemd config and feeders replace the inputs' for systemd and feeders; a disagreement with
#   the inputs is a finding. Built U-Boot: a directory with u-boot.config (beside u-boot.bin in the bootloader output)
#   replaces the explicit U-Boot lines for the arming question.
#
# SLOT PRODUCT: recipe slot, slot-boot or slot-disk. Its findings are FINDINGs. An sd-image (or any other recipe)
# product is not a slot product: its owner state is one NOTE line, never a gate failure; its ARMING finding is still a
# FINDING (a reset loop is a reset loop for any product).
#
# Usage:
#   scripts/check-watchdog-owner.sh [--root DEVICES_DIR] [--built NAME=SYSTEM_TREE]... [--built-bootloader NAME=DIR]...
#   scripts/check-watchdog-owner.sh --self-test
# Environment (read by scripts/check-devices.sh too): ATLAS_BUILT_TREES and ATLAS_BUILT_BOOTLOADERS, whitespace-separated
# NAME=DIR pairs. NAME is the product directory name under devices/products/.
# Exit 1 on any FINDING.

WD_SLOT_RE='^recipe = "(slot|slot-boot|slot-disk)"'

# ---- file readers ----------------------------------------------------------------------------------------------------

wd_recipe() { awk -F'"' '/^recipe = "/ { print $2; exit }' "$1"; }

wd_is_slot() { grep -qE "$WD_SLOT_RE" "$1"; }

# The value of key in [section] of a TOML file (quoted strings unquoted), or nothing.
wd_section_value() {  # <file> <section> <key>
  awk -v sec="[$2]" -v key="$3" '
    /^[[:space:]]*\[/ { s = $0; gsub(/[[:space:]]/, "", s); on = (s == sec); next }
    on && $0 ~ "^[[:space:]]*" key "[[:space:]]*=" {
      v = $0; sub(/^[^=]*=[[:space:]]*/, "", v); sub(/[[:space:]]*(#.*)?$/, "", v); gsub(/^"|"$/, "", v)
      print v; exit
    }' "$1"
}

# A top-level key's quoted value (first occurrence), or nothing.
wd_top_value() {  # <file> <key>
  { grep -o "^$2 = \"[^\"]*\"" "$1" || true; } | head -1 | cut -d'"' -f2
}

# The board directory a product names (its board = "..."), absolute, or nothing.
wd_board_dir() {  # <product.toml>
  local board
  board=$(wd_top_value "$1" board)
  [[ -n $board ]] || return 0
  dirname "$(realpath -m "$(dirname "$1")/$board")"
}

# The family directory a board extends (its extends = "..."), absolute, or nothing.
wd_family_dir() {  # <board dir>
  local ext
  ext=$(wd_top_value "$1/board.toml" extends)
  [[ -n $ext ]] || return 0
  dirname "$(realpath -m "$1/$ext")"
}

# The absolute component.toml paths a product lists in [components], one per line.
wd_components() {  # <product.toml>
  local pdir rel
  pdir=$(dirname "$1")
  awk '/^\[/ { on = ($0 == "[components]"); next } on && /=/ { v = $0; sub(/^[^"]*"/, "", v); sub(/".*$/, "", v); print v }' "$1" |
    while IFS= read -r rel; do realpath -m "$pdir/$rel"; done
  return 0
}

# The [[install]] blocks of a component.toml: "to<TAB>source" per block (source relative to the component).
wd_installs() {  # <component.toml>
  awk '
    function emit() { if (to != "" && src != "") printf "%s\t%s\n", to, src; to = ""; src = "" }
    /^\[\[install\]\]/ { emit(); inst = 1; next }
    /^\[/ { emit(); inst = 0; next }
    inst && /^[[:space:]]*to[[:space:]]*=/ { to = $0; sub(/^[^"]*"/, "", to); sub(/".*$/, "", to) }
    inst && /^[[:space:]]*from[[:space:]]*=/ {
      f = $0
      if (f ~ /file[[:space:]]*=/) { sub(/.*file[[:space:]]*=[[:space:]]*"/, "", f); sub(/".*$/, "", f); src = f }
      else if (f ~ /^[[:space:]]*from[[:space:]]*=[[:space:]]*"/) { sub(/^[^"]*"/, "", f); sub(/".*$/, "", f); src = f }
      else src = ""
    }
    END { emit() }' "$1"
}

# The BR2_ROOTFS_OVERLAY directories the product's own fragments name (its fragments = [...] list), absolute.
wd_overlays() {  # <product.toml>
  local pdir frag abs dir ov
  pdir=$(dirname "$1")
  while IFS= read -r frag; do
    abs=$(realpath -m "$pdir/$frag")
    [[ -f $abs ]] || continue
    dir=$(dirname "$abs")
    while IFS= read -r ov; do
      ov=${ov#@here@}
      printf '%s\n' "$(realpath -m "$dir/$ov")"
    done < <({ grep -o '^BR2_ROOTFS_OVERLAY="@here@[^"]*"' "$abs" || true; } | cut -d'"' -f2)
  done < <({ grep -o 'fragments = \[[^]]*\]' "$1" || true; } | head -1 | grep -o '"[^"]*"' | tr -d '"')
  return 0
}

# Input pairs of a product: "dest<TAB>source" for each file it installs (components, then named overlays), once each.
wd_input_pairs() {  # <product.toml>
  local p=$1 comp cdir to src ov rel
  while IFS= read -r comp; do
    [[ -f $comp ]] || continue
    cdir=$(dirname "$comp")
    while IFS=$'\t' read -r to src; do
      printf '%s\t%s\n' "$to" "$cdir/$src"
    done < <(wd_installs "$comp")
  done < <(wd_components "$p")
  local ovs=()
  while IFS= read -r ov; do ovs+=("$ov"); done < <(wd_overlays "$p")
  for ov in $(printf '%s\n' ${ovs[@]+"${ovs[@]}"} | sort -u); do
    [[ -d $ov ]] || continue
    while IFS= read -r rel; do
      printf '/%s\t%s\n' "$rel" "$ov/$rel"
    done < <(cd "$ov" && find . -type f | sed 's#^\./##' | sort)
  done | sort -u
  return 0
}

# Pairs of a rootfs (the systemd layer): the systemd config and the kernel command lines the image holds.
wd_built_pairs() {  # <system tree>
  local root=$1 f rel
  for f in "$root"/etc/systemd/system.conf "$root"/usr/lib/systemd/system.conf \
           "$root"/etc/systemd/system.conf.d/*.conf "$root"/run/systemd/system.conf.d/*.conf \
           "$root"/usr/lib/systemd/system.conf.d/*.conf \
           "$root"/boot/cmdline.txt "$root"/boot/*/cmdline.txt "$root"/boot/extlinux/extlinux.conf; do
    [[ -f $f ]] || continue
    rel=${f#"$root"}
    printf '%s\t%s\n' "$rel" "$f"
  done
  return 0
}

# Prints the effective RuntimeWatchdogSec of a pairs file as "value<TAB>where<TAB>source<TAB>line". systemd's order: the
# main file (/etc/systemd/system.conf, else /usr/lib/systemd/system.conf), then the drop-ins sorted by name (a name in
# /etc masks /run, which masks /usr/lib); the last [Manager] assignment wins; an empty value is off. A kernel command
# line's systemd.watchdog-sec= is applied after the files. Prints nothing when no file sets it.
wd_runtime_watchdog() {  # <pairs file: dest<TAB>source>
  local pairs=$1 dest src main_dest="" main_src="" name rank item d s hit value ln where="" source="" line=""
  local -A best_rank=() best_dest=() best_src=()
  while IFS=$'\t' read -r dest src; do
    case $dest in
      /etc/systemd/system.conf) main_dest=$dest; main_src=$src ;;
      /usr/lib/systemd/system.conf) if [[ -z $main_src ]]; then main_dest=$dest; main_src=$src; fi ;;
      /etc/systemd/system.conf.d/*.conf|/run/systemd/system.conf.d/*.conf|/usr/lib/systemd/system.conf.d/*.conf)
        name=${dest##*/}
        case $dest in /etc/*) rank=3 ;; /run/*) rank=2 ;; *) rank=1 ;; esac
        if [[ -z ${best_rank[$name]:-} || ${best_rank[$name]} -lt $rank ]]; then
          best_rank[$name]=$rank; best_dest[$name]=$dest; best_src[$name]=$src
        fi ;;
    esac
  done < "$pairs"
  local seq=()
  [[ -n $main_src ]] && seq+=("$main_dest"$'\t'"$main_src")
  for name in $(printf '%s\n' "${!best_dest[@]}" | sort); do
    seq+=("${best_dest[$name]}"$'\t'"${best_src[$name]}")
  done
  for item in ${seq[@]+"${seq[@]}"}; do
    d=${item%%$'\t'*}; s=${item#*$'\t'}
    hit=$(awk '
      /^[[:space:]]*\[/ { h = $0; gsub(/[[:space:]]/, "", h); mgr = (h == "[Manager]"); next }
      mgr && /^[[:space:]]*RuntimeWatchdogSec[[:space:]]*=/ {
        v = $0; sub(/^[^=]*=[[:space:]]*/, "", v); sub(/[[:space:]]+$/, "", v); last = v; ln = NR; found = 1
      }
      END { if (found) printf "%s\t%d\n", last, ln }' "$s")
    if [[ -n $hit ]]; then
      value=${hit%%$'\t'*}; ln=${hit##*$'\t'}
      where=$d; source=$s; line=$ln
    fi
  done
  while IFS=$'\t' read -r d s; do
    case $d in *cmdline.txt|*extlinux.conf) ;; *) continue ;; esac
    hit=$(awk '{ n = match($0, /systemd\.watchdog-sec=[^ ]*/); if (n) { v = substr($0, RSTART, RLENGTH); sub(/^[^=]*=/, "", v); last = v; ln = NR } }
      END { if (last != "") printf "%s\t%d\n", last, ln }' "$s")
    if [[ -n $hit ]]; then
      value=${hit%%$'\t'*}; line=${hit##*$'\t'}
      where="systemd.watchdog-sec on the kernel command line in $d"; source=$s
    fi
  done < "$pairs"
  if [[ -n $where ]]; then
    printf '%s\t%s\t%s\t%s\n' "$value" "$where" "$source" "$line"
  fi
  return 0
}

# Whether a RuntimeWatchdogSec value arms the watchdog: off for empty, off, no, false, infinity, none, or zero in any unit.
wd_arms() {  # <value>
  local v=${1,,}
  case $v in ""|off|no|false|infinity|none) return 1 ;; esac
  [[ $v =~ ^0*(\.0*)?(us|ms|s|sec|min|h|d)?$ ]] && return 1
  return 0
}

# "keep <line>" for each Lemnos hw-watchdog keepalive device, "hw <line>" for each [[hardware]] controller = "watchdog".
wd_lemnos_blocks() {  # <board file>
  awk '
    function flush() {
      if (cur == "[[devices]]" && drv && kl) print "keep " kl
      if (cur == "[[hardware]]" && wd) print "hw " wl
    }
    /^[[:space:]]*\[/ { flush(); cur = $0; gsub(/[[:space:]]/, "", cur); drv = 0; kl = 0; wd = 0; wl = 0; next }
    cur == "[[devices]]" && /^[[:space:]]*driver[[:space:]]*=[[:space:]]*"hw-watchdog"/ { drv = 1 }
    cur == "[[devices]]" && /^[[:space:]]*keepalive[[:space:]]*=[[:space:]]*true[[:space:]]*(#.*)?$/ { kl = NR }
    cur == "[[hardware]]" && /^[[:space:]]*controller[[:space:]]*=[[:space:]]*"watchdog"/ { wd = 1; wl = NR }
    END { flush() }' "$1"
}

# Feeders among a product's input files: "<path relative to devices>:<line>" for each line that writes /dev/watchdog*
# (a comment does not count). Pairs on stdin.
wd_input_feeders() {  # <devices dir>
  # One grep over every input file, in the pairs' order (a file listed twice is read twice, as before). -I skips a binary
  # file, as the per-file `grep -qI ''` test did; a file that does not exist is left out. A path holding ':' cannot be
  # split from grep's "file:line:" output, so such a list takes the file-by-file form.
  local devdir=$1 dest src files=() f
  while IFS=$'\t' read -r dest src; do [[ -f $src ]] && files+=("$src"); done
  [[ ${#files[@]} -gt 0 ]] || return 0
  for f in "${files[@]}"; do
    if [[ $f == *:* ]]; then printf '%s\n' "${files[@]}" | wd_input_feeders_each "$devdir"; return 0; fi
  done
  { grep -nIH -- '/dev/watchdog' "${files[@]}" 2>/dev/null || true; } | awk -v dev="$devdir/" '
    match($0, /:[0-9]+:/) {
      file = substr($0, 1, RSTART - 1); n = substr($0, RSTART + 1, RLENGTH - 2); text = substr($0, RSTART + RLENGTH)
      if (text ~ /^[[:space:]]*[#;]/) next
      if (index(file, dev) == 1) file = substr(file, length(dev) + 1)
      print file ":" n
    }'
  return 0
}

# The file-by-file form of wd_input_feeders, for paths it cannot split (file paths on stdin, one per line).
wd_input_feeders_each() {  # <devices dir>
  local devdir=$1 src hit
  while IFS= read -r src; do
    grep -qI '' "$src" 2>/dev/null || continue
    { grep -n '/dev/watchdog' "$src" || true; } | grep -vE '^[0-9]+:[[:space:]]*[#;]' | while IFS= read -r hit; do
      printf '%s:%s\n' "${src#"$devdir"/}" "${hit%%:*}"
    done
  done
  return 0
}

# Feeders in a system tree's text files under etc/, usr/lib, usr/bin and usr/sbin: "<path in the tree>:<line>".
wd_built_feeders() {  # <system tree>
  local root=$1 d hit file
  for d in etc usr/lib usr/bin usr/sbin; do
    [[ -d $root/$d ]] || continue
    grep -rIn '/dev/watchdog' "$root/$d" 2>/dev/null | grep -vE ':[0-9]+:[[:space:]]*[#;]' | while IFS= read -r hit; do
      file=${hit%%:*}; file=${file#"$root"}
      printf '%s:%s\n' "$file" "$(printf '%s' "${hit#*:}" | cut -d: -f1)"
    done
  done
  return 0
}

# ---- the records of one product ----------------------------------------------------------------------------------------
# Each record is one line: TYPE<TAB>TEXT. TYPE is one of:
#   owner-lemnos | owner-systemd | owner-feeder   an owner (TEXT names it with its file:line)
#   soft                                          an owner-side finding (no owner, unfed keepalive, a disagreement)
#   hard                                          a finding for any product (a broken board reference, a reset loop)
#   armed                                         the boot chain arms the watchdog (TEXT: what; where)
#   unarmed                                       the stated U-Boot config turns the autostart off (TEXT: where)
#   notchecked                                    arming not determinable from the tree (TEXT: why)
#   built                                         the built systemd tree was read (TEXT: its root)

# Lemnos: the board file's keepalive, its [[hardware]] line, and whether lemnosd is installed to feed it.
wd_lemnos_records() {  # <devices dir> <product.toml>
  local devdir=$1 p=$2 bdir name bfile rel keeps hws l c has_lemnosd=0
  bdir=$(wd_board_dir "$p")
  name=
  [[ -n $bdir ]] && name=$(wd_section_value "$bdir/board.toml" descriptions lemnos)
  [[ -n $name ]] || return 0
  bfile=$bdir/$name
  if [[ ! -f $bfile ]]; then
    printf 'hard\tits board names the Lemnos board file %s, which is not beside the board\n' "$name"; return 0
  fi
  rel=${bfile#"$devdir"/}
  keeps=$(wd_lemnos_blocks "$bfile" | awk '$1 == "keep" { print $2 }')
  hws=$(wd_lemnos_blocks "$bfile" | awk '$1 == "hw" { print $2 }')
  [[ -n $keeps ]] || return 0
  if [[ -z $hws ]]; then
    printf 'hard\t%s has a hw-watchdog keepalive (line %s) but no [[hardware]] controller = "watchdog" line: the keepalive has no watchdog to feed\n' "$rel" "$(head -1 <<<"$keeps")"
    return 0
  fi
  while IFS= read -r c; do
    [[ $c == */components/lemnosd/component.toml ]] && has_lemnosd=1
  done < <(wd_components "$p")
  if (( has_lemnosd == 0 )); then
    while IFS= read -r l; do
      printf 'soft\t%s keeps a hw-watchdog keepalive (line %s) that nothing feeds: its components do not list lemnosd, so the keepalive is not an owner\n' "$rel" "$l"
    done <<<"$keeps"
    return 0
  fi
  while IFS= read -r l; do
    printf 'owner-lemnos\tLemnos hw-watchdog keepalive (%s line %s)\n' "$rel" "$l"
  done <<<"$keeps"
}

# systemd and feeders: the built system tree when given (it replaces the inputs), else the inputs.
wd_systemd_records() {  # <devices dir> <product.toml> [system tree]
  local devdir=$1 p=$2 built=${3:-} ipairs bpairs sys_in sys_b in_v="" b_v="" f
  ipairs=$(mktemp)
  wd_input_pairs "$p" > "$ipairs"
  sys_in=$(wd_runtime_watchdog "$ipairs")
  [[ -n $sys_in ]] && in_v=$(cut -f1 <<<"$sys_in")
  if [[ -z $built ]]; then
    if [[ -n $sys_in ]] && wd_arms "$in_v"; then
      printf 'owner-systemd\t%s\n' "$(wd_systemd_text "$devdir" "$sys_in")"
    fi
    while IFS= read -r f; do
      [[ -n $f ]] && printf 'owner-feeder\tanother feeder: %s writes /dev/watchdog\n' "$f"
    done < <(wd_input_feeders "$devdir" < "$ipairs")
    rm -f "$ipairs"
    return 0
  fi
  rm -f "$ipairs"
  if [[ ! -d $built ]]; then
    printf 'hard\tthe built system tree %s is not a directory: its config cannot be read\n' "$built"
    return 0
  fi
  bpairs=$(mktemp)
  wd_built_pairs "$built" > "$bpairs"
  sys_b=$(wd_runtime_watchdog "$bpairs")
  [[ -n $sys_b ]] && b_v=$(cut -f1 <<<"$sys_b")
  rm -f "$bpairs"
  if [[ -n $sys_b ]] && wd_arms "$b_v"; then
    printf 'owner-systemd\t%s\n' "$(wd_systemd_text "$built" "$sys_b")"
  fi
  while IFS= read -r f; do
    [[ -n $f ]] && printf 'owner-feeder\tanother feeder: %s writes /dev/watchdog (in the built tree)\n' "$f"
  done < <(wd_built_feeders "$built")
  local ia=0 ba=0
  [[ -n $sys_in ]] && wd_arms "$in_v" && ia=1
  [[ -n $sys_b ]] && wd_arms "$b_v" && ba=1
  if (( ia != ba )); then
    local it="no RuntimeWatchdogSec" bt="no RuntimeWatchdogSec"
    [[ -n $sys_in ]] && it=$(wd_systemd_text "$devdir" "$sys_in")
    [[ -n $sys_b ]] && bt=$(wd_systemd_text "$built" "$sys_b")
    printf 'soft\tits built config and its inputs disagree on systemd'"'"'s watchdog (inputs: %s; built: %s); the image is not what the product says\n' "$it" "$bt"
  fi
  printf 'built\t%s\n' "$built"
}

# The text of a systemd setting: "RuntimeWatchdogSec=V (file, line N)" relative to the root it is read from.
wd_systemd_text() {  # <root> <wd_runtime_watchdog line>
  local root=$1 value where source line shown
  IFS=$'\t' read -r value where source line <<<"$2"
  shown=${source#"$root"/}
  if [[ $where == systemd.watchdog-sec* ]]; then
    printf 'systemd.watchdog-sec=%s (%s, line %s)' "$value" "$where" "$line"
  else
    printf 'RuntimeWatchdogSec=%s (%s, line %s)' "$value" "$shown" "$line"
  fi
}

# Boot-chain arming: the initramfs's Lemnos slot init, U-Boot's explicit lines and boot scripts, and a built U-Boot
# config. Explicit lines only; anything that depends on resolved U-Boot defaults is NOT CHECKED.
wd_arming_records() {  # <devices dir> <product.toml> [built bootloader dir]
  local devdir=$1 p=$2 bl=${3:-} bdir famdir f hit uboot_present=0 explicit_off="" file
  bdir=$(wd_board_dir "$p")
  if grep -qE '^initramfs = \[.*"lemnos-slot-init"' "$p"; then
    printf 'armed\tLemnos slot init in the initramfs arms the watchdog (its timeout is in Lemnos, not in this tree) at %s line %s\n' \
      "${p#"$devdir"/}" "$(grep -nE '^initramfs' "$p" | head -1 | cut -d: -f1)"
  fi
  famdir=$(wd_family_dir "$bdir")
  local sources=()
  for d in ${famdir:+"$famdir/assets"} ${bdir:+"$bdir"}; do
    [[ -d $d ]] || continue
    while IFS= read -r f; do sources+=("$f"); done < <(find "$d" -type f \( -name '*u-boot*' -o -name '*uboot*' -o -name 'boot.cmd' -o -name '*.cmd' \) 2>/dev/null | sort)
  done
  for f in ${sources[@]+"${sources[@]}"}; do
    case $f in *u-boot*|*uboot*) uboot_present=1 ;; esac
    hit=$({ grep -nE '^CONFIG_WATCHDOG_AUTOSTART=y' "$f" || true; } | head -1)
    if [[ -n $hit ]]; then
      printf 'armed\tU-Boot config CONFIG_WATCHDOG_AUTOSTART=y arms the watchdog at boot, in %s line %s\n' "${f#"$devdir"/}" "${hit%%:*}"
    fi
    hit=$({ grep -nE '^[[:space:]]*wdt[[:space:]]' "$f" || true; } | head -1)
    if [[ -n $hit ]]; then
      printf 'armed\tboot script command "wdt" in %s line %s\n' "${f#"$devdir"/}" "${hit%%:*}"
    fi
    hit=$({ grep -nE '^# CONFIG_WATCHDOG_AUTOSTART is not set' "$f" || true; } | head -1)
    [[ -n $hit && -z $explicit_off ]] && explicit_off="${f#"$devdir"/} line ${hit%%:*}"
  done
  if (( uboot_present )); then
    if [[ -n $bl ]]; then
      if [[ -f $bl/u-boot.config ]]; then
        hit=$({ grep -nE '^CONFIG_WATCHDOG_AUTOSTART=y' "$bl/u-boot.config" || true; } | head -1)
        if [[ -n $hit ]]; then
          printf 'armed\tbuilt U-Boot config CONFIG_WATCHDOG_AUTOSTART=y arms the watchdog at boot, in %s line %s\n' "$bl/u-boot.config" "${hit%%:*}"
        elif grep -qE '^# CONFIG_WATCHDOG_AUTOSTART is not set' "$bl/u-boot.config"; then
          printf 'unarmed\tbuilt U-Boot config: CONFIG_WATCHDOG_AUTOSTART is not set (%s)\n' "$bl/u-boot.config"
        else
          printf 'notchecked\tbuilt U-Boot config states neither CONFIG_WATCHDOG_AUTOSTART value (%s)\n' "$bl/u-boot.config"
        fi
      else
        printf 'notchecked\tresolved U-Boot config needs a build: no u-boot.config in %s\n' "$bl"
      fi
    elif [[ -n $explicit_off ]]; then
      printf 'unarmed\tthe stated U-Boot config turns the autostart off (%s)\n' "$explicit_off"
    else
      printf 'notchecked\tresolved U-Boot config needs a build\n'
    fi
  else
    printf 'notchecked\tno U-Boot fragment or boot script in the tree for this board: its boot firmware'"'"'s watchdog is not stated\n'
  fi
}

# ---- one product ------------------------------------------------------------------------------------------------------

# Joins words: "A", "A and B", "A, B and C".
wd_join() {
  local out="" n=$# i=0 x
  for x in "$@"; do
    i=$((i + 1))
    if [[ -z $out ]]; then out=$x
    elif [[ $i -eq $n ]]; then out="$out and $x"
    else out="$out, $x"; fi
  done
  printf '%s' "$out"
}

# Checks one product. Prints NOTE and FINDING lines; returns 1 when a FINDING is for this product.
wd_check_product() {  # <devices dir> <product.toml> [system tree] [bootloader dir]
  local devdir=$1 p=$2 sys=${3:-} bl=${4:-} pname recipe slot=0 rec type text
  local owners=() owner_kinds=() softs=() hards=() arms=() unk=() unarm=() built_root="" rc=0 n tail=()
  pname=$(basename "$(dirname "$p")")
  recipe=$(wd_recipe "$p")
  wd_is_slot "$p" && slot=1
  while IFS=$'\t' read -r type text; do
    case $type in
      owner-lemnos) owners+=("$text"); owner_kinds+=("Lemnos") ;;
      owner-systemd) owners+=("$text"); owner_kinds+=("systemd") ;;
      owner-feeder) owners+=("$text"); owner_kinds+=("feeder") ;;
      soft) softs+=("$text") ;;
      hard) hards+=("$text") ;;
      armed) arms+=("$text") ;;
      unarmed) unarm+=("$text") ;;
      notchecked) unk+=("$text") ;;
      built) built_root=$text ;;
    esac
  done < <({ wd_lemnos_records "$devdir" "$p"; wd_systemd_records "$devdir" "$p" "$sys"; wd_arming_records "$devdir" "$p" "$bl"; } | sed -n 'p')
  n=${#owners[@]}

  # Owners.
  local state
  if (( n == 0 )); then
    state="no watchdog owner"
  elif (( n >= 2 )); then
    state="two owners: $(wd_join "${owners[@]}") — remove one"
  else
    state="${owner_kinds[0]} owner: ${owners[0]}"
  fi
  if (( slot )); then
    local line
    if (( n == 0 )); then
      echo "FINDING product $pname: $state: give it exactly one, a Lemnos hw-watchdog keepalive in its board file (with lemnosd installed) or RuntimeWatchdogSec in systemd's config"; rc=1
    elif (( n >= 2 )); then
      echo "FINDING product $pname: $state"; rc=1
    elif [[ -z $built_root ]]; then
      echo "NOTE product $pname: inputs say ${owner_kinds[0]}; built config unverified"
    else
      echo "NOTE product $pname: one owner: ${owners[0]} (built config checked: $built_root)"
    fi
    for line in ${softs[@]+"${softs[@]}"}; do echo "FINDING product $pname: $line"; rc=1; done
    for line in ${hards[@]+"${hards[@]}"}; do echo "FINDING product $pname: $line"; rc=1; done
  else
    local t="NOTE product $pname ($recipe, not a slot product): $state"
    for line in ${softs[@]+"${softs[@]}"}; do t+="; $line"; done
    echo "$t"
    for line in ${hards[@]+"${hards[@]}"}; do echo "FINDING product $pname: $line"; rc=1; done
  fi

  # Arming: a FINDING for any product when something arms the watchdog and no owner feeds it.
  local a
  for a in ${arms[@]+"${arms[@]}"}; do
    if (( n == 0 )); then
      echo "FINDING product $pname: the boot chain arms the watchdog ($a) and no owner feeds it: the board would reset in a loop"; rc=1
    else
      echo "NOTE product $pname: bootloader arming: armed ($a); fed by ${owner_kinds[0]}"
    fi
  done
  for a in ${unarm[@]+"${unarm[@]}"}; do echo "NOTE product $pname: bootloader arming: not armed ($a)"; done
  for a in ${unk[@]+"${unk[@]}"}; do
    if [[ $a == "resolved U-Boot config needs a build" ]]; then
      echo "NOTE product $pname: bootloader arming: NOT CHECKED (resolved U-Boot config needs a build)"
    else
      echo "NOTE product $pname: bootloader arming: NOT CHECKED ($a)"
    fi
  done
  return $rc
}

# The rule over every product of <devices dir>. Built trees: "NAME=SYSTEM_TREE" and "NAME=BOOTLOADER_DIR" arguments.
watchdog_owner_rule() {  # <devices dir> [NAME=SYSTEM_TREE]... -- [NAME=BOOTLOADER_DIR]...
  local devdir rc=0 p pname spec name dir known sep=0
  devdir=$(realpath -m "$1")
  shift
  local -A builds=() bootloaders=()
  for spec in "$@"; do
    if [[ $spec == -- ]]; then sep=1; continue; fi
    name=${spec%%=*}; dir=${spec#*=}
    [[ $name != "$spec" && -n $name ]] || { echo "FINDING built tree '$spec' is not NAME=DIR"; rc=1; continue; }
    if (( sep )); then bootloaders[$name]=$dir; else builds[$name]=$dir; fi
  done
  known=" "
  while IFS= read -r p; do
    [[ -n $p ]] || continue
    pname=$(basename "$(dirname "$p")")
    known+="$pname "
    wd_check_product "$devdir" "$p" "${builds[$pname]:-}" "${bootloaders[$pname]:-}" || rc=1
  done < <(for p in "$devdir"/products/*/product.toml; do [[ -f $p ]] && printf '%s\n' "$p"; done)
  for name in "${!builds[@]}" "${!bootloaders[@]}"; do
    [[ $known == *" $name "* ]] || { echo "FINDING built tree named for $name: $name is not a product of $devdir"; rc=1; }
  done
  return $rc
}

# ---- the self-test: fixtures on a copy of devices/ and on system and bootloader trees ----------------------------------

wd_self_test() {  # <repo>
  local repo=$1 tmp rc=0 out dev b bl raze zbase zbring rz line
  tmp=$(mktemp -d)
  dev=$tmp/devices
  cp -a "$repo/devices" "$dev"
  # A synthetic sd-image product with no owner: the bring-up fixture with its systemd watchdog lines removed (zero-3e-base is deleted in Z2).
  mkdir -p "$dev/products/example-bringup" && cp -a "$repo/devices/tests/fixtures/bringup/." "$dev/products/example-bringup/"
  mkdir -p "$dev/products/sd-noowner" && cp -a "$repo/devices/tests/fixtures/bringup/." "$dev/products/sd-noowner/"
  sed -i -e '/^bringup-alive = /d' -e '/^bringup-alive-settings = /d' -e 's/"bringup-alive", "bringup-alive-settings", //' "$dev/products/sd-noowner/product.toml"
  # On the ZERO 3E board (it keeps a hw-watchdog keepalive nothing feeds: one NOTE, as zero-3e-base was).
  sed -i -e 's#boards/radxa-rock-4d/board.toml#boards/radxa-zero-3e/board.toml#' "$dev/products/sd-noowner/product.toml"
  raze=$dev/products/raze-base
  zbase=$dev/products/sd-noowner
  local rboard=$dev/boards/raze/lemnos-board.toml
  local zboard=$dev/boards/radxa-zero-3e/lemnos-board.toml
  local fam=$dev/families/rk35xx/assets/u-boot.fragment
  # The baseline is UNARMED: the NOTE cases are no-owner products the family does not arm. The armed cases restore and arm it below.
  sed -i '/^CONFIG_WATCHDOG_AUTOSTART=y$/d' "$fam"

  fail() { echo "watchdog owner self-test: $1" >&2; rc=1; }
  has() { [[ $out == *"$1"* ]]; }

  # The real tree. raze-base: inputs say Lemnos, exactly. The bring-up product: systemd. The base product: an sd-image
  # with no owner is one NOTE line (not a FINDING), and its U-Boot arming is NOT CHECKED.
  out=$(watchdog_owner_rule "$dev") || true
  has "NOTE product raze-base: inputs say Lemnos; built config unverified" || fail "raze-base's line is not the exact inputs line: $out"
  has "NOTE product sd-noowner (sd-image, not a slot product): no watchdog owner" || fail "sd-noowner is not one NOTE line: $out"
  has "bootloader arming: NOT CHECKED (resolved U-Boot config needs a build)" || fail "sd-noowner's U-Boot arming was not NOT CHECKED: $out"
  has "FINDING product sd-noowner" && fail "an sd-image product without an owner failed the gate: $out"
  has "NOTE product example-bringup (sd-image, not a slot product): systemd owner: RuntimeWatchdogSec=16s" || fail "the bring-up product's systemd owner was not seen: $out"
  has "rock-4d-base" && fail "rock-4d-base is still in the tree: $out"
  has "FINDING" && fail "the real tree has a FINDING: $out"

  # Neither (slot): raze-base without its keepalive. Also armed by the slot init: a FINDING for the arm too.
  cp -a "$repo/devices/boards/raze/lemnos-board.toml" "$rboard"
  sed -i '/^keepalive = true$/d' "$rboard"
  out=$(watchdog_owner_rule "$dev") || true
  has "FINDING product raze-base: no watchdog owner" || fail "neither owner was not refused: $out"
  has "the boot chain arms the watchdog (Lemnos slot init" || fail "a slot init arm with no owner was not refused: $out"
  cp -a "$repo/devices/boards/raze/lemnos-board.toml" "$rboard"

  # Both (slot): the Lemnos keepalive and a systemd drop-in in raze-base's overlay.
  mkdir -p "$raze/os/overlay/etc/systemd/system.conf.d"
  printf '[Manager]\nRuntimeWatchdogSec=15s\n' > "$raze/os/overlay/etc/systemd/system.conf.d/10-wd.conf"
  out=$(watchdog_owner_rule "$dev") || true
  has "FINDING product raze-base: two owners: Lemnos hw-watchdog keepalive (boards/raze/lemnos-board.toml line 131) and RuntimeWatchdogSec=15s (products/raze-base/os/overlay/etc/systemd/system.conf.d/10-wd.conf, line 2) — remove one" \
    || fail "both owners were not refused naming both: $out"
  rm -rf "$raze/os/overlay/etc/systemd/system.conf.d"

  # One systemd (slot): the keepalive removed, the drop-in alone.
  sed -i '/^keepalive = true$/d' "$rboard"
  mkdir -p "$raze/os/overlay/etc/systemd/system.conf.d"
  printf '[Manager]\nRuntimeWatchdogSec=15s\n' > "$raze/os/overlay/etc/systemd/system.conf.d/10-wd.conf"
  out=$(watchdog_owner_rule "$dev") || true
  has "NOTE product raze-base: inputs say systemd; built config unverified" || fail "one systemd owner was not accepted: $out"
  has "bootloader arming: armed (Lemnos slot init" || fail "the arm was not seen with its owner: $out"
  has "FINDING" && fail "one systemd owner plus the arm was refused: $out"
  rm -rf "$raze/os/overlay/etc/systemd/system.conf.d"
  cp -a "$repo/devices/boards/raze/lemnos-board.toml" "$rboard"

  # RuntimeWatchdogSec=0 is off, not an owner.
  mkdir -p "$raze/os/overlay/etc/systemd/system.conf.d"
  printf '[Manager]\nRuntimeWatchdogSec=0\n' > "$raze/os/overlay/etc/systemd/system.conf.d/10-wd.conf"
  out=$(watchdog_owner_rule "$dev") || true
  has "NOTE product raze-base: inputs say Lemnos; built config unverified" || fail "RuntimeWatchdogSec=0 was counted as an owner: $out"
  rm -rf "$raze/os/overlay/etc/systemd/system.conf.d"

  # A feeder unit beside the keepalive: a second owner.
  mkdir -p "$raze/os/overlay/usr/lib/systemd/system"
  printf '[Service]\nExecStart=/bin/sh -c "echo x > /dev/watchdog0"\n' > "$raze/os/overlay/usr/lib/systemd/system/feed.service"
  out=$(watchdog_owner_rule "$dev") || true
  has "another feeder: products/raze-base/os/overlay/usr/lib/systemd/system/feed.service:2 writes /dev/watchdog" || fail "a feeder beside the keepalive was not refused: $out"
  has "FINDING product raze-base: two owners" || fail "the feeder pair is not two owners: $out"
  rm -rf "$raze/os/overlay/usr/lib/systemd/system"

  # A keepalive nothing feeds, on an sd-image product: one NOTE line (sd-noowner with a keepalive in its board).
  cp -a "$repo/devices/boards/radxa-zero-3e/board.toml" "$dev/boards/radxa-zero-3e/board.toml"
  printf '\n[descriptions]\nlemnos = "lemnos-board.toml"\n' >> "$dev/boards/radxa-zero-3e/board.toml"
  printf '\n[[devices]]\nid = "watchdog"\ndriver = "hw-watchdog"\nkeepalive = true\n\n[[hardware]]\ncontroller = "watchdog"\n' >> "$zboard"
  out=$(watchdog_owner_rule "$dev") || true
  has "NOTE product sd-noowner (sd-image, not a slot product): no watchdog owner; boards/radxa-zero-3e/lemnos-board.toml keeps a hw-watchdog keepalive (line" || fail "an unfed keepalive on an sd-image product was not on its NOTE line: $out"
  has "nothing feeds" || fail "the NOTE line does not say nothing feeds the keepalive: $out"
  cp -a "$repo/devices/boards/radxa-zero-3e/board.toml" "$dev/boards/radxa-zero-3e/board.toml"
  cp -a "$repo/devices/boards/radxa-zero-3e/lemnos-board.toml" "$zboard"

  # A keepalive nothing feeds on a SLOT product: a FINDING (raze-base with lemnosd removed; its arm has no owner either).
  cp -a "$repo/devices/products/raze-base/product.toml" "$raze/product.toml"
  sed -i '/^lemnosd = /d' "$raze/product.toml"
  out=$(watchdog_owner_rule "$dev") || true
  has "FINDING product raze-base: boards/raze/lemnos-board.toml keeps a hw-watchdog keepalive (line 131) that nothing feeds" || fail "an unfed keepalive on a slot product was not refused: $out"
  cp -a "$repo/devices/products/raze-base/product.toml" "$raze/product.toml"

  # A slot-disk product (the recipe is named, the same owner rule applies): neither owner is a FINDING.
  sed -i 's/^recipe = "slot-boot"$/recipe = "slot-disk"/' "$raze/product.toml"
  sed -i '/^keepalive = true$/d' "$rboard"
  out=$(watchdog_owner_rule "$dev") || true
  has "FINDING product raze-base: no watchdog owner" || fail "a slot-disk product with no owner was not refused: $out"
  cp -a "$repo/devices/products/raze-base/product.toml" "$raze/product.toml"
  cp -a "$repo/devices/boards/raze/lemnos-board.toml" "$rboard"

  # Armed + no owner (any product): U-Boot's autostart line on sd-noowner, which has no owner: a FINDING.
  cp -a "$repo/devices/families/rk35xx/assets/u-boot.fragment" "$fam"
  printf 'CONFIG_WATCHDOG_AUTOSTART=y\n' >> "$fam"
  out=$(watchdog_owner_rule "$dev") || true
  has "FINDING product sd-noowner: the boot chain arms the watchdog (U-Boot config CONFIG_WATCHDOG_AUTOSTART=y arms the watchdog at boot" || fail "armed with no owner was not refused: $out"
  # Armed + one owner: the same line on the bring-up product (its systemd owner feeds it): a pass.
  has "NOTE product example-bringup: bootloader arming: armed (U-Boot config CONFIG_WATCHDOG_AUTOSTART=y" || fail "armed with an owner was refused: $out"
  has "FINDING product example-bringup" && fail "armed with an owner was a FINDING: $out"

  # A boot script's wdt command arms too (a Lemnos-free product with an owner passes; this one has none: a FINDING).
  cp -a "$repo/devices/families/rk35xx/assets/u-boot.fragment" "$fam"
  printf 'wdt dev 0\nwdt start\n' > "$dev/families/rk35xx/assets/boot.cmd"
  out=$(watchdog_owner_rule "$dev") || true
  has "FINDING product sd-noowner: the boot chain arms the watchdog (boot script command \"wdt\"" || fail "a wdt boot-script command was not seen: $out"
  rm -f "$dev/families/rk35xx/assets/boot.cmd"

  # A built U-Boot config (the bootloader dir) decides for the owner-less base product: autostart on is a FINDING.
  b=$tmp/built-boot
  mkdir -p "$b"
  printf 'CONFIG_WATCHDOG_AUTOSTART=y\n' > "$b/u-boot.config"
  out=$(watchdog_owner_rule "$dev" -- "sd-noowner=$b") || true
  has "FINDING product sd-noowner: the boot chain arms the watchdog (built U-Boot config CONFIG_WATCHDOG_AUTOSTART=y" || fail "a built U-Boot autostart was not refused: $out"
  rm -rf "$b"
  cp -a "$repo/devices/families/rk35xx/assets/u-boot.fragment" "$fam"

  # No U-Boot autostart line and no build: NOT CHECKED, exactly.
  out=$(watchdog_owner_rule "$dev") || true
  has "NOTE product sd-noowner (sd-image, not a slot product): no watchdog owner; bootloader arming: NOT CHECKED (resolved U-Boot config needs a build)" || true

  # A built system tree that disagrees with the inputs (inputs say Lemnos only; the built drop-in sets the watchdog).
  bl=$tmp/built-raze
  mkdir -p "$bl/etc/systemd/system.conf.d"
  printf '[Manager]\nRuntimeWatchdogSec=15s\n' > "$bl/etc/systemd/system.conf.d/10-image.conf"
  out=$(watchdog_owner_rule "$dev" "raze-base=$bl") || true
  has "FINDING product raze-base: two owners: Lemnos hw-watchdog keepalive (boards/raze/lemnos-board.toml line 131) and RuntimeWatchdogSec=15s (" || fail "a built tree against the inputs was not refused: $out"
  has "built config and its inputs disagree" || fail "the built/inputs disagreement was not named: $out"
  rm -rf "$bl/etc"

  # A built system tree that agrees: checked, one owner, no "unverified".
  mkdir -p "$bl/etc/systemd"
  printf '[Manager]\nRuntimeWatchdogSec=0\n' > "$bl/etc/systemd/system.conf"
  out=$(watchdog_owner_rule "$dev" "raze-base=$bl") || true
  rz=$(grep 'product raze-base' <<<"$out")
  has "built config checked" || fail "an agreeing built tree was not accepted as checked: $rz"
  [[ $rz == *unverified* ]] && fail "a built tree was given and still says unverified: $rz"
  rm -rf "$bl"

  rm -rf "$tmp"
  [[ $rc -eq 0 ]] && echo "watchdog owner self-test: passes the real tree (raze-base: inputs say Lemnos, exactly; the bring-up product: systemd; the sd-image base product: one NOTE line, not a FINDING); refuses neither, both, an unfed keepalive on a slot product, a feeder beside the keepalive, a slot-disk product with no owner, armed with no owner (U-Boot autostart, a wdt script line, a built U-Boot config), and a built system tree that disagrees with the inputs; RuntimeWatchdogSec=0 is off; armed with an owner passes; an unresolved U-Boot is NOT CHECKED"
  return $rc
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  set -uo pipefail
  repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
  root=$repo/devices
  builds=()
  bootloaders=()
  mode=check
  if [[ -n ${ATLAS_BUILT_TREES:-} ]]; then read -r -a builds <<<"$ATLAS_BUILT_TREES"; fi
  if [[ -n ${ATLAS_BUILT_BOOTLOADERS:-} ]]; then read -r -a bootloaders <<<"$ATLAS_BUILT_BOOTLOADERS"; fi
  while [[ $# -gt 0 ]]; do
    case $1 in
      --root) root=$2; shift 2 ;;
      --built) builds+=("$2"); shift 2 ;;
      --built-bootloader) bootloaders+=("$2"); shift 2 ;;
      --self-test) mode=self-test; shift ;;
      -h|--help) sed -n '2,30p' "${BASH_SOURCE[0]}"; exit 0 ;;
      *) echo "check-watchdog-owner: unknown argument: $1" >&2; exit 2 ;;
    esac
  done
  if [[ $mode == self-test ]]; then
    wd_self_test "$repo" || exit 1
    exit 0
  fi
  out=$(watchdog_owner_rule "$root" ${builds[@]+"${builds[@]}"} -- ${bootloaders[@]+"${bootloaders[@]}"}) && rc=0 || rc=$?
  printf '%s\n' "$out"
  if [[ $rc -eq 0 ]]; then
    echo "check-watchdog-owner: no FINDING"
  else
    exit 1
  fi
fi
