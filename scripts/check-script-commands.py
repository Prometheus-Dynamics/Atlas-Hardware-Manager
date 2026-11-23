#!/usr/bin/env python3
"""Every shell script an image ships calls only commands that image has.

Why: board-journal called chown and chmod, which the base image's BusyBox does not have, so its unit failed at every
boot and the journal stayed in RAM (seen on a board, 2026-11-23). The off-board check ran the script with the PC's
tools and could not see it.

The scripts of a product: every `[[install]]` of a file in the components its product.toml declares, and every file
of its os/overlay, that starts with a shell `#!` line or lands in /etc/profile or /etc/profile.d.

The claim is one file per product, devices/products/<product>/script-commands: one command per line, then where it
comes from:
    install   busybox          a BusyBox applet
    echo      ash              a builtin of BusyBox's shell
    mount     util-linux       any other word: the package whose program it is
Three checks hold it:
  the gate (no arguments)   every command a shipped script calls is on its product's list, and every line of the list
                            is called (no stale lines). Where the product's BusyBox config is a whole file
                            (os/busybox.config), each `busybox` and `ash` line is an option that file turns on. A
                            product whose BusyBox is Buildroot's default plus a fragment has no whole config in this
                            tree: its list is held to the image by the build only.
  --root DIR PRODUCT        after a build (scripts/build-product.sh): every listed command is in the root the build
                            made, a `busybox` line as a link to busybox and any other as a program of its own; every
                            literal absolute program a script runs is there; each shipped script is in the root with
                            the bytes of its source. `ash` lines are not read from a root (a builtin is no file).
  --os-root DIR PRODUCT     the same for a product whose image is not an EROFS root: DIR is the tree Buildroot made,
                            before the components are placed, so the commands are read and the scripts are not.
  --self-test               the reader of shell on made-up scripts, the old board-journal among them.

What the reader does not follow, and says: a command held in a variable ("$TOOL" args), the text given to eval or
sh -c, and a program named only inside another program's arguments (find -exec, xargs). Such a script is refused
unless the word is on NOT_FOLLOWED below with its reason.
"""
import os
import re
import sys
import tomllib

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PRODUCTS = os.path.join(REPO, "devices", "products")
SHELLS = ("/bin/sh", "/bin/ash")
# Always in BusyBox ash, whatever its config (shell/ash.c builtintab, the entries with no #if).
BUILTINS = set(": . break cd command continue exec exit export false local read readonly return set shift "
               "times trap true type umask unset wait".split())
# ash builtins that an option turns on: the list names them `ash`.
ASH_OPTIONS = {"echo": "CONFIG_ASH_ECHO", "printf": "CONFIG_ASH_PRINTF", "test": "CONFIG_ASH_TEST",
               "[": "CONFIG_ASH_TEST", "kill": "CONFIG_ASH_JOB_CONTROL", "getopts": "CONFIG_ASH_GETOPTS",
               "alias": "CONFIG_ASH_ALIAS", "sleep": "CONFIG_ASH_SLEEP"}
KEYWORDS = set("if then else elif fi do done while until for case esac in { } !".split())
# Words that hand their arguments to something this reader does not read. None is shipped today.
NOT_FOLLOWED = {"eval": None, "xargs": None, "find": None, "sh": None, "ash": None, "env": None, "nice": None,
                "nohup": None, "timeout": None, "su": None, "chroot": None}
PATH_DIRS = ("usr/bin", "usr/sbin", "bin", "sbin")


class Unread(Exception):
    pass


