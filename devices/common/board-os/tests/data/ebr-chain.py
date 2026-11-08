# A disk image for tests/data-setup.sh: MBR with p1-p3 primary and p4
# extended, whose EBR chain sits in consecutive sectors at p4's start (so an
# image can end after p5's content), the image cut after p5, and stale EBRs
# at the conventional spots (1 MiB before p6 and p7) as an old flash leaves
# them. Usage: python3 ebr-chain.py <disk> cut
import struct, sys
SECT = 512
E = 83968                      # extended partition start
LOG = [(86016, 32768), (120832, 32768), (155648, 16384)]   # p5, p6, p7
DISK = 128 * 1024 * 1024 // SECT
def entry(boot, ptype, start, size):
    return struct.pack("<B3sB3sII", boot, b"\xfe\xff\xff", ptype, b"\xfe\xff\xff", start, size)
def sector(entries):
    body = b"".join(entries) + b"\0" * 16 * (4 - len(entries))
    return b"\0" * 446 + body + b"\x55\xaa"
img = bytearray(DISK * SECT)
mbr = [entry(0, 0x0c, 2048, 16384), entry(0, 0x0c, 18432, 32768), entry(0, 0x0c, 51200, 32768),
       entry(0, 0x05, E, LOG[-1][0] + LOG[-1][1] - E)]
img[0:SECT] = sector(mbr)
# The EBR chain in consecutive sectors at the extended partition's start.
for k, (start, size) in enumerate(LOG):
    here = E + k
    ents = [entry(0, 0x83, start - here, size)]
    if k + 1 < len(LOG):
        nxt = E + k + 1
        nstart, nsize = LOG[k + 1]
        ents.append(entry(0, 0x05, nxt - E, nstart + nsize - nxt))
    img[here * SECT:(here + 1) * SECT] = sector(ents)
mode = sys.argv[2]
if mode == "cut":
    # The image ends after p5; the rest is what an old flash left: stale EBRs
    # at the conventional spots (1 MiB before p6 and p7), describing other sizes.
    end = (LOG[0][0] + LOG[0][1]) * SECT
    img[end:] = b"\xa5" * (len(img) - end)
    for spot in (118784, 153600):
        img[spot * SECT:(spot + 1) * SECT] = sector([entry(0, 0x83, 2048, 999), entry(0, 0x05, 50000, 9999)])
open(sys.argv[1], "wb").write(img)
