#!/usr/bin/env python3
"""The key of a built test program: what it loads, not every byte of its file.

A change that leaves a program's loaded code and data as they were (an unused function, a comment at the end of a
file) still rewrites its file: the build-id and the debug info move. Keyed on the whole file, every test of the
program ran again. Keyed here on its allocated sections (SHF_ALLOC: name, address, flags and bytes; a NOBITS section
by its size; the build-id note left out), it runs again only when what it loads moved (2026-11-27 probe: an unused
pub fn in atlas-topics moved no test program; a comment that shifts lines moves the panic locations' line numbers in
.data.rel.ro, so it runs again, as it must: a panic's text is part of what a test checks).

A file that is not a little-endian ELF64, or that is cut short or malformed anywhere this reads, is keyed on its whole
bytes instead: it runs again whenever any byte moves, never less. A file that cannot be read fails the key, and the
caller runs everything.

  bin-key.py alloc FILE                         the key of one file (alloc:<sha256> or whole:<sha256>)
  bin-key.py key LABEL DATA_KEY [--dir D]... -- FILE...
                                                one key over a label, the data key, every file under each D (bytes),
                                                and each FILE's alloc key (missing FILE: fails)
  bin-key.py nextest LIST_JSON DATA_KEY         one line per test program of a `cargo nextest list --list-type
                                                binaries-only --message-format json`: "<key> <binary-id>"
  bin-key.py --self-test
"""
import hashlib
import json
import os
import struct
import sys

SHF_ALLOC = 0x2
SHT_NOBITS = 8
LEFT_OUT = {".note.gnu.build-id"}


class Malformed(Exception):
    pass


def _unpack(fmt, data, off):
    if off < 0 or off + struct.calcsize(fmt) > len(data):
        raise Malformed(f"{fmt} at {off} is past the end ({len(data)} bytes)")
    return struct.unpack_from(fmt, data, off)


def alloc_digest(data):
    """The allocated sections' digest, or raises Malformed."""
    if len(data) < 64 or data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        raise Malformed("not a little-endian ELF64")
    (shoff,) = _unpack("<Q", data, 0x28)
    shentsize, shnum, shstrndx = _unpack("<HHH", data, 0x3A)
    if shentsize != 64 or shnum == 0 or shstrndx >= shnum:
        raise Malformed("no usable section header table")
    sections = [_unpack("<IIQQQQIIQQ", data, shoff + i * shentsize) for i in range(shnum)]
    str_off, str_size = sections[shstrndx][4], sections[shstrndx][5]
    if str_off + str_size > len(data):
        raise Malformed("the section names are past the end")

    def name(off):
        start = str_off + off
        end = data.find(b"\0", start, str_off + str_size)
        if off >= str_size or end < 0:
            raise Malformed("a section name is past its table")
        return data[start:end].decode("utf-8", "replace")

    digest = hashlib.sha256()
    count = 0
    for sh_name, sh_type, flags, addr, off, size, *_rest in sections:
        if not flags & SHF_ALLOC:
            continue
        section = name(sh_name)
        if section in LEFT_OUT:
            continue
        if sh_type == SHT_NOBITS:
            body = b"nobits %d" % size
        else:
            if off + size > len(data):
                raise Malformed(f"{section} is past the end")
            body = data[off:off + size]
        head = f"{section}\0{addr:x}\0{flags:x}\0{sh_type}\0{len(body)}\0".encode()
        digest.update(head)
        digest.update(body)
        count += 1
    if count == 0:
        raise Malformed("no allocated section")
    return digest.hexdigest()


def file_key(data):
    try:
        return "alloc:" + alloc_digest(data)
    except Malformed:
        return "whole:" + hashlib.sha256(data).hexdigest()


def read(path):
    with open(path, "rb") as f:
        return f.read()