def clean(text, line=1):
    """The script with quotes, variables and arithmetic replaced by one-letter marks, comments and here-documents
    dropped, and each command substitution pulled out. Returns [(chars, lines)]: the fragment itself first, then one
    per substitution, each with the line of every character."""
    frags, out, lines = [], [], []
    i, n = 0, len(text)

    def emit(c, at):
        out.append(c)
        lines.append(at)

    def closing(start, open_c, close_c):
        depth, j = 1, start
        while j < n:
            c = text[j]
            if c == "\\": j += 2; continue
            if c == "'" and open_c != "{":
                j = text.index("'", j + 1) + 1 if "'" in text[j + 1:] else n; continue
            if c == open_c: depth += 1
            elif c == close_c:
                depth -= 1
                if depth == 0: return j
            j += 1
        raise Unread(f"line {line}: no closing {close_c}")

    def dollar(at_line):
        """At text[i] == '$'. Consumes it; returns the mark."""
        nonlocal i, line
        if text.startswith("$((", i):
            end = closing(i + 3, "(", ")")
            if not text.startswith("))", end): raise Unread(f"line {line}: $(( without ))")
            line += text.count("\n", i, end); i = end + 2
            return "A"
        if text.startswith("$(", i):
            end = closing(i + 2, "(", ")")
            frags.extend(clean(text[i + 2:end], line))
            line += text.count("\n", i, end); i = end + 1
            return "S"
        if text.startswith("${", i):
            end = closing(i + 2, "{", "}")
            inner = text[i + 2:end]
            line += text.count("\n", i, end); i = end + 1
            m = re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*:?-(/[A-Za-z0-9_./-]+)", inner)
            return m.group(1) if m else "V"  # ${TOOL:-/usr/bin/tool}: the default is what the image runs
        m = re.match(r"\$([A-Za-z_][A-Za-z0-9_]*|[0-9@*#?!$-])", text[i:])
        if m: i += len(m.group(0)); return "V"
        i += 1
        return "$"

    heredocs = []
    while i < n:
        c = text[i]
        if c == "\\":
            if i + 1 < n and text[i + 1] == "\n": line += 1
            else: emit("E", line)
            i += 2
        elif c == "\n":
            emit("\n", line); line += 1; i += 1
            for word, strip in heredocs:  # the bodies start after this newline
                while i < n:
                    end = text.find("\n", i)
                    end = n if end < 0 else end
                    body = text[i:end]
                    i = min(end + 1, n); line += 1
                    if (body.lstrip("\t") if strip else body) == word: break
                else:
                    raise Unread(f"here-document {word} is not closed")
            heredocs = []
        elif c == "#" and (not out or out[-1] in " \t\n;&|()"):
            end = text.find("\n", i)
            i = n if end < 0 else end
        elif c == "'":
            end = text.find("'", i + 1)
            if end < 0: raise Unread(f"line {line}: no closing '")
            emit("Q", line); line += text.count("\n", i, end); i = end + 1
        elif c == '"':
            at = line
            i += 1
            while True:
                if i >= n: raise Unread(f"line {at}: no closing \"")
                d = text[i]
                if d == '"': i += 1; break
                if d == "\\": i += 2
                elif d == "$": dollar(line)
                elif d == "`":
                    end = text.find("`", i + 1)
                    if end < 0: raise Unread(f"line {line}: no closing `")
                    frags.extend(clean(text[i + 1:end], line)); line += text.count("\n", i, end); i = end + 1
                else:
                    if d == "\n": line += 1
                    i += 1
            emit("Q", at)
        elif c == "`":
            end = text.find("`", i + 1)
            if end < 0: raise Unread(f"line {line}: no closing `")
            frags.extend(clean(text[i + 1:end], line)); emit("S", line)
            line += text.count("\n", i, end); i = end + 1
        elif c == "$":
            at = line
            for ch in dollar(at): emit(ch, at)
        elif text.startswith("<<", i) and not text.startswith("<<<", i):
            m = re.match(r"<<(-?)\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\2", text[i:])
            if not m: raise Unread(f"line {line}: a here-document this reader does not read")
            heredocs.append((m.group(3), bool(m.group(1))))
            i += len(m.group(0))
        else:
            emit(c, line); i += 1
    return [(out, lines)] + frags


REDIRECT = re.compile(r"[0-9]*(>>|>|<)(&[0-9-]|[ \t]*[^ \t\n;&|()]+)")


def commands(text):
    """[(word, line)] of every command word, and the names of the functions the script defines."""
    found, functions = [], set()
    for chars, lines in clean(text):
        s = "".join(chars)
        # Redirections out, keeping each character's line: blank them.
        s = REDIRECT.sub(lambda m: " " * len(m.group(0)), s)
        tokens = [(m.group(0), lines[m.start()]) for m in
                  re.finditer(r"\n|;;|&&|\|\||[;|&()]|[^ \t\n;&|()]+", s)]
        seps = {"\n", ";", ";;", "&&", "||", "|", "&", "(", ")"}
        cmd, case_depth, pattern, k = True, 0, False, 0
        while k < len(tokens):
            tok, at = tokens[k]
            k += 1
            if pattern:  # case patterns up to their ')'
                if tok == "esac": case_depth -= 1; pattern = False; cmd = False
                elif tok == ")": pattern = False; cmd = True
                continue
            if tok in seps:
                cmd = True
                if tok == ";;" and case_depth: pattern = True
                continue
            if not cmd: continue
            if tok in KEYWORDS:
                if tok == "for":  # for NAME [in words]: the words are not commands
                    while k < len(tokens) and tokens[k][0] not in (";", "\n"): k += 1
                elif tok == "case":
                    while k < len(tokens) and tokens[k][0] != "in": k += 1
                    k += 1; case_depth += 1; pattern = True
                    while k < len(tokens) and tokens[k][0] == "\n": k += 1
                elif tok == "esac": case_depth -= 1; cmd = False
                elif tok in ("fi", "done", "}"): cmd = False
                continue
            if re.match(r"[A-Za-z_][A-Za-z0-9_]*=", tok): continue  # an assignment before the command
            if k + 1 < len(tokens) and tokens[k][0] == "(" and tokens[k + 1][0] == ")":
                functions.add(tok); k += 2; continue
            cmd = False
            if tok in ("exec", "command"):  # the next word, if any, is the command
                cmd = True
                found.append((tok, at))
                continue
            found.append((tok, at))
    return found, functions


