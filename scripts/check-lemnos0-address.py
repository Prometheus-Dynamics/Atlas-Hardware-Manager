#!/usr/bin/env python3
"""The Raze's lemnos0 address (scheme serial-hash-v1), off the board.

Runs the component's shipped script, devices/components/raze-base-files/files/lemnos0-address, with the
serial and output directory pointed at fixtures (LEMNOS0_SERIAL, LEMNOS0_OUT) and the component's own
data file (LEMNOS0_CONF). Checks:
  - the six dev vectors (INPUTS "RULING": the address for each serial8);
  - Raze #1's serial 0708c973b00a37bc, which must give 172.31.12.81/29 (the bench reading). A mismatch
    means the bench reading or the scheme is not what we think: the check fails and says so;
  - the failure cases: an unreadable serial, a non-hex serial, a serial shorter than 8 digits, and an
    unknown scheme each write NO network file and NO peer file, exit non-zero, and print one sentence;
  - each vector also writes /run/lemnos0/orion-peer.env with ORION_PEER_LISTEN=<address>:9200 (the board's own
    address, never a wildcard), which the comm core reads.
The derivation is the same algorithm as dev's (crates/atlas-devices/src/gadget.rs, lib.sh board_gadget_subnet
at 675bd03). The host side, parked with the gateway work, reads the same conf file's values.

Run from the repository root. `--self-test` is the same run; the gate runs it.
"""

import os
import re
import subprocess
import sys
import tempfile

COMPONENT = os.path.join("devices", "components", "raze-base-files", "files")
SCRIPT = os.path.join(COMPONENT, "lemnos0-address")
CONF = os.path.join(COMPONENT, "lemnos0-addressing.conf")

# (full serial text as the device tree gives it, the address and prefix the script must write)
VECTORS = [
    ("a317bcbee5226d57", "172.31.209.217/29"),  # Raze #2: serial8 e5226d57
    ("abcdef01", "172.31.0.113/29"),
    ("00000000", "172.31.43.201/29"),
    ("ffffffff", "172.31.141.25/29"),
    ("deadbeef", "172.31.209.1/29"),
    ("cfc0641d", "172.31.254.9/29"),  # index past the excluded 172.31.250.0/24
    ("0708c973b00a37bc", "172.31.12.81/29"),  # Raze #1, the bench reading
]

# (serial text or None for an unreadable file, why it must refuse)
REFUSALS = [
    (None, "the board serial"),  # no serial file at all
    ("zzzz1234", "not hex"),
    ("abc123", "fewer than 8"),
]

ADDRESS = re.compile(r"^Address=(\S+)$", re.M)


def run(serial, out_dir, scheme_conf=None):
    env = dict(os.environ)
    env["LEMNOS0_CONF"] = scheme_conf or CONF
    env["LEMNOS0_OUT"] = out_dir
    env["LEMNOS0_PEER_DIR"] = os.path.join(out_dir, "peer")
    if serial is None:
        env["LEMNOS0_SERIAL"] = os.path.join(out_dir, "no-such-serial")
    else:
        path = os.path.join(out_dir, "serial")
        with open(path, "w", encoding="ascii") as handle:
            handle.write(serial)
        env["LEMNOS0_SERIAL"] = path
    result = subprocess.run(["sh", SCRIPT], env=env, capture_output=True, text=True)
    return result


def network_file(out_dir):
    path = os.path.join(out_dir, "10-lemnos0.network")
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def peer_file(out_dir):
    path = os.path.join(out_dir, "peer", "orion-peer.env")
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def check():
    problems = []
    for serial, expected in VECTORS:
        with tempfile.TemporaryDirectory() as out:
            result = run(serial, out)
            text = network_file(out)
            if result.returncode != 0 or text is None:
                problems.append(f"serial {serial}: the script refused ({result.stderr.strip()})")
                continue
            found = ADDRESS.search(text)
            got = found.group(1) if found else None
            peer = peer_file(out)
            want_peer = f"ORION_PEER_LISTEN={expected.split('/')[0]}:9200\n"
            if peer != want_peer:
                problems.append(f"serial {serial}: the peer file is {peer!r}, the vector needs {want_peer!r}")
            if got != expected:
                problems.append(f"serial {serial}: wrote {got}, the vector is {expected}")
            elif "Name=lemnos0" not in text or "DHCPServer=yes" not in text or "IPv6AcceptRA=no" not in text:
                problems.append(f"serial {serial}: the network file lacks Name=lemnos0, DHCPServer=yes or IPv6AcceptRA=no")
            elif "EmitRouter=no" not in text or "EmitDNS=no" not in text:
                problems.append(f"serial {serial}: the DHCP server would announce a router or DNS")
    for serial, why in REFUSALS:
        with tempfile.TemporaryDirectory() as out:
            result = run(serial, out)
            if result.returncode == 0:
                problems.append(f"serial {serial!r} ({why}): the script exited 0")
            if network_file(out) is not None or peer_file(out) is not None:
                problems.append(f"serial {serial!r} ({why}): the script wrote a network or peer file")
            if not result.stderr.strip().startswith("lemnos0-address:"):
                problems.append(f"serial {serial!r} ({why}): no one-sentence reason was printed")
    with tempfile.TemporaryDirectory() as out:
        bad = os.path.join(out, "scheme.conf")
        with open(bad, "w", encoding="utf-8") as handle:
            handle.write("scheme=other\nbase=172.31.0.0/16\nprefix=29\nexclude=\n")
        result = run("0708c973b00a37bc", out, scheme_conf=bad)
        if result.returncode == 0 or network_file(out) is not None:
            problems.append("an unknown scheme was not refused without writing a network file")
    return problems


def main():
    problems = check()
    for problem in problems:
        print(f"check-lemnos0-address: FAIL {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"check-lemnos0-address: passed ({len(VECTORS)} vectors incl. Raze #1 172.31.12.81/29; {len(REFUSALS)} refusals, "
          "and an unknown scheme, write nothing)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
