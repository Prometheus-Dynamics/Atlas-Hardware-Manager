#!/usr/bin/env bash
# The slot root's users and enabled units: what Buildroot's fakeroot stage (mkusers, `systemctl preset-all`) leaves in
# the tree, checked on the EROFS the board boots. A root without it boots into a system whose daemons never start
# (the D1 rehearsal on builds 9 and 10, 2026-11-20: Gaia shipped output/target/ from before that stage, so passwd had
# only the skeleton's users and /etc/systemd/system had no enable links; Gaia FAKEROOT-OUTPUT).
#
#   scripts/check-slot-users.sh EROFS     read with fsck.erofs --extract: no mount, no root
#   scripts/check-slot-users.sh --self-test
#   scripts/build-product.sh runs it after every full build that makes an EROFS root.
#
# Fails when:
#   - a unit's User=, Group= or SupplementaryGroups= (the unit file or a drop-in, in /usr/lib or /etc) names a user or
#     group missing from the root's /etc/passwd or /etc/group. Not checked: a unit with DynamicUser=yes (systemd makes
#     the user), a value with a specifier (%i) or a number;
#   - a sysusers.d entry names a user or group the root lacks: `u` its user and its group, `g` its group, `m` both and
#     the membership. sysusers cannot add them at boot: the root is read-only;
#   - a unit the presets enable has no enable link: the unit has an [Install] section with WantedBy=/RequiredBy=, it
#     is not masked, and the first matching preset line (files by name, /etc before /usr/lib for one name, then
#     systemd's default: enable) says enable, yet no <target>.wants/ or .requires/ link names it. An alias (a link to
#     another unit) is skipped; a template counts only through the instances its preset line names (not
#     DefaultInstance=: Buildroot removes the tty1 getty link when its console getty is elsewhere);
#   - lemnosd.service is in the root but not enabled, or user or group `lemnos` is missing: lemnosd owns the hardware
#     watchdog slot-init arms (scripts/check-watchdog-owner.sh), so a lemnosd that does not start resets the board.
set -euo pipefail