def dir_lines(root):
    """Every file under root, by its path below root and its bytes' hash (a missing directory: none)."""
    lines = []
    for here, dirs, files in os.walk(root):
        dirs.sort()
        for f in sorted(files):
            path = os.path.join(here, f)
            lines.append(f"{os.path.relpath(path, root)} {hashlib.sha256(read(path)).hexdigest()}")
    return lines


def combined(label, data_key, dirs, files):
    digest = hashlib.sha256()
    digest.update(f"bin-key v1\0{label}\0{data_key}\0".encode())
    for d in dirs:
        digest.update(f"dir {os.path.basename(d.rstrip('/'))}\0".encode())
        for line in dir_lines(d):
            digest.update(line.encode() + b"\0")
    for f in files:
        digest.update(f"file {os.path.basename(f)} {file_key(read(f))}\0".encode())
    return digest.hexdigest()[:32]


def nextest(list_json, data_key):
    """A key per test program: its own loaded sections, those of its package's programs (a test can run them,
    CARGO_BIN_EXE_*), every build script's output (small; a test may read one at run time), and the data key."""
    with open(list_json) as f:
        listing = json.load(f)
    meta = listing["rust-build-meta"]
    target = meta["target-directory"]
    out_dirs = [os.path.join(target, d) for _, d in sorted(meta.get("build-script-out-dirs", {}).items())]
    programs = meta.get("non-test-binaries", {})
    for binary_id, binary in sorted(listing["rust-binaries"].items()):
        package_bins = [os.path.join(target, b["path"]) for b in programs.get(binary["package-id"], [])]
        files = [binary["binary-path"]] + sorted(p for p in package_bins if os.path.exists(p))
        print(combined(f"nextest {binary_id}", data_key, out_dirs, files), binary_id)


