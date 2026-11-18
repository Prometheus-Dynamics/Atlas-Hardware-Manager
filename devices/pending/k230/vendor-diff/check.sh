#!/usr/bin/env bash
# PENDING (K1): the provenance check for the K230 vendor pad table and the files it came from. Given the pristine vendor
# tree (VENDOR, the fc6cc215 checkout) and our copy of each file (OURS), every file that differs must be a recorded edit in
# allowed-diff.toml, and its diff must equal the recorded patch. Any other difference is a refusal.
# Usage: check.sh <vendor tree> <our tree> <allowed-diff.toml>
set -u
vendor=$1 ours=$2 list=$3
rc=0
# The recorded edits, as "file<TAB>patch" pairs.
recorded=$(awk '/^\[\[edit\]\]/{f=""; p=""} /^file = /{f=$0; sub(/^file = "/,"",f); sub(/"$/,"",f)} /^patch = /{p=$0; sub(/^patch = "/,"",p); sub(/"$/,"",p); print f "\t" p}' "$list")
# Every recorded file is compared with its pristine vendor version (git HEAD of the vendor tree): identical, or differing by
# exactly its recorded patch. Files outside the list are not part of the vendor table and are not checked here.
while IFS=$'\t' read -r rel patch_rel; do
  [[ -n $rel ]] || continue
  if ! git -C "$vendor" cat-file -e "HEAD:$rel" 2>/dev/null; then echo "REFUSE $rel: not in the pristine vendor tree"; rc=1; continue; fi
  tmp=$(mktemp); git -C "$vendor" show "HEAD:$rel" > "$tmp"
  if [[ ! -f $ours/$rel ]]; then echo "REFUSE $rel: missing from our tree"; rc=1; rm -f "$tmp"; continue; fi
  if cmp -s "$tmp" "$ours/$rel"; then rm -f "$tmp"; continue; fi
  if [[ -z $patch_rel ]]; then echo "REFUSE $rel: differs from the pristine vendor file and no recorded edit allows it"; rc=1; rm -f "$tmp"; continue; fi
  got=$(mktemp); cp "$tmp" "$got"
  if ! patch -s "$got" < "$(dirname "$list")/$patch_rel" 2>/dev/null || ! cmp -s "$got" "$ours/$rel"; then
    echo "REFUSE $rel: differs from the vendor file beyond its recorded patch $patch_rel"; rc=1
  fi
  rm -f "$tmp" "$got"
done <<<"$recorded"
[[ $rc -eq 0 ]] && echo "vendor-diff: ok (every difference from the pristine vendor files is a recorded edit)"
exit $rc
