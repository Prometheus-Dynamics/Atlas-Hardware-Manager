#!/bin/bash
# No-VSEL check (PR AU1, 2026-11-16). Reads the ADDED lines of the patch files given (or the rv11xx series by default)
# and refuses a patch that: (1) WRITES an IO-domain or VSEL register; (2) adds an io-domains node or a voltage-domain
# compatible; (3) switches on an IO-domain driver config; (4) changes a pad bank's supply (a *-supply named for a pad
# bank: pmuioN, vccioN, pio, rpio, io). Reads are allowed (the vendor U-Boot reads IO1_VSEL for the FSPI drive strength,
# rv1126b.c 510-511). vqmmc-supply is the SD card's IO regulator, not a bank supply, and is not matched here.
# Usage: check-no-vsel.sh [--self-test] [patch...]
#   default patches: devices/families/rv11xx/patches/{uboot,linux}/*.patch
# Exit 0: no refusal. Exit 1: a refusal (named file, line and rule). Scope: the added lines of each patch, not the built tree.
set -u
here=$(cd "$(dirname "$0")/.." && pwd)

scan() {  # <patch>... -> prints one refusal per offending added line; returns 1 if any
  python3 -I - "$@" <<'PY'
import re, sys
WRITE = re.compile(r"\b(writel|writel_relaxed|writeb|writew|writeq|writel_be|regmap_write|regmap_update_bits|"
                   r"regmap_field_write|clrsetbits|setbits|clrbits|rk_clrsetreg|rk_setreg)\s*\(")
VSEL_TOKEN = re.compile(r"(VSEL|vsel|VD_3V3|VD_1V8)")
VSEL_ASSIGN = re.compile(r"\b(io[0-9]*_vsel|io_vsel)\b\s*=")
IODOM_NODE = re.compile(r"(io[-_]domains?|pmu_io_domains|io-voltage-domain|io_voltage_domain)\b[^=;]*\{|"
                        r"&(pmu_)?io_domains\b|rockchip,[a-z0-9-]*io-voltage-domain")
IODOM_CFG = re.compile(r"^\+\s*CONFIG_(SPL_|TPL_)?ROCKCHIP_IO(DOMAIN|_VOLTAGE_DOMAIN)\s*=\s*y")
BANK_SUPPLY = re.compile(r"\b(pmuio[0-9]+|vccio[0-9]+|pio|rpio|io)(-[a-z0-9]+)*-supply\s*=")
bad = 0
for path in sys.argv[1:]:
    cur = None
    try:
        lines = open(path, encoding="utf-8", errors="replace").read().split("\n")
    except OSError as e:
        print(f"FAIL {path}: cannot read: {e}"); bad += 1; continue
    for n, line in enumerate(lines, 1):
        if line.startswith("+++ "):
            cur = line[6:]; continue
        if not line.startswith("+") or line.startswith("+++"):
            continue
        body = line[1:]
        why = []
        if WRITE.search(body) and VSEL_TOKEN.search(body):
            why.append("writes an IO-domain/VSEL register (rule 1)")
        if VSEL_ASSIGN.search(body) and "=" in body and not body.lstrip().startswith(("/*", "*", "//")):
            why.append("assigns an io_vsel register field (rule 1)")
        if IODOM_NODE.search(body):
            why.append("adds an io-domains / IO voltage-domain node (rule 2)")
        if IODOM_CFG.search(line):
            why.append("switches on an IO-domain driver config (rule 3)")
        if BANK_SUPPLY.search(body):
            why.append("sets a supply for a pad bank (rule 4)")
        for w in why:
            print(f"FAIL {path} (file {cur or '?'}, added line {n}): {w}: {body.strip()[:120]}")
            bad += 1
sys.exit(1 if bad else 0)
PY
}

if [[ ${1:-} == --self-test ]]; then
  tmp=$(mktemp -d) || exit 1
  trap 'rm -rf "$tmp"' EXIT
  mk() {  # <name> <file> <added line>: a one-hunk patch adding one line to <file>
    printf -- '--- a/%s\n+++ b/%s\n@@ -1 +1,2 @@\n context\n+%s\n' "$2" "$2" "$3" > "$tmp/$1.patch"
  }
  mk offend-vsel-write   arch/arm/mach-rockchip/rv1126b/rv1126b.c 'writel(0x1, VCCIO1_IOC_BASE + GPIO1_IOC_IO1_VSEL);'
  mk offend-iovsel-field arch/arm/mach-rockchip/rv1126b/rv1126b.c 'io2_vsel = 0x1;'
  mk offend-io-domains   arch/arm64/boot/dts/rockchip/rv1126b-x.dts '	pmu_io_domains { status = "okay"; };'
  mk offend-iodomain-cfg devices/families/rv11xx/assets/u-boot.fragment 'CONFIG_ROCKCHIP_IODOMAIN=y'
  mk offend-bank-supply  arch/arm64/boot/dts/rockchip/rv1126b-x.dts '	vccio2-supply = <&vcc_1v8>;'
  mk clean-read-vsel     arch/arm/mach-rockchip/rv1126b/rv1126b.c 'if (readl(VCCIO1_IOC_BASE + GPIO1_IOC_IO1_VSEL) & VCCIO1_VD_3V3) {'
  mk clean-iomux-write   arch/arm/mach-rockchip/rv1126b/rv1126b.c 'writel(0xffff1111, VCCIO2_IOC_BASE + GPIO2A_IOMUX_SEL_L);'
  mk clean-card-vqmmc    arch/arm64/boot/dts/rockchip/rv1126b-x.dts '	vqmmc-supply = <&vccio_sd>;'
  rc=0
  for p in offend-vsel-write offend-iovsel-field offend-io-domains offend-iodomain-cfg offend-bank-supply; do
    if scan "$tmp/$p.patch" >/dev/null; then echo "SELF-TEST FAIL: $p was NOT refused"; rc=1; else echo "self-test ok: $p refused"; fi
  done
  for p in clean-read-vsel clean-iomux-write clean-card-vqmmc; do
    if scan "$tmp/$p.patch" >/dev/null; then echo "self-test ok: $p passed"; else echo "SELF-TEST FAIL: $p was refused"; rc=1; fi
  done
  [[ $rc -eq 0 ]] && echo "no-vsel self-test: PASS (5 offending patches refused, 3 clean patches accepted)" || echo "no-vsel self-test: FAIL"
  exit $rc
fi

if [[ $# -eq 0 ]]; then
  set -- "$here"/devices/families/rv11xx/patches/uboot/*.patch "$here"/devices/families/rv11xx/patches/linux/*.patch
fi
if scan "$@"; then
  echo "no-vsel: PASS ($# patch files, added lines: no IO-domain or VSEL write, no io-domains node, no IODOMAIN config, no pad-bank supply)"
  exit 0
else
  echo "no-vsel: FAIL (see the refusals above)"
  exit 1
fi
