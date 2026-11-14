# The version and slot id of a slot-boot image: THE ONE RULE'S ONE PLACE. Sourced (not run) by
# scripts/build-product.sh, scripts/check-devices.sh and scripts/check-config-only.sh.
#
#   version  the build's own: <release>-dev.<commit count>+<commit>, the Atlas dev scheme of dev's
#            scripts/version.sh (and scripts/build-version.rs): count = `git rev-list --count HEAD`, commit =
#            `git rev-parse --short=8 HEAD`, release = Cargo.toml's [workspace.metadata.atlas] release.
#            A tree with tracked changes is refused: its commit would not be what the image holds.
#   slot     <product>-<version>, every `+` read as `-`, lowercased (Lemnos: [a-z0-9._-], 1..64 bytes).
#   CHECK_VERSION  the TEST value the checks plan with. It is not a release and names no commit.
#
# The product.toml [images.settings] does NOT set version or slot: Gaia takes them as `--setting`
# (IMAGE.NAME=VALUE, for a parameter the product leaves unset), from slot_settings_args below.

CHECK_VERSION="2026.1.0-dev.0+check"

slot_id_for() {  # <product> <version> -> the slot id
  local v=${2//+/-}
  printf '%s-%s' "$1" "${v,,}"
}

atlas_version() {  # <repo dir> -> the Atlas dev version of its HEAD (prints it; non-zero with a reason on stderr)
  local root=$1 release count commit
  if [[ -n $(git -C "$root" status --porcelain --untracked-files=no 2>/dev/null) ]]; then
    echo "version-rule: $root has tracked changes: the version would name a commit the image does not hold; commit or stash first" >&2
    return 1
  fi
  release=$(sed -n '/^\[workspace.metadata.atlas\]/,/^\[/s/^release *= *"\(.*\)"/\1/p' "$root/Cargo.toml")
  if [[ -z $release ]]; then
    echo "version-rule: $root/Cargo.toml has no [workspace.metadata.atlas] release" >&2
    return 1
  fi
  if ! count=$(git -C "$root" rev-list --count HEAD) || ! commit=$(git -C "$root" rev-parse --short=8 HEAD); then
    echo "version-rule: $root is not a git checkout: no commit count or commit to version it with" >&2
    return 1
  fi
  printf '%s-dev.%s+%s\n' "$release" "$count" "$commit"
}

# The --setting arguments a slot-boot product's image takes from the rule, one token per line (read with mapfile):
# --setting IMAGE.version=VERSION, --setting IMAGE.slot=SLOT. A product with no slot-boot recipe gets none.
slot_settings_args() {  # <product.toml> <version> -> one token per line
  local product=$1 version=$2 image pname slot
  grep -qE '^recipe = "slot-boot"' "$product" || return 0
  image=$(awk '/^\[\[images\]\]/ { on = 1; next } /^\[/ { on = 0 } on && $1 == "name" { gsub(/"/, "", $3); print $3; exit }' "$product")
  pname=$(awk '/^\[images\.settings\]/ { on = 1; next } /^\[/ { on = 0 } on && /^product = "/ { split($0, a, "\""); print a[2]; exit }' "$product")
  [[ -n $image && -n $pname ]] || { echo "version-rule: $product names no image or no settings product" >&2; return 1; }
  slot=$(slot_id_for "$pname" "$version")
  printf '%s\n' --setting "$image.version=$version" --setting "$image.slot=$slot"
}
