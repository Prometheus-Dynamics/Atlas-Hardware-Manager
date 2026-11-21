#!/usr/bin/env python3
"""Runs one shell command on the emulated board's serial console (the harness's <state>/console.sock), logging in as
root first when the console shows a login prompt (the Raze image: root, no password, on its console).

  con.py SOCK TIMEOUT CMD      prints CMD's output, exits with CMD's status; 124 on timeout, 125 when no shell came
  con.py SOCK TIMEOUT --login  only logs in (exit 0 once a root shell answers)

The markers are made by printf at run time, so the console's echo of the typed line never matches them.
"""
import os, re, socket, sys, time

sock_path, timeout, cmd = sys.argv[1], float(sys.argv[2]), sys.argv[3]
deadline = time.monotonic() + timeout
nonce = os.urandom(6).hex()
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
s.settimeout(1.0)
buf = b""


def read_some():
    global buf
    try:
        d = s.recv(65536)
        if not d:
            sys.exit(125)
        buf += d
    except socket.timeout:
        pass


def send(text):
    s.sendall(text.encode())


def wait_for(pattern, until):
    while time.monotonic() < until:
        m = re.search(pattern, buf)
        if m:
            return m
        read_some()
    return None


# A shell: ask with a probe; at a login prompt, log in as root.
logged = False
while time.monotonic() < deadline:
    buf = b""
    send(f"\nprintf '__P%s__\\n' {nonce}\n")
    m = wait_for(rb"(__P" + nonce.encode() + rb"__)|(login: ?$)|(login:)", min(deadline, time.monotonic() + 8))
    if m and m.group(1):
        logged = True
        break
    if m:
        send("root\n")
        time.sleep(2)
        continue
if not logged:
    sys.exit(125)
if cmd == "--login":
    sys.exit(0)

buf = b""
start, end = f"__S{nonce}__", f"__E{nonce}_"
send(f"printf '__S%s__\\n' {nonce}; {cmd}; printf '__E%s_%s__\\n' {nonce} $?\n")
m = wait_for(re.escape(end.encode()) + rb"(\d+)__", deadline)
if not m:
    sys.exit(124)
text = buf.decode(errors="replace")
body = text[text.find(start) + len(start):text.find(end)] if start in text else ""
sys.stdout.write(body.lstrip("\r\n").replace("\r\n", "\n"))
sys.exit(int(m.group(1)))