def calls(text):
    """(names, absolutes, refused): bare command names with their first line, literal absolute programs, and the
    words this reader cannot follow."""
    found, functions = commands(text)
    names, absolutes, refused = {}, {}, []
    for word, at in found:
        if word in functions or word in BUILTINS: continue
        if word.startswith("/"):
            if re.fullmatch(r"/[A-Za-z0-9_./-]+", word): absolutes.setdefault(word, at)
            else: refused.append((at, word, "a program path this reader cannot resolve"))
            continue
        if re.fullmatch(r"[QVS]+", word):
            continue  # "$TOOL" args: said in the docstring; the variable's default is read where it is given
        if not re.fullmatch(r"[A-Za-z0-9_.\[+-]+", word):
            refused.append((at, word, "not a plain command word")); continue
        if word in NOT_FOLLOWED and NOT_FOLLOWED[word] is None:
            refused.append((at, word, "hands its arguments to a command this check does not read"))
        names.setdefault(word, at)
    return names, absolutes, refused


def is_script(data, to):
    first = data.split(b"\n", 1)[0].decode("utf-8", "replace")
    if first.startswith("#!"):
        return first[2:].split()[0] if first[2:].split() else ""
    if to == "/etc/profile" or to.startswith("/etc/profile.d/"): return SHELLS[0]
    return None


def shipped(product_dir):
    """[(source path, path in the image, interpreter)] of the product's shell scripts."""
    with open(os.path.join(product_dir, "product.toml"), "rb") as f: product = tomllib.load(f)
    out = []
    for name, rel in sorted(product.get("components", {}).items()):
        cpath = os.path.normpath(os.path.join(product_dir, rel))
        with open(cpath, "rb") as f: comp = tomllib.load(f)
        for inst in comp.get("install", []):
            src = inst.get("from")
            src = src.get("file") if isinstance(src, dict) else None
            if not src: continue
            path = os.path.join(os.path.dirname(cpath), src)
            if not os.path.isfile(path): continue
            with open(path, "rb") as f: interp = is_script(f.read(512), inst.get("to", ""))
            if interp is not None: out.append((path, inst.get("to", ""), interp))
    overlay = os.path.join(product_dir, "os", "overlay")
    for d, _, files in os.walk(overlay):
        for n in sorted(files):
            path = os.path.join(d, n)
            to = "/" + os.path.relpath(path, overlay)
            with open(path, "rb") as f: interp = is_script(f.read(512), to)
            if interp is not None: out.append((path, to, interp))
    return out


def read_list(product_dir):
    path = os.path.join(product_dir, "script-commands")
    if not os.path.isfile(path): return None
    listed = {}
    for no, raw in enumerate(open(path), 1):
        parts = raw.split("#", 1)[0].split()
        if not parts: continue
        if len(parts) != 2: sys.exit(f"{path}:{no}: want `command  where-it-comes-from`, got: {raw.strip()}")
        if parts[0] in listed: sys.exit(f"{path}:{no}: {parts[0]} is listed twice")
        listed[parts[0]] = parts[1]
    return listed


def busybox_options(product_dir):
    """The options a whole BusyBox config turns on, or None when the product has no whole config in this tree."""
    path = os.path.join(product_dir, "os", "busybox.config")
    if not os.path.isfile(path): return None
    return {m.group(1) for m in re.finditer(r"^(CONFIG_[A-Z0-9_]+)=y$", open(path).read(), re.M)}


def rel(path):
    return os.path.relpath(path, REPO)