check() {  # <erofs>
  local erofs=$1 tmp rc=0
  for t in fsck.erofs python3; do command -v "$t" >/dev/null || { echo "check-slot-users: $t is needed" >&2; return 2; }; done
  [[ -f $erofs ]] || { echo "check-slot-users: no image: $erofs" >&2; return 2; }
  tmp=$(mktemp -d)
  fsck.erofs --extract="$tmp/root" --preserve-perms "$erofs" >"$tmp/fsck.log" 2>&1 \
    || { cat "$tmp/fsck.log" >&2; rm -rf "$tmp"; echo "check-slot-users: fsck.erofs could not read $erofs" >&2; return 1; }
  python3 -I - "$tmp/root" "$erofs" <<'PY' || rc=$?
import fnmatch, os, re, sys
root, erofs = sys.argv[1:]
bad = []
def rd(rel):
    p = os.path.join(root, rel)
    try:
        with open(p, errors="replace") as f: return f.read()
    except OSError: return None
users, groups, members = set(), set(), {}
primary = {}
for line in (rd("etc/passwd") or "").splitlines():
    f = line.split(":")
    if len(f) >= 4: users.add(f[0]); primary[f[0]] = f[3]
gid_name = {}
for line in (rd("etc/group") or "").splitlines():
    f = line.split(":")
    if len(f) >= 4:
        groups.add(f[0]); gid_name[f[2]] = f[0]
        members[f[0]] = set(x for x in f[3].split(",") if x)
UNIT_DIRS = ["etc/systemd/system", "usr/lib/systemd/system"]
def ini(text):
    """[(section, key, value)] of a unit file; continuation lines joined."""
    out, sec, buf = [], None, ""
    for raw in text.splitlines():
        line = buf + raw.strip(); buf = ""
        if line.endswith("\\"): buf = line[:-1] + " "; continue
        if not line or line[0] in "#;": continue
        if line.startswith("[") and line.endswith("]"): sec = line[1:-1]; continue
        if "=" in line:
            k, v = line.split("=", 1); out.append((sec, k.strip(), v.strip()))
    return out
def unit_texts(name):
    """The unit file (first dir that has it, /etc first) and its drop-ins, as the unit's sections and keys."""
    texts, main = [], None
    for d in UNIT_DIRS:
        p = os.path.join(root, d, name)
        if os.path.lexists(p):
            main = p; break
    if main is None: return None, []
    if os.path.islink(main) and os.readlink(main) == "/dev/null": return "masked", []
    t = rd(os.path.relpath(main, root))
    if t is not None: texts.append(t)
    base = name
    tmpl = re.sub(r"@[^.]*\.", "@.", name) if "@" in name else None
    for d in UNIT_DIRS:
        for n in [base] + ([tmpl] if tmpl and tmpl != base else []):
            dd = os.path.join(root, d, n + ".d")
            if os.path.isdir(dd):
                for c in sorted(os.listdir(dd)):
                    if c.endswith(".conf"):
                        t = rd(os.path.relpath(os.path.join(dd, c), root))
                        if t is not None: texts.append(t)
    return "file", texts
names = set()
for d in UNIT_DIRS:
    p = os.path.join(root, d)
    if os.path.isdir(p):
        for n in os.listdir(p):
            if re.search(r"\.(service|socket|timer|path|target|mount|automount|swap)$", n): names.add(n)
# 1. users and groups the units name
for n in sorted(names):
    kind, texts = unit_texts(n)
    if kind != "file": continue
    kv = [x for t in texts for x in ini(t)]
    if any(k == "DynamicUser" and v.lower() in ("yes", "true", "1", "on") for s, k, v in kv if s == "Service"): continue
    for s, k, v in kv:
        if s != "Service" or k not in ("User", "Group", "SupplementaryGroups"): continue
        for w in v.split():
            if "%" in w or w.isdigit(): continue
            if k == "User" and w not in users: bad.append(f"{n}: User={w}, but the root's /etc/passwd has no {w}")
            if k != "User" and w not in groups: bad.append(f"{n}: {k}={w}, but the root's /etc/group has no {w}")
# 2. sysusers.d
seen = {}
for d in ["usr/lib/sysusers.d", "etc/sysusers.d"]:
    p = os.path.join(root, d)
    if os.path.isdir(p):
        for c in os.listdir(p):
            if c.endswith(".conf"): seen[c] = os.path.join(d, c)
for c, rel in sorted(seen.items()):
    for line in (rd(rel) or "").splitlines():
        s = line.strip()
        if not s or s.startswith("#"): continue
        f = s.split()
        if len(f) < 2: continue
        t, name = f[0], f[1]
        where = f"{rel}: {s}"
        if t.startswith("u"):
            if name not in users: bad.append(f"{where}: user {name} is not in the root's /etc/passwd")
            idf = f[2] if len(f) > 2 else "-"
            grp = idf.split(":", 1)[1] if ":" in idf else name
            if not grp.isdigit() and grp not in groups: bad.append(f"{where}: group {grp} is not in the root's /etc/group")
        elif t.startswith("g"):
            if name not in groups: bad.append(f"{where}: group {name} is not in the root's /etc/group")
        elif t.startswith("m") and len(f) >= 3:
            g = f[2]
            if name not in users: bad.append(f"{where}: user {name} is not in the root's /etc/passwd")
            elif g not in groups: bad.append(f"{where}: group {g} is not in the root's /etc/group")
            elif name not in members.get(g, set()) and gid_name.get(primary.get(name)) != g:
                bad.append(f"{where}: {name} is not a member of {g} in the root's /etc/group")
# 3. presets and enable links
pfiles = {}
for d in ["usr/lib/systemd/system-preset", "etc/systemd/system-preset"]:
    p = os.path.join(root, d)
    if os.path.isdir(p):
        for c in os.listdir(p):
            if c.endswith(".preset"): pfiles[c] = os.path.join(d, c)  # /etc wins for one name (read last)
rules = []
for c in sorted(pfiles):
    for line in (rd(pfiles[c]) or "").splitlines():
        s = line.strip()
        if not s or s[0] in "#;": continue
        f = s.split()
        if f[0] in ("enable", "disable", "ignore") and len(f) >= 2: rules.append((f[0], f[1], f[2:], pfiles[c]))
def preset(n):
    for act, pat, inst, src in rules:
        if fnmatch.fnmatchcase(n, pat): return act, inst, src
    return "enable", [], "systemd's default (no preset line matches)"
def linked(n, targets):
    for tgt in targets:
        for d in UNIT_DIRS:
            for suf in (".wants", ".requires"):
                if os.path.lexists(os.path.join(root, d, tgt + suf, n)): return True
    return False
def enabled(n):
    """True / False / None (nothing to enable) for the unit n (an instance name for a template)."""
    kind, texts = unit_texts(re.sub(r"@[^.]+\.", "@.", n) if "@" in n else n)
    if kind != "file": return None
    kv = [x for t in texts for x in ini(t)]
    targets = [w for s, k, v in kv if s == "Install" and k in ("WantedBy", "RequiredBy") for w in v.split()]
    if not targets: return None
    return linked(n, targets)
def alias(n):
    """A unit-directory entry that links to another unit (an alias: dbus-org.*, autovt@): preset-all leaves it be."""
    for d in UNIT_DIRS:
        p = os.path.join(root, d, n)
        if os.path.lexists(p): return os.path.islink(p) and os.readlink(p) != "/dev/null"
    return False
for n in sorted(names):
    act, inst, src = preset(n)
    if act != "enable" or alias(n): continue
    if re.search(r"@\.", n):
        # A template: only the instances its preset line names. Not DefaultInstance=: Buildroot removes the tty1 getty
        # link preset-all makes when its console getty is elsewhere (993's root has none either).
        for i in inst:
            name = n.replace("@.", f"@{i}.")
            if enabled(name) is False: bad.append(f"{name}: enabled by {src}, but no enable link names it")
        continue
    if enabled(n) is False: bad.append(f"{n}: enabled by {src}, but no enable link names it")
# 4. lemnosd: the watchdog's owner
if "lemnosd.service" in names:
    kind, texts = unit_texts("lemnosd.service")
    kv = [x for t in texts for x in ini(t)] if kind == "file" else []
    targets = [w for s, k, v in kv if s == "Install" and k in ("WantedBy", "RequiredBy") for w in v.split()]
    if kind != "file" or not linked("lemnosd.service", targets):
        bad.append("lemnosd.service is not enabled: it owns the hardware watchdog slot-init arms, so the board would reset")
    if "lemnos" not in users or "lemnos" not in groups:
        bad.append("user or group lemnos is missing: lemnosd drops to it after its root phase (it owns the watchdog)")
for x in bad: print(f"check-slot-users: BAD {x}")
if bad: print(f"check-slot-users: FAILED ({len(bad)} finding(s)) in {erofs}"); sys.exit(1)
print(f"check-slot-users: ok: {len(users)} users, {len(groups)} groups; every unit's and sysusers.d's names present; every preset-enabled unit linked")
PY
  rm -rf "$tmp"
  return $rc
}

