#!/usr/bin/env python3
# Verifies a board's recorded facts (kernel/post-patch-facts.txt) against the kernel tree the gate built with the board's
# patches applied (PR A7RAILS, second ruling). It regenerates every fact from the tree and fails on any difference, so a
# recorded fact cannot drift from the patch. Run by scripts/check-kernel-fragments.sh for boards that have kernel patches.
#
#   python3 -I scripts/check-board-facts.py TREE BOARD_DIR
#
# TREE is a kernel source tree with the series and the board's patches applied. BOARD_DIR is the board's directory (it has
# board.toml, whose device_tree names the dts, and kernel/post-patch-facts.txt).
#
# Facts (the file's keys):
#   dts-sha256                 the sha256 of the board's dts
#   pio-supply BANK RAIL|none  the vcc-pX-supply the main pin controller's bank names (the rail = its regulator node name)
#   rpio-supply BANK RAIL|none the same for the R-domain pin controller (PL, PM)
#   io-bias BANK yes|no       whether the driver writes the bank's withstand mode (io_bias_banks in the A733 driver)
#   pio-ref BANK N             how many pin references (<&pio K ...>, K the bank index) the dts and its included dtsi make
#   bank-pins BANK N           the SoC's pin count per bank (a733_nr_bank_pins in the A733 driver)
#   range-required RAIL range|fixed:UV   the regulator node RAIL in the dts must NOT have min equal to max
import hashlib
import os
import re
import sys

BANKS = ["PA", "PB", "PC", "PD", "PE", "PF", "PG", "PH", "PI", "PJ", "PK"]
R_BANKS = ["PL", "PM"]


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def fail(msg):
    print(f"check-board-facts: {msg}", file=sys.stderr)
    sys.exit(1)


def board_dts(board_dir, tree):
    text = read(os.path.join(board_dir, "board.toml"))
    m = re.search(r'^device_tree\s*=\s*"([^"]+)"', text, re.M)
    if not m:
        fail(f"{board_dir}/board.toml names no device_tree")
    path = os.path.join(tree, "arch", "arm64", "boot", "dts", m.group(1) + ".dts")
    if not os.path.isfile(path):
        fail(f"the built tree has no {m.group(1)}.dts")
    return path


def block(text, head):
    """The text of the first top-level block that starts with `head` (up to its closing `};`)."""
    m = re.search(r"^" + re.escape(head) + r"\s*\{", text, re.M)
    if not m:
        return None
    depth, i = 0, m.end() - 1
    start = i
    while i < len(text):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[start:i + 1]
        i += 1
    return None


def node_name_of_label(text, label):
    """`reg_swout2: swout2 {` -> swout2."""
    m = re.search(r"^\s*" + re.escape(label) + r"\s*:\s*([A-Za-z0-9_-]+)\s*\{", text, re.M)
    return m.group(1) if m else None


def regulator_node(text, name):
    """The body of the regulator node named `name` (a node whose name is `name`, possibly with a label)."""
    m = re.search(r"^\s*(?:[A-Za-z0-9_]+\s*:\s*)?" + re.escape(name) + r"\s*\{", text, re.M)
    if not m:
        return None
    depth, i, start = 0, m.end() - 1, m.end() - 1
    while i < len(text):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[start:i + 1]
        i += 1
    return None


def pio_supplies(dts, head, banks):
    body = block(dts, head) or ""
    out = {}
    for bank in banks:
        idx = BANKS.index(bank) if bank in BANKS else None
        letter = bank[1].lower()
        m = re.search(r"vcc-p" + letter + r"-supply\s*=\s*<&([A-Za-z0-9_]+)>", body)
        if not m:
            out[bank] = "none"
        else:
            name = node_name_of_label(dts, m.group(1))
            out[bank] = name if name else m.group(1)
    return out


def io_bias_banks(driver):
    m = re.search(r"io_bias_banks\s*=\s*([^;]+);", driver)
    if not m:
        return set()
    return {int(b) for b in re.findall(r"BIT\((\d+)\)", m.group(1))}


