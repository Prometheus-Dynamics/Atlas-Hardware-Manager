#!/usr/bin/env bash
# The slot root's users and enabled units: what Buildroot's fakeroot stage (mkusers, `systemctl preset-all`) leaves in
# the tree, checked on the EROFS the board boots. A root without it boots into a system whose daemons never start
# (the D1 rehearsal on builds 9 and 10, 2026-11-20: Gaia shipped output/target/ from before that stage, so passwd had
# only the skeleton's users and /etc/systemd/system had no enable links; Gaia FAKEROOT-OUTPUT).
#
#   scripts/check-slot-users.sh EROFS [PRODUCT.toml]   read with fsck.erofs --extract: no mount, no root
#   scripts/check-slot-users.sh --self-test
#   scripts/build-product.sh runs it, with the product, after every full build that makes an EROFS root.
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
#   - a unit's ReadWritePaths=, BindPaths= or BindReadOnlyPaths= names a /data path that is not in the image and that no
#     unit ordered before it makes (systemd sets up the namespace before the unit's own commands, ExecStartPre=+ too).
# With PRODUCT (the product.toml the root was built from), also:
#   - a unit a component of the product's image declares enabled ([[services]] enable = true; Gaia links what the
#     declarations say, not what the preset files say) has no enable link in the root;
#   - a user or group of the product's os/data-owners table (USER UID GROUP GID reason: the users that can own files on
#     /data, at fixed ids) is missing from the root or has another id there, or a sysusers.d line in the root names
#     one of them with another id or with none (`-`): no file on the image states an id other than the one that ships.
#   - a line of the product's os/units table (`linked UNIT reason` / `masked UNIT reason`: units the product depends on,
#     asserted by name) does not hold: a linked unit is masked, missing or has no enable link; a masked one is not masked.
set -euo pipefail