self_test() {
  local tmp rc=0
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' RETURN
  command -v mkfs.erofs >/dev/null || { echo "check-slot-users: self-test needs mkfs.erofs" >&2; return 2; }
  mk() {  # <name> [fault...]
    local n=$1 r="$tmp/$1" a; shift
    local S=$r/usr/lib/systemd/system E=$r/etc/systemd/system
    mkdir -p "$S" "$E/multi-user.target.wants" "$r/usr/lib/systemd/system-preset" "$r/usr/lib/sysusers.d" "$r/etc"
    printf 'root:x:0:0:root:/root:/bin/sh\nlemnos:x:900:900::/:/bin/false\norion:x:901:901::/:/bin/false\n' > "$r/etc/passwd"
    printf 'root:x:0:\nlemnos:x:900:\norion:x:901:\ndialout:x:18:orion\n' > "$r/etc/group"
    printf 'u lemnos - "Lemnos daemon"\nu orion 901 "Orion"\nm orion dialout\n' > "$r/usr/lib/sysusers.d/x.conf"
    printf '[Service]\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n' > "$S/lemnosd.service"
    printf '[Service]\nUser=orion\nGroup=orion\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n' > "$S/orion-node.service"
    printf '[Service]\nDynamicUser=yes\nUser=ghost\nExecStart=/bin/true\n' > "$S/dyn.service"
    printf '[Service]\nUser=%%i\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n' > "$S/inst@.service"
    printf '[Service]\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n' > "$S/off.service"
    printf '[Service]\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n' > "$S/masked.service"
    ln -s /dev/null "$E/masked.service"
    ln -s orion-node.service "$S/dbus-org.example.service"   # an alias: skipped
    printf '[Service]\nExecStart=/bin/true\n[Install]\nWantedBy=getty.target\nDefaultInstance=tty1\n' > "$S/tty@.service"
    printf 'disable off.service\nenable orion-node.service\n' > "$r/usr/lib/systemd/system-preset/80-x.preset"
    ln -s ../../../../usr/lib/systemd/system/lemnosd.service "$E/multi-user.target.wants/lemnosd.service"
    ln -s ../../../../usr/lib/systemd/system/orion-node.service "$E/multi-user.target.wants/orion-node.service"
    for a in "$@"; do case $a in
      unit-user) mkdir -p "$E/orion-node.service.d"; printf '[Service]\nUser=nobody2\n' > "$E/orion-node.service.d/u.conf" ;;
      sysusers) printf 'u dbus - "System Message Bus"\n' >> "$r/usr/lib/sysusers.d/x.conf" ;;
      member) printf 'root:x:0:\nlemnos:x:900:\norion:x:901:\ndialout:x:18:\n' > "$r/etc/group" ;;
      nolink) rm "$E/multi-user.target.wants/orion-node.service" ;;
      nolemnosd) rm "$E/multi-user.target.wants/lemnosd.service"; printf 'disable lemnosd.service\n' > "$r/usr/lib/systemd/system-preset/10-x.preset" ;;
      tmpl) printf 'enable inst@.service x\n' >> "$r/usr/lib/systemd/system-preset/80-x.preset" ;;
      nolemnos) sed -i '/^lemnos:/d' "$r/etc/passwd" "$r/etc/group"; sed -i '/lemnos/d' "$r/usr/lib/sysusers.d/x.conf" ;;
    esac; done
    mkfs.erofs "$tmp/$n.erofs" "$r" >/dev/null 2>&1
  }
  expect_fail() {  # <name> <text>   (a failing check is expected: its words are kept, then read)
    check "$tmp/$1.erofs" > "$tmp/$1.log" 2>&1 && { echo "check-slot-users: self-test: $1 passed" >&2; rc=1; return; }
    grep -qF -- "$2" "$tmp/$1.log" || { echo "check-slot-users: self-test: $1 failed without naming '$2'" >&2; cat "$tmp/$1.log" >&2; rc=1; }
  }
  mk good
  check "$tmp/good.erofs" > "$tmp/good.log" 2>&1 || { cat "$tmp/good.log" >&2; echo "check-slot-users: self-test: the good root failed" >&2; rc=1; }
  mk unituser unit-user;  expect_fail unituser "orion-node.service: User=nobody2, but the root's /etc/passwd has no nobody2"
  mk sysusers sysusers;   expect_fail sysusers "user dbus is not in the root's /etc/passwd"
  mk member member;       expect_fail member "orion is not a member of dialout"
  mk nolink nolink;       expect_fail nolink "orion-node.service: enabled by usr/lib/systemd/system-preset/80-x.preset, but no enable link names it"
  mk nolemnosd nolemnosd; expect_fail nolemnosd "lemnosd.service is not enabled"
  mk nolemnos nolemnos;   expect_fail nolemnos "user or group lemnos is missing"
  [[ $rc -eq 0 ]] && echo "check-slot-users: self-test: passed (a unit's missing user (drop-in), a sysusers user missing, a missing membership, a preset-enabled unit without a link, lemnosd not enabled, no lemnos user and a preset-named template instance without a link each fail; DynamicUser, %i, a masked unit, an alias, a DefaultInstance and a preset disable pass)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,26p' "$0" ;;
  *) [[ $# -eq 1 ]] || { echo "usage: scripts/check-slot-users.sh EROFS" >&2; exit 2; }; check "$1" ;;
esac