# --- self-test: small ELF files made here, each checked against the rules above --------------------------------------
def make_elf(sections):
    """sections: [(name, type, flags, addr, bytes)]. Section 0 is the null section; the names table is added last."""
    names = b"\0"
    offsets = {}
    for name, *_ in sections + [(".shstrtab",)]:
        offsets[name] = len(names)
        names += name.encode() + b"\0"
    body = b""
    placed = []
    base = 64
    for name, typ, flags, addr, data in sections:
        placed.append((name, typ, flags, addr, base + len(body), len(data)))
        if typ != SHT_NOBITS:
            body += data
    names_off = base + len(body)
    body += names
    shoff = base + len(body)
    headers = struct.pack("<IIQQQQIIQQ", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
    for name, typ, flags, addr, off, size in placed:
        headers += struct.pack("<IIQQQQIIQQ", offsets[name], typ, flags, addr, off, size, 0, 0, 1, 0)
    headers += struct.pack("<IIQQQQIIQQ", offsets[".shstrtab"], 3, 0, 0, names_off, len(names), 0, 0, 1, 0)
    shnum = len(placed) + 2
    ident = b"\x7fELF" + bytes([2, 1, 1]) + bytes(9)
    header = ident + struct.pack("<HHIQQQIHHHHHH", 3, 62, 1, 0, 0, shoff, 0, 64, 56, 0, 64, shnum, shnum - 1)
    return header + body + headers


def self_test():
    text = (".text", 1, SHF_ALLOC | 0x4, 0x1000, b"\x90" * 32)
    rodata = (".rodata", 1, SHF_ALLOC, 0x2000, b"limit 45\0line 172\0")
    bss = (".bss", SHT_NOBITS, SHF_ALLOC | 0x1, 0x3000, b"\0" * 64)
    build_id = (".note.gnu.build-id", 7, SHF_ALLOC, 0x400, b"\x04\0\0\0\x14\0\0\0\x03\0\0\0GNU\0" + bytes(range(20)))
    comment = (".comment", 1, 0, 0, b"rustc 1.x\0")
    base = make_elf([build_id, text, rodata, bss])
    key = file_key(base)
    failures = []

    def expect(what, ok):
        if not ok:
            failures.append(what)

    expect("a made ELF is keyed on its allocated sections", key.startswith("alloc:"))
    expect("a non-alloc section added keeps the key", file_key(make_elf([build_id, text, rodata, bss, comment])) == key)
    other_id = (build_id[0], build_id[1], build_id[2], build_id[3], build_id[4][:16] + bytes(20))
    expect("a new build-id keeps the key", file_key(make_elf([other_id, text, rodata, bss])) == key)
    as_alloc = (comment[0], comment[1], SHF_ALLOC, 0x4000, comment[4])
    expect("the same sections with one more of them allocated moves the key",
           file_key(make_elf([build_id, text, rodata, bss, as_alloc])) != file_key(make_elf([build_id, text, rodata, bss, comment])))
    writable = (rodata[0], rodata[1], rodata[2] | 0x1, rodata[3], rodata[4])
    expect("an allocated section's flags move the key", file_key(make_elf([build_id, text, writable, bss])) != key)
    line = (rodata[0], rodata[1], rodata[2], rodata[3], b"limit 45\0line 173\0")
    expect("one .rodata byte moves the key", file_key(make_elf([build_id, text, line, bss])) != key)
    moved = (text[0], text[1], text[2], 0x1010, text[4])
    expect("a section's address moves the key", file_key(make_elf([build_id, moved, rodata, bss])) != key)
    bigger = (bss[0], bss[1], bss[2], bss[3], b"\0" * 65)
    expect("a NOBITS section's size moves the key", file_key(make_elf([build_id, text, rodata, bigger])) != key)
    for cut in (10, 63, 64 + 40, len(base) - 1):
        k = file_key(base[:cut])
        expect(f"a file cut to {cut} bytes is keyed on its whole bytes", k.startswith("whole:"))
    broken = bytearray(base)
    struct.pack_into("<Q", broken, 0x28, len(base) + 4096)  # the section headers past the end
    expect("a section table past the end is keyed on its whole bytes", file_key(bytes(broken)).startswith("whole:"))
    broken = bytearray(base)
    struct.pack_into("<H", broken, 0x3E, 200)  # the names' index past the table
    expect("a names index past the table is keyed on its whole bytes", file_key(bytes(broken)).startswith("whole:"))
    # The section headers start after the data: the third header (null, build-id, .text) is .rodata's.
    (shoff,) = struct.unpack_from("<Q", base, 0x28)
    broken = bytearray(base)
    struct.pack_into("<Q", broken, shoff + 3 * 64 + 32, len(base) * 2)  # .rodata's size past the end
    expect("a section's bytes past the end are keyed on its whole bytes", file_key(bytes(broken)).startswith("whole:"))
    expect("a file that is no ELF is keyed on its whole bytes", file_key(b"#!/bin/sh\necho hi\n").startswith("whole:"))
    # A malformed file never shares a key with another: its whole bytes decide.
    expect("two malformed files with one byte apart differ", file_key(base[:100]) != file_key(base[:99] + b"\x01"))
    if failures:
        for f in failures:
            print(f"bin-key self-test: FAILED: {f}", file=sys.stderr)
        return 1
    print("bin-key self-test: passed")
    return 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    if len(argv) == 2 and argv[0] == "alloc":
        print(file_key(read(argv[1])))
        return 0
    if len(argv) >= 4 and argv[0] == "key" and "--" in argv:
        split = argv.index("--")
        label, data_key, opts, files = argv[1], argv[2], argv[3:split], argv[split + 1:]
        dirs = [opts[i + 1] for i in range(0, len(opts) - 1, 2) if opts[i] == "--dir"]
        if not files:
            print("bin-key: no file to key", file=sys.stderr)
            return 2
        print(combined(label, data_key, dirs, files))
        return 0
    if len(argv) == 3 and argv[0] == "nextest":
        nextest(argv[1], argv[2])
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