check() {  # <erofs> [product.toml]
  local erofs=$1 tmp rc=0
  for t in fsck.erofs python3; do command -v "$t" >/dev/null || { echo "check-slot-users: $t is needed" >&2; return 2; }; done
  [[ -f $erofs ]] || { echo "check-slot-users: no image: $erofs" >&2; return 2; }
  tmp=$(mktemp -d)
  fsck.erofs --extract="$tmp/root" --preserve-perms "$erofs" >"$tmp/fsck.log" 2>&1 \
    || { cat "$tmp/fsck.log" >&2; rm -rf "$tmp"; echo "check-slot-users: fsck.erofs could not read $erofs" >&2; return 1; }
  python3 -I - "$tmp/root" "$erofs" "${2:-}" <<'PY' || rc=$?
import fnmatch, os, re, sys, tomllib
root, erofs, product = sys.argv[1:]
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
# 4b. a /data path in a unit's mount namespace must be there before the namespace is set up (coordinator's ruling
# 2026-11-20, after the D1 rehearsal's orion-node: ReadWritePaths=/data/orion/node, made by its own `+` ExecStartPre,
# failed at NAMESPACE on every start). systemd sets up ReadWritePaths=/BindPaths=/BindReadOnlyPaths= before any of the
# unit's commands, so the unit cannot make its own: the path exists in the image, or a unit ordered before it (its
# After=, or the other unit's Before=) makes exactly that path in one of its Exec lines (install -d or mkdir). A path
# with the `-` prefix (missing is allowed) is skipped; /data itself is the image's mount point.
def ordered_before(v, u, ukv):
    if any(s_ == "Unit" and k == "After" and v in val.split() for s_, k, val in ukv): return True
    vk, vt = unit_texts(v)
    return vk == "file" and any(s_ == "Unit" and k == "Before" and u in val.split() for t in vt for s_, k, val in ini(t))
def makes(v, path):
    vk, vt = unit_texts(v)
    if vk != "file": return False
    tok = re.compile(r"(?<![\w/.-])" + re.escape(path) + r"/?(?![\w/.-])")
    for t in vt:
        for s_, k, val in ini(t):
            if s_ == "Service" and k.startswith("Exec") and tok.search(val) and (re.search(r"\binstall\b.*\s-d\b", val) or re.search(r"\bmkdir\b", val)):
                return True
    return False
for n in sorted(names):
    kind, texts = unit_texts(n)
    if kind != "file": continue
    kv = [x for t in texts for x in ini(t)]
    for s_, k, val in kv:
        if s_ != "Service" or k not in ("ReadWritePaths", "BindPaths", "BindReadOnlyPaths"): continue
        for w in val.split():
            if w.startswith("-"): continue
            path = w.lstrip("+").split(":", 1)[0].rstrip("/")
            if path != "/data" and not path.startswith("/data/"): continue
            if path == "/data" or os.path.lexists(os.path.join(root, path.lstrip("/"))): continue
            if not any(v != n and ordered_before(v, n, kv) and makes(v, path) for v in sorted(names)):
                bad.append(f"{n}: {k}={path} is set up before the unit's commands run, and no unit ordered before it makes "
                           f"{path} (its own ExecStartPre cannot: the namespace comes first)")
# 5. the product: its declared-enabled units and its data owners' fixed ids
uid_of = {}
for line in (rd("etc/passwd") or "").splitlines():
    f = line.split(":")
    if len(f) >= 4: uid_of[f[0]] = (f[2], f[3])
gid_of = {}
for line in (rd("etc/group") or "").splitlines():
    f = line.split(":")
    if len(f) >= 3: gid_of[f[0]] = f[2]
declared, owners, named = [], [], []
if product:
    pdir = os.path.dirname(os.path.abspath(product))
    with open(product, "rb") as f: prod = tomllib.load(f)
    comps = prod.get("components", {})
    for img in prod.get("images", []):
        for name in img.get("components", []):
            if name not in comps: continue
            with open(os.path.normpath(os.path.join(pdir, comps[name])), "rb") as f: c = tomllib.load(f)
            for sv in c.get("services", []):
                if sv.get("enable"): declared.append((sv["unit"], name))
    table = os.path.join(pdir, "os", "data-owners")
    if os.path.isfile(table):
        for n, line in enumerate(open(table), 1):
            t = line.strip()
            if not t or t.startswith("#"): continue
            f = t.split(None, 4)
            if len(f) < 5 or not f[1].isdigit() or not f[3].isdigit():
                bad.append(f"{table}:{n}: not USER UID GROUP GID reason: {t}"); continue
            owners.append((f[0], f[1], f[2], f[3]))
    utable = os.path.join(pdir, "os", "units")
    if os.path.isfile(utable):
        for n, line in enumerate(open(utable), 1):
            t = line.strip()
            if not t or t.startswith("#"): continue
            f = t.split(None, 2)
            if len(f) < 3 or f[0] not in ("linked", "masked"):
                bad.append(f"{utable}:{n}: not linked|masked UNIT reason: {t}"); continue
            named.append((f[0], f[1]))
for unit, comp in declared:
    tmpl = re.sub(r"@[^.]+\.", "@.", unit) if "@" in unit else unit
    kind, texts = unit_texts(tmpl)
    if kind != "file":
        bad.append(f"{unit}: declared enabled by component {comp}, but the root has {'it masked' if kind == 'masked' else 'no unit file'}"); continue
    kv = [x for t in texts for x in ini(t)]
    targets = [w for s_, k, v in kv if s_ == "Install" and k in ("WantedBy", "RequiredBy") for w in v.split()]
    if not targets: bad.append(f"{unit}: declared enabled by component {comp}, but its unit has no WantedBy=/RequiredBy=")
    elif not linked(unit, targets): bad.append(f"{unit}: declared enabled by component {comp}, but no enable link names it")
for want, unit in named:
    kind, texts = unit_texts(unit)
    if want == "masked":
        if kind != "masked": bad.append(f"{unit}: os/units says masked, the root has {'it unmasked' if kind == 'file' else 'no unit file'}")
        continue
    if kind != "file":
        bad.append(f"{unit}: os/units says linked, the root has {'it masked' if kind == 'masked' else 'no unit file'}"); continue
    kv = [x for t in texts for x in ini(t)]
    targets = [w for s_, k, v in kv if s_ == "Install" and k in ("WantedBy", "RequiredBy") for w in v.split()]
    if not targets or not linked(unit, targets): bad.append(f"{unit}: os/units says linked, but no enable link names it")
for user, uid, group, gid in owners:
    if user not in uid_of: bad.append(f"data owner {user}: not in the root's /etc/passwd (pinned {uid}:{gid})")
    elif uid_of[user] != (uid, gid): bad.append(f"data owner {user}: the root has {uid_of[user][0]}:{uid_of[user][1]}, the pin is {uid}:{gid} (os/data-owners)")
    if group not in gid_of: bad.append(f"data owner group {group}: not in the root's /etc/group (pinned {gid})")
    elif gid_of[group] != gid: bad.append(f"data owner group {group}: the root has gid {gid_of[group]}, the pin is {gid} (os/data-owners)")
    for c, rel in sorted(seen.items()):
        for line in (rd(rel) or "").splitlines():
            f = line.split()
            if len(f) >= 3 and f[0][:1] in ("u", "g") and f[1] in (user, group):
                want = uid if f[0].startswith("u") and f[1] == user else gid
                idf = f[2].split(":", 1)[0]
                if idf != want: bad.append(f"{rel}: {line.strip()}: states id {idf} for {f[1]}, the image ships {want} (os/data-owners)")
for x in bad: print(f"check-slot-users: BAD {x}")
if bad: print(f"check-slot-users: FAILED ({len(bad)} finding(s)) in {erofs}"); sys.exit(1)
print(f"check-slot-users: ok: {len(users)} users, {len(groups)} groups; every unit's and sysusers.d's names present; every preset-enabled unit linked" + (f"; {len(declared)} declared-enabled unit(s) linked, {len(owners)} data owner(s) at their pinned ids, {len(named)} os/units line(s) as stated" if product else ""))
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
    if [[ " $* " == *" dyn-line "* ]]; then printf 'u lemnos - "Lemnos daemon"\n' > "$r/usr/lib/sysusers.d/x.conf"
    else printf 'u lemnos 900 "Lemnos daemon"\n' > "$r/usr/lib/sysusers.d/x.conf"; fi
    printf 'u orion 901 "Orion"\nm orion dialout\n' >> "$r/usr/lib/sysusers.d/x.conf"
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
    # The /data namespace rule: maker.service makes /data/y, orion-node is ordered after it and lists /data/y (and an
    # optional -/data/opt) as writable.
    printf '[Service]\nType=oneshot\nExecStartPre=/bin/sh -c '"'"'install -d -m 0750 /data/y'"'"'\nExecStart=/bin/true\n' > "$S/maker.service"
    mkdir -p "$E/orion-node.service.d"
    printf '[Unit]\nAfter=maker.service\n[Service]\nReadWritePaths=/data/y -/data/opt\n' > "$E/orion-node.service.d/10-state.conf"
    for a in "$@"; do case $a in
      unit-user) mkdir -p "$E/orion-node.service.d"; printf '[Service]\nUser=nobody2\n' > "$E/orion-node.service.d/u.conf" ;;
      sysusers) printf 'u dbus - "System Message Bus"\n' >> "$r/usr/lib/sysusers.d/x.conf" ;;
      member) printf 'root:x:0:\nlemnos:x:900:\norion:x:901:\ndialout:x:18:\n' > "$r/etc/group" ;;
      nolink) rm "$E/multi-user.target.wants/orion-node.service" ;;
      nolemnosd) rm "$E/multi-user.target.wants/lemnosd.service"; printf 'disable lemnosd.service\n' > "$r/usr/lib/systemd/system-preset/10-x.preset" ;;
      tmpl) printf 'enable inst@.service x\n' >> "$r/usr/lib/systemd/system-preset/80-x.preset" ;;
      wrong-id) sed -i 's/^lemnos:x:900:900/lemnos:x:905:900/' "$r/etc/passwd" ;;
      wrong-gid) sed -i 's/^orion:x:901:$/orion:x:951:/' "$r/etc/group" ;;
      dyn-line) : ;;
      nolink-declared) rm "$E/multi-user.target.wants/orion-node.service"; printf 'disable orion-node.service\n' > "$r/usr/lib/systemd/system-preset/10-x.preset" ;;
      unmask) rm "$E/masked.service" ;;
      rwp-self) printf '[Service]\nExecStartPre=+/bin/sh -c '"'"'install -d /data/z'"'"'\nReadWritePaths=/data/z\n' > "$E/orion-node.service.d/20-self.conf" ;;
      rwp-unordered) printf '[Unit]\nAfter=\n[Service]\nReadWritePaths=/data/y\n' > "$E/orion-node.service.d/10-state.conf" ;;
      nolemnos) sed -i '/^lemnos:/d' "$r/etc/passwd" "$r/etc/group"; sed -i '/lemnos/d' "$r/usr/lib/sysusers.d/x.conf" ;;
    esac; done
    mkfs.erofs "$tmp/$n.erofs" "$r" >/dev/null 2>&1
  }
  expect_fail() {  # <name> <text>   (a failing check is expected: its words are kept, then read)
    check "$tmp/$1.erofs" > "$tmp/$1.log" 2>&1 && { echo "check-slot-users: self-test: $1 passed" >&2; rc=1; return; }
    grep -qF -- "$2" "$tmp/$1.log" || { echo "check-slot-users: self-test: $1 failed without naming '$2'" >&2; cat "$tmp/$1.log" >&2; rc=1; }
  }
  # A product for the product mode: one component declaring lemnosd and orion-node enabled, and a data-owners table.
  mkdir -p "$tmp/prod/os" "$tmp/prod/c"
  printf 'version = 1\nname = "p"\n[components]\nc = "c/component.toml"\n[[images]]\nname = "img"\ncomponents = ["c"]\n' > "$tmp/prod/product.toml"
  printf 'version = 1\nname = "c"\n[[services]]\nunit = "lemnosd.service"\nenable = true\n[[services]]\nunit = "orion-node.service"\nenable = true\n' > "$tmp/prod/c/component.toml"
  printf '# t\nlemnos 900 lemnos 900  test\norion 901 orion 901  test\n' > "$tmp/prod/os/data-owners"
  printf '# t\nlinked orion-node.service  test\nmasked masked.service  test\n' > "$tmp/prod/os/units"
  expect_fail_p() {  # <name> <text>   (the product mode)
    check "$tmp/$1.erofs" "$tmp/prod/product.toml" > "$tmp/$1.p.log" 2>&1 && { echo "check-slot-users: self-test: $1 passed (product)" >&2; rc=1; return; }
    grep -qF -- "$2" "$tmp/$1.p.log" || { echo "check-slot-users: self-test: $1 failed (product) without naming '$2'" >&2; cat "$tmp/$1.p.log" >&2; rc=1; }
  }
  mk good
  check "$tmp/good.erofs" "$tmp/prod/product.toml" > "$tmp/good.p.log" 2>&1 || { cat "$tmp/good.p.log" >&2; echo "check-slot-users: self-test: the good root failed with its product" >&2; rc=1; }
  mk wrongid wrong-id;    expect_fail_p wrongid "data owner lemnos: the root has 905:900, the pin is 900:900"
  mk wronggid wrong-gid;  expect_fail_p wronggid "data owner group orion: the root has gid 951, the pin is 901"
  mk unmasked unmask;     expect_fail_p unmasked "masked.service: os/units says masked, the root has it unmasked"
  mk rwpself rwp-self;           expect_fail rwpself "orion-node.service: ReadWritePaths=/data/z is set up before the unit's commands run, and no unit ordered before it makes /data/z"
  mk rwpunordered rwp-unordered; expect_fail rwpunordered "orion-node.service: ReadWritePaths=/data/y is set up before the unit's commands run, and no unit ordered before it makes /data/y"
  mk unitnolink nolink;   expect_fail_p unitnolink "orion-node.service: os/units says linked, but no enable link names it"
  mk dynline dyn-line;    expect_fail_p dynline "states id - for lemnos, the image ships 900"
  mk undeclared nolink-declared; expect_fail_p undeclared "orion-node.service: declared enabled by component c, but no enable link names it"
  check "$tmp/good.erofs" > "$tmp/good.log" 2>&1 || { cat "$tmp/good.log" >&2; echo "check-slot-users: self-test: the good root failed" >&2; rc=1; }
  mk unituser unit-user;  expect_fail unituser "orion-node.service: User=nobody2, but the root's /etc/passwd has no nobody2"
  mk sysusers sysusers;   expect_fail sysusers "user dbus is not in the root's /etc/passwd"
  mk member member;       expect_fail member "orion is not a member of dialout"
  mk nolink nolink;       expect_fail nolink "orion-node.service: enabled by usr/lib/systemd/system-preset/80-x.preset, but no enable link names it"
  mk nolemnosd nolemnosd; expect_fail nolemnosd "lemnosd.service is not enabled"
  mk nolemnos nolemnos;   expect_fail nolemnos "user or group lemnos is missing"
  [[ $rc -eq 0 ]] && echo "check-slot-users: self-test: passed (a unit's missing user (drop-in), a sysusers user missing, a missing membership, a preset-enabled unit without a link, lemnosd not enabled, no lemnos user and a preset-named template instance without a link each fail; with a product, a data owner at another uid or gid, a sysusers line stating no id for one, a declared-enabled unit without a link, an os/units unit unmasked or without its link fail; a /data ReadWritePaths= made only by the unit itself, or by a unit not ordered before it, fails, and one made by a unit ordered before it, or with the - prefix, passes; DynamicUser, %i, a masked unit, an alias, a DefaultInstance and a preset disable pass)"
  return $rc
}

case ${1:-} in
  --self-test) self_test ;;
  -h|--help|"") sed -n '2,32p' "$0" ;;
  *) [[ $# -eq 1 || $# -eq 2 ]] || { echo "usage: scripts/check-slot-users.sh EROFS [PRODUCT.toml]" >&2; exit 2; }; check "$@" ;;
esac
