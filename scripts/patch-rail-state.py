#!/usr/bin/env python3
# Prints the state of one regulator in the post-image of a kernel patch: "<rail>=range", "<rail>=fixed:<uV>",
# "<rail>=open" or "<rail>=absent". Used by scripts/check-devices.sh (pads_rail_state).
#
# It reads the context and added lines of every hunk of each .dts/.dtsi in a unified diff and finds the regulator node
# named <rail>. A node whose min and max are equal is fixed; a node with a min and a different max is a range; a node
# with neither is open (no constraint). A node the patch does not show is absent.
#
# It fails, naming the file, when the patch cannot be read or carries no file at all, so an unreadable patch never
# reads as "absent".
import re
import sys

if len(sys.argv) != 3:
    sys.exit("usage: patch-rail-state.py <patch> <rail>")
patch, rail = sys.argv[1], sys.argv[2]
if not rail:
    sys.exit("patch-rail-state: an empty rail name")
post, cur = {}, None
try:
    with open(patch, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("+++ b/"):
                cur = line[6:]
                post.setdefault(cur, [])
                continue
            if cur is None or line.startswith(("--- ", "diff ", "index ", "@@")):
                continue
            if line[:1] in (" ", "+"):
                post[cur].append(line[1:])
except (OSError, UnicodeDecodeError) as e:
    sys.exit(f"patch-rail-state: {patch}: cannot read ({e})")
if not post:
    sys.exit(f"patch-rail-state: {patch}: no file in the patch (no '+++ b/' line)")
head = re.compile(r"(^|\s)" + re.escape(rail) + r" \{")
for name, lines in post.items():
    if not (name.endswith(".dts") or name.endswith(".dtsi")):
        continue
    for i, l in enumerate(lines):
        if not head.search(l):
            continue
        mn = mx = None
        for m in lines[i + 1 :]:
            a = re.search(r"regulator-min-microvolt = <(\d+)>", m)
            b = re.search(r"regulator-max-microvolt = <(\d+)>", m)
            if a:
                mn = int(a.group(1))
            if b:
                mx = int(b.group(1))
            if re.match(r"^\s*};", m):
                break
        if mn is None and mx is None:
            print(rail + "=open")
        elif mn is not None and mn == mx:
            print(rail + "=fixed:" + str(mn))
        else:
            print(rail + "=range")
        sys.exit(0)
print(rail + "=absent")
