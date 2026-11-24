#!/usr/bin/env python3
# Prints "IMAGE COMPRESSION" for each erofs image in a Gaia plan (`gaia plan --json`): the constant `compression` of the
# node labelled <image>::erofs. A compression that is not a constant prints "-". Used by scripts/check-devices.sh
# (erofs_rule).
#
# It fails, naming the file, when the plan cannot be read or has no node list: a plan that says nothing must never read
# as a plan with no erofs images (the inline form this replaced could pass on a truncated quote).
import json
import sys

if len(sys.argv) != 2:
    sys.exit("usage: plan-erofs-images.py <plan.json>")
path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as f:
        nodes = json.load(f)["graph"]["graph"]["nodes"]
except (OSError, ValueError, KeyError, TypeError) as e:
    sys.exit(f"plan-erofs-images: {path}: not a plan with a node list ({type(e).__name__}: {e})")
if not isinstance(nodes, list) or not nodes:
    sys.exit(f"plan-erofs-images: {path}: the plan has no nodes")
for node in nodes:
    label = node.get("label", "")
    if label.endswith("::erofs"):
        const = node.get("consts", {}).get("compression", {})
        value = const.get("value", "-") if const.get("type") == "str" else "-"
        print(label[: -len("::erofs")], value)