def bank_pins(driver):
    m = re.search(r"a733_nr_bank_pins\[[^\]]*\]\s*=\s*(?:/\*.*?\*/\s*)?\{([^}]*)\}", driver, re.S)
    if not m:
        fail("the driver's a733_nr_bank_pins table is not found")
    nums = [int(x) for x in re.findall(r"\d+", m.group(1))]
    if len(nums) < len(BANKS):
        fail("the driver's bank table is shorter than the A733's bank list")
    return dict(zip(BANKS, nums))


def pio_refs(texts):
    counts = {b: 0 for b in BANKS}
    for t in texts:
        for m in re.finditer(r"<&pio\s+(\d+)\s", t):
            k = int(m.group(1))
            if k < len(BANKS):
                counts[BANKS[k]] += 1
    return counts


def regenerate(tree, board_dir):
    dts_path = board_dts(board_dir, tree)
    dts = read(dts_path)
    dtsi_path = os.path.join(os.path.dirname(dts_path), "sun60i-a733.dtsi")
    dtsi = read(dtsi_path) if os.path.isfile(dtsi_path) else ""
    driver = read(os.path.join(tree, "drivers", "pinctrl", "sunxi", "pinctrl-sun60i-a733.c"))
    facts = {}
    facts["dts-sha256"] = [hashlib.sha256(open(dts_path, "rb").read()).hexdigest()]
    sup = pio_supplies(dts, "&pio", BANKS)
    for b in BANKS:
        facts[("pio-supply", b)] = [sup[b]]
    rsup = pio_supplies(dts, "&r_pio", R_BANKS)
    for b in R_BANKS:
        facts[("rpio-supply", b)] = [rsup[b]]
    bits = io_bias_banks(driver)
    for i, b in enumerate(BANKS):
        facts[("io-bias", b)] = ["yes" if i in bits else "no"]
    for b in R_BANKS:
        facts[("io-bias", b)] = ["no"]  # the R-domain pin controller has no withstand variant (the series' a733-r description)
    refs = pio_refs([dts, dtsi])
    for b in BANKS:
        facts[("pio-ref", b)] = [str(refs[b])]
    pins = bank_pins(driver)
    for b in BANKS:
        facts[("bank-pins", b)] = [str(pins[b])]
    return facts, dts_path


def recorded(path):
    facts = {}
    for raw in read(path).splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        parts = line.split()
        if parts[0] == "dts-sha256":
            facts["dts-sha256"] = [parts[1]]
        elif parts[0] == "range-required":
            facts[("range-required", parts[1])] = [parts[2]]
        else:
            facts[(parts[0], parts[1])] = [parts[2]]
    return facts


def main():
    if len(sys.argv) != 3:
        fail("usage: check-board-facts.py TREE BOARD_DIR")
    tree, board_dir = sys.argv[1], sys.argv[2]
    facts_path = os.path.join(board_dir, "kernel", "post-patch-facts.txt")
    if not os.path.isfile(facts_path):
        fail(f"{facts_path} is missing")
    want = recorded(facts_path)
    got, dts_path = regenerate(tree, board_dir)
    # range-required: the rails the recorded file says must stay ranges, read from the tree's dts
    for key in list(want):
        if isinstance(key, tuple) and key[0] == "range-required":
            body = regulator_node(read(dts_path), key[1])
            if body is None:
                got[key] = ["absent"]
                continue
            mn = re.search(r"regulator-min-microvolt\s*=\s*<(\d+)>", body)
            mx = re.search(r"regulator-max-microvolt\s*=\s*<(\d+)>", body)
            if mn and mx and mn.group(1) == mx.group(1):
                got[key] = ["fixed:" + mn.group(1)]
            else:
                got[key] = ["range"]
    bad = []
    for key in sorted(set(want) | set(got), key=str):
        w = want.get(key)
        g = got.get(key)
        if w is None:
            bad.append(f"  recorded fact missing from the file: {key} (the tree says {g})")
        elif g is None:
            bad.append(f"  recorded fact the tree cannot check: {key}")
        elif w != g:
            bad.append(f"  {key}: the file says {w}, the built tree says {g}")
    if bad:
        fail(f"{os.path.relpath(facts_path)} disagrees with the built kernel (patched with the board's kernel patches):\n"
             + "\n".join(bad) + "\n  Regenerate the facts from the tree, or fix the patch.")
    n = len(want)
    print(f"check-board-facts: {n} recorded facts match the built tree ({os.path.relpath(dts_path, tree)})")


if __name__ == "__main__":
    main()
