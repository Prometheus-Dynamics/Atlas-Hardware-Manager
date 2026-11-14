#!/usr/bin/env python3
"""The Raze's Orion USB link: one name on both sides, and the gadget it comes from.

The comm core declares the USB link by `links.usb = <name>` in its settings (Orion comm-app settings.rs:271-272).
The Lemnos bridge offers the gadget's endpoints under `LEMNOS_ORION_LINKS=<device id>=<name>`. Both must give the
SAME <name>, and <device id> must be a usb-gadget device of the Raze's board file. Checked here:
  - devices/components/orion/files/orion.conf: `links.usb` is exactly one name, and it is the one the bridge offers;
  - devices/components/lemnos/files/lemnos-orion-links.conf: `LEMNOS_ORION_LINKS` is `<device id>=<name>` with one pair;
  - the device id is a `usb-gadget` device in devices/boards/raze/lemnos-board.toml;
  - the gadget's `interfaces` text, if it gives one, puts <name> on interface 0 (the link must be first);
  - devices/components/orion/files/orion-comm-peer.conf reads its address file with `EnvironmentFile=-` (a missing file
    must not fail the comm core) and orders on lemnos0-address with Wants= (not Requires=).
Run from the repository root. `--self-test` checks the rule on fixtures (a name mismatch, a device id that is not a
gadget, and a Requires= that would make a failed address program stop the comm core are each refused).
"""

import re
import sys
import tomllib

ORION_CONF = "devices/components/orion/files/orion.conf"
BRIDGE_CONF = "devices/components/lemnos/files/lemnos-orion-links.conf"
PEER_DROP_IN = "devices/components/orion/files/orion-comm-peer.conf"
BOARD = "devices/boards/raze/lemnos-board.toml"
NAME = re.compile(r"^[A-Za-z0-9._:-]{1,63}$")


def setting(text, key):
    """The value of `key = value` in a settings file (not a comment), or None."""
    for line in text.splitlines():
        code = line.split("#", 1)[0] if not line.lstrip().startswith("#") else ""
        match = re.match(rf"^\s*{re.escape(key)}\s*=\s*(.+?)\s*$", code)
        if match:
            return match.group(1)
    return None


def environment_value(text, key):
    """The value of `Environment=KEY=VALUE` in a unit drop-in, or None."""
    for line in text.splitlines():
        match = re.match(rf"^\s*Environment\s*=\s*{re.escape(key)}=(.+?)\s*$", line)
        if match:
            return match.group(1)
    return None


def check(orion_text, bridge_text, peer_text, board):
    problems = []
    links = setting(orion_text, "links.usb")
    if links is None:
        problems.append(f"{ORION_CONF}: links.usb is not set")
        links = ""
    elif "," in links or not NAME.match(links):
        problems.append(f"{ORION_CONF}: links.usb = {links!r} is not one link name")
    offered = environment_value(bridge_text, "LEMNOS_ORION_LINKS")
    if offered is None:
        problems.append(f"{BRIDGE_CONF}: LEMNOS_ORION_LINKS is not set: the bridge would offer no link")
        return problems
    pairs = offered.split(",")
    if len(pairs) != 1 or "=" not in pairs[0]:
        problems.append(f"{BRIDGE_CONF}: LEMNOS_ORION_LINKS = {offered!r} is not one <device id>=<name> pair")
        return problems
    device, name = pairs[0].split("=", 1)
    if name != links:
        problems.append(f"the link name differs: {ORION_CONF} says links.usb = {links!r}, the bridge offers {name!r}")
    gadgets = [d for d in board.get("devices", []) if d.get("id") == device and d.get("driver") == "usb-gadget"]
    if not gadgets:
        problems.append(f"{BRIDGE_CONF}: the device id {device!r} is not a usb-gadget device of {BOARD}")
    for gadget in gadgets:
        text = gadget.get("config", {}).get("interfaces")
        if text is not None:
            first = next((line.split() for line in text.splitlines() if line.strip().startswith("interface")), None)
            if first is None or first[-1] != links:
                problems.append(f"{BOARD}: the gadget {device!r} puts {first[-1] if first else 'nothing'} on interface 0, not {links!r}")
    if not re.search(r"^\s*EnvironmentFile\s*=\s*-", peer_text, re.M):
        problems.append(f"{PEER_DROP_IN}: EnvironmentFile must have the leading - (an absent file must not fail the comm core)")
    if not re.search(r"^\s*Wants\s*=\s*lemnos0-address\.service", peer_text, re.M):
        problems.append(f"{PEER_DROP_IN}: the comm core must Wants= lemnos0-address.service")
    if re.search(r"^\s*Requires\s*=", peer_text, re.M):
        problems.append(f"{PEER_DROP_IN}: Requires= would make a failed address program stop the comm core: use Wants=")
    return problems


def read(path):
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def self_test():
    board = tomllib.loads(read(BOARD))
    orion, bridge, peer = read(ORION_CONF), read(BRIDGE_CONF), read(PEER_DROP_IN)
    cases = {
        "the names agree and the device is a gadget": (orion, bridge, peer, board, True),
        "a name mismatch is refused": (orion.replace("links.usb = orion-link", "links.usb = orion-other"), bridge, peer, board, False),
        "a device id that is not a gadget is refused": (orion, bridge.replace("gadget=", "power=").replace("LEMNOS_ORION_LINKS=power", "LEMNOS_ORION_LINKS=power"), peer, board, False),
        "Requires= instead of Wants= is refused": (orion, bridge, peer.replace("Wants=", "Requires="), board, False),
    }
    failed = False
    for label, (o, b, p, bd, passes) in cases.items():
        problems = check(o, b, p, bd)
        if (not problems) != passes:
            print(f"check-orion-link: self-test: {label} gave {problems}", file=sys.stderr)
            failed = True
    if failed:
        return 1
    print("check-orion-link: self-test: a name mismatch, a non-gadget id, and Requires= are each refused; the match passes")
    return 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    board = tomllib.loads(read(BOARD))
    problems = check(read(ORION_CONF), read(BRIDGE_CONF), read(PEER_DROP_IN), board)
    for problem in problems:
        print(f"check-orion-link: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    print("check-orion-link: passed (links.usb = orion-link on the comm core and on the bridge, device gadget is a usb-gadget)")
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