def used(product_dir, errors):
    """{command: 'script:line'} and {absolute: 'script:line'} over the product's scripts."""
    names, absolutes = {}, {}
    for path, to, interp in shipped(product_dir):
        if interp not in SHELLS:
            errors.append(f"{rel(path)}: runs under {interp or 'no interpreter'}; this check reads {' and '.join(SHELLS)} scripts only")
            continue
        try:
            n, a, refused = calls(open(path, encoding="utf-8").read())
        except Unread as e:
            errors.append(f"{rel(path)}: not read: {e}"); continue
        for at, word, why in refused: errors.append(f"{rel(path)}:{at}: `{word}`: {why}")
        for w, at in n.items(): names.setdefault(w, f"{rel(path)}:{at}")
        for w, at in a.items(): absolutes.setdefault(w, f"{rel(path)}:{at}")
    return names, absolutes


def gate():
    errors, count = [], 0
    for pname in sorted(os.listdir(PRODUCTS)):
        pdir = os.path.join(PRODUCTS, pname)
        if not os.path.isfile(os.path.join(pdir, "product.toml")): continue
        names, _ = used(pdir, errors)
        listed = read_list(pdir)
        if listed is None:
            if names: errors.append(f"products/{pname}: ships shell scripts and has no script-commands list (first: {sorted(names.items())[0][1]})")
            continue
        options = busybox_options(pdir)
        for w, where in sorted(names.items()):
            if w not in listed: errors.append(f"{where}: `{w}` is not on products/{pname}/script-commands: the image is not known to have it")
        for w, source in sorted(listed.items()):
            if w not in names:
                errors.append(f"products/{pname}/script-commands: `{w}` is listed and no shipped script calls it"); continue
            if source == "ash" and w not in ASH_OPTIONS:
                errors.append(f"products/{pname}/script-commands: `{w}` is not an ash builtin this check knows")
            if options is None: continue
            if source == "ash":
                opt = ASH_OPTIONS.get(w)
                if opt and opt not in options: errors.append(f"products/{pname}/script-commands: `{w}` is an ash builtin only with {opt}=y, and os/busybox.config does not set it")
            elif source == "busybox":
                opt = "CONFIG_TEST" if w == "[" else "CONFIG_" + re.sub(r"[^A-Z0-9]", "_", w.upper())
                if opt not in options: errors.append(f"products/{pname}/script-commands: `{w}` needs {opt}=y, and os/busybox.config does not set it ({names[w]} calls it)")
        count += len(shipped(pdir))
    for e in errors: print(f"check-script-commands: {e}", file=sys.stderr)
    if errors: sys.exit(1)
    print(f"check-script-commands: {count} shipped script(s) call only what their product lists")


def resolve(root, relpath):
    """The path inside root that relpath names, every link followed inside the root; None if it is not there."""
    todo, cur, hops = [p for p in relpath.strip("/").split("/") if p], [], 0
    while todo:
        p = todo.pop(0)
        if p == ".": continue
        if p == "..": cur = cur[:-1]; continue
        full = os.path.join(root, *cur, p)
        if os.path.islink(full):
            hops += 1
            if hops > 40: return None
            t = os.readlink(full)
            if t.startswith("/"): cur = []
            todo = [q for q in t.split("/") if q] + todo
        elif os.path.lexists(full): cur.append(p)
        else: return None
    return os.path.join(root, *cur)


def in_root(root, pname, scripts_too):
    pdir = os.path.join(PRODUCTS, pname)
    errors = []
    names, absolutes = used(pdir, errors)
    listed = read_list(pdir) or {}
    for w, source in sorted(listed.items()):
        if source == "ash": continue
        hits = [r for r in (resolve(root, f"{d}/{w}") for d in PATH_DIRS) if r and os.path.isfile(r)]
        if not hits:
            errors.append(f"`{w}` ({source}) is on products/{pname}/script-commands and in none of {', '.join('/' + d for d in PATH_DIRS)} of the root ({names.get(w, 'no caller')})")
            continue
        is_busybox = os.path.basename(hits[0]) == "busybox"
        if (source == "busybox") != is_busybox:
            errors.append(f"`{w}` is listed as {source}; in the root it is {'a BusyBox applet' if is_busybox else 'its own program, ' + os.path.relpath(hits[0], root)}")
    for w, where in sorted(absolutes.items()):
        r = resolve(root, w)
        if not (r and os.path.isfile(r)): errors.append(f"{where}: runs {w}, which is not in the root")
    for path, to, interp in shipped(pdir) if scripts_too else []:
        r = resolve(root, to)
        if not (r and os.path.isfile(r)):
            errors.append(f"{rel(path)}: not in the root at {to}")
        elif open(r, "rb").read() != open(path, "rb").read():
            errors.append(f"{rel(path)}: the root's {to} does not have its bytes")
        i = resolve(root, interp) if interp in SHELLS else None
        if interp in SHELLS and not (i and os.path.isfile(i)): errors.append(f"{rel(path)}: its interpreter {interp} is not in the root")
    for e in errors: print(f"check-script-commands: {e}", file=sys.stderr)
    if errors: sys.exit(1)
    what = f"and {len(shipped(pdir))} script(s) are in the root" if scripts_too else "are in the OS tree (the scripts themselves are placed after it and are not read here)"
    print(f"check-script-commands: products/{pname}: {len(listed)} listed command(s) {what} {root}")


