#!/usr/bin/env python3
"""Wired Ethernet on the Raze base image, and the USB gadget's link kept out of it.

Every systemd-networkd file that a raze-base image installs is matched, using networkd's own
[Match] rules for Name= and Type=, against the links a board can have:
- The USB gadget's link lemnos0 (networkd type `gadget`; also tried as `ether`, in case a gadget driver reports
  that) must match no installed file. Its one file is generated at boot as /run/systemd/network/10-lemnos0.network.
  A second DHCP client on it would break the USB way in.
- Each Ethernet link (end0 end1 eth0 eth1, the names the old 993 image listed, and enp0s2, the board profile's
  virtio NIC) must match the wired-ethernet file, with DHCP=yes and LinkLocalAddressing=yes.

Run from the repository root. `--self-test` checks the matcher on fixtures.
"""

import fnmatch
import os
import sys
import tomllib

PRODUCT = "devices/products/raze-base/product.toml"
ETHERNET_FILE = "/etc/systemd/network/20-wired-ethernet.network"
GADGET = [("lemnos0", "gadget"), ("lemnos0", "ether")]
ETHERNET = [("end0", "ether"), ("end1", "ether"), ("eth0", "ether"), ("eth1", "ether"), ("enp0s2", "ether")]


def list_match(values, item):
    """networkd's glob-list match (strv_fnmatch_full with a leading '!' inverting the whole list)."""
    if not values:
        return True
    patterns = " ".join(values).split()
    invert = False
    if patterns and patterns[0].startswith("!"):
        invert = True
        patterns[0] = patterns[0][1:]
    hit = any(fnmatch.fnmatchcase(item, p) for p in patterns if p)
    return hit != invert


def parse(text):
    """{section: {key: [values]}} keeping repeated keys, as networkd reads them."""
    sections, current = {}, None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line[0] in "#;":
            continue
        if line.startswith("[") and line.endswith("]"):
            current = sections.setdefault(line[1:-1], {})
            continue
        key, _, value = line.partition("=")
        if current is not None:
            current.setdefault(key.strip(), []).append(value.strip())
    return sections


def matches(network, name, kind):
    match = network.get("Match", {})
    return list_match(match.get("Name", []), name) and list_match(match.get("Type", []), kind)


def installed_networks():
    """{install path: parsed file} for every .network the raze-base image's components install."""
    with open(PRODUCT, "rb") as handle:
        product = tomllib.load(handle)
    base = os.path.dirname(PRODUCT)
    found = {}
    for image in product.get("images", []):
        for comp in image.get("components", []):
            path = os.path.normpath(os.path.join(base, product["components"][comp]))
            with open(path, "rb") as handle:
                component = tomllib.load(handle)
            for item in component.get("install", []):
                to, src = item.get("to", ""), item.get("from", {})
                if to.endswith(".network") and "file" in src:
                    with open(os.path.join(os.path.dirname(path), src["file"])) as f:
                        found[to] = parse(f.read())
    return found


def check(networks):
    errors = []
    for name, kind in GADGET:
        hits = [to for to, net in networks.items() if matches(net, name, kind)]
        if hits:
            errors.append(f"{name} (type {kind}) is matched by {', '.join(sorted(hits))}: a second network config on the gadget's link")
    eth = networks.get(ETHERNET_FILE)
    if eth is None:
        errors.append(f"no {ETHERNET_FILE} in the image")
    else:
        for name, kind in ETHERNET:
            first = sorted(to for to, net in networks.items() if matches(net, name, kind))
            if not first or os.path.basename(first[0]) != os.path.basename(ETHERNET_FILE):
                # networkd takes the first file by file name across its directories.
                errors.append(f"{name} (type {kind}) is configured by {first[0] if first else 'nothing'}, not {ETHERNET_FILE}")
        net = eth.get("Network", {})
        if net.get("DHCP") != ["yes"]:
            errors.append(f"{ETHERNET_FILE}: DHCP={net.get('DHCP')}, not yes")
        if net.get("LinkLocalAddressing") != ["yes"]:
            errors.append(f"{ETHERNET_FILE}: LinkLocalAddressing={net.get('LinkLocalAddressing')}, not yes (IPv4 link-local when no DHCP server answers, IPv6 always)")
    return errors


def self_test():
    good = parse("[Match]\nType=ether\nName=!lemnos*\n[Network]\nDHCP=yes\nLinkLocalAddressing=yes\n")
    assert check({ETHERNET_FILE: good}) == [], check({ETHERNET_FILE: good})
    # Type alone still keeps a `gadget` lemnos0 out, but not one reported as ether.
    loose = parse("[Match]\nType=ether\n[Network]\nDHCP=yes\nLinkLocalAddressing=yes\n")
    assert any("lemnos0 (type ether)" in e for e in check({ETHERNET_FILE: loose}))
    # Name lists: a leading ! inverts the whole list.
    assert not list_match(["!lemnos* usb0"], "usb0") and list_match(["!lemnos* usb0"], "end0")
    assert list_match(["end0 end1 eth0 eth1"], "eth1") and not list_match(["end0 end1 eth0 eth1"], "enp0s2")
    # The old bring-up file (Name=!usb0) would catch lemnos0 reported as ether.
    bringup = parse("[Match]\nType=ether\nName=!usb0\n[Network]\nDHCP=yes\n")
    errs = check({ETHERNET_FILE: bringup})
    assert any("lemnos0 (type ether)" in e for e in errs) and any("LinkLocalAddressing" in e for e in errs)
    # No file, and an earlier file that takes the Ethernet links first.
    assert check({}) == [f"no {ETHERNET_FILE} in the image"]
    early = {"/etc/systemd/network/10-eth.network": parse("[Match]\nName=eth0\n"), ETHERNET_FILE: good}
    assert any("eth0 (type ether) is configured by /etc/systemd/network/10-eth.network" in e for e in check(early))
    print("check-wired-ethernet self-test: ok")


def main():
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return 0
    networks = installed_networks()
    errors = check(networks)
    for e in errors:
        print(f"FAIL: {e}", file=sys.stderr)
    if errors:
        return 1
    print(f"wired ethernet: {len(networks)} network file(s) in raze-base; lemnos0 matched by none, {len(ETHERNET)} Ethernet names by {ETHERNET_FILE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
