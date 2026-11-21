#!/usr/bin/env python3
"""con.py for a real board: one shell command on a serial tty (the gadget's ttyGS0 on the PC's ttyACM by-id path),
logging in as root when a login prompt shows (the image's console rule: root, no password).
  ttycon.py DEV TIMEOUT CMD   prints CMD's output, exits with its status; 124 timeout, 125 no shell, 126 no device
Typed text goes in 48-byte pieces with a pause (long typed lines were cut on the emulated console)."""
import os, re, sys, termios, time, tty
dev, timeout, cmd = sys.argv[1], float(sys.argv[2]), sys.argv[3]
deadline = time.monotonic() + timeout
while not os.path.exists(dev):
    if time.monotonic() > deadline: sys.exit(126)
    time.sleep(1)
fd = os.open(dev, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
tty.setraw(fd); a = termios.tcgetattr(fd); a[2] &= ~termios.CRTSCTS; a[2] |= termios.CLOCAL; termios.tcsetattr(fd, termios.TCSANOW, a)
nonce = os.urandom(6).hex(); buf = b""
def read_some():
    global buf
    try: buf += os.read(fd, 65536)
    except BlockingIOError: time.sleep(0.05)
def send(text):
    b = text.encode()
    for i in range(0, len(b), 48):
        os.write(fd, b[i:i+48]); time.sleep(0.03)
def wait_for(p, until):
    while time.monotonic() < until:
        m = re.search(p, buf)
        if m: return m
        read_some()
    return None
logged = False
while time.monotonic() < deadline:
    buf = b""
    send(f"\nprintf '__P%s__\\n' {nonce}\n")
    m = wait_for(rb"(__P" + nonce.encode() + rb"__)|(login: ?$)|(login:)", min(deadline, time.monotonic() + 8))
    if m and m.group(1): logged = True; break
    if m: send("root\n"); time.sleep(2)
if not logged: sys.exit(125)
if cmd == "--login": sys.exit(0)
buf = b""
start, end = f"__S{nonce}__", f"__E{nonce}_"
send(f"printf '__S%s__\\n' {nonce}; {cmd}; printf '__E%s_%s__\\n' {nonce} $?\n")
m = wait_for(re.escape(end.encode()) + rb"(\d+)__", deadline)
if not m: sys.exit(124)
text = buf.decode(errors="replace")
body = text[text.find(start) + len(start):text.find(end)] if start in text else ""
sys.stdout.write(body.lstrip("\r\n").replace("\r\n", "\n")); sys.exit(int(m.group(1)))