def self_test():
    def names(text): return set(calls(text)[0])
    cases = [
        ("the old board-journal line",
         'install -d -m 2755 "$D" || { say "x"; exit 1; }\nchown root:g "$D" && chmod 2755 "$D" || { say "no"; exit 1; }\n',
         {"install", "say", "chown", "chmod"}),
        ("a pipeline in a substitution inside quotes", 'x="$(printf \'%s\' "$s" | tr A-F a-f | cut -c1-8)"\n', {"printf", "tr", "cut"}),
        ("a function is not a command", 'fail() {\n\techo "$*" >&2\n\trm -f "$D/x"\n}\n[ -r "$C" ] || fail "no"\n', {"echo", "rm", "["}),
        ("case patterns are not commands", 'case "$1" in\n\tstart) start ;;\n\tstop | halt) stop; sleep 1 ;;\n\t*) echo no >&2; exit 1 ;;\nesac\n',
         {"start", "stop", "sleep", "echo"}),
        ("a for loop's words are not commands", 'for dev in /sys/class/net/* mount; do\n\tcat "$dev/type"\ndone\n', {"cat"}),
        ("a here-document's body is not read", 'cat > "$t" <<NET || fail "x"\n[Match]\nchown=1\nNET\nmv "$t" "$o"\n', {"cat", "fail", "mv"}),
        ("comments and quoted text are not read", '# chown here\necho "chmod 0644; touch x" # stat\n', {"echo"}),
        ("arithmetic is not read", 'n=$((last - 1))\nx=$(int2ip $((a + 1)))\n', {"int2ip"}),
        ("exec's command", 'exec 0</dev/console 1>/dev/console 2>&1\nexec reboot -f\n', {"reboot"}),
        ("a background job and a subshell", 'supervise "$a" >> "$LOG" 2>&1 < /dev/null &\n(cd /x && ls) | wc -l\n', {"supervise", "ls", "wc"}),
        ("backticks", 'if [ "`id -u`" -eq 0 ]; then\n\texport PS1=Q\nfi\n', {"id", "["}),
        ("a parameter with quotes inside", 's8=${s8#"${s8%????????}"}\nmkdir -p "$D"\n', {"mkdir"}),
        ("an assignment before a command", 'LEMNOS0_OUT=$NET "$ADDRESS" >> "$LOG" 2>&1 && [ -r "$R/x" ]\nIFS=. read -r a b\n', {"["}),
    ]
    bad = 0
    for what, text, want in cases:
        try: got = names(text)
        except Unread as e: got = {f"unread: {e}"}
        if got != want:
            bad += 1
            print(f"check-script-commands: self-test: {what}: read {sorted(got)}, want {sorted(want)}", file=sys.stderr)
    a = calls('exec ${T:-/usr/bin/systemctl} reboot\n[ -x /sbin/init ] && exec /sbin/init "$@"\n')[1]
    if set(a) != {"/usr/bin/systemctl", "/sbin/init"}:
        bad += 1; print(f"check-script-commands: self-test: absolute programs: read {sorted(a)}", file=sys.stderr)
    r = calls('find / -exec chown x {} +\neval "$x"\n')[2]
    if {w for _, w, _ in r} != {"find", "eval"}:
        bad += 1; print(f"check-script-commands: self-test: find -exec and eval are not both refused: {r}", file=sys.stderr)
    try:
        calls('echo "no end\n'); bad += 1
        print("check-script-commands: self-test: an unclosed quote was read", file=sys.stderr)
    except Unread:
        pass
    if bad: sys.exit(1)
    print(f"check-script-commands: self-test: {len(cases) + 3} cases read as written")


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args: gate()
    elif args == ["--self-test"]: self_test()
    elif len(args) == 3 and args[0] in ("--root", "--os-root"): in_root(args[1], args[2], args[0] == "--root")
    elif len(args) == 2 and args[0] == "--list":
        errs = []
        n, a = used(os.path.join(PRODUCTS, args[1]), errs)
        for w, where in sorted(n.items()): print(f"{w}\t{where}")
        for w, where in sorted(a.items()): print(f"{w}\t{where}")
        for e in errs: print(f"! {e}", file=sys.stderr)
    else: sys.exit(__doc__)
